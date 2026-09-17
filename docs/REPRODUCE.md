# Reproducing the Benchmarks

Environment and procedure used for all cross-system numbers in the paper
(session of 2026-09-16). Following it is only needed to re-measure; the raw
records already ship in [`bench/results/`](../bench/results/).

## Host

- Cloud VM: **2 vCPU (x86_64), 3.6 GB RAM + 8 GB swapfile**, OpenCloudOS 9.4 (RHEL family, `dnf`)
- Single socket, shared cores; keep the host idle during measurement (timing-sensitive)
- Dataset: **SIFT1M** (1M × 128, L2) converted to `.fbin` + ground truth via `tools/prepare_sift1m.py`

## Protocol

- Metric: **recall@10 vs QPS**, 10,000 queries, **3 repetitions**, median reported
- Reference thread count: **1T** (single thread); 2T reported separately for vamana/diskann-rsi
- Build parameters
  - vamana / diskann-rsi: R=32, l_build=100, α ∈ {1.0, 1.2}
  - hnswlib: M=16, ef_construction=200; variants: stock 0.8.0, master-default, variant4 (experimental pruning heuristic)
  - nsg: kNN graph via efanna (K=200, several hours), NSG build L=40, R=50, C=500; search parallelized with OpenMP (2T)

## Host preparation pitfalls (learned 2026-09-15/16)

- Use `dnf` (no apt); enable EPEL+CRB if a package is missing
- Create the 8 GB swapfile **before** the efanna kNN-graph stage (OOM risk at 3.6 GB RAM)
- Build with `-j2` (incl. `CARGO_BUILD_JOBS=2`, `OMP_NUM_THREADS=2`)
- hnswlib on newer GCC may need small patches; see commit history of the bench scripts
- A previous measurement day (2026-09-15) showed environmental interference
  (hnswlib build 530s vs 242s; ~20% QPS skew). Prefer same-session data and
  filter `bench/results/*.jsonl` by timestamp when in doubt.

## Steps

1. `cargo build --release` (fat LTO; ~6 min)
2. Prepare SIFT1M: `python3 tools/prepare_sift1m.py /path/to/sift`
3. Configure `job.json` (data paths, R/l_build/α, search_l ladder {16..256}, recall_target 0.95)
4. Run: `cargo run --release -p diskann-benchmark-runner`
5. Records are appended as JSONL: `system, build_params, search_params, recall_at_10, qps, qps_median, qps_reps, build_time_s, mem_bytes, ts`
