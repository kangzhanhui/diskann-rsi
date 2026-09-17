# Benchmarks

All numbers below come from a single measurement session on **2026-09-16** on one
Cloud VM: **2 vCPU / 3.6 GB RAM + 8 GB swap, OpenCloudOS 9.4**.
Dataset: **SIFT1M** (1M × 128, L2). Metric: **recall@10 vs QPS**, 10k queries,
3 repetitions, medians reported. Single-thread (1T) is the reference protocol;
NSG's search binary uses OpenMP with 2 threads and is labelled 2T.

Raw records: [`../bench/results/`](../bench/results/)
(`vamana.jsonl`, `diskann_opt.jsonl`, `hnswlib.jsonl`, `nsg.jsonl`).
Earlier (2026-09-15) rows exist in the same files and show visible
environmental interference (build 530s vs 242s; ~20% QPS skew); the 2026-09-16
same-session rows are authoritative — filter by timestamp when consuming.

## 1. QPS at recall ≥ 0.95 (higher is better)

| # | System | QPS | recall@10 | Op. point | Build | Peak mem |
|---|---|---|---|---|---|---|
| 1 | hnswlib-master-default | **6077** | 0.9637 | ef=64 | 238.6s | 1.31 GB |
| 2 | hnswlib-stock (0.8.0) | 6017 | 0.9636 | ef=64 | 242.0s | 1.31 GB |
| 3 | vamana (stock, α=1.0) | 5708 | 0.9667 | Ls=64 | 255.0s | 1.27 GB |
| 4 | hnswlib-variant4 (pruning heuristic v4) | 5632 | 0.9617 | ef=64 | 260.4s | 1.31 GB |
| 5 | diskann-rsi (α=1.0) | 5078 | 0.9669 | Ls=64 | **131.9s** | 1.28 GB |
| 6 | nsg | 2643 | 0.9525 | Ls=32 | 451s † | 1.40 GB |

† NSG additionally needs an efanna kNN-graph pre-computation (K=200) of several
hours; the 451s is the NSG construction step only. NSG search ran at 2 threads;
even so it stays below half of the others' 1T QPS.

## 2. Full recall–QPS ladders (1T; NSG 2T; `param: recall/QPS`)

| System | low → high operating points |
|---|---|
| vamana α=1.0 | Ls16 .826/15382 · Ls32 .914/9705 · Ls64 .967/5708 · Ls128 .990/3178 · Ls256 .998/1756 |
| vamana α=1.2 | Ls16 .881/12513 · Ls32 .949/7853 · Ls64 .984/4534 · Ls128 .996/2658 · Ls256 .999/1448 |
| diskann-rsi α=1.0 | Ls16 .824/10587 · Ls32 .914/7897 · Ls64 .967/5078 · Ls128 .990/3090 · Ls256 .998/1719 |
| diskann-rsi α=1.2 | Ls16 .881/9398 · Ls32 .949/6650 · Ls64 .983/4164 · Ls128 .996/2502 · Ls256 .999/1435 |
| hnswlib-stock | ef16 .801/16738 · ef32 .903/10281 · ef64 .964/6017 · ef128 .989/3279 · ef256 .997/1764 |
| hnswlib-master-default | ef16 .801/16368 · ef32 .904/10448 · ef64 .964/6077 · ef128 .989/3258 · ef256 .997/1792 |
| hnswlib-variant4 | ef16 .817/15621 · ef32 .907/9853 · ef64 .962/5632 · ef128 .986/3174 · ef256 .996/1728 |
| nsg (2T) | Ls16 .877/2929 · Ls32 .952/2632 · Ls64 .986/2099 · Ls128 .996/1547 · Ls256 .999/1030 |

diskann-rsi vs stock vamana (1T): −31% @Ls16, −19% @Ls32, −11% @Ls64,
−3% @Ls128, −2% @Ls256. Recalls match to ±0.001 at every rung → identical
graph quality; the delta is search-kernel speed only.

## 3. Two-thread scaling

| Op. point | vamana 1T→2T | diskann-rsi 1T→2T |
|---|---|---|
| Ls=64 (recall ≈ 0.967) | 5708→9993 (1.75×) | 5078→9472 (1.87×) |
| Ls=16 (recall ≈ 0.825) | 15382→27772 (1.81×) | 10587→19428 (1.84×) |

## 4. Per-optimization measurements (RSI harness, SIFT1M)

| Change | Effect |
|---|---|
| Epoch-stamped dense visited set (u32 stamps) | +10–15% 1T QPS (10K bench) |
| 16-bit stamps (4 MB → 2 MB @1M) | visited-set time −45%, expand_beam −19% |
| Query-path package cumulative | 1T QPS@0.95 2994.6 → 4641.1 (**+55%**); L=35 latency 330→215µs; 2T 8486.6 (1.83×) |
| Adaptive early stop (ratio 1.2, min_hops 8) | −10% cmps @L=60, −50% @L=200, recall unchanged |
| early_stop ratio 1.15 | −8.7~−25% cmps near knee, but recall ceiling capped at 0.9817 → reverted to 1.2 |
| prefetch_lookahead sweep {8,16,24} | default 8 already optimal (lever only) |
| beam_width > 1 @1T | +8–25% cmps, no recall gain (lever only) |
| fat LTO | +10–12% search QPS (10K A/B), identical cmps/recall |

Recall-stacking phase (2026-09-14, 10K synthetic, metric recall@10):
l_build 50→100 (+1.3%), α 1.2→1.5 (+6.2%, QPS cost), Latin-hypercube starts
(+0.7%), max_degree 32→48→64→80 + insert-retry (0.985→0.995→0.998→1.000),
multi-insert measured as a regression (0.985→0.979).

## 5. Negative results (measured, not adopted)

| Idea | Outcome |
|---|---|
| SQ8 traversal + FP rerank | recall −0.001, but no QPS gain (5238 vs 4641 at metric point); quantization pays only under bandwidth pressure |
| Candidate-queue merge insertion | slower → reverted |
| Random-projection data reordering | no measurable effect (working set L3-resident) |
| Partial-distance early exit | 86.9% aborts but after ~2.9/4 chunks → net loss for 128-dim/4-chunk kernel |
| hnswlib variant4 pruning heuristic | loses to same-source default at ≥0.95 band (5632 vs 6077) |

## 6. Why the RSI-harness +55% does not transfer

1. **Protocol differences** — the harness uses its own data layout, baseline
   pairing, and early-stop configuration; its baseline is not the stock C++
   Vamana used here.
2. **Hardware regime** — at 1M×128 the working set is largely L3-resident on
   this 2-vCPU host, leaving little room for the cache-residency win, while
   the added kernel diversity-phase logic costs constant per-expansion
   overhead that dominates at small search_l.
3. **LTO asymmetry** — the RSI binary carries fat LTO (+10–12%); the deficit
   is measured despite that exclusive advantage.

Regime-independent wins: **index build 1.9× faster** (131.9s vs 255.0s) and
**2T scaling 1.87× vs 1.75×**.
