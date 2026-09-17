#!/usr/bin/env python3
"""Recall-QPS curves for the paper, plotted from the frozen raw measurements.

Input :  bench/results/{vamana,diskann_opt,hnswlib,nsg}.jsonl
Output:  paper/fig_recall_qps.pdf

Usage:  python3 bench/plots/recall_qps.py
"""
import json
import glob
import os
import statistics
from collections import defaultdict

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RESULTS = os.path.join(ROOT, "bench", "results")
OUT = os.path.join(ROOT, "paper", "fig_recall_qps.pdf")

# (system, alpha) -> display label / style.  Only the 1-thread reference protocol
# is shown, except NSG which was measured with 2 threads (marked as such).
SERIES = [
    (("hnswlib-master-default", None), "hnswlib (master-default)", "#7f7f7f", "o", "-"),
    (("vamana", 1.0), "vamana stock, alpha=1.0", "#1f77b4", "s", "-"),
    (("vamana", 1.2), "vamana stock, alpha=1.2", "#aec7e8", "s", "--"),
    (("diskann-rsi", 1.0), "diskann-rsi, alpha=1.0", "#d62728", "^", "-"),
    (("diskann-rsi", 1.2), "diskann-rsi, alpha=1.2", "#ff9896", "^", "--"),
    (("nsg", None), "NSG (2 threads)", "#2ca02c", "D", ":"),
]


def load():
    points = defaultdict(lambda: defaultdict(list))
    for path in glob.glob(os.path.join(RESULTS, "*.jsonl")):
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                row = json.loads(line)
                threads = row.get("search_params", {}).get("threads")
                if row["system"] == "nsg":
                    if threads is not None:
                        continue
                elif threads != 1:
                    continue
                alpha = row.get("build_params", {}).get("alpha")
                key = (row["system"], alpha)
                op = row["search_params"].get("search_l", row["search_params"].get("ef"))
                points[key][op].append((row["qps_median"] if "qps_median" in row else row["qps"],
                                        row["recall_at_10"]))
    return points


def main():
    points = load()
    fig, ax = plt.subplots(figsize=(6.4, 4.2), dpi=200)
    for key, label, color, marker, ls in SERIES:
        data = points.get(key)
        if not data:
            print(f"warning: no rows for {key}")
            continue
        # per operating point: median over repeats (robust to the earlier,
        # environmentally-interfered measurement round present in the raw rows)
        curve = sorted(((statistics.median([q for q, _ in v]),
                         statistics.median([r for _, r in v]))
                        for v in data.values()), key=lambda t: t[0])
        qs = [c[0] for c in curve]
        rs = [c[1] for c in curve]
        ax.plot(qs, rs, marker=marker, color=color, linestyle=ls, markersize=4,
                linewidth=1.4, label=label)

    ax.axhline(0.95, color="k", linestyle="-.", linewidth=0.8, alpha=0.6)
    ax.text(1400, 0.952, "recall = 0.95", fontsize=7, color="k", alpha=0.7)
    ax.set_xscale("log")
    ax.set_xlabel("QPS (log scale, 10k queries, single thread unless noted)")
    ax.set_ylabel("recall@10")
    ax.set_ylim(0.78, 1.005)
    ax.set_title("SIFT1M: recall@10 vs QPS, one machine, one protocol", fontsize=9)
    ax.grid(True, which="both", alpha=0.25, linewidth=0.5)
    ax.legend(fontsize=7, loc="lower right", framealpha=0.9)
    fig.tight_layout()
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    fig.savefig(OUT)
    print("wrote", OUT)
    for key, label, *_ in SERIES:
        data = points.get(key)
        if data:
            n = sum(len(v) for v in data.values())
            print(f"  {label}: {len(data)} operating points, {n} raw rows")


if __name__ == "__main__":
    main()