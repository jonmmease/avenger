mod common;
use avenger_datafusion_preaggregate::*;
use common::*;
use datafusion::{
    common::Result,
    logical_expr::{col, lit, LogicalPlan, LogicalPlanBuilder},
};

#[tokio::test]
async fn finishing_queries_rebuild_selected_groups_before_each_operator() -> Result<()> {
    let queries = [
        "SELECT g, COUNT(*) AS n FROM rows GROUP BY g ORDER BY n DESC, g ASC NULLS LAST LIMIT 2 OFFSET 1",
        "SELECT g FROM rows GROUP BY g HAVING COUNT(*) > 1 ORDER BY COUNT(*) DESC, g LIMIT 1",
        "SELECT g, COUNT(*) AS n FROM rows GROUP BY g ORDER BY g LIMIT 0",
        "SELECT g, COUNT(*) AS n FROM rows GROUP BY g ORDER BY g LIMIT 2 OFFSET 100",
        "SELECT g, COUNT(*) AS n, ROW_NUMBER() OVER (ORDER BY COUNT(*) DESC, g) AS pos FROM rows GROUP BY g ORDER BY pos",
        "SELECT g, COUNT(*) AS n, RANK() OVER (ORDER BY COUNT(*)) AS r, DENSE_RANK() OVER (ORDER BY COUNT(*)) AS dr FROM rows GROUP BY g ORDER BY g",
        "SELECT g, COUNT(*) AS n, LAG(COUNT(*)) OVER (ORDER BY g) AS prev, LEAD(COUNT(*), 1, 0) OVER (ORDER BY g) AS next FROM rows GROUP BY g ORDER BY g",
        "SELECT g, SUM(COUNT(*)) OVER (ORDER BY g ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g ORDER BY g",
        "SELECT g, SUM(COUNT(*)) OVER (ORDER BY COUNT(*) RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g ORDER BY g",
        "SELECT g, SUM(COUNT(*)) OVER (ORDER BY COUNT(*) GROUPS BETWEEN 1 PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g ORDER BY g",
        "SELECT g, COUNT(*) AS n, SUM(COUNT(*)) OVER (PARTITION BY g ORDER BY g ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS n2 FROM rows GROUP BY g HAVING COUNT(*) > 1 ORDER BY n DESC, g LIMIT 2",
        "SELECT * FROM (SELECT g, COUNT(*) AS n, ROW_NUMBER() OVER (ORDER BY COUNT(*) DESC, g) AS pos FROM rows GROUP BY g) q WHERE pos <= 2 ORDER BY pos",
        "SELECT AVG(n) AS mean, MEDIAN(n) AS median, COUNT(DISTINCT n) AS distinct_counts FROM (SELECT g, COUNT(*) AS n FROM rows GROUP BY g) q",
        "SELECT n, COUNT(*) AS groups FROM (SELECT g, COUNT(*) AS n FROM rows GROUP BY g) q GROUP BY n HAVING COUNT(*) > 0 ORDER BY n",
        "SELECT COUNT(*) FILTER (WHERE n > 1) AS large, AVG(n) FILTER (WHERE n > 0) AS mean FROM (SELECT g, COUNT(*) AS n FROM rows GROUP BY g) q",
        "SELECT COUNT(*) AS groups, AVG(n) AS mean FROM (SELECT g, COUNT(*) AS n FROM rows GROUP BY g ORDER BY n DESC, g LIMIT 2) q",
        "SELECT MAX(running) AS last FROM (SELECT g, SUM(COUNT(*)) OVER (ORDER BY g ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g) q",
        "SELECT COUNT(*) AS groups FROM (SELECT COUNT(*) AS n FROM rows) q",
        "SELECT COUNT(*) AS groups FROM (SELECT g, COUNT(*) AS n FROM rows GROUP BY g) q",
        "SELECT AVG(running) AS a FROM (SELECT g, SUM(SUM(x * 2.0) FILTER (WHERE keep)) OVER (ORDER BY g ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g HAVING COUNT(*) > 0 ORDER BY g LIMIT 3 OFFSET 1) q",
    ];
    for partitions in [1, 4] {
        let ctx = context(partitions)?;
        for sql in queries {
            let q = query(&ctx, sql).await?;
            let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
            assert!(
                p.materialization_plan().is_some(),
                "{sql}: {:?}",
                p.explain()
            );
            let materialization = p
                .materialization_plan()
                .unwrap()
                .display_indent()
                .to_string();
            assert!(
                !materialization.contains("Limit:") && !materialization.contains("WindowAggr:"),
                "{materialization}"
            );
            for predicate in [
                lit(true),
                lit(false),
                col("cell").eq(lit(0_i32)),
                col("cell").eq(lit(1_i32)),
                col("cell").eq(lit(2_i32)),
            ] {
                compare_ordered(&ctx, &p, predicate)
                    .await
                    .map_err(|e| e.context(sql))?;
            }
        }
        // The optimizer can encode a top-k directly in Sort.fetch.
        let q = FilterQuery::new(ctx.table("t").await?.into_unoptimized_plan(), |rows| {
            let mut plan = LogicalPlanBuilder::from(rows)
                .aggregate(
                    vec![col("g")],
                    vec![datafusion::functions_aggregate::expr_fn::count(lit(1)).alias("n")],
                )?
                .sort(vec![col("n").sort(false, false), col("g").sort(true, true)])?
                .build()?;
            let LogicalPlan::Sort(sort) = &mut plan else {
                unreachable!()
            };
            sort.fetch = Some(1);
            Ok(plan)
        })?;
        let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
        compare_ordered(&ctx, &p, col("cell").eq(lit(1_i32))).await?;
    }
    Ok(())
}

#[tokio::test]
async fn unsupported_nearest_targets_and_before_target_operators_stay_direct() -> Result<()> {
    let ctx = context(1)?;
    for sql in [
        "SELECT AVG(n) FROM (SELECT g, COUNT(DISTINCT x) AS n FROM rows GROUP BY g) q",
        "SELECT COUNT(*) FROM (SELECT * FROM rows LIMIT 2) q",
        "SELECT COUNT(*) FROM (SELECT *, ROW_NUMBER() OVER (ORDER BY g) AS r FROM rows) q",
        "SELECT COUNT(*) FROM rows a JOIN rows b ON a.g = b.g",
    ] {
        let p =
            PreaggregatePlanner::default().prepare(query(&ctx, sql).await?, vec![col("cell")])?;
        assert!(p.materialization_plan().is_none(), "{sql}");
        assert_eq!(
            p.bind(lit(true))?.diagnostics().strategy,
            QueryStrategy::Direct
        );
    }
    Ok(())
}

#[tokio::test]
async fn nullable_groups_window_filters_and_multiple_window_stages() -> Result<()> {
    use datafusion::datasource::view::ViewTable;
    use std::sync::Arc;
    let ctx = context(4)?;
    let source = ctx
        .sql("SELECT CASE WHEN g = 'nulls' THEN NULL ELSE g END AS g, cell, x, keep FROM t")
        .await?
        .into_unoptimized_plan();
    ctx.deregister_table("t")?;
    ctx.register_table("t", Arc::new(ViewTable::new(source, None)))?;
    for sql in [
        "SELECT g, COUNT(*) AS n, ROW_NUMBER() OVER (ORDER BY g NULLS FIRST) AS a, ROW_NUMBER() OVER (ORDER BY COUNT(*) DESC, g NULLS LAST) AS b FROM rows GROUP BY g ORDER BY g NULLS FIRST",
        "SELECT g, SUM(COUNT(*)) FILTER (WHERE g IS NOT NULL) OVER (ORDER BY g NULLS LAST ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running FROM rows GROUP BY g ORDER BY g NULLS LAST",
        "SELECT g, SUM(COUNT(*)) OVER (PARTITION BY g IS NULL ORDER BY COUNT(*) GROUPS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS n FROM rows GROUP BY g ORDER BY g NULLS LAST",
    ] {
        let p = PreaggregatePlanner::default().prepare(query(&ctx, sql).await?, vec![col("cell")])?;
        for predicate in [lit(true), lit(false), col("cell").eq(lit(0_i32)),col("cell").eq(lit(1_i32))] { compare_ordered(&ctx, &p, predicate).await?; }
    }
    Ok(())
}
