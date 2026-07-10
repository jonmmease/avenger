//! The "server" side: owns the data. Generates a local parquet file of raw
//! events on first run, compiles the daily-totals chart over it, BAKES it
//! (the param-free aggregate executes once and only its few thousand output
//! rows embed in the artifact), and writes the self-contained artifact for
//! the client to render with no data access.
//!
//! Before writing the artifact it prints a measured three-way comparison of
//! what a client interaction costs under different architectures:
//!
//!   1. thin client — every param change is a server query over parquet;
//!   2. unbaked spec — the self-contained spec ships WITHOUT baking, so it
//!      embeds every raw row and re-aggregates them per interaction;
//!   3. baked spec — the aggregate is pre-evaluated server-side; the client
//!      filters a few thousand pre-aggregated rows per interaction.

use std::{error::Error, sync::Arc, time::Instant};

use avenger_chart::{
    bake::{BakePolicy, ContextBakeStatus},
    plot::CompiledPlot,
};
use chart_bake_server_client::{
    artifact_path, daily_totals_chart, data_dir, init_logging, parquet_path,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrameWriteOptions,
    functions_aggregate::expr_fn::sum,
    prelude::{ParquetReadOptions, SessionContext, col, placeholder},
};
use indexmap::IndexMap;

/// The pipeline as SQL, used for the thin-client baseline query over
/// parquet. The chart builds the same pipeline over its own source.
const TRIPS_QUERY: &str = "SELECT * FROM (\
       SELECT region, day, SUM(value) AS total \
       FROM trips GROUP BY region, day) t \
     WHERE total > $min ORDER BY region, day";

const REGIONS: &[&str] = &[
    "APAC", "Benelux", "DACH", "Iberia", "LATAM", "Nordics", "UK", "US",
];
const DAYS: usize = 365;
const EVENTS_PER_REGION_DAY: usize = 1_370;
/// `$min` values used for the timing comparison (the interactive sweep
/// covers 0..100k).
const TIMED_PARAMS: &[f64] = &[25_000.0, 50_000.0, 75_000.0];

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let parquet = parquet_path();
    if !parquet.exists() {
        print!(
            "generating {} raw events... ",
            REGIONS.len() * DAYS * EVENTS_PER_REGION_DAY
        );
        write_trips_parquet().await?;
        println!("done");
    }
    let parquet_bytes = std::fs::metadata(&parquet)?.len();
    println!(
        "data: {} ({} rows, {:.1} MB)",
        parquet.display(),
        REGIONS.len() * DAYS * EVENTS_PER_REGION_DAY,
        parquet_bytes as f64 / 1e6
    );

    // ── Architecture 1: thin client — a server query over parquet per
    // interaction (no chart machinery involved).
    let parquet_ctx = SessionContext::new();
    parquet_ctx
        .register_parquet(
            "trips",
            parquet.to_string_lossy().as_ref(),
            ParquetReadOptions::default(),
        )
        .await?;
    let query_over_parquet = median_ms(|min| {
        let ctx = parquet_ctx.clone();
        async move {
            let rows = ctx
                .sql(TRIPS_QUERY)
                .await?
                .with_param_values(vec![("min", ScalarValue::Float64(Some(min)))])?
                .collect()
                .await?;
            Ok(rows.iter().map(|b| b.num_rows()).sum::<usize>())
        }
    })
    .await?;

    // ── The compiled chart: the same pipeline built over an UNNAMED
    // in-memory scan of the parquet rows. Unnamed scans serialize INLINE
    // (Arrow IPC bytes inside the plan), so the unbaked spec below is
    // genuinely self-contained — it ships every raw row. Two notes:
    // - a named registered table would serialize as a reference and the
    //   spec would silently depend on the consuming session having it;
    // - compiling over the parquet scan directly fails — DataFusion 54's
    //   logical-plan codec cannot round-trip ParquetFormat. Once fixed
    //   upstream, `register_parquet` becomes the natural source here.
    let server_ctx = SessionContext::new();
    let batches = parquet_ctx
        .read_parquet(
            parquet.to_string_lossy().as_ref(),
            ParquetReadOptions::default(),
        )
        .await?
        .collect()
        .await?;
    let schema = batches[0].schema();
    let raw_rows = concat_batches(&schema, &batches)?;
    let data = server_ctx
        .read_batch(raw_rows)?
        .aggregate(
            vec![col("region"), col("day")],
            vec![sum(col("value")).alias("total")],
        )?
        .filter(col("total").gt(placeholder("$min")))?
        .sort(vec![
            col("region").sort(true, false),
            col("day").sort(true, false),
        ])?;
    let compiled = daily_totals_chart(data).compile(&server_ctx).await?;

    // ── Architecture 2: the unbaked self-contained spec. It embeds every
    // raw row and re-runs the aggregate per interaction.
    let unbaked_size = bincode::serialize(&compiled)?.len();
    let unbaked_eval = median_ms(|min| {
        let compiled = &compiled;
        let ctx = &server_ctx;
        async move {
            let outcome = compiled.evaluate(ctx, Some(min_params(min))).await?;
            Ok(outcome.scene_graph.marks.len())
        }
    })
    .await?;

    // ── Architecture 3: bake, then evaluate the artifact in a session with
    // zero data access.
    let bake_start = Instant::now();
    let (baked, report) = compiled.bake(&server_ctx, &BakePolicy::default()).await?;
    let bake_elapsed = bake_start.elapsed();

    println!("\nbake report ({} ms):", bake_elapsed.as_millis());
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

    let artifact_bytes = bincode::serialize(&baked)?;
    let artifact = artifact_path();
    std::fs::create_dir_all(artifact.parent().expect("artifact dir"))?;
    std::fs::write(&artifact, &artifact_bytes)?;

    // The client's view: deserialize into a FRESH session. The first
    // evaluation pays the one-time manifest decode; the decoded tables are
    // cached on the plot instance afterwards.
    let client_ctx = SessionContext::new();
    let client_plot: CompiledPlot = bincode::deserialize(&artifact_bytes)?;
    let first = Instant::now();
    client_plot
        .evaluate(&client_ctx, Some(min_params(TIMED_PARAMS[0])))
        .await?;
    let baked_first_ms = first.elapsed().as_secs_f64() * 1e3;
    let baked_eval = median_ms(|min| {
        let plot = &client_plot;
        let ctx = &client_ctx;
        async move {
            let outcome = plot.evaluate(ctx, Some(min_params(min))).await?;
            Ok(outcome.scene_graph.marks.len())
        }
    })
    .await?;

    println!("\nper-interaction cost of a $min change:");
    println!("  architecture                    ships to client    per interaction");
    println!(
        "  1. thin client (server query)   {:>10}         {:>7.1} ms + round trip",
        format_bytes(parquet_bytes as usize),
        query_over_parquet
    );
    println!(
        "  2. unbaked spec (rows embedded) {:>10}         {:>7.1} ms",
        format_bytes(unbaked_size),
        unbaked_eval
    );
    println!(
        "  3. baked spec (pre-evaluated)   {:>10}         {:>7.1} ms  (first eval {:.1} ms incl. decode)",
        format_bytes(artifact_bytes.len()),
        baked_eval,
        baked_first_ms
    );
    println!(
        "\nwrote {} — run the client: cargo run --release -p chart-bake-server-client --bin client",
        artifact.display()
    );
    Ok(())
}

fn min_params(min: f64) -> IndexMap<String, ScalarValue> {
    IndexMap::from([("min".to_string(), ScalarValue::Float64(Some(min)))])
}

/// Median wall-clock milliseconds of `work` across the timed params, after
/// one warmup call.
async fn median_ms<F, Fut>(work: F) -> Result<f64, Box<dyn Error>>
where
    F: Fn(f64) -> Fut,
    Fut: Future<Output = Result<usize, Box<dyn Error>>>,
{
    work(TIMED_PARAMS[0]).await?;
    let mut samples = Vec::with_capacity(TIMED_PARAMS.len());
    for &min in TIMED_PARAMS {
        let start = Instant::now();
        let _rows = work(min).await?;
        samples.push(start.elapsed());
    }
    samples.sort();
    Ok(samples[samples.len() / 2].as_secs_f64() * 1e3)
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} KB", bytes as f64 / 1e3)
    }
}

/// Deterministic pseudo-random trip events (no rand dependency): every
/// (region, day) gets `EVENTS_PER_REGION_DAY` events whose values follow a
/// region-specific profile with a seasonal swing, so daily totals spread
/// across the 0..100k `$min` sweep.
async fn write_trips_parquet() -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(data_dir())?;
    let total_rows = REGIONS.len() * DAYS * EVENTS_PER_REGION_DAY;
    let mut regions = Vec::with_capacity(total_rows);
    let mut days = Vec::with_capacity(total_rows);
    let mut values = Vec::with_capacity(total_rows);
    let mut state: u64 = 0x5DEECE66D;
    for (region_index, region) in REGIONS.iter().enumerate() {
        let region_scale = 0.35 + region_index as f64 * 0.09;
        for day in 0..DAYS {
            let season = 1.0 + 0.35 * (day as f64 / DAYS as f64 * std::f64::consts::TAU).sin();
            for _ in 0..EVENTS_PER_REGION_DAY {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let unit = ((state >> 11) as f64) / ((1u64 << 53) as f64);
                regions.push(*region);
                days.push(day as f64);
                values.push(unit * 100.0 * region_scale * season);
            }
        }
    }
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("day", DataType::Float64, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(regions)) as _,
            Arc::new(Float64Array::from(days)) as _,
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
