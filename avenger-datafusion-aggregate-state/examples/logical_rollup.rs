//! Run with `cargo run -p avenger-datafusion-aggregate-state --example logical_rollup`.
mod common;

use avenger_datafusion_aggregate_state::expr_fn::{
    avg_finalize, avg_merge, avg_merge_state, avg_state, var_samp_finalize, var_samp_merge,
    var_samp_merge_state, var_samp_state,
};
use datafusion::{
    common::Result,
    functions_aggregate::expr_fn::{avg, var_sample},
    logical_expr::{col, lit, Expr, LogicalPlanBuilder},
};

#[tokio::main]
async fn main() -> Result<()> {
    // Function objects travel with these expressions; SQL registration is unnecessary.
    let ctx = common::context()?;
    let raw = ctx.table("flights").await?.into_unoptimized_plan();
    let cells = LogicalPlanBuilder::from(raw.clone())
        .aggregate(
            vec![col("airline"), col("delay_cell")],
            vec![
                avg_state(col("distance")).alias("a"),
                var_samp_state(col("distance")).alias("v"),
            ],
        )?
        .build()?;
    println!("Build reusable cells:\n{}\n", cells.display_indent());
    common::materialize(&ctx, "cells", cells).await?;
    let cells = ctx.table("cells").await?.into_unoptimized_plan();

    for (label, lo, hi) in [("Initial brush", 0, 1), ("Dragged brush", 1, 2)] {
        let brush = col("delay_cell")
            .gt_eq(lit(lo))
            .and(col("delay_cell").lt_eq(lit(hi)));
        let merged = LogicalPlanBuilder::from(cells.clone())
            .filter(brush.clone())?
            .aggregate(
                vec![col("airline")],
                vec![
                    avg_merge(col("a")).alias("average_distance"),
                    var_samp_merge(col("v")).alias("variance"),
                ],
            )?
            .sort(vec![col("airline").sort(true, false)])?
            .build()?;
        println!(
            "{label}: cells {lo} through {hi}\n{}\n",
            merged.display_indent()
        );
        let actual = common::show(&ctx, "Merged result:", merged).await?;
        let native = LogicalPlanBuilder::from(raw.clone())
            .filter(brush)?
            .aggregate(
                vec![col("airline")],
                vec![
                    avg(col("distance")).alias("average_distance"),
                    var_sample(col("distance")).alias("variance"),
                ],
            )?
            .sort(vec![col("airline").sort(true, false)])?
            .build()?;
        common::verify(&ctx, &actual, native).await?;
    }

    let rollup = LogicalPlanBuilder::from(cells)
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                avg_merge_state(col("a")).alias("a"),
                var_samp_merge_state(col("v")).alias("v"),
            ],
        )?
        .build()?;
    common::materialize(&ctx, "total_state", rollup).await?;
    let finalized =
        LogicalPlanBuilder::from(ctx.table("total_state").await?.into_unoptimized_plan())
            .project(vec![
                avg_finalize(col("a")).alias("average_distance"),
                var_samp_finalize(col("v")).alias("variance"),
            ])?
            .build()?;
    let actual = common::show(&ctx, "Finalize the retained total state:", finalized).await?;
    let native = LogicalPlanBuilder::from(raw)
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                avg(col("distance")).alias("average_distance"),
                var_sample(col("distance")).alias("variance"),
            ],
        )?
        .build()?;
    common::verify(&ctx, &actual, native).await?;
    Ok(())
}
