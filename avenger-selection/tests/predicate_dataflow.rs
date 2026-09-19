mod common;
use avenger_datafusion_dataflow::{
    CachePolicy, DataflowBuilder, Error as FlowError, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{array::Int64Array, datatypes::DataType},
    common::ScalarValue,
    logical_expr::{col, lit, LogicalPlanBuilder},
};
use std::sync::Arc;

#[tokio::test]
async fn three_views_bind_predicates_and_reuse_unchanged_results(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let delay = interval("delay_hist", "delay");
    let distance = interval("distance_hist", "distance");
    let airlines = point("airlines", "carrier");
    let filters = [&delay, &distance, &airlines].map(|p| cross(p.view().clone()));
    let s = state(Resolution::Intersect).apply_all([
        SelectionUpdate::set(&delay, between("delay", 10, 30)),
        SelectionUpdate::set(&distance, between("distance", 500, 1500)),
        SelectionUpdate::set(&airlines, values("carrier", ["AA".into(), "DL".into()])),
    ])?;
    for cache in [CachePolicy::default(), CachePolicy::Disabled] {
        let cached = !matches!(cache, CachePolicy::Disabled);
        let mut b = DataflowBuilder::new();
        let rows = flights();
        let snapshot = TableSnapshot::from_batches(rows.schema(), vec![rows])?;
        let source = b.table_input("flights", snapshot.schema().clone())?;
        let source_node = b.add_plan(
            "source",
            LogicalPlanBuilder::from(source.plan_ref())
                .filter(lit(true))?
                .build()?,
        )?;
        let mut predicates = Vec::new();
        let mut outputs = Vec::new();
        for name in ["delay", "distance", "airlines"] {
            let predicate = b.expr_input(name, DataType::Boolean)?;
            let rows = b.add_plan(
                format!("{name}_rows"),
                LogicalPlanBuilder::from(source_node.plan_ref())
                    .filter(predicate.expr_ref())?
                    .build()?,
            )?;
            outputs.push(b.table_output(name, &rows)?);
            predicates.push(predicate);
        }
        let prepared = Runtime::new(RuntimeConfig {
            cache,
            ..Default::default()
        })?
        .prepare(&b.finish()?)
        .await?;
        let bind = |s: &SelectionSet| -> std::result::Result<_, Box<dyn std::error::Error>> {
            let mut inputs = prepared.inputs().table(&source, snapshot.clone())?;
            for (input, filter) in predicates.iter().zip(&filters) {
                inputs = inputs.expr(input, filter.predicate(s)?)?;
            }
            Ok(inputs.finish()?)
        };
        let before = bind(&s)?;
        let first = prepared.query(&outputs, &[], &before).await?;
        for (out, want) in outputs
            .iter()
            .zip([vec![0, 1, 2, 3], vec![1, 2, 4], vec![1, 2, 5, 6]])
        {
            assert_eq!(ids(first.table(out)?.batches()), want);
        }
        let s2 = s.set(&delay, between("delay", 20, 40))?;
        let after = bind(&s2)?;
        let second = prepared.query(&outputs, &[], &after).await?;
        for (out, want) in outputs
            .iter()
            .zip([vec![0, 1, 2, 3], vec![2, 3, 4], vec![2, 3, 5]])
        {
            assert_eq!(ids(second.table(out)?.batches()), want);
        }
        if cached {
            assert_eq!(second.report().physical_plans, 2);
        }
        assert_eq!(ids(first.table(&outputs[1])?.batches()), vec![1, 2, 4]);
        let restored = prepared.query(&outputs, &[], &before).await?;
        if cached {
            assert_eq!(restored.report().physical_plans, 0);
        }
    }
    Ok(())
}

#[tokio::test]
async fn canonical_updates_reuse_expression_bindings_and_errors_name_the_usage_site(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let p = point("points", "id");
    let s = state(Resolution::Union)
        .set(&p, values("id", [2_i64.into(), 1_i64.into(), 2_i64.into()]))?;
    let reordered = s.set(&p, values("id", [1_i64.into(), 2_i64.into()]))?;
    assert_eq!(
        membership().predicate(&s)?,
        membership().predicate(&reordered)?
    );
    let mut b = DataflowBuilder::new();
    let rows = flights();
    let source = b.table_snapshot(
        "flights",
        TableSnapshot::from_batches(rows.schema(), vec![rows])?,
    )?;
    let pred = b.expr_input("selection", DataType::Boolean)?;
    let filtered = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(source.plan_ref())
            .filter(pred.expr_ref())?
            .build()?,
    )?;
    let output = b.table_output("rows", &filtered)?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = prepared
        .inputs()
        .expr(&pred, membership().predicate(&s)?)?
        .finish()?;
    let first = prepared.query(&[output], &[], &inputs).await?;
    assert_eq!(ids(first.table(&output)?.batches()), vec![1, 2]);
    let inputs = prepared
        .inputs()
        .expr(&pred, membership().predicate(&reordered)?)?
        .finish()?;
    assert_eq!(
        prepared
            .query(&[output], &[], &inputs)
            .await?
            .report()
            .physical_plans,
        0
    );
    let bad = state(Resolution::Union).set(
        &point("missing", "missing"),
        values("missing", [1_i64.into()]),
    )?;
    let err = prepared
        .inputs()
        .expr(&pred, membership().predicate(&bad)?)
        .unwrap_err();
    assert!(matches!(err, FlowError::InvalidExprInput { .. }));
    let message = err.to_string();
    assert!(
        message.contains("selection")
            && message.contains("filtered")
            && message.contains("missing"),
        "{message}"
    );
    let bad = state(Resolution::Union).set(
        &point("bad", "id"),
        values("id", [ScalarValue::Binary(Some(vec![0xff]))]),
    )?;
    assert!(prepared
        .inputs()
        .expr(&pred, membership().predicate(&bad)?)
        .is_err());
    Ok(())
}

#[tokio::test]
async fn nested_facet_bindings_use_chart_assigned_view_ids(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut b = DataflowBuilder::new();
    let data = flights();
    let source = b.table_snapshot(
        "flights",
        TableSnapshot::from_batches(data.schema(), vec![data])?,
    )?;
    let (regions, (carriers, predicate, output)) = b.partition_by(
        "regions",
        source.plan_ref(),
        vec![col("region")],
        |region| {
            region
                .partition_by(
                    "carriers",
                    region.rows().plan_ref(),
                    vec![col("carrier")],
                    |scope| {
                        let input = scope.expr_input("selection", DataType::Boolean)?;
                        let rows = scope.add_plan(
                            "filtered",
                            LogicalPlanBuilder::from(scope.rows().plan_ref())
                                .filter(input.expr_ref())?
                                .build()?,
                        )?;
                        Ok((input, scope.table_output("rows", &rows)?))
                    },
                )
                .map(|(scope, (input, rows))| (scope, input, rows))
        },
    )?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    // The chart maps this ID to the East/AA nested dataflow instance below.
    let origin = ViewId::new("east-aa-histogram")?;
    let p = producer("brush", origin.clone(), &["delay"]);
    let s = state(Resolution::Intersect).set(&p, between("delay", 20, 40))?;
    let east_aa = regions
        .instance([ScalarValue::from("East")])?
        .child(&carriers, [ScalarValue::from("AA")])?;
    let own = cross(origin);
    let inputs = prepared
        .inputs()
        .scope_defaults(&carriers, |inputs| {
            inputs.expr(&predicate, membership().predicate(&s).unwrap())
        })?
        .at(&east_aa, |inputs| {
            inputs.expr(&predicate, own.predicate(&s).unwrap())
        })?
        .finish()?;
    let first = prepared.query(&[output], &[], &inputs).await?;
    let east = first
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("East")])?)
        .unwrap();
    let aa = east
        .scope(&carriers)?
        .get(&carriers.key([ScalarValue::from("AA")])?)
        .unwrap();
    assert_eq!(ids(aa.table(&output)?.batches()), vec![0, 1, 3]);
    let null = east
        .scope(&carriers)?
        .get(&carriers.key([ScalarValue::Utf8(None)])?)
        .unwrap();
    assert!(ids(null.table(&output)?.batches()).is_empty());
    let west = first
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("West")])?)
        .unwrap();
    assert_eq!(
        ids(west
            .scope(&carriers)?
            .get(&carriers.key([ScalarValue::from("DL")])?)
            .unwrap()
            .table(&output)?
            .batches()),
        vec![2]
    );
    let changed = s.set(&p, between("delay", 10, 20))?;
    let rebound = inputs
        .edit()
        .scope_defaults(&carriers, |inputs| {
            inputs.expr(&predicate, membership().predicate(&changed).unwrap())
        })?
        .finish()?;
    let second = prepared.query(&[output], &[], &rebound).await?;
    let east = second
        .scope(&regions)?
        .get(&regions.key([ScalarValue::from("East")])?)
        .unwrap();
    assert_eq!(
        ids(east
            .scope(&carriers)?
            .get(&carriers.key([ScalarValue::from("AA")])?)
            .unwrap()
            .table(&output)?
            .batches()),
        vec![0, 1, 3]
    );
    assert_eq!(
        ids(east
            .scope(&carriers)?
            .get(&carriers.key([ScalarValue::Utf8(None)])?)
            .unwrap()
            .table(&output)?
            .batches()),
        vec![6]
    );
    assert_eq!(ids(aa.table(&output)?.batches()), vec![0, 1, 3]);
    Ok(())
}

#[tokio::test]
async fn finite_and_nan_predicates_are_valid_immutable_expression_inputs(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2]))),
        (
            "x",
            Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                0.0,
                f64::NAN,
                f64::INFINITY,
            ])),
        ),
    ]);
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot(
        "data",
        TableSnapshot::from_batches(data.schema(), vec![data])?,
    )?;
    let pred = b.expr_input("selection", DataType::Boolean)?;
    let rows = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(source.plan_ref())
            .filter(pred.expr_ref())?
            .build()?,
    )?;
    let output = b.table_output("rows", &rows)?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let p = point("point", "x");
    for (value, expected) in [
        (values("x", [f64::NAN.into()]), vec![1]),
        (
            range("x", std::ops::Bound::Unbounded, std::ops::Bound::Unbounded),
            vec![0],
        ),
    ] {
        let s = state(Resolution::Union).set(&p, value)?;
        let input = prepared
            .inputs()
            .expr(&pred, membership().predicate(&s)?)?
            .finish()?;
        let result = prepared.query(&[output], &[], &input).await?;
        assert_eq!(ids(result.table(&output)?.batches()), expected);
        assert_eq!(
            prepared
                .query(&[output], &[], &input)
                .await?
                .report()
                .physical_plans,
            0
        );
    }
    Ok(())
}
