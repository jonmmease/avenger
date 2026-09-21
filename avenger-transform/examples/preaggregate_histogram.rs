//! Warm aggregate states before brushing, then reuse them through the dataflow cache.
use avenger_datafusion_dataflow::{
    CacheConfig, CachePolicy, DataflowBuilder, Result, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_datafusion_preaggregate::{dataflow::Query, FilterQuery, PreaggregatePlanner};
use avenger_transform::{self as transform, expr_fn, BinOptions};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array},
        datatypes::DataType,
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    logical_expr::{col, lit, scalar_subquery, LogicalPlanBuilder},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let batch = RecordBatch::try_from_iter([
        (
            "delay",
            Arc::new(Float64Array::from(vec![0.0, 10.0, 20.0, 0.0, 10.0, 20.0])) as ArrayRef,
        ),
        (
            "distance",
            Arc::new(Float64Array::from(vec![
                100.0, 200.0, 300.0, 100.0, 200.0, 600.0,
            ])) as ArrayRef,
        ),
    ])?;
    let mut flow = DataflowBuilder::new();
    let rows = flow.table_snapshot(
        "flights",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let fixed = flow.expr_input("fixed_filter", DataType::Boolean)?;
    let step = flow.scalar_input("distance_step", DataType::Float64)?;
    let extent = flow.add_plan(
        "distance_extent",
        transform::extent(rows.plan_ref(), col("distance"))?,
    )?;
    let parameters = flow.add_scalar(
        "distance_bins",
        transform::bin_parameters(
            scalar_subquery(Arc::new(extent.plan_ref())),
            BinOptions {
                step: Some(step.expr_ref()),
                ..Default::default()
            },
        )?,
    )?;
    let bins = flow.add_plan(
        "binned_flights",
        transform::bin(
            rows.plan_ref(),
            col("distance"),
            parameters.expr_ref(),
            ["lo", "hi"],
        )?,
    )?;
    // Build the visible aggregate section before registering it as a graph node.
    // Delay remains an exact retained dimension, independent of distance display bins.
    let query = FilterQuery::new(
        transform::filter(bins.plan_ref(), fixed.expr_ref())?,
        |rows| {
            let counts = transform::aggregate(
                rows,
                vec![col("lo"), col("hi")],
                vec![
                    expr_fn::count().alias("flights"),
                    expr_fn::mean(col("distance")).alias("mean_distance"),
                ],
            )?;
            LogicalPlanBuilder::from(counts)
                .sort(vec![col("lo").sort(true, true)])?
                .build()
        },
    )?;
    let family = PreaggregatePlanner::default().prepare(query, vec![col("delay")])?;
    let histogram = Query::install(&mut flow, "histogram", family)?;
    let warmup = histogram
        .materialization_output()
        .expect("count and mean are eligible");
    let idle = histogram.bind(lit(true))?;
    let dataflow = flow.finish()?;
    println!("Warm-up SQL:\n{}\n", dataflow.sql().table_output(&warmup)?);
    println!(
        "Rollup SQL:\n{}\n",
        dataflow.sql().table_output(&idle.output())?
    );
    let prepared = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 256,
        }),
        ..Default::default()
    })?
    .prepare(&dataflow)
    .await?;
    let inputs = idle
        .apply(
            prepared
                .inputs()
                .scalar(&step, 200.0.into())?
                .expr(&fixed, lit(true))?,
        )?
        .finish()?;
    let warm = prepared.query(&[warmup], &[], &inputs).await?;
    println!(
        "Hover before a selection: {} state rows\nExecuted nodes: {:?}\n",
        warm.table(&warmup)?.num_rows(),
        warm.report().executed_nodes
    );
    for (label, predicate, fixed_filter, width) in [
        (
            "Brush delay [0, 15)",
            col("delay").lt(lit(15.0)),
            lit(true),
            200.0,
        ),
        (
            "Drag to delay [10, 25)",
            col("delay")
                .gt_eq(lit(10.0))
                .and(col("delay").lt(lit(25.0))),
            lit(true),
            200.0,
        ),
        (
            "Change another view's filter",
            col("delay").lt(lit(15.0)),
            col("distance").lt(lit(500.0)),
            200.0,
        ),
        (
            "Change display bins",
            col("delay").lt(lit(15.0)),
            lit(true),
            100.0,
        ),
        (
            "Unretained distance predicate",
            col("distance").lt(lit(250.0)),
            lit(true),
            200.0,
        ),
    ] {
        let binding = histogram.bind(predicate)?;
        let inputs = binding
            .apply(
                inputs
                    .edit()
                    .scalar(&step, width.into())?
                    .expr(&fixed, fixed_filter)?,
            )?
            .finish()?;
        let result = prepared.query(&[binding.output()], &[], &inputs).await?;
        println!(
            "{label}: {:?}\n{}",
            binding.diagnostics(),
            pretty_format_batches(result.table(&binding.output())?.batches())?
        );
        println!(
            "Executed nodes: {:?}\nCache hits: {}\n",
            result.report().executed_nodes,
            result.report().cache_hits
        );
    }
    Ok(())
}
