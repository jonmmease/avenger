mod common;
use avenger_datafusion_aggregate_state::{
    expr_fn::{avg_merge, avg_state},
    register_all,
};
use common::{compare, context, materialize};
use datafusion::{
    common::Result,
    functions::core::coalesce,
    logical_expr::{col, expr::NullTreatment, Expr, ExprFunctionExt, LogicalPlanBuilder},
    prelude::SessionContext,
};
use std::sync::Arc;

#[tokio::test]
async fn sql_names_registration_and_conflicts() -> Result<()> {
    let mut ctx = context(2)?;
    register_all(&mut ctx)?;
    materialize(
        &ctx,
        "states",
        "SELECT avgState(x) AS a, AVGSTATE(x) AS b, \"avgState\"(x) AS c FROM t",
    )
    .await?;
    compare(
        &ctx,
        "SELECT avgMerge(a) AS a, AVGMERGE(b) AS b, \"avgMerge\"(c) AS c FROM states",
        "SELECT avg(x) AS a, avg(x) AS b, avg(x) AS c FROM t",
    )
    .await?;
    let mut conflicting = SessionContext::new();
    let collision = coalesce().as_ref().clone().with_aliases(["avgstate"]);
    conflicting.register_udf(collision);
    assert!(register_all(&mut conflicting)
        .unwrap_err()
        .to_string()
        .contains("already registered"));
    assert!(
        datafusion::logical_expr::registry::FunctionRegistry::udaf(&conflicting, "sumstate")
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_modifiers_are_rejected() -> Result<()> {
    let ctx = context(2)?;
    for sql in [
        "SELECT avgState(DISTINCT x) FROM t",
        "SELECT avgState(x ORDER BY cell) FROM t",
        "SELECT avgState(x) IGNORE NULLS FROM t",
    ] {
        assert!(ctx.sql(sql).await.is_err(), "accepted {sql}");
    }
    let input = ctx.table("t").await?.into_unoptimized_plan();
    for expr in [
        avg_state(col("x")).distinct().build()?,
        avg_state(col("x"))
            .order_by(vec![col("cell").sort(true, false)])
            .build()?,
        avg_state(col("x"))
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
        .aggregate(vec![col("cell")], vec![avg_state(col("x")).alias("s")])?
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
        .aggregate(Vec::<Expr>::new(), vec![avg_merge(col("s")).alias("v")])?
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
