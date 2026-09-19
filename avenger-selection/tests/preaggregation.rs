mod common;

use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{
        array::{
            ArrayRef, Date32Array, Decimal128Array, Float32Array, Float64Array, Int64Array,
            StringArray, UInt64Array,
        },
        datatypes::DataType,
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::{avg, count, max, median, min},
    logical_expr::{
        col, create_udaf, create_udf, lit, Expr, ExprFunctionExt, LogicalPlan, LogicalPlanBuilder,
        Volatility,
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
    assert_results(&actual, &expected, FLOAT_MEASURES);
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
                vec![
                    count(col("carrier")).alias("n"),
                    avg(col("distance")).alias("mean"),
                ],
            )?
            .filter(col("n").gt(lit(0_i64)))?
            .project(vec![
                col("bin").alias("bucket"),
                (col("n") + lit(1_i64)).alias("display"),
                col("mean"),
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
        median(col("delay")),
        count(col("carrier")).distinct().build()?,
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

#[tokio::test]
async fn typed_measures_preserve_empty_groups_weights_and_moments() -> TestResult {
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let mut selections = vec![inactive.clone()];
    for (lower, upper) in [(0, 2), (1, 2), (2, 3), (3, 4), (10, 20)] {
        selections.push(inactive.apply(
            &id(),
            SelectionUpdate::set(&focus, between("delay", lower, upper)),
        )?);
    }
    selections.push(inactive.apply(&id(), SelectionUpdate::set(&focus, values("delay", [])))?);
    let numeric: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(vec![
            Some(2),
            Some(4),
            Some(12),
            Some(14),
            Some(18),
            None,
            Some(5),
            None,
        ])),
        Arc::new(UInt64Array::from(vec![
            Some(2),
            Some(4),
            Some(12),
            Some(14),
            Some(18),
            None,
            Some(5),
            None,
        ])),
        Arc::new(Float32Array::from(vec![
            Some(2.),
            Some(4.),
            Some(12.),
            Some(14.),
            Some(18.),
            None,
            Some(5.),
            None,
        ])),
        Arc::new(Float64Array::from(vec![
            Some(2.),
            Some(4.),
            Some(12.),
            Some(14.),
            Some(18.),
            None,
            Some(5.),
            None,
        ])),
        Arc::new(
            Decimal128Array::from(vec![
                Some(200),
                Some(400),
                Some(1200),
                Some(1400),
                Some(1800),
                None,
                Some(500),
                None,
            ])
            .with_precision_and_scale(12, 2)?,
        ),
    ];
    for measure in numeric {
        let data = batch(vec![
            (
                "delay",
                Arc::new(Int64Array::from(vec![0, 0, 1, 1, 1, 2, 3, 2])),
            ),
            (
                "carrier",
                Arc::new(StringArray::from(vec![
                    "AA", "AA", "AA", "AA", "AA", "AA", "DL", "UA",
                ])),
            ),
            ("x", measure),
        ]);
        let fields = data
            .schema()
            .fields()
            .iter()
            .map(|field| {
                field
                    .as_ref()
                    .clone()
                    .with_metadata(std::collections::HashMap::from([(
                        "source".to_owned(),
                        "selection-fixture".to_owned(),
                    )]))
            })
            .collect::<Vec<_>>();
        let data = RecordBatch::try_new(
            Arc::new(datafusion::arrow::datatypes::Schema::new(fields)),
            data.columns().to_vec(),
        )?;
        for data in [data.clone(), data.slice(0, 0)] {
            for groups in [vec![], vec![col("carrier")]] {
                let query = membership().query(source(data.clone()), |rows| {
                    LogicalPlanBuilder::from(rows)
                        .aggregate(groups, measures("x"))?
                        .build()
                })?;
                let family = query.plan(&inactive).focus(&focus).build()?;
                assert_eq!(
                    family.explain().strategy,
                    QueryStrategy::Preaggregated,
                    "{:?}: {:?}",
                    data.schema(),
                    family.explain()
                );
                for state in &selections {
                    compare(&query, &family, state).await;
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn extrema_preserve_ordered_types_and_builtin_aliases() -> TestResult {
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let inputs: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![
            Some("B"),
            None,
            Some("A"),
            Some("Z"),
        ])),
        Arc::new(Date32Array::from(vec![
            Some(100),
            None,
            Some(99),
            Some(400),
        ])),
    ];
    for input in inputs {
        let data = batch(vec![
            ("delay", Arc::new(Int64Array::from(vec![0, 0, 1, 2]))),
            ("x", input),
        ]);
        let query = membership().query(source(data), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(
                    Vec::<Expr>::new(),
                    vec![min(col("x")).alias("min"), max(col("x")).alias("max")],
                )?
                .build()
        })?;
        let family = query.plan(&inactive).focus(&focus).build()?;
        for (lower, upper) in [(0, 3), (1, 2), (10, 20)] {
            compare(
                &query,
                &family,
                &inactive.apply(
                    &id(),
                    SelectionUpdate::set(&focus, between("delay", lower, upper)),
                )?,
            )
            .await;
        }
    }
    let ctx = SessionContext::new();
    let variance = ctx.state().aggregate_functions()["var_sample"].clone();
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(
                Vec::<Expr>::new(),
                vec![variance.call(vec![col("delay")]).alias("var_samp")],
            )?
            .build()
    })?;
    compare(
        &query,
        &query.plan(&inactive).focus(&focus).build()?,
        &inactive,
    )
    .await;

    let custom = create_udaf(
        "sum",
        vec![DataType::Int64],
        Arc::new(DataType::Int64),
        Volatility::Immutable,
        Arc::new(|_| panic!("planning must not instantiate an accumulator")),
        Arc::new(vec![DataType::Int64]),
    );
    let query = membership().query(source(flights()), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(Vec::<Expr>::new(), vec![custom.call(vec![col("delay")])])?
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
    Ok(())
}

#[tokio::test]
async fn global_extrema_document_native_grouped_infinity_bounds() -> TestResult {
    let focus = interval("brush", "delay");
    let inactive = state(Resolution::Intersect);
    let data = batch(vec![
        ("delay", Arc::new(Int64Array::from(vec![0]))),
        ("x", Arc::new(Float64Array::from(vec![f64::INFINITY]))),
    ]);
    let query = membership().query(source(data), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(Vec::<Expr>::new(), vec![min(col("x")).alias("min")])?
            .build()
    })?;
    let BoundQuery::Preaggregated {
        materialization,
        aggregate,
    } = query
        .plan(&inactive)
        .focus(&focus)
        .build()?
        .bind(&inactive)?
    else {
        panic!("expected optimized extrema")
    };
    let batches = run(materialization).await;
    let relation = source(batches[0].clone());
    let actual = run(aggregate.over(relation)?).await;
    let direct = run(query.logical_plan(&inactive)?).await;
    assert_eq!(
        ScalarValue::try_from_array(actual[0].column(0), 0)?,
        ScalarValue::Float64(Some(f64::MAX))
    );
    assert_eq!(
        ScalarValue::try_from_array(direct[0].column(0), 0)?,
        ScalarValue::Float64(Some(f64::INFINITY))
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_nested_measure_uses_the_direct_query() -> TestResult {
    use datafusion::arrow::{array::ListArray, datatypes::Int64Type};
    let data = batch(vec![
        ("delay", Arc::new(Int64Array::from(vec![0]))),
        (
            "x",
            Arc::new(ListArray::from_iter_primitive::<Int64Type, _, _>([Some(
                vec![Some(2), Some(1)],
            )])),
        ),
    ]);
    let inactive = state(Resolution::Intersect);
    let focus = interval("brush", "delay");
    let query = membership().query(source(data), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(Vec::<Expr>::new(), vec![min(col("x"))])?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    assert_eq!(
        family.explain().direct_reason,
        Some(DirectReason::UnsupportedAggregate)
    );
    let BoundQuery::Direct { plan, .. } = family.bind(&inactive)? else {
        panic!("expected direct fallback")
    };
    assert_results(
        &run(plan).await,
        &run(query.logical_plan(&inactive)?).await,
        &[],
    );
    Ok(())
}

#[tokio::test]
async fn decimal_average_overflow_is_a_query_error_without_a_retry() -> TestResult {
    let huge = 10_i128.pow(38) - 1;
    let data = batch(vec![
        ("delay", Arc::new(Int64Array::from(vec![0, 1]))),
        (
            "x",
            Arc::new(Decimal128Array::from(vec![1, huge]).with_precision_and_scale(38, 0)?),
        ),
    ]);
    let inactive = state(Resolution::Intersect);
    let focus = interval("brush", "delay");
    let query = membership().query(source(data), |rows| {
        LogicalPlanBuilder::from(rows)
            .aggregate(Vec::<Expr>::new(), vec![avg(col("x")).alias("mean")])?
            .build()
    })?;
    let family = query.plan(&inactive).focus(&focus).build()?;
    let safe = inactive.apply(&id(), SelectionUpdate::set(&focus, between("delay", 0, 1)))?;
    // The unselected huge cell is retained as state without premature finalization.
    compare(&query, &family, &safe).await;
    let overflowing =
        inactive.apply(&id(), SelectionUpdate::set(&focus, between("delay", 1, 2)))?;
    let BoundQuery::Preaggregated {
        materialization,
        aggregate,
    } = family.bind(&overflowing)?
    else {
        panic!("expected average states")
    };
    let materialized = run(materialization).await;
    let merged = aggregate.over(source(materialized[0].clone()))?;
    let ctx = SessionContext::new();
    for plan in [merged, query.logical_plan(&overflowing)?] {
        let error = ctx
            .execute_logical_plan(plan)
            .await?
            .collect()
            .await
            .unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("overflow"),
            "{error}"
        );
    }
    Ok(())
}
