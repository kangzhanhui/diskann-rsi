/*
 * Copyright (c) Microsoft Corporation.
 * Licensed under the MIT license.
 */

//! Converts `.fvecs`/`.ivecs` files (the row-prefixed format used by the public TEXMEX
//! benchmark datasets such as SIFT1M) into DiskANN's `.fbin` and ground-truth formats.
//!
//! `.fvecs` rows are `[dim: i32][dim x f32]` and `.ivecs` rows are `[dim: i32][dim x i32]`.
//! `.fbin` is `[npoints: u32][ndims: u32]` followed by tightly packed row-major data.

use std::{
    fs::File,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::{bail, Context};
use bytemuck::cast_slice;
use clap::{Parser, Subcommand};
use diskann_tools::utils::init_subscriber;
use diskann_utils::io::{read_bin, Metadata};
use diskann_vector::distance::{DistanceProvider, Metric};

#[derive(Debug, Parser)]
#[command(about = "Convert .fvecs/.ivecs datasets to .fbin and ground-truth formats")]
struct ConvertVecsArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Convert a .fvecs vector file to .fbin.
    Fvecs2Fbin {
        /// Input .fvecs file.
        #[arg(long, short)]
        input: PathBuf,
        /// Output .fbin file.
        #[arg(long, short)]
        output: PathBuf,
    },
    /// Convert an .ivecs ground-truth file to the DiskANN ground-truth format, computing
    /// exact (squared L2) distances for each ground-truth neighbor.
    Ivecs2Gt {
        /// Input .ivecs file with one top-K row per query.
        #[arg(long, short)]
        input: PathBuf,
        /// Base vectors (.fbin) the ground-truth ids refer to.
        #[arg(long, short)]
        base: PathBuf,
        /// Query vectors (.fbin), one per .ivecs row.
        #[arg(long, short)]
        query: PathBuf,
        /// Output ground-truth file.
        #[arg(long, short)]
        output: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    init_subscriber();

    match ConvertVecsArgs::parse().command {
        Command::Fvecs2Fbin { input, output } => fvecs_to_fbin(&input, &output),
        Command::Ivecs2Gt {
            input,
            base,
            query,
            output,
        } => ivecs_to_ground_truth(&input, &base, &query, &output),
    }
}

/// Read the row dimensionality and row count of a `.fvecs`/`.ivecs` file.
fn vecs_shape(file: &mut File, elem_size: usize) -> anyhow::Result<(usize, usize)> {
    let file_len = file.metadata()?.len() as usize;
    if file_len < 4 {
        bail!("file too small to contain a row dimension");
    }

    let mut dim_buf = [0u8; 4];
    file.read_exact(&mut dim_buf)?;
    file.seek(SeekFrom::Start(0))?;
    let dim = i32::from_le_bytes(dim_buf);
    if dim <= 0 {
        bail!("invalid row dimension {dim}");
    }
    let dim = dim as usize;

    let row_bytes = 4 + dim * elem_size;
    if !file_len.is_multiple_of(row_bytes) {
        bail!("file size {file_len} is not a multiple of the row size {row_bytes} (dim {dim})");
    }

    Ok((file_len / row_bytes, dim))
}

fn fvecs_to_fbin(input: &PathBuf, output: &PathBuf) -> anyhow::Result<()> {
    let mut reader = File::open(input).with_context(|| format!("open {input:?}"))?;
    let (nrows, dim) = vecs_shape(&mut reader, size_of::<f32>())?;
    tracing::info!("converting {nrows} x {dim} f32 vectors from {input:?}");
    reader.seek(SeekFrom::Start(0))?;

    let mut writer =
        BufWriter::new(File::create(output).with_context(|| format!("create {output:?}"))?);
    Metadata::new(nrows as u32, dim as u32)?.write(&mut writer)?;

    // Convert in chunks to bound peak memory on multi-GB inputs.
    let row_bytes = 4 + dim * size_of::<f32>();
    let chunk_rows = (64 * 1024 * 1024 / row_bytes).max(1);
    let mut raw_chunk = vec![0u8; chunk_rows * row_bytes];
    let mut out_chunk: Vec<f32> = Vec::with_capacity(chunk_rows * dim);

    let mut rows_done = 0;
    while rows_done < nrows {
        let rows = (nrows - rows_done).min(chunk_rows);
        reader.read_exact(&mut raw_chunk[..rows * row_bytes])?;

        out_chunk.clear();
        for row in raw_chunk[..rows * row_bytes].chunks_exact(row_bytes) {
            let payload: &[f32] = cast_slice(&row[4..]);
            out_chunk.extend_from_slice(payload);
        }
        writer.write_all(cast_slice(&out_chunk))?;

        rows_done += rows;
    }

    writer.flush()?;
    tracing::info!("wrote {rows_done} vectors to {output:?}");
    Ok(())
}

fn ivecs_to_ground_truth(
    input: &PathBuf,
    base: &PathBuf,
    query: &PathBuf,
    output: &PathBuf,
) -> anyhow::Result<()> {
    let mut reader = File::open(input).with_context(|| format!("open {input:?}"))?;
    let (num_queries, k) = vecs_shape(&mut reader, size_of::<i32>())?;
    tracing::info!("converting {num_queries} x {k} ground-truth rows from {input:?}");
    reader.seek(SeekFrom::Start(0))?;

    // The ids of all queries fit comfortably in memory (num_queries * k * 4 bytes).
    let row_bytes = 4 + k * size_of::<i32>();
    let mut raw = vec![0u8; num_queries * row_bytes];
    reader.read_exact(&mut raw)?;

    let mut gt_ids: Vec<u32> = Vec::with_capacity(num_queries * k);
    for row in raw.chunks_exact(row_bytes) {
        let payload: &[i32] = cast_slice(&row[4..]);
        for &id in payload {
            if id < 0 {
                bail!("negative neighbor id {id}");
            }
            gt_ids.push(id as u32);
        }
    }

    // Load base and query vectors to compute exact distances for the ground-truth ids.
    let mut base_reader = File::open(base).with_context(|| format!("open {base:?}"))?;
    let base = read_bin::<f32>(&mut base_reader).with_context(|| format!("read {base:?}"))?;
    let mut query_reader = File::open(query).with_context(|| format!("open {query:?}"))?;
    let queries = read_bin::<f32>(&mut query_reader).with_context(|| format!("read {query:?}"))?;

    if queries.nrows() != num_queries {
        bail!(
            "query count {} does not match ground-truth rows {num_queries}",
            queries.nrows()
        );
    }
    if queries.ncols() != base.ncols() {
        bail!(
            "query dim {} does not match base dim {}",
            queries.ncols(),
            base.ncols()
        );
    }

    let dim = base.ncols();
    let distance = <f32 as DistanceProvider<f32>>::distance_comparer(Metric::L2, Some(dim));

    let base_view = base.as_view();
    let query_view = queries.as_view();
    let mut gt_distances: Vec<f32> = Vec::with_capacity(num_queries * k);
    for (q, ids) in gt_ids.chunks_exact(k).enumerate() {
        let query_row = query_view.row(q);
        for &id in ids {
            let id = id as usize;
            if id >= base.nrows() {
                bail!(
                    "ground-truth id {id} out of range for {} base vectors",
                    base.nrows()
                );
            }
            gt_distances.push(distance.call(query_row, base_view.row(id)));
        }
    }

    let mut writer =
        BufWriter::new(File::create(output).with_context(|| format!("create {output:?}"))?);
    Metadata::new(num_queries as u32, k as u32)?.write(&mut writer)?;
    writer.write_all(cast_slice(&gt_ids))?;
    writer.write_all(cast_slice(&gt_distances))?;
    writer.flush()?;

    tracing::info!("wrote ground truth for {num_queries} queries (k={k}) to {output:?}");
    Ok(())
}
