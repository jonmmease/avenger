//! Run with `cargo run -p avenger-datafusion-preaggregate --example logical_plans`.
mod support;
use avenger_datafusion_preaggregate::{FilterQuery, PreaggregatePlanner};
use datafusion::{
    common::Result,
    functions_aggregate::expr_fn::{avg, count},
    logical_expr::{col, lit, ExprFunctionExt, LogicalPlanBuilder},
};

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = support::context()?;
    let query = FilterQuery::new(
        ctx.table("flights").await?.into_unoptimized_plan(),
        |rows| {
            LogicalPlanBuilder::from(rows)
                .filter(col("distance").is_not_null())?
                .aggregate(
                    vec![col("airline")],
                    vec![
                        count(lit(1_i64)).alias("flights"),
                        avg(col("distance") * lit(2.0))
                            .filter(col("domestic"))
                            .build()?
                            .alias("mean_roundtrip"),
                    ],
                )?
                .sort(vec![col("airline").sort(true, true)])?
                .build()
        },
    )?;
    let prepared = PreaggregatePlanner::default().prepare(query, vec![col("delay")])?;
    support::demonstrate(&ctx, prepared).await
}
