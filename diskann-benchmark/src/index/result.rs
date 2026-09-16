/*
 * Copyright (c) Microsoft Corporation.
 * Licensed under the MIT license.
 */

use diskann_benchmark_core as benchmark_core;
use diskann_benchmark_runner::utils::{percentiles, MicroSeconds};
use serde::Serialize;

use crate::{
    index::build::BuildStats,
    utils::{self, DisplayWrapper, MaybeDisplay},
};

//////////////////
// BuildResult  //
//////////////////
#[derive(Debug, Serialize)]
pub(crate) struct BuildResult {
    pub(crate) build: Option<BuildStats>,
    pub(crate) search: AggregatedSearchResults,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) search_summary: Option<TopkSweepSummary>,
}

impl BuildResult {
    pub(crate) fn new(build: Option<BuildStats>, search: AggregatedSearchResults) -> Self {
        Self {
            build,
            search,
            search_summary: None,
        }
    }
}

impl std::fmt::Display for BuildResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref build) = self.build {
            write!(f, "{}", build)?;
        }

        self.search.fmt(f)?;

        if let Some(ref summary) = self.search_summary {
            writeln!(f, "{}", summary)?;
        }

        Ok(())
    }
}

//////////////////////
// QuantBuildResult //
//////////////////////

#[cfg(any(feature = "product-quantization", feature = "scalar-quantization",))]
#[derive(Debug, Serialize)]
pub(crate) struct QuantBuildResult {
    pub(crate) quant_training_time: MicroSeconds,
    pub(crate) build: BuildResult,
}

#[cfg(any(feature = "product-quantization", feature = "scalar-quantization",))]
impl std::fmt::Display for QuantBuildResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Quant Training Time: {}s",
            self.quant_training_time.as_seconds()
        )?;
        self.build.fmt(f)
    }
}

///////////////////
// SearchResults //
///////////////////

#[derive(Debug, Serialize)]
pub(crate) enum AggregatedSearchResults {
    Topk(Vec<SearchResults>),
    Range(Vec<RangeSearchResults>),
}

/// The default recall threshold for [`TopkSweepSummary`] when the job does not specify one.
pub(crate) const DEFAULT_RECALL_TARGET: f64 = 0.95;

//////////////////////
// TopkSweepSummary //
//////////////////////

/// Sweep-level summary reporting the best QPS achieved at or above a recall target.
///
/// This is the standard comparison metric for ANN search ("QPS at a given recall"): it
/// rewards configurations that reach the recall target cheaply and ignores extra recall
/// bought with wasted computation.
#[derive(Debug, Serialize)]
pub(crate) struct TopkSweepSummary {
    /// The recall threshold a configuration must reach to qualify.
    pub(crate) recall_target: f64,
    /// One entry per swept thread count.
    pub(crate) per_thread: Vec<QpsAtRecall>,
}

impl TopkSweepSummary {
    /// Compute the summary from a completed top-k sweep.
    pub(crate) fn compute(results: &[SearchResults], recall_target: f64) -> Self {
        let mut thread_counts: Vec<usize> = results.iter().map(|r| r.num_tasks).collect();
        thread_counts.sort_unstable();
        thread_counts.dedup();

        let per_thread = thread_counts
            .into_iter()
            .map(|num_tasks| {
                let best = results
                    .iter()
                    .filter(|r| r.num_tasks == num_tasks && r.recall.average >= recall_target)
                    .map(|r| (percentiles::mean(&r.qps).unwrap_or(0.0), r))
                    .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                match best {
                    Some((qps, r)) => QpsAtRecall {
                        num_tasks,
                        qps_at_recall: qps,
                        search_l: Some(r.search_l),
                        achieved_recall: Some(r.recall.average),
                    },
                    None => QpsAtRecall {
                        num_tasks,
                        qps_at_recall: 0.0,
                        search_l: None,
                        achieved_recall: None,
                    },
                }
            })
            .collect();

        Self {
            recall_target,
            per_thread,
        }
    }
}

impl std::fmt::Display for TopkSweepSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QPS @ recall>={:.2}: ", self.recall_target)?;
        for (i, entry) in self.per_thread.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            match (entry.search_l, entry.achieved_recall) {
                (Some(search_l), Some(recall)) => write!(
                    f,
                    "{}T={:.1} (Ls={}, recall={:.4})",
                    entry.num_tasks, entry.qps_at_recall, search_l, recall
                )?,
                _ => write!(f, "{}T=0.0 (no config reached target)", entry.num_tasks)?,
            }
        }
        Ok(())
    }
}

/// The best-QPS-at-recall entry for a single thread count.
#[derive(Debug, Serialize)]
pub(crate) struct QpsAtRecall {
    pub(crate) num_tasks: usize,
    /// The highest mean QPS among configs with recall >= target; 0.0 if none qualify.
    pub(crate) qps_at_recall: f64,
    /// The `search_l` achieving `qps_at_recall`, if any config qualified.
    pub(crate) search_l: Option<usize>,
    /// The recall of the qualifying config, if any.
    pub(crate) achieved_recall: Option<f64>,
}

impl std::fmt::Display for AggregatedSearchResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Topk(v) => write!(f, "{}", DisplayWrapper(v.as_slice()))?,
            Self::Range(v) => write!(f, "{}", DisplayWrapper(v.as_slice()))?,
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchResults {
    pub(crate) num_tasks: usize,
    pub(crate) search_n: usize,
    pub(crate) search_l: usize,
    pub(crate) qps: Vec<f64>,
    pub(crate) search_latencies: Vec<MicroSeconds>,
    pub(crate) mean_latencies: Vec<f64>,
    pub(crate) p90_latencies: Vec<MicroSeconds>,
    pub(crate) p99_latencies: Vec<MicroSeconds>,
    pub(crate) recall: utils::recall::RecallMetrics,
    pub(crate) mean_cmps: f32,
    pub(crate) mean_hops: f32,
}

impl SearchResults {
    pub fn new(summary: benchmark_core::search::graph::knn::Summary) -> Self {
        let benchmark_core::search::graph::knn::Summary {
            setup,
            parameters,
            end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            recall,
            mean_cmps,
            mean_hops,
            ..
        } = summary;

        let qps = end_to_end_latencies
            .iter()
            .map(|latency| recall.num_queries as f64 / latency.as_seconds())
            .collect();

        Self {
            num_tasks: setup.tasks.into(),
            search_n: parameters.k_value().get(),
            search_l: parameters.knn.l_value().get(),
            qps,
            search_latencies: end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            recall: (&recall).into(),
            mean_cmps: mean_cmps as f32,
            mean_hops: mean_hops as f32,
        }
    }
}

fn format_search_results_table<F>(
    f: &mut std::fmt::Formatter<'_>,
    results: &[SearchResults],
    batch_formatter: Option<F>,
) -> std::fmt::Result
where
    F: Fn(usize) -> String,
{
    if results.is_empty() {
        return Ok(());
    }

    let has_batch = batch_formatter.is_some();
    let headers: &[&str] = if has_batch {
        &[
            "Batch",
            "Ls",
            "KNN",
            "Avg cmps",
            "Avg hops",
            "QPS - mean(max)",
            "Avg Latency",
            "p99 Latency",
            "Recall",
            "Threads",
        ]
    } else {
        &[
            "Ls",
            "KNN",
            "Avg cmps",
            "Avg hops",
            "QPS - mean(max)",
            "Avg Latency",
            "p99 Latency",
            "Recall",
            "Threads",
        ]
    };

    let mut table = diskann_benchmark_runner::utils::fmt::Table::new(headers, results.len());
    results.iter().enumerate().for_each(|(i, r)| {
        let mut row = table.row(i);
        let mut col_idx = 0;

        if let Some(ref formatter) = batch_formatter {
            row.insert(formatter(i), col_idx);
            col_idx += 1;
        }

        row.insert(r.search_l, col_idx);
        row.insert(r.search_n, col_idx + 1);
        row.insert(r.mean_cmps, col_idx + 2);
        row.insert(r.mean_hops, col_idx + 3);
        row.insert(
            format!(
                "{:.1} ({:.1})",
                MaybeDisplay(percentiles::mean(&r.qps), "missing"),
                MaybeDisplay(percentiles::max_f64(&r.qps), "missing"),
            ),
            col_idx + 4,
        );
        row.insert(
            format!(
                "{:.1}us ({:.1}us)",
                MaybeDisplay(percentiles::mean(&r.mean_latencies), "missing"),
                MaybeDisplay(percentiles::max_f64(&r.mean_latencies), "missing"),
            ),
            col_idx + 5,
        );
        row.insert(
            format!(
                "{:.1}us ({:.1})",
                MaybeDisplay(percentiles::mean(&r.p99_latencies), "missing"),
                MaybeDisplay(r.p99_latencies.iter().max(), "missing"),
            ),
            col_idx + 6,
        );
        row.insert(format!("{:3}", r.recall.average), col_idx + 7);
        row.insert(r.num_tasks, col_idx + 8);
    });

    write!(f, "{}", table)
}

impl std::fmt::Display for DisplayWrapper<'_, [SearchResults]> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format_search_results_table(f, self, None::<fn(usize) -> String>)
    }
}

////////////////////////
// RangeSearchResults //
////////////////////////

#[derive(Debug, Serialize)]
pub(crate) struct RangeSearchResults {
    pub(crate) num_tasks: usize,
    pub(crate) initial_l: usize,
    pub(crate) qps: Vec<f64>,
    pub(crate) search_latencies: Vec<MicroSeconds>,
    pub(crate) mean_latencies: Vec<f64>,
    pub(crate) p90_latencies: Vec<MicroSeconds>,
    pub(crate) p99_latencies: Vec<MicroSeconds>,
    pub(crate) average_precision: utils::recall::AveragePrecisionMetrics,
}

impl RangeSearchResults {
    pub fn new(summary: benchmark_core::search::graph::range::Summary) -> Self {
        let benchmark_core::search::graph::range::Summary {
            setup,
            parameters,
            end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            average_precision,
            ..
        } = summary;

        let qps = end_to_end_latencies
            .iter()
            .map(|latency| average_precision.num_queries as f64 / latency.as_seconds())
            .collect();

        Self {
            num_tasks: setup.tasks.into(),
            initial_l: parameters.starting_l().into(),
            qps,
            search_latencies: end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            average_precision: (&average_precision).into(),
        }
    }

    pub fn new_filtered(summary: benchmark_core::search::graph::filtered_range::Summary) -> Self {
        let benchmark_core::search::graph::filtered_range::Summary {
            setup,
            parameters,
            end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            average_precision,
            ..
        } = summary;

        let qps = end_to_end_latencies
            .iter()
            .map(|latency| average_precision.num_queries as f64 / latency.as_seconds())
            .collect();

        Self {
            num_tasks: setup.tasks.into(),
            initial_l: parameters.starting_l().into(),
            qps,
            search_latencies: end_to_end_latencies,
            mean_latencies,
            p90_latencies,
            p99_latencies,
            average_precision: (&average_precision).into(),
        }
    }
}

fn format_range_search_results_table<F>(
    f: &mut std::fmt::Formatter<'_>,
    results: &[RangeSearchResults],
    batch_formatter: Option<F>,
) -> std::fmt::Result
where
    F: Fn(usize) -> String,
{
    if results.is_empty() {
        return Ok(());
    }

    let has_batch = batch_formatter.is_some();
    let headers: &[_] = if has_batch {
        &[
            "Batch",
            "initial Ls",
            "QPS - mean(max)",
            "Avg Latency",
            "p99 Latency",
            "Average Precision",
            "Threads",
        ]
    } else {
        &[
            "initial Ls",
            "QPS - mean(max)",
            "Avg Latency",
            "p99 Latency",
            "Average Precision",
            "Threads",
        ]
    };

    let mut table = diskann_benchmark_runner::utils::fmt::Table::new(headers, results.len());
    results.iter().enumerate().for_each(|(i, r)| {
        let mut row = table.row(i);
        let mut col_idx = 0;

        if let Some(ref formatter) = batch_formatter {
            row.insert(formatter(i), col_idx);
            col_idx += 1;
        }

        row.insert(r.initial_l, col_idx);
        row.insert(
            format!(
                "{:.1} ({:.1})",
                MaybeDisplay(percentiles::mean(&r.qps), "missing"),
                MaybeDisplay(percentiles::max_f64(&r.qps), "missing"),
            ),
            col_idx + 1,
        );
        row.insert(
            format!(
                "{:.1}us ({:.1}us)",
                MaybeDisplay(percentiles::mean(&r.mean_latencies), "missing"),
                MaybeDisplay(percentiles::max_f64(&r.mean_latencies), "missing"),
            ),
            col_idx + 2,
        );
        row.insert(
            format!(
                "{:.1}us ({:.1})",
                MaybeDisplay(percentiles::mean(&r.p99_latencies), "missing"),
                MaybeDisplay(r.p99_latencies.iter().max(), "missing"),
            ),
            col_idx + 3,
        );
        row.insert(
            format!("{:3}", r.average_precision.average_precision),
            col_idx + 4,
        );
        row.insert(r.num_tasks, col_idx + 5);
    });

    write!(f, "{}", table)
}

impl std::fmt::Display for DisplayWrapper<'_, [RangeSearchResults]> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format_range_search_results_table(f, self, None::<fn(usize) -> String>)
    }
}
