mod common;

use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        datatypes::DataType,
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::{count, sum},
    logical_expr::{
        col, create_udf, lit, Expr, ExprFunctionExt, LogicalPlan, LogicalPlanBuilder, Volatility,
    },
    prelude::SessionContext,
};
use std::{
    ops::Bound,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

async fn run(plan: LogicalPlan) -> Vec<RecordBatch> {
    SessionContext::new()
        .execute_logical_plan(plan)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap()
}
fn rows(batches: &[RecordBatch]) -> Vec<String> {
    let mut rows = batches
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(|r| {
                b.columns()
                    .iter()
                    .map(|a| ScalarValue::try_from_array(a, r).unwrap().to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            })
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}
fn source(batch: RecordBatch) -> LogicalPlan {
    SessionContext::new()
        .read_batch(batch)
        .unwrap()
        .into_unoptimized_plan()
}
async fn compare(query: &SelectionQuery, family: &QueryFamily, state: &SelectionSet) {
    let direct = query.logical_plan(state).unwrap();
    let BoundQuery::Preaggregated {
        materialization,
        aggregate,
    } = family.bind(state).unwrap()
    else {
        panic!(
            "expected preaggregation: {:?}",
            family.bind(state).unwrap().explain()
        );
    };
    let expected_schema = direct.schema().clone();
    // Materialize and rescan real batches, rather than allowing an optimizer to
    // collapse both stages into one query and hide an invalid state rewrite.
    let schema = Arc::new(materialization.schema().as_arrow().clone());
    let batches = run(materialization).await;
    let table = datafusion::datasource::MemTable::try_new(schema, vec![batches]).unwrap();
    let relation = SessionContext::new()
        .read_table(Arc::new(table))
        .unwrap()
        .into_unoptimized_plan();
    let actual = aggregate.over(relation).unwrap();
    assert_eq!(actual.schema(), &expected_schema);
    let expected = run(direct).await;
    let actual = run(actual).await;
    if let (Some(a), Some(e)) = (actual.first(), expected.first()) {
        assert_eq!(a.schema(), e.schema());
    }
    assert_eq!(rows(&actual), rows(&expected));
}

#[tokio::test]
async fn counts_preserve_groups_nullability_aliases_and_empty_input() -> TestResult {
    let data = batch(vec![
        (
            "delay",
            Arc::new(Int64Array::from(vec![
                Some(0),
                Some(1),
                Some(1),
                Some(2),
                None,
            ])),
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec![
                Some("AA"),
                Some("AA"),
                None,
                None,
                Some("DL"),
            ])),
        ),
    ]);
    let focus = interval("brush", "delay");
    for empty in [EmptySelection::MatchAll, EmptySelection::MatchNone] {
        let filter = filter(
            SelectionConsumer::new(view("receiver")),
            SelectionFilter::membership(&id(), empty),
        );
        let inactive = state(Resolution::Intersect);
        let states = [
            inactive.clone(),
            inactive.apply(&id(), SelectionUpdate::set(&focus, between("delay", 1, 3)))?,
            inactive.apply(&id(), SelectionUpdate::set(&focus, between("delay", 8, 9)))?,
            inactive.apply(&id(), SelectionUpdate::set(&focus, values("delay", [])))?,
            inactive.apply(
                &id(),
                SelectionUpdate::set(&focus, values("delay", [ScalarValue::Int64(None)])),
            )?,
        ];
        for groups in [vec![], vec![col("carrier")], vec![col("delay")]] {
            for data in [data.clone(), data.slice(0, 0)] {
                let query = filter.query(source(data), |rows| {
                    LogicalPlanBuilder::from(rows)
                        .aggregate(
                            groups.clone(),
                            vec![
                                count(lit(1_i64)).alias("rows"),
                                count(col("carrier")).alias("valid"),
                            ],
                        )?
                        .build()
                })?;
                let family = query.plan(&inactive).focus(&focus).build()?;
                assert_eq!(family.explain().strategy, QueryStrategy::Preaggregated);
                for snapshot in &states {
                    compare(&query, &family, snapshot).await;
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn fixed_predicates_and_suffix_preserve_original_group_names() -> TestResult {
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .filter(col("region").eq(lit("East")))?
            .aggregate(
                vec![(col("distance") / lit(500_i64)).alias("bin")],
                vec![count(col("carrier")).alias("n")],
            )?
            .filter(col("n").gt(lit(0_i64)))?
            .project(vec![
                col("bin").alias("bucket"),
                (col("n") + lit(1_i64)).alias("display"),
            ])?
            .sort(vec![col("bucket").sort(false, true)])?
            .alias("histogram")?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    for bounds in [(0, 100), (10, 30), (80, 100)] {
        let selected = inactive.apply(
            &id(),
            SelectionUpdate::set(&focus, between("delay", bounds.0, bounds.1)),
        )?;
        compare(&query, &family, &selected).await;
    }
    let fixed = point("airlines", "carrier");
    let selected = inactive.apply(
        &id(),
        SelectionUpdate::set(&fixed, values("carrier", ["AA".into()])),
    )?;
    compare(&query, &family, &selected).await;
    let selected = selected.apply(
        &id(),
        SelectionUpdate::set(&fixed, values("carrier", ["DL".into()])),
    )?;
    compare(&query, &family, &selected).await;
    Ok(())
}

#[tokio::test]
async fn correlated_keys_mapping_and_pixel_membership() -> TestResult {
    use avenger_scales_datafusion::BuiltinScale;
    use datafusion::arrow::array::Float32Array;
    let original = producer(
        "brush",
        view("focus"),
        SelectionKind::Interval,
        &["x", "category"],
    );
    let grid = |size| {
        PixelGrid::new(
            BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0.0, 40.0])),
            Arc::new(Float32Array::from(vec![40.0, 0.0])),
            Default::default(),
            0.0,
            size,
        )
    };
    let pixels = original.with_pixel_grids([(ProjectionId::new("x")?, grid(10.0)?)])?;
    let consumer = SelectionConsumer::new(view("target"))
        .with_projection(original.address(), &ProjectionId::new("x")?, col("delay"))?
        .with_projection(
            original.address(),
            &ProjectionId::new("category")?,
            col("carrier"),
        )?;
    let query = filter(consumer, SelectionFilter::cross_filter([&id()])).query(
        source(flights()),
        |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![col("region")], vec![count(lit(1_i64)).alias("n")])?
                .build()
        },
    )?;
    let inactive = state(Resolution::Intersect);
    for focus in [&original, &pixels] {
        let family = query.plan(&inactive).focus(focus).build()?;
        let values = SelectionValue::Tuples(vec![
            SelectionTuple {
                terms: vec![
                    term(
                        "x",
                        ValueTest::Range {
                            lower: Bound::Included(0_i64.into()),
                            upper: Bound::Excluded(15_i64.into()),
                        },
                    ),
                    term("category", ValueTest::Equal("AA".into())),
                ],
            },
            SelectionTuple {
                terms: vec![
                    term(
                        "x",
                        ValueTest::Range {
                            lower: Bound::Included(15_i64.into()),
                            upper: Bound::Included(30_i64.into()),
                        },
                    ),
                    term("category", ValueTest::Equal("DL".into())),
                ],
            },
        ]);
        let selected = inactive.apply(&id(), SelectionUpdate::set(focus, values))?;
        compare(&query, &family, &selected).await;
        compare(&query, &family, &inactive).await;
        if focus == &pixels {
            let resized = pixels.with_pixel_grids([(ProjectionId::new("x")?, grid(1.0)?)])?;
            let changed = selected.apply(
                &id(),
                SelectionUpdate::set(
                    &resized,
                    selected
                        .get(&id())?
                        .contributions()
                        .next()
                        .unwrap()
                        .value()
                        .clone(),
                ),
            )?;
            let bound = family.bind(&changed)?;
            assert_eq!(
                bound.explain().direct_reason,
                Some(DirectReason::IncompatibleFocus)
            );
            let BoundQuery::Direct { plan, .. } = bound else {
                unreachable!()
            };
            assert_eq!(
                rows(&run(plan).await),
                rows(&run(query.logical_plan(&changed)?).await)
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn factorization_checks_resolution_empty_policies_and_exclusions() -> TestResult {
    let focus = interval("focus", "delay");
    let other = point("other", "carrier");
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64))])?
            .build()
    })?;
    let inactive = state(Resolution::Intersect);
    let family = query.plan(&inactive).focus(&focus).build()?;
    for resolution in [Resolution::Global, Resolution::Union] {
        let changed = state(resolution).apply(
            &id(),
            SelectionUpdate::set(&other, values("carrier", ["AA".into()])),
        )?;
        assert_eq!(
            family.bind(&changed)?.explain().direct_reason,
            Some(DirectReason::UnsupportedFactorization)
        );
    }
    for mode in [SelectionMode::Membership, SelectionMode::CrossFilter] {
        let consumer = SelectionConsumer::new(other.address().origin.clone());
        let tree = SelectionFilter::Selection {
            id: id(),
            usage: SelectionUse {
                mode,
                empty: EmptySelection::MatchNone,
            },
        };
        let query = filter(consumer.clone(), tree.clone()).query(source(flights()), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64))])?
                .build()
        })?;
        let family = query.plan(&inactive).focus(&focus).build()?;
        let excluded =
            inactive.apply(&id(), SelectionUpdate::set(&other, values("carrier", [])))?;
        compare(&query, &family, &inactive).await;
        compare(&query, &family, &excluded).await;
        for tree in [
            SelectionFilter::Not(Box::new(tree.clone())),
            SelectionFilter::Any(vec![tree]),
        ] {
            let query = filter(consumer.clone(), tree).query(source(flights()), |rows| {
                LogicalPlanBuilder::from(rows)
                    .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64))])?
                    .build()
            })?;
            assert_eq!(
                query
                    .plan(&inactive)
                    .focus(&focus)
                    .build()?
                    .explain()
                    .direct_reason,
                Some(DirectReason::UnsupportedFactorization)
            );
        }
    }
    Ok(())
}

#[test]
fn unsupported_recipes_and_query_shapes_keep_direct_execution() -> TestResult {
    let inactive = state(Resolution::Intersect);
    let focus = interval("brush", "delay");
    for aggregate in [
        sum(col("delay")),
        count(col("carrier")).distinct().build()?,
        count(col("carrier"))
            .filter(col("delay").gt(lit(0_i64)))
            .build()?,
        count(col("delay") + lit(1_i64)),
    ] {
        let query = membership().query(source(flights()), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(Vec::<Expr>::new(), vec![aggregate])?
                .build()
        })?;
        assert_eq!(
            query
                .plan(&inactive)
                .focus(&focus)
                .build()?
                .explain()
                .direct_reason,
            Some(DirectReason::UnsupportedAggregate)
        );
    }
    for repeated in [false, true] {
        let query = membership().query(source(flights()), |rows| {
            let mut builder = LogicalPlanBuilder::from(rows.clone());
            if repeated {
                builder = builder.union(rows)?;
            } else {
                builder = builder.limit(0, Some(2))?;
            }
            builder
                .aggregate(Vec::<Expr>::new(), vec![count(lit(1_i64))])?
                .build()
        })?;
        assert_eq!(
            query
                .plan(&inactive)
                .focus(&focus)
                .build()?
                .explain()
                .direct_reason,
            Some(DirectReason::UnsupportedQueryShape)
        );
    }
    Ok(())
}

#[test]
fn planning_and_binding_do_not_invoke_projection_functions_and_over_checks_schema() -> TestResult {
    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = calls.clone();
    let udf = create_udf(
        "observe",
        vec![DataType::Int64],
        DataType::Int64,
        Volatility::Immutable,
        Arc::new(move |args| {
            invoked.fetch_add(1, Ordering::Relaxed);
            Ok(args[0].clone())
        }),
    );
    let focus = ProducerDefinition::new(
        address("focus", view("focus")),
        SelectionKind::Interval,
        vec![Projection::new(
            ProjectionId::new("delay")?,
            udf.call(vec![col("delay")]),
        )?],
    )?;
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(vec![col("carrier")], vec![count(lit(1_i64))])?
            .build()
    })?;
    let inactive = state(Resolution::Intersect);
    let family = query.plan(&inactive).focus(&focus).build()?;
    assert_eq!(
        family.explain().direct_reason,
        Some(DirectReason::UnsupportedGroupingExpression)
    );
    family.bind(&inactive)?;
    let safe_focus = interval("focus", "delay");
    let family = query.plan(&inactive).focus(&safe_focus).build()?;
    let BoundQuery::Preaggregated {
        materialization,
        aggregate,
    } = family.bind(&inactive)?
    else {
        panic!()
    };
    assert!(aggregate.over(source(flights())).is_err());
    aggregate.over(materialization)?;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    for volatility in [Volatility::Stable, Volatility::Volatile] {
        let udf = create_udf(
            "unstable",
            vec![DataType::Int64],
            DataType::Int64,
            volatility,
            Arc::new(|args| Ok(args[0].clone())),
        );
        let query = membership().query(source(flights()), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![udf.call(vec![col("delay")])], vec![count(lit(1_i64))])?
                .build()
        })?;
        assert_eq!(
            query
                .plan(&inactive)
                .focus(&focus)
                .build()?
                .explain()
                .direct_reason,
            Some(DirectReason::NonImmutableQuery)
        );
    }
    Ok(())
}

#[tokio::test]
async fn fallible_groups_do_not_evaluate_unselected_rows() -> TestResult {
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let selected = inactive.apply(
        &id(),
        SelectionUpdate::set(&focus, between("delay", 10, 31)),
    )?;
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                vec![(lit(1_i64) / col("delay")).alias("inverse")],
                vec![count(lit(1_i64))],
            )?
            .build()
    })?;
    let family = query.plan(&selected).focus(&focus).build()?;
    let BoundQuery::Direct { plan, reason } = family.bind(&selected)? else {
        panic!()
    };
    assert_eq!(reason, DirectReason::UnsupportedGroupingExpression);
    assert_eq!(
        rows(&run(plan).await),
        rows(&run(query.logical_plan(&selected)?).await)
    );
    Ok(())
}

#[tokio::test]
async fn exact_float_keys_preserve_nonfinite_null_and_signed_zero_membership() -> TestResult {
    use datafusion::arrow::array::Float64Array;
    let data = batch(vec![(
        "x",
        Arc::new(Float64Array::from(vec![
            Some(-0.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::from_bits(f64::NAN.to_bits() + 1)),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
            Some(1.0),
            None,
        ])),
    )]);
    let focus = point("points", "x");
    let inactive = state(Resolution::Intersect);
    let query = membership().query(source(data), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<Expr>::new(),
                vec![count(lit(1_i64)).alias("n"), count(col("x")).alias("valid")],
            )?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    for value in [
        None,
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(f64::NEG_INFINITY),
    ] {
        let selected = inactive.apply(
            &id(),
            SelectionUpdate::set(&focus, values("x", [ScalarValue::Float64(value)])),
        )?;
        compare(&query, &family, &selected).await;
    }
    compare(&query, &family, &inactive).await;
    Ok(())
}
