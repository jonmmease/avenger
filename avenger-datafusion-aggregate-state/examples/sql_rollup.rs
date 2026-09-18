//! Run with `cargo run -p avenger-datafusion-aggregate-state --example sql_rollup`.
mod common;

use datafusion::common::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut ctx = common::context()?;
    avenger_datafusion_aggregate_state::register_all(&mut ctx)?;

    let sql = "SELECT airline, delay_cell, count_state() AS n,
                      avg_state(distance) AS a, var_samp_state(distance) AS v
               FROM flights GROUP BY airline, delay_cell";
    println!("Build reusable cells:\n{sql}\n");
    common::materialize(&ctx, "cells", ctx.sql(sql).await?.into_unoptimized_plan()).await?;

    // The same collected cell table serves both the initial brush and its drag.
    for (label, lo, hi) in [("Initial brush", 0, 1), ("Dragged brush", 1, 2)] {
        let sql = format!(
            "SELECT airline, count_merge(n) AS flights,
                                 avg_merge(a) AS average_distance, var_samp_merge(v) AS variance
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

    let sql = "SELECT airline, avg_merge_state(a) AS a, var_samp_merge_state(v) AS v
               FROM cells GROUP BY airline";
    println!("Retain a second level of reusable states:\n{sql}\n");
    common::materialize(
        &ctx,
        "airline_states",
        ctx.sql(sql).await?.into_unoptimized_plan(),
    )
    .await?;
    let sql =
        "SELECT avg_merge(a) AS average_distance, var_samp_merge(v) AS variance FROM airline_states";
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
