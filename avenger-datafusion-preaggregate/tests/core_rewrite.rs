mod common;
use avenger_datafusion_preaggregate::*;
use common::*;
use datafusion::{
    common::Result,
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
};
use std::ops::Not;

#[tokio::test]
async fn logical_and_sql_queries() -> Result<()> {
    for partitions in [1, 4] {
        let ctx = context(partitions)?;
        let source = ctx.table("t").await?.into_unoptimized_plan();
        let q = FilterQuery::new(source.clone(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![col("g")], vec![count(col("x")).alias("n")])?
                .build()
        })?;
        let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
        assert!(p.materialization_plan().is_some(), "{:?}", p.explain());
        for predicate in [
            lit(true),
            lit(false),
            col("cell").eq(lit(1_i32)),
            col("g").eq(lit("a")),
        ] {
            compare(&ctx, &p, predicate).await?;
        }
        let q = query(
            &ctx,
            "SELECT g, COUNT(*) AS n FROM rows GROUP BY g ORDER BY g",
        )
        .await?;
        let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
        assert!(p.materialization_plan().is_some(), "{:?}", p.explain());
        compare(&ctx, &p, col("cell").eq(lit(1_i32))).await?;
    }
    Ok(())
}

#[tokio::test]
async fn ownership_predicate_validation_and_runtime_bindings() -> Result<()> {
    use avenger_datafusion_preaggregate::runtime::ParameterExpressions;
    use datafusion::{
        arrow::datatypes::DataType,
        logical_expr::{expr::Placeholder, Expr},
    };
    let ctx = context(1)?;
    let source = ctx.table("t").await?.into_unoptimized_plan();
    assert!(FilterQuery::builder(source.clone())
        .finish(source.clone())
        .is_err());
    let first = FilterQuery::builder(source.clone());
    let second = FilterQuery::builder(source.clone());
    assert!(first.finish(second.rows()).is_err());
    let q = FilterQuery::new(source.clone(), |rows| {
        LogicalPlanBuilder::from(rows.clone())
            .union(rows)?
            .aggregate(
                Vec::<datafusion::logical_expr::Expr>::new(),
                vec![count(lit(1))],
            )?
            .build()
    })?;
    let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
    assert_eq!(
        p.explain().direct_reason,
        Some(DirectReason::UnsupportedQueryShape)
    );
    assert!(matches!(p.bind(lit(true))?, BoundQuery::Direct { .. }));
    let q = query(&ctx, "SELECT g, COUNT(*) AS n FROM rows GROUP BY g").await?;
    let planner = PreaggregatePlanner::default();
    assert!(planner.prepare(q.clone(), vec![col("missing")]).is_err());
    let p = planner.prepare(q.clone(), vec![col("cell")])?;
    let another = planner.prepare(q, vec![col("cell")])?;
    assert!(p.bind(lit(1_i32)).is_err());
    assert!(p.bind(col("missing").eq(lit(1))).is_err());
    let parameter = |id: &str| {
        Expr::Placeholder(Placeholder::new_with_field(
            id.to_owned(),
            Some(std::sync::Arc::new(
                datafusion::arrow::datatypes::Field::new("predicate", DataType::Boolean, true),
            )),
        ))
    };
    assert!(p.bind(parameter("$1")).is_err());
    assert_eq!(
        p.bind(col("x").gt(lit(0.0)))?.diagnostics().direct_reason,
        Some(DirectReason::PredicateNeedsUnretainedExpression)
    );
    let templates = p.parameterize(ParameterExpressions {
        source: parameter("$1"),
        retained: parameter("$2"),
    })?;
    let good = p.bind(col("cell").eq(lit(1_i32)))?;
    templates.check_binding(good.predicates())?;
    assert!(templates
        .check_binding(another.bind(lit(true))?.predicates())
        .is_err());
    templates.check_binding(p.clone().bind(lit(true))?.predicates())?;
    let bad = p.bind(col("x").gt(lit(0.0)))?;
    assert!(bad.predicates().retained().is_none());
    assert_eq!(
        p.bind_with_policy(lit(true), QueryPolicy::ForceDirect)?
            .diagnostics()
            .direct_reason,
        Some(DirectReason::Forced)
    );
    if let BoundQuery::Preaggregated { rollup, .. } = good {
        assert!(rollup.with_materialization(source).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn computed_dimensions_alias_lineage_and_correlated_predicates() -> Result<()> {
    let ctx = context(4)?;
    let q = query(
        &ctx,
        "SELECT COUNT(*) AS n FROM rows AS r GROUP BY r.g, r.cell / 2",
    )
    .await?;
    let p = PreaggregatePlanner::default().prepare(q, vec![])?;
    assert!(p.materialization_plan().is_some(), "{:?}", p.explain());
    assert_eq!(
        p.bind(col("cell").eq(lit(1_i32)))?
            .diagnostics()
            .direct_reason,
        Some(DirectReason::PredicateNeedsUnretainedExpression)
    );
    let bin = col("cell") / lit(2_i64);
    let predicate = bin
        .clone()
        .eq(lit(0_i32))
        .and(col("g").eq(lit("a")))
        .or(bin.eq(lit(1_i32)).and(col("g").eq(lit("b"))));
    compare(&ctx, &p, predicate).await?;
    let q = query(&ctx, "SELECT COUNT(*) AS n FROM rows r GROUP BY r.g").await?;
    let p = PreaggregatePlanner::default().prepare(q, vec![col("g"), col("cell"), col("cell")])?;
    assert_eq!(
        p.explain()
            .materialization_schema
            .as_ref()
            .unwrap()
            .fields()
            .len(),
        3
    );
    compare(
        &ctx,
        &p,
        col("cell")
            .in_list(vec![lit(0_i32), lit(1_i32)], false)
            .and(col("g").eq(lit("a")).not()),
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn preparation_never_invokes_udfs_and_matching_uses_implementation_identity() -> Result<()> {
    use datafusion::{
        arrow::datatypes::DataType,
        common::DFSchema,
        logical_expr::{create_udf, expr::ScalarFunction, Volatility},
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[derive(Debug)]
    struct Trusted;
    impl ExpressionProperties for Trusted {
        fn scalar_function(&self, _: &ScalarFunction, _: &DFSchema) -> ScalarFunctionProperties {
            ScalarFunctionProperties {
                total: true,
                respects_grouping_equality: true,
            }
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let make = || {
        let calls = calls.clone();
        create_udf(
            "same_name",
            vec![DataType::Int32],
            DataType::Int32,
            Volatility::Immutable,
            Arc::new(move |args| {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(args[0].clone())
            }),
        )
    };
    let f = make();
    let g = make();
    let ctx = context(1)?;
    let q = FilterQuery::new(ctx.table("t").await?.into_unoptimized_plan(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<datafusion::logical_expr::Expr>::new(),
                vec![count(f.call(vec![col("cell")]))],
            )?
            .build()
    })?;
    let planner = PreaggregatePlanner::default().with_expression_properties(Arc::new(Trusted));
    let p = planner.prepare(q, vec![f.call(vec![col("cell")])])?;
    assert!(p.materialization_plan().is_some());
    for _ in 0..3 {
        assert_eq!(
            p.bind(f.call(vec![col("cell")]).eq(lit(1_i32)))?
                .diagnostics()
                .strategy,
            QueryStrategy::Preaggregated
        );
    }
    assert_eq!(
        p.bind(g.call(vec![col("cell")]).eq(lit(1_i32)))?
            .diagnostics()
            .direct_reason,
        Some(DirectReason::PredicateNeedsUnretainedExpression)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    for volatility in [Volatility::Stable, Volatility::Volatile] {
        let f = create_udf(
            "unstable",
            vec![DataType::Int32],
            DataType::Int32,
            volatility,
            Arc::new(|_| panic!("planning evaluated a function")),
        );
        let q = FilterQuery::new(ctx.table("t").await?.into_unoptimized_plan(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(
                    Vec::<datafusion::logical_expr::Expr>::new(),
                    vec![count(f.call(vec![col("cell")]))],
                )?
                .build()
        })?;
        assert_eq!(
            planner
                .prepare(q, vec![col("cell")])?
                .explain()
                .direct_reason,
            Some(DirectReason::NonImmutableQuery)
        );
    }
    Ok(())
}

#[tokio::test]
async fn total_functions_need_a_separate_grouping_equality_contract() -> Result<()> {
    use datafusion::{
        arrow::datatypes::DataType,
        common::DFSchema,
        logical_expr::{create_udf, expr::ScalarFunction, Volatility},
    };
    use std::sync::Arc;
    #[derive(Debug)]
    struct TotalOnly;
    impl ExpressionProperties for TotalOnly {
        fn scalar_function(&self, _: &ScalarFunction, _: &DFSchema) -> ScalarFunctionProperties {
            ScalarFunctionProperties {
                total: true,
                respects_grouping_equality: false,
            }
        }
    }
    let f = create_udf(
        "sign_sensitive",
        vec![DataType::Float64],
        DataType::Boolean,
        Volatility::Immutable,
        Arc::new(|_| panic!("preparation must not evaluate functions")),
    );
    let ctx = context(1)?;
    let q = FilterQuery::new(ctx.table("t").await?.into_unoptimized_plan(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(vec![col("g")], vec![count(f.call(vec![col("x")]))])?
            .build()
    })?;
    let p = PreaggregatePlanner::default()
        .with_expression_properties(Arc::new(TotalOnly))
        .prepare(q, vec![col("x")])?;
    assert!(p.materialization_plan().is_some());
    assert_eq!(
        p.bind(f.call(vec![col("x")]))?.diagnostics().direct_reason,
        Some(DirectReason::UnsupportedPredicate)
    );
    assert_eq!(
        p.bind(col("x").is_null())?.diagnostics().strategy,
        QueryStrategy::Preaggregated
    );
    Ok(())
}

#[tokio::test]
async fn output_alias_metadata_survives_binding_and_materialization_substitution() -> Result<()> {
    use datafusion::common::metadata::FieldMetadata;
    use std::collections::HashMap;
    let ctx = context(1)?;
    let metadata = FieldMetadata::from(HashMap::from([("unit".to_owned(), "flights".to_owned())]));
    let q = FilterQuery::new(ctx.table("t").await?.into_unoptimized_plan(), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                vec![col("g")],
                vec![count(lit(1_i64)).alias_with_metadata("n", Some(metadata))],
            )?
            .build()
    })?;
    let p = PreaggregatePlanner::default().prepare(q, vec![col("cell")])?;
    compare(&ctx, &p, col("cell").eq(lit(1_i32))).await?;
    Ok(())
}

#[tokio::test]
async fn null_tests_still_validate_missing_and_ambiguous_columns() -> Result<()> {
    let ctx = context(1)?;
    let source = ctx.table("t").await?.into_unoptimized_plan();
    let a = LogicalPlanBuilder::from(source.clone())
        .alias("a")?
        .build()?;
    let b = LogicalPlanBuilder::from(source).alias("b")?.build()?;
    let source = LogicalPlanBuilder::from(a).cross_join(b)?.build()?;
    let q = FilterQuery::new(source, |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<datafusion::logical_expr::Expr>::new(),
                vec![count(lit(1_i64))],
            )?
            .build()
    })?;
    let planner = PreaggregatePlanner::default();
    assert!(planner.prepare(q.clone(), vec![col("g")]).is_err());
    let p = planner.prepare(q, vec![col("a.g")])?;
    for predicate in [col("missing").is_null(), col("g").is_null()] {
        assert!(p.bind(predicate.clone()).is_err());
        assert!(p
            .bind_with_policy(predicate, QueryPolicy::ForceDirect)
            .is_err());
    }
    compare(&ctx, &p, col("a.g").is_not_null()).await?;
    Ok(())
}

#[tokio::test]
async fn concrete_predicates_reject_parameters_inside_subqueries() -> Result<()> {
    let ctx = context(1)?;
    let query = query(&ctx, "SELECT COUNT(*) AS n FROM rows").await?;
    let p = PreaggregatePlanner::default().prepare(query, vec![col("cell")])?;
    let expression = ctx
        .state()
        .create_logical_plan("SELECT (SELECT CAST($1 AS BIGINT)) > 0 AS predicate")
        .await?;
    let datafusion::logical_expr::LogicalPlan::Projection(projection) = expression else {
        unreachable!()
    };
    assert!(p.bind(projection.expr[0].clone()).is_err());
    Ok(())
}
