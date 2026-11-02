mod common;
use common::{compare, context, materialize};
use datafusion::common::Result;

/// Average states keep the sum type of each input type, through a merge and a rollup.
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
            "SELECT g, cell, avg_state(x) AS s FROM typed GROUP BY g, cell",
        )
        .await?;
        compare(
            &ctx,
            "SELECT g, avg_merge(s) AS v FROM states GROUP BY g ORDER BY g",
            "SELECT g, avg(x) AS v FROM typed GROUP BY g ORDER BY g",
        )
        .await?;
        materialize(
            &ctx,
            "rolled",
            "SELECT g, avg_merge_state(s) AS s FROM states GROUP BY g",
        )
        .await?;
        compare(
            &ctx,
            "SELECT g, avg_merge(s) AS v FROM rolled GROUP BY g ORDER BY g",
            "SELECT g, avg(x) AS v FROM typed GROUP BY g ORDER BY g",
        )
        .await
        .unwrap_or_else(|e| panic!("type {i}: {e}"));
    }
    Ok(())
}
