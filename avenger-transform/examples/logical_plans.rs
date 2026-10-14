//! Build and execute transforms directly, without a dataflow runtime.
use avenger_transform::{self as transform, expr_fn, BinOptions};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    common::Result,
    logical_expr::{col, lit, scalar_subquery, LogicalPlanBuilder},
    prelude::SessionContext,
    sql::unparser::Unparser,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_from_iter([(
        "delay",
        Arc::new(Float64Array::from(vec![
            Some(-5.0),
            Some(0.0),
            Some(4.0),
            Some(12.0),
            Some(18.0),
            Some(29.0),
            None,
        ])) as ArrayRef,
    )])?;
    ctx.register_batch("flights", batch)?;
    let rows = ctx.table("flights").await?.into_unoptimized_plan();
    let extent = transform::extent(rows.clone(), col("delay"))?;
    let parameters = transform::bin_parameters(
        scalar_subquery(Arc::new(extent.clone())),
        BinOptions {
            maxbins: Some(lit(6.0)),
            ..Default::default()
        },
    )?;
    let bins = transform::bin(
        rows,
        col("delay"),
        parameters.clone(),
        ["delay_start", "delay_end"],
    )?;
    let rows = transform::formula(bins, col("delay") / lit(60.0), "delay_hours")?;
    let rows = transform::filter(rows, col("delay").gt_eq(lit(0.0)))?;
    let counts = transform::aggregate(
        rows,
        vec![col("delay_start"), col("delay_end")],
        vec![
            expr_fn::count().alias("flights"),
            expr_fn::mean(col("delay_hours")).alias("mean_hours"),
        ],
    )?;
    let counts = LogicalPlanBuilder::from(counts)
        .sort(vec![col("delay_start").sort(true, true)])?
        .build()?;
    let sql = Unparser::default().with_pretty(true);
    println!("Extent SQL:\n{:#}\n", sql.plan_to_sql(&extent)?);
    println!(
        "Bin parameter expression:\n{:#}\n",
        sql.expr_to_sql(&parameters)?
    );
    println!("Histogram SQL:\n{:#}\n", sql.plan_to_sql(&counts)?);
    let result = ctx.execute_logical_plan(counts).await?.collect().await?;
    println!("Nonnegative delays:\n{}", pretty_format_batches(&result)?);
    Ok(())
}
