mod common;
use common::{compare, context, materialize};
use datafusion::common::Result;

#[tokio::test]
async fn materialized_average_and_successive_rollups() -> Result<()> {
    for partitions in [1, 4] {
        let ctx = context(partitions)?;
        materialize(
            &ctx,
            "cells",
            "SELECT g, cell, avgState(x) AS s FROM t GROUP BY g, cell",
        )
        .await?;
        compare(
            &ctx,
            "SELECT g, avgMerge(s) AS v FROM cells WHERE cell <= 1 GROUP BY g ORDER BY g",
            "SELECT g, avg(x) AS v FROM t WHERE cell <= 1 GROUP BY g ORDER BY g",
        )
        .await?;
        materialize(
            &ctx,
            "groups",
            "SELECT g, avgMergeState(s) AS s FROM cells GROUP BY g",
        )
        .await?;
        materialize(&ctx, "total", "SELECT avgMergeState(s) AS s FROM groups").await?;
        compare(
            &ctx,
            "SELECT avgMerge(s) AS v FROM total",
            "SELECT avg(x) AS v FROM t",
        )
        .await?;
        compare(
            &ctx,
            "SELECT avgFinalize(s) AS v FROM total",
            "SELECT avg(x) AS v FROM t",
        )
        .await?;
        compare(
            &ctx,
            "SELECT avgMerge(s) AS v FROM cells WHERE false",
            "SELECT avg(x) AS v FROM t WHERE false",
        )
        .await?;
        materialize(&ctx, "empty", "SELECT avgState(x) AS s FROM t WHERE false").await?;
        compare(
            &ctx,
            "SELECT avgFinalize(s) AS v FROM empty",
            "SELECT avg(x) AS v FROM t WHERE false",
        )
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn typed_average_states() -> Result<()> {
    for (i, cast) in [
        "BIGINT",
        "BIGINT UNSIGNED",
        "REAL",
        "DOUBLE",
        "DECIMAL(12, 2)",
        "DECIMAL(30, 6)",
    ]
    .iter()
    .enumerate()
    {
        let ctx = context(3)?;
        materialize(
            &ctx,
            "typed",
            &format!("SELECT g, cell, CAST(abs(x) AS {cast}) AS x FROM t"),
        )
        .await?;
        materialize(
            &ctx,
            "states",
            "SELECT g, cell, avgState(x) AS s FROM typed GROUP BY g, cell",
        )
        .await?;
        compare(
            &ctx,
            "SELECT g, avgMerge(s) AS v FROM states GROUP BY g ORDER BY g",
            "SELECT g, avg(x) AS v FROM typed GROUP BY g ORDER BY g",
        )
        .await?;
        materialize(
            &ctx,
            "rolled",
            "SELECT g, avgMergeState(s) AS s FROM states GROUP BY g",
        )
        .await?;
        compare(
            &ctx,
            "SELECT g, avgFinalize(s) AS v FROM rolled ORDER BY g",
            "SELECT g, avg(x) AS v FROM typed GROUP BY g ORDER BY g",
        )
        .await
        .unwrap_or_else(|e| panic!("type {i}: {e}"));
    }
    Ok(())
}
