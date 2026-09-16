#!/usr/bin/env python3
"""One-time SIFT1M data preparation for the DiskANN benchmark harness.

Downloads sift-128-euclidean.hdf5 (the standard ANN-benchmarks SIFT1M distribution:
1M x 128d float32 base vectors, 10K queries, exact top-100 ground truth) and writes
the DiskANN formats used by job.json:

  generated_data/sift1m/sift_base_1m_128d.fbin   - base vectors (.fbin)
  generated_data/sift1m/sift_query_10k_128d.fbin - query vectors (.fbin)
  generated_data/sift1m/gt_sift_10k_k100.bin     - ground truth (header + u32 ids + f32 dists)

Usage (requires numpy + h5py):
  python3 tools/prepare_sift1m.py

For datasets distributed as raw .fvecs/.ivecs (e.g. GIST1M), use the Rust converter
instead: cargo run --release -p diskann-tools --bin convert_vecs -- --help
"""

import struct
import sys
import urllib.request
from pathlib import Path

import h5py
import numpy as np

URL = "http://vectors.erikbern.com/sift-128-euclidean.hdf5"
OUT_DIR = Path(__file__).resolve().parent.parent / "generated_data" / "sift1m"


def write_fbin(path: Path, mat: np.ndarray) -> None:
    mat = np.ascontiguousarray(mat, dtype=np.float32)
    with open(path, "wb") as f:
        f.write(struct.pack("<II", mat.shape[0], mat.shape[1]))
        mat.tofile(f)
    print(f"wrote {path} ({mat.shape[0]} x {mat.shape[1]} f32)")


def write_groundtruth(path: Path, ids: np.ndarray, dists: np.ndarray) -> None:
    ids = np.ascontiguousarray(ids, dtype=np.uint32)
    dists = np.ascontiguousarray(dists, dtype=np.float32)
    assert ids.shape == dists.shape
    with open(path, "wb") as f:
        f.write(struct.pack("<II", ids.shape[0], ids.shape[1]))
        ids.tofile(f)
        dists.tofile(f)
    print(f"wrote {path} ({ids.shape[0]} x {ids.shape[1]} ids+dists)")


def main() -> int:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    hdf5_path = OUT_DIR / "sift-128-euclidean.hdf5"

    if not hdf5_path.exists():
        print(f"downloading {URL} -> {hdf5_path}")
        urllib.request.urlretrieve(URL, hdf5_path)

    with h5py.File(hdf5_path, "r") as h5:
        # ann-benchmarks hdf5 layout: train/test (+ base/queries in some variants)
        base_key = "train" if "train" in h5 else "base"
        query_key = "test" if "test" in h5 else "queries"
        base = h5[base_key][:]  # (1_000_000, 128) float32
        queries = h5[query_key][:]  # (10_000, 128) float32
        neighbors = h5["neighbors"][:]  # (10_000, 100) int32/uint32
        # Distances in the hdf5 are NOT squared-L2-consistent with the benchmark's
        # Metric::L2 convention in all distributions; recompute exact squared L2.
        print("recomputing exact squared-L2 ground-truth distances...")
        base_t = base.astype(np.float32)
        dists = np.empty(neighbors.shape, dtype=np.float32)
        for q in range(queries.shape[0]):
            diff = base_t[neighbors[q]] - queries[q]
            dists[q] = np.einsum("ij,ij->i", diff, diff, dtype=np.float32)

    write_fbin(OUT_DIR / "sift_base_1m_128d.fbin", base)
    write_fbin(OUT_DIR / "sift_query_10k_128d.fbin", queries)
    write_groundtruth(OUT_DIR / "gt_sift_10k_k100.bin", neighbors, dists)
    return 0


if __name__ == "__main__":
    sys.exit(main())
