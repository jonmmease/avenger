mod common;
use avenger_datafusion_aggregate_state::{merge, state, Family};
use common::{compare, context, materialize};
use datafusion::{
    common::Result,
    logical_expr::{col, expr::NullTreatment, Expr, ExprFunctionExt, LogicalPlanBuilder},
    prelude::SessionContext,
};
use std::sync::Arc;

#[tokio::test]
async fn unsupported_modifiers_are_rejected() -> Result<()> {
    let ctx = context(2)?;
    for sql in [
        "SELECT avg_state(DISTINCT x) FROM t",
        "SELECT avg_state(x ORDER BY cell) FROM t",
        "SELECT avg_state(x) IGNORE NULLS FROM t",
    ] {
        let result = match ctx.sql(sql).await {
            Ok(frame) => frame.collect().await.map(|_| ()),
            Err(error) => Err(error),
        };
        assert!(result.is_err(), "accepted {sql}");
    }
    let input = ctx.table("t").await?.into_unoptimized_plan();
    for expr in [
        state(Family::Avg, col("x")).distinct().build()?,
        state(Family::Avg, col("x"))
            .order_by(vec![col("cell").sort(true, false)])
            .build()?,
        state(Family::Avg, col("x"))
            .null_treatment(NullTreatment::RespectNulls)
            .build()?,
    ] {
        let plan = LogicalPlanBuilder::from(input.clone())
            .aggregate(Vec::<Expr>::new(), vec![expr])?
            .build()?;
        assert!(ctx
            .execute_logical_plan(plan)
            .await?
            .collect()
            .await
            .is_err());
    }
    Ok(())
}

#[tokio::test]
async fn programmatic_plans_do_not_need_registration() -> Result<()> {
    let source = context(2)?;
    let ctx = SessionContext::new();
    let plan = LogicalPlanBuilder::from(source.table("t").await?.into_unoptimized_plan())
        .aggregate(
            vec![col("cell")],
            vec![state(Family::Avg, col("x")).alias("s")],
        )?
        .build()?;
    let frame = ctx.execute_logical_plan(plan).await?;
    let schema = Arc::new(frame.schema().as_arrow().clone());
    let batches = frame.collect().await?;
    ctx.register_table(
        "states",
        Arc::new(datafusion::datasource::MemTable::try_new(
            schema,
            vec![batches],
        )?),
    )?;
    let plan = LogicalPlanBuilder::from(ctx.table("states").await?.into_unoptimized_plan())
        .aggregate(
            Vec::<Expr>::new(),
            vec![merge(Family::Avg, col("s")).alias("v")],
        )?
        .build()?;
    let actual = ctx.execute_logical_plan(plan).await?.collect().await?;
    let expected = source
        .sql("SELECT avg(x) AS v FROM t")
        .await?
        .collect()
        .await?;
    common::assert_results(&actual, &expected, 1e-10)?;
    Ok(())
}

/// A merge whose partial phase DataFusion skips converts each exported state to native partial
/// columns. A filter that removes every state of a group gives the native result of no rows.
#[tokio::test]
async fn merges_without_partial_aggregation_match_native_aggregates() -> Result<()> {
    let ctx = context(2)?;
    for setting in [
        "SET datafusion.execution.skip_partial_aggregation_probe_rows_threshold = 0",
        "SET datafusion.execution.skip_partial_aggregation_probe_ratio_threshold = 0.0",
    ] {
        ctx.sql(setting).await?.collect().await?;
    }
    for family in [
        "count",
        "sum",
        "min",
        "max",
        "avg",
        "var_samp",
        "var_pop",
        "stddev_samp",
        "stddev_pop",
    ] {
        let states = format!("{family}_states");
        let sql = format!("SELECT g, cell, {family}_state(x) AS s FROM t GROUP BY g, cell");
        materialize(&ctx, &states, &sql).await?;
        // The group `one` has only cell 1, so the filter removes all its states.
        compare(
            &ctx,
            &format!(
                "SELECT g, {family}_merge(s) FILTER (WHERE cell <> 1) AS v \
                 FROM {states} GROUP BY g ORDER BY g"
            ),
            &format!(
                "SELECT g, {family}(x) FILTER (WHERE cell <> 1) AS v \
                 FROM t GROUP BY g ORDER BY g"
            ),
        )
        .await?;
    }
    Ok(())
}
