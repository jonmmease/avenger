mod common;
use common::{compare, context, materialize};
use datafusion::common::Result;

const FAMILIES: &[(&str, &str)] = &[
    ("count", "count"),
    ("sum", "sum"),
    ("min", "min"),
    ("max", "max"),
    ("avg", "avg"),
    ("varSamp", "var_samp"),
    ("varPop", "var_pop"),
    ("stddevSamp", "stddev_samp"),
    ("stddevPop", "stddev_pop"),
];

#[tokio::test]
async fn families_match_native_across_materializations() -> Result<()> {
    for partitions in [1, 4] {
        for &(family, native) in FAMILIES {
            let ctx = context(partitions)?;
            materialize(
                &ctx,
                "states",
                &format!("SELECT g, cell, {family}State(x) AS s FROM t GROUP BY g, cell"),
            )
            .await?;
            for condition in ["true", "cell <= 1", "cell = 2", "false"] {
                compare(&ctx, &format!("SELECT g, {family}Merge(s) AS v FROM states WHERE {condition} GROUP BY g ORDER BY g"),
                    &format!("SELECT g, {native}(x) AS v FROM t WHERE {condition} GROUP BY g ORDER BY g")).await?;
                compare(
                    &ctx,
                    &format!("SELECT {family}Merge(s) AS v FROM states WHERE {condition}"),
                    &format!("SELECT {native}(x) AS v FROM t WHERE {condition}"),
                )
                .await?;
            }
            materialize(
                &ctx,
                "rolled",
                &format!("SELECT g, {family}MergeState(s) AS s FROM states GROUP BY g"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT g, {family}Finalize(s) AS v FROM rolled ORDER BY g"),
                &format!("SELECT g, {native}(x) AS v FROM t GROUP BY g ORDER BY g"),
            )
            .await?;
            materialize(
                &ctx,
                "empty",
                &format!("SELECT {family}State(x) AS s FROM t WHERE false"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}Merge(s) AS v FROM empty"),
                &format!("SELECT {native}(x) AS v FROM t WHERE false"),
            )
            .await?;
            materialize(
                &ctx,
                "empty_roll",
                &format!("SELECT {family}MergeState(s) AS s FROM states WHERE false"),
            )
            .await?;
            compare(
                &ctx,
                &format!("SELECT {family}Finalize(s) AS v FROM empty_roll"),
                &format!("SELECT {native}(x) AS v FROM t WHERE false"),
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn aggregate_filters_apply_to_merge_inputs_and_preserve_empty_groups() -> Result<()> {
    for &(family, native) in FAMILIES {
        let ctx = context(4)?;
        materialize(
            &ctx,
            "states",
            &format!(
                "SELECT g, cell, {family}State(x) FILTER (WHERE keep) AS s FROM t GROUP BY g, cell"
            ),
        )
        .await?;
        compare(&ctx, &format!("SELECT g, {family}Merge(s) FILTER (WHERE cell = 1) AS v FROM states GROUP BY g ORDER BY g"),
            &format!("SELECT g, {native}(x) FILTER (WHERE keep AND cell = 1) AS v FROM t GROUP BY g ORDER BY g")).await?;
        materialize(&ctx, "filtered", &format!("SELECT g, {family}MergeState(s) FILTER (WHERE cell = 1) AS s FROM states GROUP BY g")).await?;
        compare(&ctx, &format!("SELECT g, {family}Finalize(s) AS v FROM filtered ORDER BY g"),
            &format!("SELECT g, {native}(x) FILTER (WHERE keep AND cell = 1) AS v FROM t GROUP BY g ORDER BY g")).await?;
        compare(&ctx, &format!("SELECT g, {family}Merge(CASE WHEN cell = 1 THEN s END) AS v FROM states GROUP BY g ORDER BY g"),
            &format!("SELECT g, {native}(x) FILTER (WHERE keep AND cell = 1) AS v FROM t GROUP BY g ORDER BY g")).await?;
    }
    Ok(())
}

#[tokio::test]
async fn count_rows_and_flat_extrema() -> Result<()> {
    let ctx = context(4)?;
    materialize(
        &ctx,
        "counts",
        "SELECT cell, countState() AS s FROM t GROUP BY cell",
    )
    .await?;
    compare(
        &ctx,
        "SELECT countMerge(s) AS v FROM counts",
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
                "SELECT cell, minState({expr}) AS lo, maxState({expr}) AS hi FROM t GROUP BY cell"
            ),
        )
        .await?;
        compare(
            &ctx,
            &format!("SELECT minMerge(lo) AS lo, maxMerge(hi) AS hi FROM {name}"),
            &format!("SELECT min({expr}) AS lo, max({expr}) AS hi FROM t"),
        )
        .await?;
    }
    Ok(())
}
