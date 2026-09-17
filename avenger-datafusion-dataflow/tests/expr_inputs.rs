mod common;
use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datafusion::{
        common::ScalarValue,
        functions::datetime::expr_fn::now,
        functions_aggregate::expr_fn::sum,
        logical_expr::{
            col, create_udf, lit, scalar_subquery, ColumnarValue, Expr, JoinType,
            LogicalPlanBuilder, Volatility,
        },
    },
    CachePolicy, DataflowBuilder, Error, InputHandle, Result, Runtime, RuntimeConfig,
    TableSnapshot,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn rows() -> TableSnapshot {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int64, false),
        Field::new("y", DataType::Int64, false),
    ]));
    TableSnapshot::from_batches(
        schema.clone(),
        vec![RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(Int64Array::from(vec![30, 20, 10])),
            ],
        )
        .unwrap()],
    )
    .unwrap()
}

#[tokio::test]
async fn bindings_change_columns_preserve_schema_and_reuse_results() -> Result<()> {
    for cache in [CachePolicy::default(), CachePolicy::Disabled] {
        let enabled = !matches!(cache, CachePolicy::Disabled);
        let mut b = DataflowBuilder::new();
        let asset = b.table_snapshot("asset", rows())?;
        let source = b.add_plan(
            "source",
            LogicalPlanBuilder::from(asset.plan_ref())
                .project(vec![col("x"), col("y")])?
                .build()?,
        )?;
        let predicate = b.expr_input("selection", DataType::Boolean)?;
        let measure = b.expr_input("measure", DataType::Int64)?;
        let output = b.add_plan(
            "selected",
            LogicalPlanBuilder::from(source.plan_ref())
                .filter(predicate.expr_ref())?
                .project(vec![measure.expr_ref().alias("value")])?
                .build()?,
        )?;
        let schema = output.schema().clone();
        let out = b.table_output("rows", &output)?;
        let runtime = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?;
        let p = runtime.prepare(&b.finish()?).await?;
        let a = p
            .inputs()
            .expr(&predicate, col("x").gt(lit(1_i64)))?
            .expr(&measure, col("y"))?
            .finish()?;
        let first = p.query(&[out], &[], &a).await?;
        assert_eq!(common::values(first.table(&out)?), vec![20, 10]);
        assert_eq!(first.table(&out)?.schema().as_ref(), schema.as_arrow());
        let b = a
            .edit()
            .expr(&predicate, col("y").gt(lit(15_i64)))?
            .expr(&measure, col("x").alias("ignored"))?
            .finish()?;
        let second = p.query(&[out], &[], &b).await?;
        assert_eq!(common::values(second.table(&out)?), vec![1, 2]);
        assert_eq!(second.table(&out)?.schema().as_ref(), schema.as_arrow());
        let again = p.query(&[out], &[], &a).await?;
        assert_eq!(common::values(again.table(&out)?), vec![20, 10]);
        if enabled {
            assert_eq!(second.report().physical_plans, 1);
            assert_eq!(again.report().physical_plans, 0);
        } else {
            assert_eq!(again.report().physical_plans, 2);
        }
        let nulls = a
            .edit()
            .expr(&predicate, lit(ScalarValue::Boolean(None)))?
            .finish()?;
        assert_eq!(
            p.query(&[out], &[], &nulls).await?.table(&out)?.num_rows(),
            0
        );
        assert!(p
            .inputs()
            .expr(&predicate, col("missing").gt(lit(1)))
            .is_err());
        assert!(p.inputs().expr(&measure, lit("wrong type")).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn every_consumer_has_its_own_alias_and_schema_context() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("source", rows())?;
    let expr = b.expr_input("measure", DataType::Int64)?;
    let mut outs = Vec::new();
    for alias in ["left", "right"] {
        let node = b.add_plan(
            alias,
            LogicalPlanBuilder::from(source.plan_ref())
                .alias(alias)?
                .project(vec![expr.expr_ref().alias("value")])?
                .build()?,
        )?;
        outs.push(b.table_output(alias, &node)?);
    }
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = p.inputs().expr(&expr, col("x") + lit(1_i32))?.finish()?;
    let result = p.query(&outs, &[], &inputs).await?;
    for out in &outs {
        assert_eq!(common::values(result.table(out)?), vec![2, 3, 4]);
    }
    assert!(p.inputs().expr(&expr, col("left.x")).is_err());

    let mut b = DataflowBuilder::new();
    let input = b.expr_input("value", DataType::Int64)?;
    let x = b.table_snapshot("x", rows())?;
    let a = b.add_plan(
        "a",
        LogicalPlanBuilder::from(x.plan_ref())
            .project(vec![input.expr_ref()])?
            .build()?,
    )?;
    b.table_output("a", &a)?;
    let other = b.table_snapshot("other", common::snapshot(&[1]))?;
    b.add_plan(
        "unrequested",
        LogicalPlanBuilder::from(other.plan_ref())
            .project(vec![input.expr_ref()])?
            .build()?,
    )?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    assert!(
        p.inputs().expr(&input, col("x")).is_err(),
        "unrequested consumers still validate"
    );
    Ok(())
}

#[tokio::test]
async fn unused_bindings_differ_from_empty_scalar_contexts() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let unused = b.expr_input("unused", DataType::Boolean)?;
    let value = b.add_scalar("value", lit(7_i64))?;
    let out = b.scalar_output("value", &value)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    assert!(matches!(p.inputs().finish(), Err(Error::MissingInput(_))));
    let inputs = p.inputs().expr(&unused, col("anything"))?.finish()?;
    let first = p.query(&[], &[out], &inputs).await?;
    let changed = inputs
        .edit()
        .expr(&unused, col("somewhere.else"))?
        .finish()?;
    let second = p.query(&[], &[out], &changed).await?;
    assert_eq!(first.scalar(&out)?, second.scalar(&out)?);
    assert_eq!(second.report().physical_plans, 0);
    assert!(p.inputs().expr(&unused, now()).is_err());
    assert!(p.inputs().expr(&unused, unused.expr_ref()).is_err());
    assert!(p.inputs().expr(&unused, sum(col("x"))).is_err());
    assert!(p
        .inputs()
        .expr(
            &unused,
            scalar_subquery(Arc::new(
                LogicalPlanBuilder::empty(true)
                    .project(vec![lit(1)])?
                    .build()?
            ))
        )
        .is_err());

    let mut b = DataflowBuilder::new();
    let expr = b.expr_input("expr", DataType::Int64)?;
    let value = b.add_scalar("scalar", expr.expr_ref())?;
    let out = b.scalar_output("scalar", &value)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    assert!(p.inputs().expr(&expr, col("anything")).is_err());
    let inputs = p.inputs().expr(&expr, lit(9_i64))?.finish()?;
    assert_eq!(
        p.query(&[], &[out], &inputs).await?.scalar(&out)?,
        &ScalarValue::Int64(Some(9))
    );
    Ok(())
}

#[tokio::test]
async fn immutable_functions_are_not_executed_during_validation_and_have_distinct_keys(
) -> Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let make = |value, volatility| {
        let calls = calls.clone();
        create_udf(
            "same_name",
            vec![],
            DataType::Int64,
            volatility,
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(value))))
            }),
        )
    };
    let mut b = DataflowBuilder::new();
    let expr = b.expr_input("expression", DataType::Int64)?;
    let scalar = b.add_scalar("value", expr.expr_ref())?;
    let out = b.scalar_output("value", &scalar)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let a = p
        .inputs()
        .expr(&expr, make(1, Volatility::Immutable).call(vec![]))?
        .finish()?;
    let b = a
        .edit()
        .expr(&expr, make(2, Volatility::Immutable).call(vec![]))?
        .finish()?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        p.query(&[], &[out], &a).await?.scalar(&out)?,
        &ScalarValue::Int64(Some(1))
    );
    assert_eq!(
        p.query(&[], &[out], &b).await?.scalar(&out)?,
        &ScalarValue::Int64(Some(2))
    );
    let before = calls.load(Ordering::SeqCst);
    for volatility in [Volatility::Stable, Volatility::Volatile] {
        assert!(a
            .edit()
            .expr(&expr, make(3, volatility).call(vec![]))
            .is_err());
    }
    assert_eq!(p.query(&[], &[out], &a).await?.report().physical_plans, 0);
    assert_eq!(calls.load(Ordering::SeqCst), before);
    Ok(())
}

#[tokio::test]
async fn aggregate_arguments_and_join_filters_bind_in_row_context() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("source", rows())?;
    let measure = b.expr_input("measure", DataType::Int64)?;
    let predicate = b.expr_input("predicate", DataType::Boolean)?;
    let agg = b.add_plan(
        "total",
        LogicalPlanBuilder::from(source.plan_ref())
            .aggregate(
                Vec::<Expr>::new(),
                vec![sum(measure.expr_ref()).alias("total")],
            )?
            .build()?,
    )?;
    let out = b.table_output("total", &agg)?;
    let joined = b.add_plan(
        "joined",
        LogicalPlanBuilder::from(source.plan_ref())
            .alias("l")?
            .join(
                LogicalPlanBuilder::from(source.plan_ref())
                    .alias("r")?
                    .build()?,
                JoinType::Inner,
                (vec!["l.x"], vec!["r.x"]),
                Some(predicate.expr_ref()),
            )?
            .build()?,
    )?;
    let joined_out = b.table_output("joined", &joined)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = p
        .inputs()
        .expr(&measure, col("y"))?
        .expr(&predicate, col("l.x").gt(lit(1_i64)))?
        .finish()?;
    let result = p.query(&[out, joined_out], &[], &inputs).await?;
    assert_eq!(common::values(result.table(&out)?), vec![60]);
    assert_eq!(result.table(&joined_out)?.num_rows(), 2);
    assert!(
        p.inputs().expr(&predicate, col("x").gt(lit(1))).is_err(),
        "ambiguous joined column"
    );
    Ok(())
}

#[tokio::test]
async fn interfaces_and_protobuf_preserve_kinds_and_fresh_ownership() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let table = b.table_input("table", common::schema())?;
    b.scalar_input("scalar", DataType::Int64)?;
    let expr = b.expr_input("expr", DataType::Boolean)?;
    let node = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(table.plan_ref())
            .filter(expr.expr_ref())?
            .build()?,
    )?;
    b.table_output("out", &node)?;
    let flow = b.finish()?;
    let handles: Vec<_> = flow.interface().root().inputs().collect();
    assert_eq!(
        handles.iter().map(InputHandle::name).collect::<Vec<_>>(),
        ["table", "scalar", "expr"]
    );
    assert!(matches!(handles[0], InputHandle::Table(_)));
    assert!(matches!(handles[1], InputHandle::Scalar(_)));
    assert!(matches!(handles[2], InputHandle::Expr(_)));
    assert!(matches!(
        flow.interface().root().scalar_input("expr"),
        Err(Error::InputKindMismatch { actual: "expr", .. })
    ));
    let runtime = Runtime::new(Default::default())?;
    let decoded = runtime.decode_dataflow(&flow.to_bytes()?)?;
    let names = decoded.interface().root();
    let p = runtime.prepare(&decoded).await?;
    assert!(matches!(
        p.inputs().expr(&expr, lit(true)),
        Err(Error::ForeignHandle)
    ));
    let inputs = p
        .inputs()
        .table(&names.table_input("table")?, common::snapshot(&[1, 2, 3]))?
        .scalar(&names.scalar_input("scalar")?, 0_i64.into())?
        .expr(&names.expr_input("expr")?, col("value").gt(lit(1_i64)))?
        .finish()?;
    let out = names.table_output("out")?;
    assert_eq!(
        common::values(p.query(&[out], &[], &inputs).await?.table(&out)?),
        vec![2, 3]
    );
    assert_eq!(
        p.query(&[out], &[], &inputs).await?.report().physical_plans,
        0
    );
    Ok(())
}

#[tokio::test]
async fn expression_keys_preserve_nested_float_bits_and_charge_literal_buffers() -> Result<()> {
    use avenger_datafusion_dataflow::{arrow::array::Float64Array, CacheConfig};
    let list = |bits| {
        ScalarValue::List(ScalarValue::new_list(
            &[ScalarValue::Float64(Some(f64::from_bits(bits)))],
            &DataType::Float64,
            true,
        ))
    };
    let mut b = DataflowBuilder::new();
    let input = b.expr_input("value", list(0).data_type())?;
    let scalar = b.add_scalar("value", input.expr_ref())?;
    let out = b.scalar_output("value", &scalar)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    for bits in [0, 1_u64 << 63, 0x7ff8000000000001, 0x7ff8000000000002, 0] {
        let inputs = p.inputs().expr(&input, lit(list(bits)))?.finish()?;
        let result = p.query(&[], &[out], &inputs).await?;
        let ScalarValue::List(array) = result.scalar(&out)? else {
            panic!("expected list")
        };
        let values = array.value(0);
        assert_eq!(
            values
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(0)
                .to_bits(),
            bits
        );
    }
    let mut b = DataflowBuilder::new();
    let input = b.expr_input("predicate", DataType::Boolean)?;
    let scalar = b.add_scalar("result", input.expr_ref())?;
    let out = b.scalar_output("result", &scalar)?;
    let runtime = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_bytes: 2048,
            max_entries: 10,
        }),
        ..Default::default()
    })?;
    let p = runtime.prepare(&b.finish()?).await?;
    let value = "x".repeat(20_000);
    let inputs = p
        .inputs()
        .expr(&input, lit(value.clone()).eq(lit(value)))?
        .finish()?;
    let result = p.query(&[], &[out], &inputs).await?;
    assert_eq!(result.scalar(&out)?, &ScalarValue::Boolean(Some(true)));
    assert_eq!(
        runtime.cache_stats().entries,
        0,
        "literal keys count toward the cache budget"
    );
    assert_eq!(
        p.query(&[], &[out], &inputs).await?.report().physical_plans,
        1
    );
    Ok(())
}

#[tokio::test]
async fn equijoin_keys_and_existing_subqueries_keep_local_contexts() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("source", rows())?;
    let key = b.expr_input("key", DataType::Int64)?;
    let predicate = b.expr_input("predicate", DataType::Boolean)?;
    let left = LogicalPlanBuilder::from(source.plan_ref())
        .alias("l")?
        .build()?;
    let right = LogicalPlanBuilder::from(source.plan_ref())
        .alias("r")?
        .build()?;
    let joined = b.add_plan(
        "joined",
        LogicalPlanBuilder::from(left)
            .join_with_expr_keys(
                right,
                JoinType::Inner,
                (
                    vec![col("l.x") + key.expr_ref()],
                    vec![col("r.x") + key.expr_ref()],
                ),
                None,
            )?
            .build()?,
    )?;
    let table = b.table_output("joined", &joined)?;
    let subquery = LogicalPlanBuilder::from(source.plan_ref())
        .alias("sub")?
        .filter(predicate.expr_ref())?
        .aggregate(Vec::<Expr>::new(), vec![sum(col("y"))])?
        .build()?;
    let total = b.add_scalar("total", scalar_subquery(Arc::new(subquery)))?;
    let scalar = b.scalar_output("total", &total)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = p
        .inputs()
        .expr(&key, col("x"))?
        .expr(&predicate, col("sub.x").gt(lit(1_i64)))?
        .finish()?;
    let result = p.query(&[table], &[scalar], &inputs).await?;
    assert_eq!(result.table(&table)?.num_rows(), 3);
    assert_eq!(result.scalar(&scalar)?, &ScalarValue::Int64(Some(30)));
    assert!(p.inputs().expr(&key, col("l.x")).is_err());
    assert!(p
        .inputs()
        .expr(&predicate, col("l.x").gt(lit(1_i64)))
        .is_err());
    Ok(())
}
