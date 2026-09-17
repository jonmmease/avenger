#[path = "common/scoped.rs"]
mod scoped;
use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{i256, DataType},
        record_batch::RecordBatch,
    },
    datafusion::{
        common::ScalarValue,
        execution::context::SessionContext,
        functions::datetime::expr_fn::now,
        functions_aggregate::expr_fn::max,
        logical_expr::{
            col, create_udf, lit, scalar_subquery, ColumnarValue, Expr, LogicalPlanBuilder,
            Volatility,
        },
    },
    DataflowBuilder, Error, ExecutionConfig, Result, ReuseScope, Runtime, RuntimeConfig,
    TableSnapshot,
};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

#[tokio::test]
async fn nested_mixed_outputs_match_independent_datafusion_queries() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let f = scoped::build(&mut graph)?;
    let root = graph.add_scalar("root", lit(123_i64))?;
    let root = graph.scalar_output("root", &root)?;
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    let source = scoped::sales();
    let east_2025 = f
        .regions
        .instance([ScalarValue::from("East")])?
        .child(&f.region.years, [ScalarValue::from(2025_i32)])?;
    let inputs = prepared
        .inputs()
        .table(&f.sales, source.clone())?
        .scalar(&f.multiplier, ScalarValue::from(2_i64))?
        .scope_defaults(&f.regions, |b| {
            b.scalar(&f.region.limit, ScalarValue::from(10_i64))
        })?
        .scope_defaults(&f.region.years, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(1_i64))?
                .table(&f.region.year.selected, scoped::products(&["A"]))
        })?
        .at(&east_2025, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(3_i64))?
                .table(&f.region.year.selected, scoped::products(&["B"]))
        })?
        .finish()?;
    let result = prepared
        .query(
            &[f.region.year.rows, f.region.year.rows],
            &[root, f.region.year.threshold, f.region.year.maximum],
            &inputs,
        )
        .await?;
    assert_eq!(result.scalar(&root)?, &ScalarValue::from(123_i64));
    assert!(matches!(
        result.table(&f.region.year.rows),
        Err(Error::OutOfScope(_))
    ));
    let context = SessionContext::new();
    for (region_key, region) in result.scope(&f.regions)?.iter() {
        assert!(matches!(
            region.table(&f.region.rows),
            Err(Error::UnrequestedOutput)
        ));
        assert!(matches!(region.scalar(&root), Err(Error::OutOfScope(_))));
        for (year_key, panel) in region.scope(&f.region.years)?.iter() {
            let special = panel.instance() == &east_2025;
            let threshold = if special { 60_i64 } else { 20 };
            let oracle = context
                .read_batch(source.batches()[0].clone())?
                .filter(col("region").eq(lit(region_key.values()[0].clone())))?
                .filter(col("year").eq(lit(year_key.values()[0].clone())))?
                .filter(col("product").eq(lit(if special { "B" } else { "A" })))?
                .filter(col("amount").gt(lit(threshold)))?;
            let table = TableSnapshot::from_batches(
                source.schema().clone(),
                oracle.clone().collect().await?,
            )?;
            assert_eq!(
                scoped::amounts(panel.table(&f.region.year.rows)?),
                scoped::amounts(&table)
            );
            assert_eq!(panel.table(&f.region.year.rows)?.schema(), source.schema());
            assert_eq!(
                panel.scalar(&f.region.year.threshold)?,
                &ScalarValue::from(threshold)
            );
            let maximum = oracle
                .aggregate(Vec::<Expr>::new(), vec![max(col("amount"))])?
                .collect()
                .await?;
            assert_eq!(
                panel.scalar(&f.region.year.maximum)?,
                &ScalarValue::try_from_array(maximum[0].column(0), 0)?
            );
        }
    }
    assert_eq!(result.scope(&f.regions)?.len(), 2);
    assert_eq!(
        result
            .report()
            .scopes
            .iter()
            .find(|s| s.name == "regions/years")
            .unwrap()
            .instances,
        3
    );
    assert_eq!(
        result
            .report()
            .executed_nodes
            .iter()
            .filter(|name| name.as_str() == "regions::regional")
            .count(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn composite_computed_keys_preserve_rows_and_scalar_cardinality_per_instance() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let (panels, (rows, maximum, bad, empty, zero_columns)) = graph.partition_by(
        "panels",
        input.plan_ref(),
        vec![col("region"), (col("year") + lit(1_i32)).alias("next_year")],
        |scope| {
            let local = scope.rows();
            let ordered = LogicalPlanBuilder::from(local.plan_ref())
                .sort(vec![col("amount").sort(false, false)])?
                .limit(0, Some(1))?
                .project(vec![col("amount")])?
                .build()?;
            let maximum = scope.add_scalar("maximum", scalar_subquery(Arc::new(ordered)))?;
            let bad = scope.add_scalar(
                "bad",
                scalar_subquery(Arc::new(
                    LogicalPlanBuilder::from(local.plan_ref())
                        .project(vec![col("amount")])?
                        .build()?,
                )),
            )?;
            let no_rows = LogicalPlanBuilder::from(local.plan_ref())
                .filter(lit(false))?
                .project(vec![col("amount")])?
                .build()?;
            let empty = scope.add_scalar("empty", scalar_subquery(Arc::new(no_rows)))?;
            let zero_columns = scope.add_plan(
                "zero_columns",
                LogicalPlanBuilder::from(local.plan_ref())
                    .project(Vec::<Expr>::new())?
                    .build()?,
            )?;
            Ok((
                scope.table_output("rows", &local)?,
                scope.scalar_output("maximum", &maximum)?,
                scope.scalar_output("bad", &bad)?,
                scope.scalar_output("empty", &empty)?,
                scope.table_output("zero_columns", &zero_columns)?,
            ))
        },
    )?;
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    let source = scoped::data(&[
        (Some("East"), 2024, "A", 10),
        (None, 2024, "A", 20),
        (Some("West"), 2025, "B", 30),
        (Some("East"), 2024, "B", 40),
        (None, 2024, "B", 50),
    ]);
    let source = TableSnapshot::from_batches(
        source.schema().clone(),
        vec![
            source.batches()[0].slice(0, 2),
            source.batches()[0].slice(2, 3),
        ],
    )?;
    let inputs = prepared.inputs().table(&input, source.clone())?.finish()?;
    let result = prepared
        .query(&[rows, zero_columns], &[maximum, empty], &inputs)
        .await?;
    let collection = result.scope(&panels)?;
    assert_eq!(collection.len(), 3);
    for (region, year, expected) in [
        (Some("East"), 2025_i32, vec![10, 40]),
        (Some("West"), 2026, vec![30]),
        (None, 2025, vec![20, 50]),
    ] {
        let key = panels.key([
            ScalarValue::Utf8(region.map(str::to_owned)),
            ScalarValue::from(year),
        ])?;
        let panel = collection.get(&key).unwrap();
        assert_eq!(scoped::amounts(panel.table(&rows)?), expected);
        assert_eq!(panel.table(&rows)?.schema(), source.schema());
        assert_eq!(
            panel.scalar(&maximum)?,
            &ScalarValue::from(*expected.last().unwrap())
        );
        assert_eq!(panel.scalar(&empty)?, &ScalarValue::Int64(None));
        assert_eq!(panel.table(&zero_columns)?.schema().fields().len(), 0);
        assert_eq!(panel.table(&zero_columns)?.num_rows(), expected.len());
    }
    assert!(collection
        .get(&panels.key([ScalarValue::from("East"), ScalarValue::from(2026_i32)])?)
        .is_none());
    let failure = prepared.query(&[rows], &[bad], &inputs).await.unwrap_err();
    let Error::Scoped { instance, source } = failure else {
        panic!("expected scoped cardinality failure")
    };
    assert!(instance.to_string().contains("panels"));
    assert!(source.to_string().contains("bad"));
    let old_address = collection.iter().next().unwrap().1.instance().clone();
    let old_table = collection.iter().next().unwrap().1.table(&rows)?.clone();
    let old_rows = old_table.num_rows();
    let changed = inputs
        .edit()
        .table(&input, scoped::data(&[(Some("North"), 2030, "A", 9)]))?
        .finish()?;
    let next = prepared.query(&[rows], &[], &changed).await?;
    assert_eq!(next.scope(&panels)?.len(), 1);
    drop(result);
    assert_eq!(old_table.num_rows(), old_rows);
    assert!(!old_address.to_string().is_empty());
    Ok(())
}

#[tokio::test]
async fn supported_key_types_group_and_lookup_with_the_same_equivalence() -> Result<()> {
    use ScalarValue::*;
    let samples = [
        Null,
        Boolean(Some(true)),
        Int8(Some(-1)),
        Int16(Some(-2)),
        Int32(Some(-3)),
        Int64(Some(-4)),
        UInt8(Some(1)),
        UInt16(Some(2)),
        UInt32(Some(3)),
        UInt64(Some(4)),
        Utf8(Some("x".into())),
        LargeUtf8(Some("x".into())),
        Utf8View(Some("x".into())),
        Binary(Some(vec![0, 255])),
        LargeBinary(Some(vec![0, 255])),
        BinaryView(Some(vec![0, 255])),
        Date32(Some(1)),
        Date64(Some(86_400_000)),
        TimestampSecond(Some(1), None),
        TimestampMillisecond(Some(1), Some(Arc::from("UTC"))),
        TimestampMicrosecond(Some(1), None),
        TimestampNanosecond(Some(1), None),
        Decimal128(Some(123), 10, 2),
        Decimal256(Some(i256::from_i128(123)), 40, 2),
    ];
    for sample in samples {
        let null = ScalarValue::try_from(sample.data_type())?;
        let array = ScalarValue::iter_to_array([sample.clone(), sample.clone(), null.clone()])?;
        let batch = RecordBatch::try_from_iter(vec![("key", array)])?;
        let source = TableSnapshot::from_batches(batch.schema(), vec![batch])?;
        let mut graph = DataflowBuilder::new();
        let input = graph.table_input("source", source.schema().clone())?;
        let (panels, output) =
            graph.partition_by("panels", input.plan_ref(), vec![col("key")], |scope| {
                scope.table_output("rows", &scope.rows())
            })?;
        let prepared = Runtime::new(RuntimeConfig::default())?
            .prepare(&graph.finish()?)
            .await?;
        let inputs = prepared.inputs().table(&input, source)?.finish()?;
        let result = prepared.query(&[output], &[], &inputs).await?;
        let collection = result.scope(&panels)?;
        assert_eq!(
            collection.len(),
            if sample == null { 1 } else { 2 },
            "{sample:?}"
        );
        assert_eq!(
            collection
                .get(&panels.key([sample.clone()])?)
                .unwrap()
                .table(&output)?
                .num_rows(),
            if sample == null { 3 } else { 2 }
        );
        assert!(collection.get(&panels.key([null])?).is_some());
    }
    Ok(())
}

#[tokio::test]
async fn a_parent_can_have_no_children_and_nested_sources_read_parent_inputs() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("source", scoped::schema())?;
    let (regions, (limit, years, rows)) =
        graph.partition_by("regions", input.plan_ref(), vec![col("region")], |scope| {
            let limit = scope.scalar_input("limit", DataType::Int64)?;
            let source = LogicalPlanBuilder::from(scope.rows().plan_ref())
                .filter(col("amount").gt(limit.expr_ref()))?
                .build()?;
            let (years, rows) =
                scope.partition_by("years", source, vec![col("year")], |scope| {
                    scope.table_output("rows", &scope.rows())
                })?;
            Ok((limit, years, rows))
        })?;
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    let inputs = prepared
        .inputs()
        .table(&input, scoped::sales())?
        .scope_defaults(&regions, |b| b.scalar(&limit, ScalarValue::from(100_i64)))?
        .at(&regions.instance([ScalarValue::from("East")])?, |b| {
            b.scalar(&limit, ScalarValue::from(35_i64))
        })?
        .finish()?;
    let result = prepared.query(&[rows], &[], &inputs).await?;
    assert_eq!(result.scope(&regions)?.len(), 2);
    let west = result
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("West")])?)
        .unwrap();
    assert!(west.scope(&years)?.is_empty());
    let east = result
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("East")])?)
        .unwrap();
    assert_eq!(east.scope(&years)?.len(), 1);
    assert_eq!(
        scoped::amounts(
            east.scope(&years)?
                .get(&years.key([ScalarValue::from(2025_i32)])?)
                .unwrap()
                .table(&rows)?
        ),
        [40, 80]
    );
    Ok(())
}

#[tokio::test]
async fn volatile_values_are_shared_in_defining_frames_and_accessors_do_no_work() -> Result<()> {
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let draw = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::from(
                counter.fetch_add(1, Ordering::SeqCst),
            )))
        }),
    );
    let row_calls = Arc::new(AtomicI64::new(0));
    let counter = row_calls.clone();
    let row_draw = create_udf(
        "row_draw",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |args| {
            let ColumnarValue::Array(values) = &args[0] else {
                panic!("column argument")
            };
            counter.fetch_add(values.len() as i64, Ordering::SeqCst);
            Ok(ColumnarValue::Array(Arc::new(
                Int64Array::from_iter_values(0..values.len() as i64),
            )))
        }),
    );
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let global = graph.add_scalar("global", draw.call(vec![]))?;
    let global_output = graph.scalar_output("global", &global)?;
    let (regions, (years, value, clock, a, b)) =
        graph.partition_by("regions", input.plan_ref(), vec![col("region")], |scope| {
            let regional = scope.add_scalar("regional", draw.call(vec![]))?;
            scope
                .partition_by(
                    "years",
                    scope.rows().plan_ref(),
                    vec![col("year")],
                    |scope| {
                        let local = scope.add_scalar("local", draw.call(vec![]))?;
                        let combined = scope.add_scalar(
                            "combined",
                            global.expr_ref() + regional.expr_ref() + local.expr_ref(),
                        )?;
                        let clock = scope.add_scalar("clock", now())?;
                        let rows = scope.add_plan(
                            "draw_rows",
                            LogicalPlanBuilder::from(scope.rows().plan_ref())
                                .project(vec![row_draw.call(vec![col("amount")]).alias("amount")])?
                                .build()?,
                        )?;
                        let copy = scope.add_plan("copy", rows.plan_ref())?;
                        Ok((
                            scope.scalar_output("value", &combined)?,
                            scope.scalar_output("clock", &clock)?,
                            scope.table_output("a", &rows)?,
                            scope.table_output("b", &copy)?,
                        ))
                    },
                )
                .map(|(years, (value, clock, a, b))| (years, value, clock, a, b))
        })?;
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    assert!(prepared
        .explain()
        .nodes
        .iter()
        .filter(|node| ["combined", "clock", "draw_rows", "copy"].contains(&node.name.as_str()))
        .all(|node| node.reuse_scope == ReuseScope::EvaluationLocal));
    let inputs = prepared.inputs().table(&input, scoped::sales())?.finish()?;
    let result = prepared
        .query(&[a, b, a], &[value, clock, global_output, value], &inputs)
        .await?;
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(row_calls.load(Ordering::SeqCst), 6);
    let physical_plans = result.report().physical_plans;
    for _ in 0..5 {
        for (_, region) in result.scope(&regions)?.iter() {
            for (key, panel) in region.scope(&years)?.iter() {
                assert_eq!(
                    scoped::amounts(panel.table(&a)?),
                    scoped::amounts(panel.table(&b)?)
                );
                assert!(region.scope(&years)?.get(key).is_some());
                assert_eq!(
                    panel.scalar(&clock)?,
                    &ScalarValue::TimestampNanosecond(
                        result.report().query_start_time.timestamp_nanos_opt(),
                        None
                    )
                );
                panel.scalar(&value)?;
            }
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(row_calls.load(Ordering::SeqCst), 6);
    assert_eq!(result.report().physical_plans, physical_plans);
    let requested = [value];
    let (left, right) = tokio::join!(
        prepared.query(&[], &requested, &inputs),
        prepared.query(&[], &requested, &inputs)
    );
    assert_ne!(left?.report().evaluation_id, right?.report().evaluation_id);
    assert_eq!(calls.load(Ordering::SeqCst), 18);
    Ok(())
}

#[tokio::test]
async fn resource_failure_releases_all_frames_and_reservations() -> Result<()> {
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let (panels, rows) = graph.partition_by(
        "panels",
        input.plan_ref(),
        vec![col("region"), col("year")],
        |scope| scope.table_output("rows", &scope.rows()),
    )?;
    let prepared = Runtime::new(RuntimeConfig {
        cache: avenger_datafusion_dataflow::CachePolicy::Disabled,
        execution: ExecutionConfig {
            max_active_queries: 1,
            max_materialized_bytes: 32_000,
        },
        ..RuntimeConfig::default()
    })?
    .prepare(&graph.finish()?)
    .await?;
    let inputs = prepared
        .inputs()
        .table(
            &input,
            scoped::data(&vec![(Some("East"), 2025, "A", 1); 1000]),
        )?
        .finish()?;
    assert!(prepared.query(&[rows], &[], &inputs).await.is_err());
    let small = inputs
        .edit()
        .table(&input, scoped::data(&[(Some("East"), 2025, "A", 1)]))?
        .finish()?;
    let result = prepared.query(&[rows], &[], &small).await?;
    assert_eq!(result.scope(&panels)?.len(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_releases_the_single_outer_permit_and_active_memory() -> Result<()> {
    use std::sync::{atomic::AtomicBool, Condvar, Mutex};
    let block_once = Arc::new(AtomicBool::new(true));
    let entered = Arc::new(tokio::sync::Notify::new());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (udf_entered, udf_gate) = (entered.clone(), gate.clone());
    let blocker = create_udf(
        "block_once",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            if block_once.swap(false, Ordering::SeqCst) {
                udf_entered.notify_one();
                let (lock, wake) = &*udf_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
            }
            Ok(ColumnarValue::Scalar(ScalarValue::from(1_i64)))
        }),
    );
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let (panels, value) =
        graph.partition_by("panels", input.plan_ref(), vec![col("region")], |scope| {
            let value = scope.add_scalar("value", blocker.call(vec![]))?;
            scope.scalar_output("value", &value)
        })?;
    let prepared = Runtime::new(RuntimeConfig {
        cache: avenger_datafusion_dataflow::CachePolicy::Disabled,
        execution: ExecutionConfig {
            max_active_queries: 1,
            max_materialized_bytes: 32_000,
        },
        ..RuntimeConfig::default()
    })?
    .prepare(&graph.finish()?)
    .await?;
    let inputs = prepared.inputs().table(&input, scoped::sales())?.finish()?;
    let (running, bindings) = (prepared.clone(), inputs.clone());
    let mut task = tokio::spawn(async move { running.query(&[], &[value], &bindings).await });
    tokio::select! {
        _ = entered.notified() => {},
        completed = &mut task => panic!("query completed before reaching the scoped UDF: {completed:?}"),
    }
    task.abort();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(task.await.unwrap_err().is_cancelled());
    let result = prepared.query(&[], &[value], &inputs).await?;
    assert_eq!(result.scope(&panels)?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn nullable_subquery_keys_have_nullable_schemas_and_one_null_group() -> Result<()> {
    let key = LogicalPlanBuilder::empty(false)
        .project(vec![lit(1_i64).alias("key")])?
        .build()?;
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let (panels, rows) = graph.partition_by(
        "panels",
        input.plan_ref(),
        vec![scalar_subquery(Arc::new(key)).alias("key")],
        |scope| scope.table_output("rows", &scope.rows()),
    )?;
    assert!(panels.key_schema().field(0).is_nullable());
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    let inputs = prepared.inputs().table(&input, scoped::sales())?.finish()?;
    let result = prepared.query(&[rows], &[], &inputs).await?;
    assert_eq!(result.scope(&panels)?.len(), 1);
    assert_eq!(
        result
            .scope(&panels)?
            .get(&panels.key([ScalarValue::Int64(None)])?)
            .unwrap()
            .table(&rows)?
            .num_rows(),
        6
    );
    Ok(())
}

#[tokio::test]
async fn volatile_partition_keys_run_once_before_child_transforms() -> Result<()> {
    let rows_processed = Arc::new(AtomicI64::new(0));
    let counter = rows_processed.clone();
    let key = create_udf(
        "key",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |args| {
            let ColumnarValue::Array(values) = &args[0] else {
                panic!("column expected")
            };
            counter.fetch_add(values.len() as i64, Ordering::SeqCst);
            Ok(ColumnarValue::Array(Arc::new(Int64Array::from(vec![
                1;
                values
                    .len(
                    )
            ]))))
        }),
    );
    let mut graph = DataflowBuilder::new();
    let input = graph.table_input("sales", scoped::schema())?;
    let (panels, (rows, constant)) = graph.partition_by(
        "panels",
        input.plan_ref(),
        vec![key.call(vec![col("amount")])],
        |scope| {
            let filtered = scope.add_plan(
                "filtered",
                LogicalPlanBuilder::from(scope.rows().plan_ref())
                    .filter(lit(false))?
                    .build()?,
            )?;
            let constant = scope.add_scalar("constant", lit(7_i64))?;
            Ok((
                scope.table_output("rows", &filtered)?,
                scope.scalar_output("constant", &constant)?,
            ))
        },
    )?;
    let prepared = Runtime::new(RuntimeConfig::default())?
        .prepare(&graph.finish()?)
        .await?;
    assert!(prepared
        .explain()
        .nodes
        .iter()
        .all(|node| node.reuse_scope == ReuseScope::EvaluationLocal));
    let inputs = prepared.inputs().table(&input, scoped::sales())?.finish()?;
    for expected_calls in [6, 12] {
        let result = prepared.query(&[rows, rows], &[constant], &inputs).await?;
        assert_eq!(result.scope(&panels)?.len(), 1);
        let panel = result
            .scope(&panels)?
            .get(&panels.key([ScalarValue::from(1_i64)])?)
            .unwrap();
        assert_eq!(panel.table(&rows)?.num_rows(), 0);
        assert_eq!(panel.scalar(&constant)?, &ScalarValue::from(7_i64));
        assert_eq!(rows_processed.load(Ordering::SeqCst), expected_calls);
    }
    Ok(())
}
