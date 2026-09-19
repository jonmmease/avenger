mod common;
use avenger_datafusion_preaggregate::*;
use common::*;
use datafusion::{
    common::Result,
    logical_expr::{col, lit},
};

#[tokio::test]
async fn native_scalar_expressions_match_direct_queries() -> Result<()> {
    for partitions in [1, 4] {
        let ctx = context(partitions)?;
        for expression in [
            "x + 2.0",
            "x - 2.0",
            "x * 2.0",
            "x + cell",
            "x / 2.0",
            "cell + 1",
            "cell * 2",
            "-cell",
            "CASE WHEN cell = 0 THEN 0 ELSE cell / cell END",
            "CAST(x AS INT)",
            "CAST(cell AS DECIMAL(10,2)) + CAST(cell AS DECIMAL(10,2))",
            "CAST(cell AS BIGINT)",
            "TRY_CAST(g AS DOUBLE)",
            "CASE WHEN keep THEN x * 2.0 ELSE x + 1.0 END",
        ] {
            let sql = format!("SELECT g, COUNT({expression}) AS n, SUM({expression}) AS s, AVG({expression}) FILTER (WHERE keep) AS a FROM rows GROUP BY g");
            let q = query(&ctx, &sql).await?;
            let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
            assert!(
                p.materialization_plan().is_some(),
                "{expression}: {:?}",
                p.explain()
            );
            for predicate in [
                lit(true),
                lit(false),
                col("cell").eq(lit(0_i32)),
                col("cell").eq(lit(1_i32)),
            ] {
                compare(&ctx, &p, predicate).await?;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn warmup_requires_valid_expressions_on_rows_outside_the_current_selection() -> Result<()> {
    use datafusion::{
        arrow::{
            array::{BooleanArray, StringArray},
            record_batch::RecordBatch,
        },
        datasource::MemTable,
        prelude::SessionContext,
    };
    use std::sync::Arc;
    let ctx = SessionContext::new();
    let data = RecordBatch::try_from_iter(vec![
        ("text", Arc::new(StringArray::from(vec!["12", "bad"])) as _),
        ("keep", Arc::new(BooleanArray::from(vec![true, false])) as _),
    ])?;
    ctx.register_table(
        "t",
        Arc::new(MemTable::try_new(data.schema(), vec![vec![data]])?),
    )?;
    let q = query(&ctx, "SELECT SUM(CAST(text AS BIGINT)) AS s FROM rows").await?;
    let p = PreaggregatePlanner::default().prepare(q, vec![col("keep")])?;
    assert!(p.materialization_plan().is_some());
    assert_eq!(
        p.bind(col("keep"))?.diagnostics().strategy,
        QueryStrategy::Preaggregated
    );
    let BoundQuery::Direct { plan, .. } =
        p.bind_with_policy(col("keep"), QueryPolicy::ForceDirect)?
    else {
        unreachable!()
    };
    let result = ctx.execute_logical_plan(plan).await?.collect().await?;
    assert_eq!(
        rows(&result)?[0][0],
        datafusion::common::ScalarValue::Int64(Some(12))
    );
    let materialization = p.materialization_plan().unwrap().clone();
    let error = ctx
        .execute_logical_plan(materialization)
        .await?
        .collect()
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bad"), "{error}");

    let q = query(&ctx, "SELECT SUM(TRY_CAST(text AS BIGINT)) AS s FROM rows").await?;
    let p = PreaggregatePlanner::default().prepare(q, vec![col("keep")])?;
    compare(&ctx, &p, col("keep")).await?;
    Ok(())
}

#[tokio::test]
async fn float32_arithmetic_retains_native_intermediate_precision() -> Result<()> {
    use datafusion::{
        arrow::{
            array::{Float32Array, Int32Array},
            record_batch::RecordBatch,
        },
        functions_aggregate::expr_fn::{avg, count, sum},
        logical_expr::{ExprFunctionExt, LogicalPlanBuilder},
        prelude::SessionContext,
    };
    use std::sync::Arc;
    let ctx = SessionContext::new();
    let data = RecordBatch::try_from_iter(vec![
        (
            "x",
            Arc::new(Float32Array::from(vec![
                Some(1.25),
                None,
                Some(4.5),
                Some(-3.0),
            ])) as _,
        ),
        ("cell", Arc::new(Int32Array::from(vec![0, 0, 1, 2])) as _),
    ])?;
    for value in [
        col("x") + lit(2_f32),
        col("x") - lit(2_f32),
        col("x") * lit(2_f32),
    ] {
        let q = FilterQuery::new(
            ctx.read_batch(data.clone())?.into_unoptimized_plan(),
            |rows| {
                LogicalPlanBuilder::from(rows)
                    .aggregate(
                        Vec::<datafusion::logical_expr::Expr>::new(),
                        vec![
                            count(value.clone()).alias("n"),
                            sum(value.clone()).alias("s"),
                            avg(value)
                                .filter(col("cell").lt(lit(2_i32)))
                                .build()?
                                .alias("a"),
                        ],
                    )?
                    .build()
            },
        )?;
        let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
        for predicate in [lit(true), lit(false), col("cell").eq(lit(1_i32))] {
            compare(&ctx, &p, predicate).await?;
        }
    }
    Ok(())
}
