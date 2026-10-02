//! Reuse separate extent and bin calculations while changing a brush or bin option.
use avenger_datafusion_dataflow::{
    CacheConfig, CachePolicy, DataflowBuilder, Result, Runtime, RuntimeConfig, TableSnapshot,
};
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
    let batch = RecordBatch::try_from_iter([(
        "delay",
        Arc::new(Float64Array::from(vec![-5.0, 0.0, 4.0, 12.0, 18.0, 29.0])) as ArrayRef,
    )])?;
    let mut flow = DataflowBuilder::new();
    let rows = flow.table_snapshot(
        "flights",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let maxbins = flow.scalar_input("maxbins", DataType::Float64)?;
    let selection = flow.expr_input("selection", DataType::Boolean)?;
    let extent = flow.add_plan(
        "delay_extent",
        transform::extent(rows.plan_ref(), col("delay"))?,
    )?;
    let extent = flow.add_scalar(
        "delay_extent_value",
        scalar_subquery(Arc::new(extent.plan_ref())),
    )?;
    let parameters = flow.add_scalar(
        "delay_bin_parameters",
        transform::bin_parameters(
            extent.expr_ref(),
            BinOptions {
                maxbins: Some(maxbins.expr_ref()),
                ..Default::default()
            },
        )?,
    )?;
    let bins = flow.add_plan(
        "binned_flights",
        transform::bin(
            rows.plan_ref(),
            col("delay"),
            parameters.expr_ref(),
            ["delay_start", "delay_end"],
        )?,
    )?;
    let counts = transform::aggregate(
        transform::filter(bins.plan_ref(), selection.expr_ref())?,
        vec![col("delay_start"), col("delay_end")],
        vec![expr_fn::count().alias("flights")],
    )?;
    let counts = flow.add_plan(
        "delay_counts",
        LogicalPlanBuilder::from(counts)
            .sort(vec![col("delay_start").sort(true, true)])?
            .build()?,
    )?;
    let output = flow.table_output("histogram", &counts)?;
    let parameters_output = flow.scalar_output("parameters", &parameters)?;
    let dataflow = flow.finish()?;
    println!("Bin parameters:\n{}\n", dataflow.sql().scalar(&parameters)?);
    println!("Binning SQL:\n{}\n", dataflow.sql().table(&bins)?);
    println!("Histogram SQL:\n{}\n", dataflow.sql().table(&counts)?);
    let prepared = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 256,
        }),
        ..Default::default()
    })?
    .prepare(&dataflow)
    .await?;
    for (label, bins, predicate) in [
        ("Initial", 6.0, lit(true)),
        ("Brush: delay >= 0", 6.0, col("delay").gt_eq(lit(0.0))),
        ("Drag: delay >= 10", 6.0, col("delay").gt_eq(lit(10.0))),
        ("Change maxbins to 3", 3.0, col("delay").gt_eq(lit(10.0))),
        ("Return to initial inputs", 6.0, lit(true)),
    ] {
        let inputs = prepared
            .inputs()
            .scalar(&maxbins, bins.into())?
            .expr(&selection, predicate)?
            .finish()?;
        let result = prepared
            .query(&[output], &[parameters_output], &inputs)
            .await?;
        println!(
            "{label}\nParameters: {}",
            result.scalar(&parameters_output)?
        );
        println!(
            "{}",
            pretty_format_batches(result.table(&output)?.batches())?
        );
        println!(
            "Executed nodes: {:?}\nCache hits: {}\n",
            result.report().executed_nodes,
            result.report().cache_hits
        );
    }
    Ok(())
}
