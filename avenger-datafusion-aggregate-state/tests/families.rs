mod common;
use common::{compare, context, materialize};
use datafusion::common::Result;

/// The families, whose names are the names of their native aggregates.
const FAMILIES: &[&str] = &[
    "count",
    "sum",
    "min",
    "max",
    "avg",
    "var_samp",
    "var_pop",
    "stddev_samp",
    "stddev_pop",
];

#[tokio::test]
async fn families_match_native_across_materializations() -> Result<()> {
    for partitions in [1, 4] {
        for &family in FAMILIES {
            let ctx = context(partitions)?;
            materialize(
                &ctx,
                "states",
                &format!("SELECT g, cell, {family}_state(x) AS s FROM t GROUP BY g, cell"),
            )
            .await?;
            for condition in ["true", "cell <= 1", "cell = 2", "false"] {
                compare(&ctx, &format!("SELECT g, {family}_merge(s) AS v FROM states WHERE {condition} GROUP BY g ORDER BY g"),
                    &format!("SELECT g, {family}(x) AS v FROM t WHERE {condition} GROUP BY g ORDER BY g")).await?;
                compare(
                    &ctx,
                    &format!("SELECT {family}_merge(s) AS v FROM states WHERE {condition}"),
                    &format!("SELECT {family}(x) AS v FROM t WHERE {condition}"),
                )
                .await?;
            }
            // Two levels of rollup: the cells to a state for each group, then to one state.
            materialize(
                &ctx,
                "rolled",
                &format!("SELECT g, {family}_merge_state(s) AS s FROM states GROUP BY g"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT g, {family}_merge(s) AS v FROM rolled GROUP BY g ORDER BY g"),
                &format!("SELECT g, {family}(x) AS v FROM t GROUP BY g ORDER BY g"),
            )
            .await?;
            materialize(
                &ctx,
                "total",
                &format!("SELECT {family}_merge_state(s) AS s FROM rolled"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}_merge(s) AS v FROM total"),
                &format!("SELECT {family}(x) AS v FROM t"),
            )
            .await?;
            materialize(
                &ctx,
                "empty",
                &format!("SELECT {family}_state(x) AS s FROM t WHERE false"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}_merge(s) AS v FROM empty"),
                &format!("SELECT {family}(x) AS v FROM t WHERE false"),
            )
            .await?;
            materialize(
                &ctx,
                "empty_roll",
                &format!("SELECT {family}_merge_state(s) AS s FROM states WHERE false"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}_merge(s) AS v FROM empty_roll"),
                &format!("SELECT {family}(x) AS v FROM t WHERE false"),
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn aggregate_filters_apply_to_merge_inputs_and_preserve_empty_groups() -> Result<()> {
    for &family in FAMILIES {
        let ctx = context(4)?;
        materialize(
            &ctx,
            "states",
            &format!(
                "SELECT g, cell, {family}_state(x) FILTER (WHERE keep) AS s FROM t GROUP BY g, cell"
            ),
        )
        .await?;
        let expected = format!(
            "SELECT g, {family}(x) FILTER (WHERE keep AND cell = 1) AS v FROM t GROUP BY g ORDER BY g"
        );
        compare(&ctx, &format!("SELECT g, {family}_merge(s) FILTER (WHERE cell = 1) AS v FROM states GROUP BY g ORDER BY g"),
            &expected).await?;
        materialize(&ctx, "filtered", &format!("SELECT g, {family}_merge_state(s) FILTER (WHERE cell = 1) AS s FROM states GROUP BY g")).await?;
        compare(
            &ctx,
            &format!("SELECT g, {family}_merge(s) AS v FROM filtered GROUP BY g ORDER BY g"),
            &expected,
        )
        .await?;
        compare(&ctx, &format!("SELECT g, {family}_merge(CASE WHEN cell = 1 THEN s END) AS v FROM states GROUP BY g ORDER BY g"),
            &expected).await?;
    }
    Ok(())
}

#[tokio::test]
async fn count_rows_and_flat_extrema() -> Result<()> {
    let ctx = context(4)?;
    materialize(
        &ctx,
        "counts",
        "SELECT cell, count_state() AS s FROM t GROUP BY cell",
    )
    .await?;
    compare(
        &ctx,
        "SELECT count_merge(s) AS v FROM counts",
        "SELECT count(*) AS v FROM t",
    )
    .await?;
    for (i, expr) in [
        "g",
        "keep",
        "CAST('2026-01-01' AS DATE) + cell",
        "CAST(cell AS TIMESTAMP)",
        "CAST(x AS DECIMAL(12,2))",
        "CAST(abs(x) AS BIGINT UNSIGNED)",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("extrema_{i}");
        materialize(
            &ctx,
            &name,
            &format!(
                "SELECT cell, min_state({expr}) AS lo, max_state({expr}) AS hi FROM t GROUP BY cell"
            ),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT min_merge(lo) AS lo, max_merge(hi) AS hi FROM {name}"),
            &format!("SELECT min({expr}) AS lo, max({expr}) AS hi FROM t"),
        )
        .await?;
    }
    Ok(())
}
