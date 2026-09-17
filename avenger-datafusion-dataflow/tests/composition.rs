mod common;
#[path = "common/scoped.rs"]
mod scoped;

use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::{
        common::ScalarValue,
        functions::datetime::expr_fn::now,
        functions_aggregate::expr_fn::sum,
        logical_expr::{
            col, create_udf, lit, scalar_subquery, ColumnarValue, LogicalPlanBuilder, Volatility,
        },
    },
    CachePolicy, DataflowBuilder, Error, ExecutionConfig, Result, Runtime, RuntimeConfig,
};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

#[tokio::test]
async fn base_and_local_inputs_keep_ownership_and_share_one_execution() -> Result<()> {
    for cache in [CachePolicy::Disabled, CachePolicy::default()] {
        let runtime = Runtime::new(RuntimeConfig {
            cache,
            execution: ExecutionConfig {
                max_active_queries: 1,
                ..Default::default()
            },
            ..Default::default()
        })?;
        let mut b = DataflowBuilder::new();
        let base_table = b.table_input("rows", common::schema())?;
        let base_scalar = b.scalar_input("offset", DataType::Int64)?;
        let base_expr = b.expr_input("selected", DataType::Boolean)?;
        let source = b.add_plan("source", base_table.plan_ref())?;
        let source_out = b.table_output("source", &source)?;
        let scalar = b.add_scalar("next", base_scalar.expr_ref() + lit(1_i64))?;
        let scalar_out = b.scalar_output("next", &scalar)?;
        let filtered = b.add_plan(
            "filtered",
            LogicalPlanBuilder::from(source.plan_ref())
                .filter(base_expr.expr_ref())?
                .build()?,
        )?;
        b.table_output("filtered", &filtered)?;
        let definition = b.finish()?;
        let base = runtime.prepare(&definition).await?;
        let mut a = DataflowBuilder::with_base(&base.interface());
        let imported = a.import_table("source", &source_out)?;
        let imported_scalar = a.import_scalar("next", &scalar_out)?;
        let echoed = a.scalar_output("echoed", &imported_scalar)?;
        let local_table = a.table_input("rows", common::schema())?;
        let local_scalar = a.scalar_input("offset", DataType::Int64)?;
        let local_expr = a.expr_input("selected", DataType::Boolean)?;
        let combined = a.add_plan(
            "combined",
            LogicalPlanBuilder::from(imported.plan_ref())
                .union(local_table.plan_ref())?
                .filter(base_expr.expr_ref().and(local_expr.expr_ref()))?
                .project(vec![(col("value")
                    + base_scalar.expr_ref()
                    + local_scalar.expr_ref()
                    + imported_scalar.expr_ref())
                .alias("value")])?
                .sort(vec![col("value").sort(true, true)])?
                .build()?,
        )?;
        let out = a.table_output("combined", &combined)?;
        let direct = a.add_plan(
            "direct",
            LogicalPlanBuilder::from(base_table.plan_ref())
                .filter(base_expr.expr_ref())?
                .build()?,
        )?;
        let direct_out = a.table_output("direct", &direct)?;
        let total = a.add_scalar(
            "total",
            scalar_subquery(Arc::new(
                LogicalPlanBuilder::from(combined.plan_ref())
                    .aggregate(
                        Vec::<avenger_datafusion_dataflow::datafusion::logical_expr::Expr>::new(),
                        vec![sum(col("value"))],
                    )?
                    .build()?,
            )),
        )?;
        let total_out = a.scalar_output("total", &total)?;
        let additional = a.finish()?;
        assert_eq!(additional.num_nodes(), 3);
        assert!(matches!(
            runtime.prepare(&additional).await,
            Err(Error::BaseRequired)
        ));
        assert!(matches!(additional.to_bytes(), Err(Error::Artifact(_))));
        let extension = base.prepare_extension(&additional).await?;
        assert_eq!(extension.explain().imports.len(), 2);
        assert_eq!(extension.interface().root().inputs().count(), 3);
        let base_inputs = base
            .inputs()
            .table(&base_table, common::snapshot(&[1, 2]))?
            .scalar(&base_scalar, 2_i64.into())?
            .expr(&base_expr, col("value").gt_eq(lit(2_i64)))?
            .finish()?;
        let local_inputs = extension
            .inputs()
            .table(&local_table, common::snapshot(&[3, 4]))?
            .scalar(&local_scalar, 10_i64.into())?
            .expr(&local_expr, col("value").lt_eq(lit(3_i64)))?
            .finish()?;
        assert!(matches!(
            extension.inputs().scalar(&base_scalar, 0_i64.into()),
            Err(Error::ForeignHandle)
        ));
        assert!(matches!(
            base.inputs().expr(&local_expr, lit(true)),
            Err(Error::ForeignHandle)
        ));
        assert!(matches!(
            extension
                .query(&[out], &[], &local_inputs, &base_inputs)
                .await,
            Err(Error::ForeignHandle)
        ));
        assert!(matches!(
            extension
                .query(&[source_out], &[], &base_inputs, &local_inputs)
                .await,
            Err(Error::ForeignHandle)
        ));
        let result = extension
            .query(
                &[out, direct_out],
                &[total_out, echoed],
                &base_inputs,
                &local_inputs,
            )
            .await?;
        assert_eq!(common::values(result.table(&out)?), [17, 18]);
        assert_eq!(common::values(result.table(&direct_out)?), [2]);
        assert_eq!(result.scalar(&total_out)?, &ScalarValue::from(35_i64));
        assert_eq!(result.scalar(&echoed)?, &ScalarValue::from(3_i64));
        assert_eq!(result.report().physical_plans, 5);
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .all(|n| !n.contains("filtered")));
        assert_eq!(
            result
                .report()
                .executed_nodes
                .iter()
                .filter(|n| *n == "base::source")
                .count(),
            1
        );
        let changed = local_inputs
            .edit()
            .scalar(&local_scalar, 20_i64.into())?
            .finish()?;
        assert_eq!(
            common::values(
                extension
                    .query(&[out], &[], &base_inputs, &changed)
                    .await?
                    .table(&out)?
            ),
            [27, 28]
        );
        // Each preparation of the same definition is a valid, independent base.
        runtime
            .prepare(&definition)
            .await?
            .prepare_extension(&additional)
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn imports_require_the_associated_root_interface() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("source", common::snapshot(&[1]))?;
    let root = b.table_output("source", &source)?;
    let (_scope, (scalar, table, predicate, out)) =
        b.partition_by("panels", source.plan_ref(), vec![col("value")], |s| {
            let scalar = s.scalar_input("scalar", DataType::Int64)?;
            let table = s.table_input("table", common::schema())?;
            let predicate = s.expr_input("predicate", DataType::Boolean)?;
            Ok((scalar, table, predicate, s.table_output("rows", &s.rows())?))
        })?;
    let definition = b.finish()?;
    let mut other = DataflowBuilder::new();
    let other_node = other.table_snapshot("source", common::snapshot(&[1]))?;
    let other_out = other.table_output("source", &other_node)?;
    let other = other.finish()?;
    let mut a = DataflowBuilder::with_base(&definition.interface());
    assert!(matches!(
        a.import_table("bad", &out),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        a.import_table("bad", &other_out),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        a.add_plan("bad", source.plan_ref()),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        a.add_plan("bad", table.plan_ref()),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        a.add_scalar("bad", scalar.expr_ref()),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        a.add_scalar("bad", predicate.expr_ref()),
        Err(Error::OutOfScope(_))
    ));
    a.import_table("source", &root)?;
    assert!(matches!(
        a.import_table("source", &root),
        Err(Error::DuplicateName { .. })
    ));
    let additional = a.finish()?;
    let runtime = Runtime::new(Default::default())?;
    let wrong_base = runtime.prepare(&other).await?;
    assert!(matches!(
        wrong_base.prepare_extension(&additional).await,
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        wrong_base.prepare_extension(&other).await,
        Err(Error::BaseRequired)
    ));
    Ok(())
}

#[tokio::test]
async fn reused_expression_inputs_are_validated_for_additional_contexts() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let table = b.table_snapshot("source", common::snapshot(&[1, 2]))?;
    let expr = b.expr_input("selection", DataType::Boolean)?;
    let unused = b.expr_input("unused", DataType::Boolean)?;
    let filtered = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(table.plan_ref())
            .filter(expr.expr_ref())?
            .build()?,
    )?;
    let output = b.table_output("filtered", &filtered)?;
    let base = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    let imported = a.import_table("source", &output)?;
    let renamed = a.add_plan(
        "renamed",
        LogicalPlanBuilder::from(imported.plan_ref())
            .project(vec![col("value").alias("different")])?
            .filter(expr.expr_ref())?
            .build()?,
    )?;
    let out = a.table_output("out", &renamed)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    let local = extension.inputs().finish()?;
    let invalid = base
        .inputs()
        .expr(&expr, col("value").gt(lit(1_i64)))?
        .expr(&unused, lit(true))?
        .finish()?;
    let err = extension
        .query(&[out], &[], &invalid, &local)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidExprInput { .. }));
    assert!(err.to_string().contains("additional::root::renamed"));
    // Extra validation does not change the original binding's valid base context.
    assert_eq!(
        common::values(base.query(&[output], &[], &invalid).await?.table(&output)?),
        [2]
    );
    let valid = invalid.edit().expr(&expr, lit(true))?.finish()?;
    assert_eq!(
        extension
            .query(&[out], &[], &valid, &local)
            .await?
            .table(&out)?
            .num_rows(),
        2
    );

    let mut a = DataflowBuilder::with_base(&base.interface());
    let imported = a.import_table("source", &output)?;
    let selected = a.add_plan(
        "selected",
        LogicalPlanBuilder::from(imported.plan_ref())
            .filter(unused.expr_ref())?
            .build()?,
    )?;
    let out = a.table_output("out", &selected)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    let inputs = valid
        .edit()
        .expr(&unused, col("value").eq(lit(1_i64)))?
        .finish()?;
    assert_eq!(
        common::values(
            extension
                .query(&[out], &[], &inputs, &extension.inputs().finish()?)
                .await?
                .table(&out)?
        ),
        [1]
    );
    Ok(())
}

#[tokio::test]
async fn named_volatile_values_and_query_time_are_shared_across_programs() -> Result<()> {
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let draw = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(
                counter.fetch_add(1, Ordering::SeqCst).into(),
            ))
        }),
    );
    let mut b = DataflowBuilder::new();
    let draw = b.add_scalar("draw", draw.call(vec![]))?;
    let draw_out = b.scalar_output("draw", &draw)?;
    let time = b.add_scalar("time", now())?;
    let time_out = b.scalar_output("time", &time)?;
    let runtime = Runtime::new(Default::default())?;
    let base = runtime.prepare(&b.finish()?).await?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    let draw1 = a.import_scalar("draw1", &draw_out)?;
    let draw2 = a.import_scalar("draw2", &draw_out)?;
    let time = a.import_scalar("time", &time_out)?;
    let draws = a.add_scalar("sum", draw1.expr_ref() + draw2.expr_ref())?;
    let sum_out = a.scalar_output("sum", &draws)?;
    let same = a.add_scalar("same_time", now().eq(time.expr_ref()))?;
    let same_out = a.scalar_output("same_time", &same)?;
    let direct = a.scalar_output("direct", &draw1)?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let base_inputs = base.inputs().finish()?;
    drop(base);
    let local = extension.inputs().finish()?;
    for expected in [0_i64, 1] {
        let result = extension
            .query(&[], &[sum_out, same_out, direct], &base_inputs, &local)
            .await?;
        assert_eq!(result.scalar(&sum_out)?, &ScalarValue::from(expected * 2));
        assert_eq!(result.scalar(&direct)?, &ScalarValue::from(expected));
        assert_eq!(result.scalar(&same_out)?, &ScalarValue::from(true));
        assert_eq!(result.report().physical_plans, 4);
        assert_eq!(result.report().cache_hits, 0);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(runtime.cache_stats().entries, 0);
    Ok(())
}

#[tokio::test]
async fn additional_nested_scopes_share_root_values_and_keep_local_overrides() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let table = b.table_input("sales", scoped::schema())?;
    let source = b.add_plan("source", table.plan_ref())?;
    let output = b.table_output("source", &source)?;
    let selected = b.expr_input("selected", DataType::Boolean)?;
    // An unrelated base scope must not be discovered by the extension.
    b.partition_by("unrelated", source.plan_ref(), vec![col("region")], |s| {
        s.table_output("rows", &s.rows())
    })?;
    let base = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let mut a = DataflowBuilder::with_base(&base.interface());
    let source = a.import_table("source", &output)?;
    let (regions, (years, cutoff, rows)) =
        a.partition_by("regions", source.plan_ref(), vec![col("region")], |s| {
            let (years, (cutoff, rows)) =
                s.partition_by("years", s.rows().plan_ref(), vec![col("year")], |s| {
                    let cutoff = s.scalar_input("cutoff", DataType::Int64)?;
                    let filtered = s.add_plan(
                        "filtered",
                        LogicalPlanBuilder::from(s.rows().plan_ref())
                            .filter(selected.expr_ref().and(col("amount").gt(cutoff.expr_ref())))?
                            .build()?,
                    )?;
                    Ok((cutoff, s.table_output("rows", &filtered)?))
                })?;
            Ok((years, cutoff, rows))
        })?;
    let extension = base.prepare_extension(&a.finish()?).await?;
    let inputs = base
        .inputs()
        .table(&table, scoped::sales())?
        .expr(&selected, lit(true))?
        .finish()?;
    let local = extension
        .inputs()
        .scope_defaults(&years, |b| b.scalar(&cutoff, 0_i64.into()))?
        .finish()?;
    let first = extension.query(&[rows], &[], &inputs, &local).await?;
    assert_eq!(
        first
            .report()
            .executed_nodes
            .iter()
            .filter(|n| *n == "base::source")
            .count(),
        1
    );
    assert!(first
        .report()
        .scopes
        .iter()
        .all(|scope| !scope.name.contains("unrelated")));
    let east = regions
        .instance([ScalarValue::from("East")])?
        .child(&years, [2025_i32.into()])?;
    let changed = local
        .edit()
        .at(&east, |b| b.scalar(&cutoff, 50_i64.into()))?
        .finish()?;
    let result = extension.query(&[rows], &[], &inputs, &changed).await?;
    assert_eq!(result.report().physical_plans, 1);
    assert!(result.report().cache_hits >= 3);
    let table = result
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("East")])?)
        .unwrap()
        .scope(&years)?
        .get(&years.key([2025_i32.into()])?)
        .unwrap()
        .table(&rows)?;
    assert_eq!(scoped::amounts(table), [80]);
    Ok(())
}

#[tokio::test]
async fn decoded_base_interfaces_can_construct_native_extensions() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let value = b.scalar_input("value", DataType::Int64)?;
    let scalar = b.add_scalar("scalar", value.expr_ref() + lit(1_i64))?;
    b.scalar_output("scalar", &scalar)?;
    let runtime = Runtime::new(Default::default())?;
    let decoded = runtime.decode_dataflow(&b.finish()?.to_bytes()?)?;
    let base = runtime.prepare(&decoded).await?;
    let names = base.interface().root();
    let mut a = DataflowBuilder::with_base(&base.interface());
    let imported = a.import_scalar("import", &names.scalar_output("scalar")?)?;
    let out = a.scalar_output("out", &imported)?;
    let additional = a.finish()?;
    assert_eq!(additional.num_nodes(), 0);
    let extension = base.prepare_extension(&additional).await?;
    let bindings = base
        .inputs()
        .scalar(&names.scalar_input("value")?, 5_i64.into())?
        .finish()?;
    let result = extension
        .query(&[], &[out], &bindings, &extension.inputs().finish()?)
        .await?;
    assert_eq!(result.scalar(&out)?, &ScalarValue::from(6_i64));
    assert_eq!(result.report().physical_plans, 1);
    Ok(())
}
