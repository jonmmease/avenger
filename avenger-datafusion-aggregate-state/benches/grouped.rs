//! Execute with `cargo bench -p avenger-datafusion-aggregate-state --bench grouped`.
//! Override workload size with AGG_STATE_ROWS and AGG_STATE_ITERATIONS.
use std::{hint::black_box, sync::Arc, time::Instant};

use datafusion::{
    arrow::{
        array::{BooleanArray, Float64Array, UInt32Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::Result,
    datasource::MemTable,
    physical_plan::collect,
    prelude::{SessionConfig, SessionContext},
};

async fn measure(ctx: &SessionContext, label: &str, sql: &str, iterations: usize) -> Result<()> {
    let frame = ctx.sql(sql).await?;
    let plan = frame.clone().create_physical_plan().await?;
    black_box(collect(plan, ctx.task_ctx()).await?);
    let mut elapsed = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        // RepartitionExec contains execution state; build a fresh plan per run.
        let plan = frame.clone().create_physical_plan().await?;
        let start = Instant::now();
        black_box(collect(plan, ctx.task_ctx()).await?);
        elapsed.push(start.elapsed().as_secs_f64() * 1000.);
    }
    elapsed.sort_by(f64::total_cmp);
    println!("{label:30} {:9.3} ms median", elapsed[iterations / 2]);
    Ok(())
}

async fn materialize(ctx: &SessionContext, name: &str, sql: &str) -> Result<()> {
    let frame = ctx.sql(sql).await?;
    let schema = Arc::new(frame.schema().as_arrow().clone());
    let batches = frame.collect().await?;
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    ctx.register_table(name, Arc::new(MemTable::try_new(schema, vec![batches])?))?;
    println!("{name}: {rows} materialized rows");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let rows: usize = std::env::var("AGG_STATE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);
    let iterations: usize = std::env::var("AGG_STATE_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    assert!(rows > 0 && iterations > 0);
    println!(
        "DataFusion 54.1.0; {rows} rows, 4 partitions, {iterations} timed runs after warm-up."
    );
    println!("Times include execution and collection; parsing and physical planning are excluded.");
    for groups in [128, 16_384] {
        println!("\n{groups} target groups, up to 8 interaction cells per group:");
        let mut ctx =
            SessionContext::new_with_config(SessionConfig::new().with_target_partitions(4));
        avenger_datafusion_aggregate_state::register_all(&mut ctx)?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("g", DataType::UInt32, false),
            Field::new("cell", DataType::UInt32, false),
            Field::new("x", DataType::Float64, false),
            Field::new("flag", DataType::Boolean, false),
        ]));
        let mut partitions = vec![vec![]; 4];
        for (index, start) in (0..rows).step_by(8192).enumerate() {
            let end = (start + 8192).min(rows);
            partitions[index % 4].push(RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(UInt32Array::from_iter_values(
                        (start..end).map(|i| (i % groups) as u32),
                    )),
                    Arc::new(UInt32Array::from_iter_values(
                        (start..end).map(|i| ((i / groups) % 8) as u32),
                    )),
                    Arc::new(Float64Array::from_iter_values(
                        (start..end).map(|i| ((i * 17) % 1009) as f64),
                    )),
                    Arc::new(BooleanArray::from_iter(
                        (start..end).map(|i| Some(i % 3 == 0)),
                    )),
                ],
            )?);
        }
        ctx.register_table("raw", Arc::new(MemTable::try_new(schema, partitions)?))?;
        let build = "SELECT g, cell, sumState(x) AS s, avgState(x) AS a, varSampState(x) AS v FROM raw GROUP BY g, cell";
        measure(
            &ctx,
            "native cell aggregates",
            "SELECT g, cell, sum(x), avg(x), var_samp(x) FROM raw GROUP BY g, cell",
            iterations,
        )
        .await?;
        measure(&ctx, "State cell aggregates", build, iterations).await?;
        materialize(&ctx, "cells", build).await?;
        measure(
            &ctx,
            "native raw aggregation",
            "SELECT g, sum(x), avg(x), var_samp(x) FROM raw GROUP BY g",
            iterations,
        )
        .await?;
        measure(
            &ctx,
            "Merge materialized cells",
            "SELECT g, sumMerge(s), avgMerge(a), varSampMerge(v) FROM cells GROUP BY g",
            iterations,
        )
        .await?;
        measure(&ctx, "native filtered aggregation", "SELECT g, avg(x) FILTER (WHERE cell < 4), var_samp(x) FILTER (WHERE cell < 4) FROM raw GROUP BY g", iterations).await?;
        measure(&ctx, "Merge filtered cells", "SELECT g, avgMerge(a) FILTER (WHERE cell < 4), varSampMerge(v) FILTER (WHERE cell < 4) FROM cells GROUP BY g", iterations).await?;
        measure(
            &ctx,
            "Finalize batched numeric",
            "SELECT avgFinalize(a), varSampFinalize(v) FROM cells",
            iterations,
        )
        .await?;
        measure(
            &ctx,
            "native boolean grouping",
            "SELECT g, min(flag) FROM raw GROUP BY g",
            iterations,
        )
        .await?;
        measure(
            &ctx,
            "State scalar group fallback",
            "SELECT g, minState(flag) FROM raw GROUP BY g",
            iterations,
        )
        .await?;
        materialize(
            &ctx,
            "booleans",
            "SELECT g, minState(flag) AS s FROM raw GROUP BY g",
        )
        .await?;
        measure(
            &ctx,
            "Finalize extrema payload",
            "SELECT minFinalize(s) FROM booleans",
            iterations,
        )
        .await?;
    }
    Ok(())
}
