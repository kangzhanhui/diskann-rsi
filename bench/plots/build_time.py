#!/usr/bin/env python3
# Copyright (C) 2026 TerseMind. Licensed under the MIT license.
"""Plot recorded SIFT1M construction times for the README.

Run with Python and matplotlib: python3 bench/plots/build_time.py
The Vamana build record is dated Sep 15; its Sep 16 search rows have null
build_time_s. DiskANN-RSI's build record is dated Sep 16. Those original
dates are preserved in the figure rather than inferred from the search run.
"""

import json
from datetime import datetime, timezone
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[2]


def construction_record(filename):
    records = []
    for line in (ROOT / "bench" / "results" / filename).read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if row["build_params"] != {"R": 32, "l_build": 100, "alpha": 1.0}:
            continue
        if row.get("build_time_s") is not None:
            records.append(row)
    if not records:
        raise ValueError(f"Missing construction record: {filename}")
    times = {r["build_time_s"] for r in records}
    if len(times) != 1:
        raise ValueError(f"Multiple construction times require explicit selection: {times}")
    return max(records, key=lambda r: r["ts"])


def main():
    rows = [construction_record("vamana.jsonl"), construction_record("diskann_opt.jsonl")]
    labels = ["Stock Vamana", "DiskANN-RSI"]
    times = [r["build_time_s"] for r in rows]
    plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 12, "svg.fonttype": "none"})
    fig, ax = plt.subplots(figsize=(9.8, 4.0), dpi=180)
    fig.patch.set_facecolor("white")
    ax.set_facecolor("white")
    bars = ax.barh(labels, times, color=["#94a3b8", "#0f766e"], height=0.48)
    ax.invert_yaxis()
    for bar, row in zip(bars, rows):
        seconds = row["build_time_s"]
        ax.text(seconds + 5, bar.get_y() + bar.get_height() / 2, f"{seconds:.1f}s",
                va="center", fontweight="bold", color="#0f172a", fontsize=14)
    ax.set_xlim(0, 305)
    ax.set_xlabel("Construction time (seconds; lower is better)", color="#475569", labelpad=12)
    ax.set_axisbelow(True)
    ax.grid(axis="x", alpha=0.15)
    ax.tick_params(axis="both", length=0, labelcolor="#475569", pad=8)
    for spine in ax.spines.values():
        spine.set_visible(False)
    fig.suptitle("SIFT1M: recorded index construction time", x=0.06, ha="left",
                 fontsize=17, fontweight="bold", color="#0f172a", y=0.96)
    fig.text(0.06, 0.84, "Same graph parameters: R=32, l_build=100, alpha=1.0", color="#64748b", fontsize=11)
    dates = [datetime.fromtimestamp(r["ts"], timezone.utc).date().isoformat() for r in rows]
    fig.text(0.06, 0.06, f"Frozen build records: Vamana {dates[0]}; DiskANN-RSI {dates[1]} (UTC).",
             color="#64748b", fontsize=9)
    fig.subplots_adjust(left=0.21, right=0.94, bottom=0.28, top=0.76)
    out = ROOT / "docs" / "media"
    out.mkdir(parents=True, exist_ok=True)
    for ext in ("svg", "png"):
        fig.savefig(out / f"build-time.{ext}", facecolor="white")
    svg = out / "build-time.svg"
    svg.write_text("\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n")
    plt.close(fig)
    print({label: row["build_time_s"] for label, row in zip(labels, rows)})


if __name__ == "__main__":
    main()
