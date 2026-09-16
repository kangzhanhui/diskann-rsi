/*
 * Copyright (c) Microsoft Corporation.
 * Licensed under the MIT license.
 */

#![warn(missing_debug_implementations, missing_docs)]

//! Scratch space for in-memory index based search

use std::{collections::VecDeque, marker::PhantomData};

use crate::{
    graph::glue::VisitedTracker,
    neighbor::{Neighbor, NeighborPriorityQueue},
    utils::{IntoUsize, VectorId},
};
use diskann_utils::object_pool::AsPooled;

/// In-mem index related limits
pub const GRAPH_SLACK_FACTOR: f64 = 1.3_f64;

/// Scratch space used during graph search.
///
/// This struct contains three important members used by both the sync and async indexes:
/// `query`, `best`, and `visited`.
///
/// The member `id_scratch` is only used by the sync index.
///
/// Members `labels` and `beta` are used by the async index for beta-filtered search.
#[derive(Debug)]
pub struct SearchScratch<I, Q = NeighborPriorityQueue<I>>
where
    I: VectorId,
{
    /// A priority queue of the best candidates seen during search. This data structure is
    /// also responsible for determining the best unvisited candidate.
    ///
    /// Used by both sync and async.
    ///
    /// When used in a paged search context, this queue is unbounded.
    pub best: Q,

    /// A record of all ids visited during a search.
    ///
    /// Used by both sync and async.
    ///
    /// This is used to prevent multiple requests to the same `id` from the vector providers.
    pub visited: StampedVisitedSet<I>,

    /// A buffer for adjacency lists.
    ///
    /// Only used by sync.
    ///
    /// Adjacency lists in the sync provider are guarded by read/write locks. The
    /// `id_scratch` is used to copy out the contents of an adjacency list to minimize the
    /// duration the lock is held.
    pub id_scratch: Vec<I>,

    /// A list of beam search nodes used during search. This is used when beam search is enabled
    /// to temporarily hold beam of nodes in each hop.
    pub beam_nodes: Vec<I>,

    /// A queue of nodes to visit during range search
    /// Does not need to be ordered by distance
    pub range_frontier: VecDeque<I>,

    /// A list of nodes that are in range of the query
    /// Only used during range search
    pub in_range: Vec<Neighbor<I>>,

    /// A tracker for how many hops we have taken during the current search
    pub hops: u32,

    /// A tracker for how many comparisons we have made during the current search
    pub cmps: u32,
}

/// The priority queue in `SearchScratch` operates in two modes:
///
/// * Fixed: The queue has a fixed capacity and discards the worst members. This is used by
///   the standard nearest-neighbor search methods.
///
/// * Resizable: Allows the priority queue to be unbounded in size. This is used by paged
///   search to allow multiple rounds of searching.
#[derive(Debug, Clone, Copy)]
pub enum PriorityQueueConfiguration {
    /// Configure the priority queue in fixed-L mode with the provided `search_l`.
    Fixed(usize),
    /// Configure the priority queue in resizeable mode with the provided `search_l`.
    Resizable(usize),
}

/// A visited-id set backed by a dense, epoch-stamped array.
///
/// This is a drop-in replacement for the `HashSet` previously used to deduplicate graph
/// traversal. `insert`/`contains` become unchecked array accesses instead of hashing,
/// which matters because every raw adjacency entry scanned during search passes through
/// this set (tens of millions of probes per query sweep on large graphs).
///
/// Each slot holds the 16-bit epoch in which the id was last inserted (2 bytes per id,
/// keeping the array cache-resident at million-point scale); `clear` simply bumps the
/// epoch, so resetting the set between queries is O(1) regardless of how many ids were
/// visited. The backing array grows on demand (doubling) up to the largest id seen, and
/// is reused across queries via the scratch pool.
///
/// Ids at or beyond [`MAX_DENSE_SLOTS`] (sentinel ids such as `u32::MAX`, which test
/// providers use for start points) are tracked in a small overflow hash set instead of
/// growing the dense array to absurd sizes.
#[derive(Debug)]
pub struct StampedVisitedSet<I> {
    /// One slot per id; a slot equal to `epoch` means the id is in the set.
    stamps: Vec<u16>,

    /// The current epoch. Wraps to 1 (with a full zeroing pass) on u16 overflow.
    epoch: u16,

    /// The number of ids inserted in the current epoch.
    len: usize,

    /// Sentinel/sparse ids that must not grow the dense array.
    overflow: hashbrown::HashSet<I>,

    _marker: PhantomData<I>,
}

/// Ids beyond this many slots are treated as sparse outliers and tracked in the overflow
/// set instead of growing the dense array. This covers every realistic id space (DiskANN
/// targets at most ~1B points) while keeping sentinel ids like `u32::MAX` from forcing a
/// 16 GiB allocation.
const MAX_DENSE_SLOTS: usize = 1 << 30;

impl<I> Default for StampedVisitedSet<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> StampedVisitedSet<I> {
    /// Create an empty set with no preallocated storage.
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Create an empty set with the stamp array presized to `capacity` slots.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            stamps: vec![0; capacity.min(MAX_DENSE_SLOTS)],
            epoch: 1,
            len: 0,
            overflow: hashbrown::HashSet::new(),
            _marker: PhantomData,
        }
    }

    /// Return whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reset the set to empty in O(1) by advancing the epoch.
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflow.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Wrapped through all 2^32 epochs: every stamp is potentially live, so
            // zero the array and restart the epoch counter.
            self.stamps.fill(0);
            self.epoch = 1;
        }
    }
}

impl<I> StampedVisitedSet<I>
where
    I: IntoUsize + Copy + Eq + std::hash::Hash,
{
    /// Insert `id` into the set, returning `true` if it was not already present.
    pub fn insert(&mut self, id: I) -> bool {
        let idx = id.into_usize();
        if idx >= self.stamps.len() {
            if idx >= MAX_DENSE_SLOTS {
                return self.overflow.insert(id);
            }
            let new_len = (idx + 1).max(self.stamps.len() * 2).max(64);
            self.stamps.resize(new_len, 0);
        }

        if self.stamps[idx] == self.epoch {
            return false;
        }

        self.stamps[idx] = self.epoch;
        self.len += 1;
        true
    }

    /// Return whether `id` is in the set.
    pub fn contains(&self, id: &I) -> bool {
        let idx = (*id).into_usize();
        if idx < self.stamps.len() {
            self.stamps[idx] == self.epoch
        } else {
            self.overflow.contains(id)
        }
    }
}

impl<I> Extend<I> for StampedVisitedSet<I>
where
    I: IntoUsize + Copy + Eq + std::hash::Hash,
{
    fn extend<T: IntoIterator<Item = I>>(&mut self, iter: T) {
        for id in iter {
            self.insert(id);
        }
    }
}

impl<I> VisitedTracker<I> for StampedVisitedSet<I>
where
    I: IntoUsize + Copy + Eq + std::hash::Hash,
{
    fn mark_visited(&mut self, item: &I) -> bool {
        self.insert(*item)
    }

    fn is_visited(&self, item: &I) -> bool {
        self.contains(item)
    }
}

impl<I> SearchScratch<I, NeighborPriorityQueue<I>>
where
    I: VectorId,
{
    /// Create a new `SearchScratch` with an uninitializd `dim`-dimensional query.
    ///
    /// This method is used when pre-allocating many scratch spaces and should not be used
    /// for general searching (use [`Self::new`] instead.
    ///
    /// # Parameters
    ///
    /// * `dim`: The number of dimensions to allocate for the internal query vector.
    /// * `nbest`: The number of best candidates to track.
    /// * `size_hint`: If provided, hints at the capacity for preallocating the `visited` set.
    ///
    /// # Note
    ///
    /// This method does not enable the beta-filtering feature.
    pub fn new(nbest: PriorityQueueConfiguration, size_hint: Option<usize>) -> Self {
        let visited = match size_hint {
            Some(size_hint) => StampedVisitedSet::with_capacity(size_hint),
            None => StampedVisitedSet::new(),
        };

        let best = match nbest {
            PriorityQueueConfiguration::Fixed(capacity) => NeighborPriorityQueue::new(capacity),
            PriorityQueueConfiguration::Resizable(capacity) => {
                NeighborPriorityQueue::auto_resizable_with_search_param_l(capacity)
            }
        };

        Self {
            best,
            visited,
            id_scratch: Vec::new(),
            beam_nodes: Vec::new(),
            in_range: Vec::new(),
            range_frontier: VecDeque::new(),
            hops: 0,
            cmps: 0,
        }
    }

    /// Reconfigures the queue lengths of `self` for a longer or shorter search.
    ///
    /// # Parameters
    ///
    /// * `nbest`: The new number of candidates to track.
    pub fn resize(&mut self, nbest: usize) {
        self.best.reconfigure(nbest);
    }

    /// Clear internal data structures.
    ///
    /// This allows `self` to be used for another search.
    pub fn clear(&mut self) {
        self.best.clear();
        self.visited.clear();
        self.id_scratch.clear();
        self.beam_nodes.clear();
        self.in_range.clear();
        self.range_frontier.clear();

        self.hops = 0;
        self.cmps = 0;
    }

    /// Return the currently configured `search_l`: the number of best candidates to track.
    #[cfg(test)]
    pub fn search_l(&self) -> usize {
        self.best.search_l()
    }
}

/// Estimate the size of the node visited set. This needs to be upper bound of the number of
/// nodes visited during search. The formula is based on initial theoretical analysis and
/// adjusted with MARGIN_FACTOR obtained from empirical data.
///
/// The formula is:
/// ```math
/// MARGIN_FACTOR * max_degree * GRAPH_SLACK_FACTOR * search_list_size
/// ```
///
/// ## Explanation
///
/// * `visited_nodes <= number_of_hops * max_degree`.
/// * `number_of_hops` is generally `MARGIN_FACTOR * L`.
/// * `MAX_DEGREE` is adjusted with `GRAPH_SLACK_FACTOR`.
///
/// In future, this could be more precisely computed by either of two approaches:
///
/// 1. Analyzing data in index build time based on theory
/// 2. At run time by analyzing about 1000 queries and taking maximum size
pub(crate) fn estimate_node_visited_set_size(max_degree: usize, search_list_size: usize) -> usize {
    // The `MARGIN_FACTOR` is obtained by running search queries on the Sift 1M and
    // OpenAI datasets, checking maximum visited nodes, and comparing it with the formula.
    const MARGIN_FACTOR: f64 = 1.1;

    (MARGIN_FACTOR * max_degree as f64 * GRAPH_SLACK_FACTOR * search_list_size as f64).ceil()
        as usize
}

impl<I> AsPooled<&SearchScratchParams> for SearchScratch<I, NeighborPriorityQueue<I>>
where
    I: VectorId,
{
    fn create(param: &SearchScratchParams) -> Self {
        SearchScratch::new(
            PriorityQueueConfiguration::Fixed(param.l_value + param.num_frozen_pts),
            Some(estimate_node_visited_set_size(
                param.max_degree,
                param.l_value,
            )),
        )
    }

    fn modify(&mut self, param: &SearchScratchParams) {
        self.clear();
        if self.best.is_resizable() {
            // Scratch is used only in Fixed configuration. If it is resizable, then convert to fixed.
            self.best = NeighborPriorityQueue::new(param.l_value + param.num_frozen_pts);
        } else {
            self.best.reconfigure(param.l_value + param.num_frozen_pts);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchScratchParams {
    pub l_value: usize,
    pub max_degree: usize,
    pub num_frozen_pts: usize,
}

///////////
// Tests //
///////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_new() {
        {
            let x = SearchScratch::<u64, NeighborPriorityQueue<u64>>::new(
                PriorityQueueConfiguration::Fixed(10),
                None,
            );

            assert!(!x.best.is_resizable());
            assert_eq!(x.search_l(), 10);
            assert_eq!(x.best.search_l(), 10);

            assert!(x.visited.is_empty());
            assert!(x.id_scratch.is_empty());

            assert!(x.hops == 0);
            assert!(x.cmps == 0);
        }

        {
            let x = SearchScratch::<u64, NeighborPriorityQueue<u64>>::new(
                PriorityQueueConfiguration::Resizable(10),
                None,
            );

            assert!(x.best.is_resizable());
            assert_eq!(x.search_l(), 10);
            assert_eq!(x.best.search_l(), 10);

            assert!(x.visited.is_empty());
            assert!(x.id_scratch.is_empty());

            assert!(x.hops == 0);
            assert!(x.cmps == 0);
        }
    }

    #[test]
    pub fn test_resize() {
        let mut x = SearchScratch::<u64, _>::new(PriorityQueueConfiguration::Fixed(5), None);
        assert_eq!(x.search_l(), 5);
        x.resize(10);
        assert_eq!(x.search_l(), 10);
    }

    #[test]
    pub fn test_reconfigure() {
        let mut x = SearchScratch::<u64, NeighborPriorityQueue<u64>>::new(
            PriorityQueueConfiguration::Fixed(5),
            None,
        );
        assert_eq!(x.search_l(), 5);

        x.resize(10);
        assert_eq!(x.search_l(), 10);
    }

    #[test]
    pub fn test_clear() {
        let mut x = SearchScratch::<u64, NeighborPriorityQueue<u64>>::new(
            PriorityQueueConfiguration::Fixed(5),
            None,
        );

        x.visited.insert(1);
        x.visited.insert(10);

        x.id_scratch.push(1);
        x.id_scratch.push(10);

        x.best.insert(Neighbor::new(1, 1.0));
        x.best.insert(Neighbor::new(10, 2.0));
        assert_eq!(x.best.size(), 2);

        // Do the clear.
        x.clear();
        assert!(x.visited.is_empty());
        assert!(x.id_scratch.is_empty());
        assert_eq!(x.best.size(), 0);

        assert!(x.hops == 0);
        assert!(x.cmps == 0);
    }

    #[test]
    pub fn test_stamped_visited_set() {
        let mut set = StampedVisitedSet::<u32>::new();
        assert!(set.is_empty());

        assert!(set.insert(5));
        assert!(!set.insert(5));
        assert!(set.contains(&5));
        assert!(!set.contains(&4));
        assert!(!set.is_empty());

        // Ids beyond the current allocation grow the stamp array on demand.
        assert!(set.insert(10_000));
        assert!(set.contains(&10_000));
        assert!(!set.contains(&9_999));

        // Clearing is logical (epoch bump): previously visited ids read as unvisited.
        set.clear();
        assert!(set.is_empty());
        assert!(!set.contains(&5));
        assert!(!set.contains(&10_000));
        assert!(set.insert(5));
        assert!(set.contains(&5));

        set.extend([7, 8, 8, 9]);
        assert!(set.contains(&7));
        assert!(set.contains(&9));
        assert!(!set.insert(8));
    }

    #[test]
    pub fn test_stamped_visited_set_epoch_wrap() {
        let mut set = StampedVisitedSet::<u32>::with_capacity(4);
        set.insert(1);
        // Simulate a set that has exhausted its epoch counter.
        set.epoch = u16::MAX;
        set.clear();
        assert!(!set.contains(&1));
        assert!(set.insert(1));
        assert!(set.contains(&1));
    }

    #[test]
    pub fn test_stamped_visited_set_sentinel_ids() {
        // Test providers use sentinel ids like u32::MAX for start points; these must
        // work without growing the dense array to absurd sizes.
        let mut set = StampedVisitedSet::<u32>::new();
        assert!(set.insert(u32::MAX));
        assert!(!set.insert(u32::MAX));
        assert!(set.contains(&u32::MAX));
        assert!(!set.contains(&(u32::MAX - 1)));
        assert!(set.stamps.len() < 1024);

        // Mixing dense and sentinel ids works.
        assert!(set.insert(3));
        assert!(set.contains(&3));
        assert!(set.contains(&u32::MAX));

        set.clear();
        assert!(!set.contains(&u32::MAX));
        assert!(set.overflow.is_empty());
    }
}
