//! The "server" side: owns the data. Generates a local parquet file on
//! first run, compiles the chart over it, BAKES it (folding the parquet
//! rows into the artifact), and writes the self-contained artifact for the
//! client to render with no data access.

use std::{error::Error, sync::Arc};

use avenger_chart::bake::{BakePolicy, ContextBakeStatus};
use chart_bake_server_client::{
    SALES_QUERY, artifact_path, data_dir, init_logging, parquet_path, sales_threshold_chart,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    dataframe::DataFrameWriteOptions,
    prelude::SessionContext,
};

const REGIONS: &[&str] = &[
    "APAC", "Benelux", "DACH", "Iberia", "LATAM", "Nordics", "UK", "US",
];
const ROWS_PER_REGION: usize = 1_250;

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let parquet = parquet_path();
    if !parquet.exists() {
        write_sales_parquet().await?;
        println!("generated {}", parquet.display());
    }

    // NOTE: the chart compiles over an in-memory copy of the parquet rows
    // rather than the parquet scan itself — round-tripping ParquetFormat
    // through the logical-plan codec is broken in DataFusion 54 (compile
    // fails on plan serialization). Once that is fixed upstream this can
    // become `ctx.register_parquet("sales", ...)` directly.
    let ctx = SessionContext::new();
    let parquet_df = ctx
        .read_parquet(
            parquet.to_string_lossy().as_ref(),
            datafusion::prelude::ParquetReadOptions::default(),
        )
        .await?;
    let batches = parquet_df.collect().await?;
    let schema = batches[0].schema();
    let table = datafusion::datasource::MemTable::try_new(schema, vec![batches])?;
    ctx.register_table("sales", Arc::new(table))?;

    let data = ctx.sql(SALES_QUERY).await?;
    let compiled = sales_threshold_chart(data).compile(&ctx).await?;
    let (baked, report) = compiled.bake(&ctx, &BakePolicy::default()).await?;

    println!("bake report:");
    println!("  source tables folded: {:?}", report.source_tables);
    println!("  remaining params: {:?}", report.remaining_params);
    println!("  self contained: {}", report.self_contained);
    for status in &report.contexts {
        match status {
            ContextBakeStatus::Baked {
                context_id,
                primary_table,
                self_contained,
                ..
            } => println!(
                "  baked {context_id:?} -> {primary_table} (self-contained: {self_contained})"
            ),
            ContextBakeStatus::NotBaked { context_id, reason } => {
                println!("  live  {context_id:?}: {reason:?}")
            }
        }
    }

    let artifact = artifact_path();
    std::fs::create_dir_all(artifact.parent().expect("artifact dir"))?;
    std::fs::write(&artifact, bincode::serialize(&baked)?)?;
    println!(
        "wrote {} ({} bytes) — run the client: cargo run --release -p chart-bake-server-client --bin client",
        artifact.display(),
        std::fs::metadata(&artifact)?.len()
    );
    Ok(())
}

/// Deterministic pseudo-random sales rows (no rand dependency).
async fn write_sales_parquet() -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(data_dir())?;
    let mut regions = Vec::with_capacity(REGIONS.len() * ROWS_PER_REGION);
    let mut values = Vec::with_capacity(REGIONS.len() * ROWS_PER_REGION);
    let mut state: u64 = 0x5DEECE66D;
    for (region_index, region) in REGIONS.iter().enumerate() {
        for _ in 0..ROWS_PER_REGION {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = ((state >> 11) as f64) / ((1u64 << 53) as f64);
            regions.push(*region);
            // Give each region a different value profile so the thresholded
            // totals reorder as $min sweeps.
            let skew = 0.5 + region_index as f64 * 0.85;
            values.push((unit.powf(1.0 / skew) * 100.0 * 100.0).round() / 100.0);
        }
    }
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(regions)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )?;

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch)?;
    df.write_parquet(
        parquet_path().to_string_lossy().as_ref(),
        DataFrameWriteOptions::new(),
        None,
    )
    .await?;
    Ok(())
}
