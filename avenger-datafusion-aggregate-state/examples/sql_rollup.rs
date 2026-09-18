//! Run with `cargo run -p avenger-datafusion-aggregate-state --example sql_rollup`.
mod common;

use datafusion::common::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut ctx = common::context()?;
    avenger_datafusion_aggregate_state::register_all(&mut ctx)?;

    let sql = "SELECT airline, delay_cell, countState() AS n,
                      avgState(distance) AS a, varSampState(distance) AS v
               FROM flights GROUP BY airline, delay_cell";
    println!("Build reusable cells:\n{sql}\n");
    common::materialize(&ctx, "cells", ctx.sql(sql).await?.into_unoptimized_plan()).await?;
    let summary = "SELECT airline, delay_cell, countFinalize(n) AS flights,
                          avgFinalize(a) AS average_distance, varSampFinalize(v) AS variance
                   FROM cells ORDER BY airline, delay_cell";
    common::show(
        &ctx,
        "Each cell, finalized for display:",
        ctx.sql(summary).await?.into_unoptimized_plan(),
    )
    .await?;

    // The same collected cell table serves both the initial brush and its drag.
    for (label, lo, hi) in [("Initial brush", 0, 1), ("Dragged brush", 1, 2)] {
        let sql = format!(
            "SELECT airline, countMerge(n) AS flights,
                                 avgMerge(a) AS average_distance, varSampMerge(v) AS variance
                          FROM cells WHERE delay_cell BETWEEN {lo} AND {hi}
                          GROUP BY airline ORDER BY airline"
        );
        println!("{label}: cells {lo} through {hi}\n{sql}\n");
        let actual = common::show(
            &ctx,
            "Merged result:",
            ctx.sql(&sql).await?.into_unoptimized_plan(),
        )
        .await?;
        let native = format!("SELECT airline, count(*) AS flights,
                                    avg(distance) AS average_distance, var_samp(distance) AS variance
                             FROM flights WHERE delay_cell BETWEEN {lo} AND {hi}
                             GROUP BY airline ORDER BY airline");
        common::verify(
            &ctx,
            &actual,
            ctx.sql(&native).await?.into_unoptimized_plan(),
        )
        .await?;
    }

    let sql = "SELECT airline, avgMergeState(a) AS a, varSampMergeState(v) AS v
               FROM cells GROUP BY airline";
    println!("Retain a second level of reusable states:\n{sql}\n");
    common::materialize(
        &ctx,
        "airline_states",
        ctx.sql(sql).await?.into_unoptimized_plan(),
    )
    .await?;
    let sql =
        "SELECT avgMerge(a) AS average_distance, varSampMerge(v) AS variance FROM airline_states";
    let actual = common::show(&ctx, sql, ctx.sql(sql).await?.into_unoptimized_plan()).await?;
    let native =
        "SELECT avg(distance) AS average_distance, var_samp(distance) AS variance FROM flights";
    common::verify(
        &ctx,
        &actual,
        ctx.sql(native).await?.into_unoptimized_plan(),
    )
    .await?;
    Ok(())
}
