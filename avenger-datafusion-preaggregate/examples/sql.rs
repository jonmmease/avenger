//! Compose filtered computed measures, a window, top-k, and an outer aggregate.
//! Run with `cargo run -p avenger-datafusion-preaggregate --example sql`.
mod support;
use avenger_datafusion_preaggregate::{FilterQuery, PreaggregatePlanner};
use datafusion::{common::Result, datasource::view::ViewTable, logical_expr::col};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = support::context()?;
    let builder = FilterQuery::builder(ctx.table("flights").await?.into_unoptimized_plan());
    ctx.register_table(
        "selected_rows",
        Arc::new(ViewTable::new(builder.rows(), None)),
    )?;
    let parsed = ctx
        .sql(
            "SELECT AVG(running_distance) AS mean_running_distance
         FROM (
           SELECT airline,
             SUM(SUM(distance * 2.0) FILTER (WHERE domestic)) OVER (
               ORDER BY airline ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
             ) AS running_distance
           FROM selected_rows
           WHERE distance IS NOT NULL
           GROUP BY airline
           HAVING COUNT(*) > 0
           ORDER BY airline
           LIMIT 2 OFFSET 1
         ) ranked",
        )
        .await;
    ctx.deregister_table("selected_rows")?;
    let query = builder.finish(parsed?.into_unoptimized_plan())?;
    let prepared = PreaggregatePlanner::default().prepare(query, vec![col("delay")])?;
    support::demonstrate(&ctx, prepared).await
}
