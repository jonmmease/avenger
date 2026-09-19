mod common;
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{
        array::{
            Array, BooleanArray, Decimal128Array, Float16Array, Float32Array, Float64Array,
            Int64Array, TimestampNanosecondArray,
        },
        datatypes::DataType,
    },
    common::ScalarValue,
    functions::datetime::expr_fn::now,
    functions_aggregate::expr_fn::sum,
    logical_expr::{
        col, create_udf, lit, scalar_subquery, ColumnarValue, LogicalPlanBuilder, Volatility,
    },
    prelude::SessionContext,
};
use std::{
    ops::Bound::{Excluded, Included, Unbounded},
    sync::Arc,
};

#[tokio::test]
async fn two_histograms_and_categories_cross_filter_one_shared_selection() {
    let delay = interval("delay_hist", "delay");
    let distance = interval("distance_hist", "distance");
    let carrier = point("airlines", "carrier");
    let s = state(Resolution::Intersect)
        .apply_all([
            SelectionUpdate::set(&delay, between("delay", 10, 30)),
            SelectionUpdate::set(&distance, between("distance", 500, 1500)),
            SelectionUpdate::set(&carrier, values("carrier", ["AA".into(), "DL".into()])),
        ])
        .unwrap();
    for (p, expected) in [
        (&delay, vec![0, 1, 2, 3]),
        (&distance, vec![1, 2, 4]),
        (&carrier, vec![1, 2, 5, 6]),
    ] {
        let filter = cross(p.address().origin.clone());
        assert_eq!(
            selected(flights(), filter.predicate(&s).unwrap()).await,
            expected
        );
    }
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![1, 2]
    );
}
#[tokio::test]
async fn absence_all_excluded_and_active_empty_have_distinct_meaning() {
    let p = point("p", "carrier");
    let inactive = state(Resolution::Intersect);
    let usage = SelectionUse {
        mode: SelectionMode::CrossFilter,
        empty: EmptySelection::MatchNone,
    };
    let own = filter(
        p.address().origin.clone(),
        SelectionFilter::Selection { id: id(), usage },
    );
    assert!(selected(flights(), own.predicate(&inactive).unwrap())
        .await
        .is_empty());
    let empty = inactive.set(&p, SelectionValue::Tuples(vec![])).unwrap();
    assert_eq!(
        selected(flights(), own.predicate(&empty).unwrap()).await,
        vec![0, 1, 2, 3, 4, 5, 6]
    );
    assert!(selected(flights(), membership().predicate(&empty).unwrap())
        .await
        .is_empty());
    let cleared = empty.clear(&p).unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&cleared).unwrap()).await,
        vec![0, 1, 2, 3, 4, 5, 6]
    );
    assert!(selected(flights(), own.predicate(&cleared).unwrap())
        .await
        .is_empty());
    let missing = SelectionFilter::Any(vec![
        SelectionFilter::All(vec![]),
        SelectionFilter::membership(
            &SelectionId::new("missing").unwrap(),
            EmptySelection::MatchAll,
        ),
    ]);
    assert!(matches!(
        filter(view("summary"), missing).predicate(&empty),
        Err(Error::MissingSelection(_))
    ));
}
#[tokio::test]
async fn producer_resolution_and_outer_boolean_composition() {
    let a = point("a", "carrier");
    let b = point("b", "region");
    for (resolution, expected) in [
        (Resolution::Intersect, vec![0, 1, 3]),
        (Resolution::Union, vec![0, 1, 3, 4, 6]),
    ] {
        let s = state(resolution)
            .apply_all([
                SelectionUpdate::set(&a, values("carrier", ["AA".into()])),
                SelectionUpdate::set(&b, values("region", ["East".into()])),
            ])
            .unwrap();
        assert_eq!(
            selected(flights(), membership().predicate(&s).unwrap()).await,
            expected
        );
    }
    let s = state(Resolution::Global)
        .toggle(&a, SelectionValue::tuple(tuple("carrier", "AA")))
        .unwrap()
        .toggle(&b, SelectionValue::tuple(tuple("region", "East")))
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![0, 1, 3, 4, 6]
    );
    assert_eq!(
        selected(flights(), cross(view("a")).predicate(&s).unwrap()).await,
        vec![0, 1, 3, 6]
    );
    let negated = filter(
        view("summary"),
        SelectionFilter::Not(Box::new(SelectionFilter::membership(
            &id(),
            EmptySelection::MatchAll,
        ))),
    );
    assert_eq!(
        selected(flights(), negated.predicate(&s).unwrap()).await,
        vec![2, 5]
    );
    let empty_any = filter(view("summary"), SelectionFilter::Any(vec![]));
    assert!(selected(flights(), empty_any.predicate(&s).unwrap())
        .await
        .is_empty());
}
#[tokio::test]
async fn correlated_tuples_and_binned_points_preserve_conjunctions() {
    let p = producer(
        "points",
        view("points"),
        SelectionKind::Point,
        &["carrier", "region"],
    );
    let values = SelectionValue::Tuples(vec![
        vec![
            term("carrier", ValueTest::Equal("AA".into())),
            term("region", ValueTest::Equal("East".into())),
        ],
        vec![
            term("region", ValueTest::Equal("West".into())),
            term("carrier", ValueTest::Equal("DL".into())),
        ],
    ]);
    let s = state(Resolution::Union).set(&p, values).unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![0, 1, 2, 3]
    );
    let p = point("bin", "delay");
    let s = state(Resolution::Union)
        .set(&p, between("delay", 10, 30))
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![1, 2, 4, 5, 6]
    );
    let p = producer(
        "brush2d",
        view("brush2d"),
        SelectionKind::Interval,
        &["delay", "distance"],
    );
    let s = state(Resolution::Union)
        .set(
            &p,
            SelectionValue::tuple(vec![
                term(
                    "delay",
                    ValueTest::Range {
                        lower: Included(10_i64.into()),
                        upper: Excluded(30_i64.into()),
                    },
                ),
                term(
                    "distance",
                    ValueTest::Range {
                        lower: Included(500_i64.into()),
                        upper: Excluded(1500_i64.into()),
                    },
                ),
            ]),
        )
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![1, 2, 5, 6]
    );
}
#[tokio::test]
async fn nullable_categories_and_negation_produce_non_null_booleans() {
    let p = point("carrier", "carrier");
    let s = state(Resolution::Union)
        .set(
            &p,
            SelectionValue::tuple(vec![term(
                "carrier",
                ValueTest::OneOf(vec![ScalarValue::Utf8(None), "AA".into()]),
            )]),
        )
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![0, 1, 3, 4, 6]
    );
    let negative = filter(
        view("summary"),
        SelectionFilter::Not(Box::new(SelectionFilter::membership(
            &id(),
            EmptySelection::MatchAll,
        ))),
    );
    assert_eq!(
        selected(flights(), negative.predicate(&s).unwrap()).await,
        vec![2, 5]
    );
    let batches = SessionContext::new()
        .read_batch(flights())
        .unwrap()
        .select(vec![membership().predicate(&s).unwrap().alias("selected")])
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert!(batches.iter().all(|b| b.column(0).null_count() == 0));
}
#[tokio::test]
async fn range_endpoint_matrix_excludes_nulls_and_nonfinite_rows() {
    let data = batch(vec![
        ("id", Arc::new(Int64Array::from_iter_values(0..7))),
        (
            "x",
            Arc::new(Float64Array::from(vec![
                Some(f64::NEG_INFINITY),
                Some(1.0),
                Some(2.0),
                Some(3.0),
                Some(f64::INFINITY),
                Some(f64::NAN),
                None,
            ])),
        ),
    ]);
    let p = interval("brush", "x");
    for (lo, hi, want) in [
        (Included(1.0.into()), Included(3.0.into()), vec![1, 2, 3]),
        (Excluded(1.0.into()), Included(3.0.into()), vec![2, 3]),
        (Included(1.0.into()), Excluded(3.0.into()), vec![1, 2]),
        (Excluded(1.0.into()), Excluded(3.0.into()), vec![2]),
        (Unbounded, Unbounded, vec![1, 2, 3]),
        (
            Included(f64::NEG_INFINITY.into()),
            Included(f64::INFINITY.into()),
            vec![1, 2, 3],
        ),
        (Included(f64::INFINITY.into()), Unbounded, vec![]),
        (Unbounded, Included(2.0.into()), vec![1, 2]),
        (Included(2.0.into()), Unbounded, vec![2, 3]),
        (Included(2.0.into()), Included(2.0.into()), vec![2]),
        (Excluded(2.0.into()), Included(2.0.into()), vec![]),
        (Included(2.0.into()), Excluded(2.0.into()), vec![]),
    ] {
        let s = state(Resolution::Union)
            .set(&p, range("x", lo, hi))
            .unwrap();
        assert_eq!(
            selected(data.clone(), membership().predicate(&s).unwrap()).await,
            want
        );
    }
    let s = state(Resolution::Union)
        .set(&p, range("x", Unbounded, Unbounded))
        .unwrap();
    let negated = filter(
        view("summary"),
        SelectionFilter::Not(Box::new(SelectionFilter::membership(
            &id(),
            EmptySelection::MatchAll,
        ))),
    );
    assert_eq!(
        selected(data, negated.predicate(&s).unwrap()).await,
        vec![0, 4, 5, 6]
    );
}
#[tokio::test]
async fn floating_point_equality_matches_all_nan_payloads_and_both_zeros() {
    let arrays: Vec<(Arc<dyn Array>, ScalarValue)> = vec![
        (
            Arc::new(Float16Array::from(vec![
                half::f16::ZERO,
                half::f16::NEG_ZERO,
                half::f16::NAN,
                half::f16::from_bits(0xfe42),
                half::f16::ONE,
            ])),
            ScalarValue::Float16(Some(half::f16::NAN)),
        ),
        (
            Arc::new(Float32Array::from(vec![
                0.0,
                -0.0,
                f32::NAN,
                f32::from_bits(0xffc00042),
                1.0,
            ])),
            ScalarValue::Float32(Some(f32::NAN)),
        ),
        (
            Arc::new(Float64Array::from(vec![
                0.0,
                -0.0,
                f64::NAN,
                f64::from_bits(0xfff8000000000042),
                1.0,
            ])),
            ScalarValue::Float64(Some(f64::NAN)),
        ),
    ];
    let p = point("p", "x");
    for (array, nan) in arrays {
        let zero = ScalarValue::try_from_array(&array, 0).unwrap();
        let data = batch(vec![
            ("id", Arc::new(Int64Array::from_iter_values(0..5))),
            ("x", array),
        ]);
        for (value, expected) in [(nan, vec![2, 3]), (zero, vec![0, 1])] {
            let s = state(Resolution::Union)
                .set(&p, values("x", [value]))
                .unwrap();
            assert_eq!(
                selected(data.clone(), membership().predicate(&s).unwrap()).await,
                expected
            );
        }
    }
}
#[tokio::test]
async fn exact_integer_decimal_and_timestamp_values_do_not_round_through_float() {
    let big = 9_007_199_254_740_992_i64;
    let data = batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2]))),
        (
            "integer",
            Arc::new(Int64Array::from(vec![big, big + 1, big + 2])),
        ),
        (
            "decimal",
            Arc::new(
                Decimal128Array::from(vec![big as i128, big as i128 + 1, big as i128 + 2])
                    .with_precision_and_scale(30, 4)
                    .unwrap(),
            ),
        ),
        (
            "time",
            Arc::new(
                TimestampNanosecondArray::from(vec![big, big + 1, big + 2]).with_timezone("UTC"),
            ),
        ),
    ]);
    for (field, value) in [
        ("integer", ScalarValue::Int64(Some(big + 1))),
        (
            "decimal",
            ScalarValue::Decimal128(Some(big as i128 + 1), 30, 4),
        ),
        (
            "time",
            ScalarValue::TimestampNanosecond(Some(big + 1), Some("UTC".into())),
        ),
    ] {
        let p = interval("p", field);
        let s = state(Resolution::Union)
            .set(&p, range(field, Included(value.clone()), Included(value)))
            .unwrap();
        assert_eq!(
            selected(data.clone(), membership().predicate(&s).unwrap()).await,
            vec![1]
        );
    }
}
#[tokio::test]
async fn consumer_mappings_are_qualified_by_producer_and_row_lineage() {
    let a = producer("a", view("a"), SelectionKind::Point, &["value"]);
    let b = producer("b", view("b"), SelectionKind::Point, &["value"]);
    let s = state(Resolution::Intersect)
        .apply_all([
            SelectionUpdate::set(&a, values("value", ["AA".into()])),
            SelectionUpdate::set(&b, values("value", ["East".into()])),
        ])
        .unwrap();
    let projection = ProjectionId::new("value").unwrap();
    let consumer = membership()
        .with_projection(a.address(), &projection, col("carrier"))
        .unwrap()
        .with_projection(b.address(), &projection, col("region"))
        .unwrap();
    assert!(consumer
        .clone()
        .with_projection(a.address(), &projection, col("other"))
        .is_err());
    assert_eq!(
        selected(flights(), consumer.predicate(&s).unwrap()).await,
        vec![0, 1, 3]
    );

    let identity = RowIdentity::new(DataType::Int64).unwrap();
    let p = ProducerDefinition::row_ids(address("ids", view("rows")), identity.clone());
    let s = state(Resolution::Union)
        .set(
            &p,
            SelectionValue::RowIds(
                RowIdSelection::new(&identity, vec![2_i64.into(), 5_i64.into()]).unwrap(),
            ),
        )
        .unwrap();
    assert!(membership().predicate(&s).is_err());
    let wrong = membership()
        .with_row_identity(&RowIdentity::new(DataType::Int64).unwrap(), col("id"))
        .unwrap();
    assert!(wrong.predicate(&s).is_err());
    let correct = membership()
        .with_row_identity(&identity, col("id"))
        .unwrap();
    assert_eq!(
        selected(flights(), correct.predicate(&s).unwrap()).await,
        vec![2, 5]
    );
    assert_eq!(
        selected(flights(), cross(view("rows")).predicate(&s).unwrap()).await,
        vec![0, 1, 2, 3, 4, 5, 6]
    );
}
#[tokio::test]
async fn same_view_layers_exclude_all_own_producers_but_not_sibling_or_nested_facets() {
    let scope = ScopeId::new("region_year").unwrap();
    let east = ViewAddress {
        view: ViewId::new("hist").unwrap(),
        scope: vec![FacetKey::new(scope.clone(), vec!["East".into(), 2026_i32.into()]).unwrap()],
    };
    let west = ViewAddress {
        view: east.view.clone(),
        scope: vec![FacetKey::new(scope, vec!["West".into(), 2026_i32.into()]).unwrap()],
    };
    let nested = ViewAddress {
        view: east.view.clone(),
        scope: vec![
            east.scope[0].clone(),
            FacetKey::new(ScopeId::new("carrier").unwrap(), vec!["AA".into()]).unwrap(),
        ],
    };
    let a = producer("brush", east.clone(), SelectionKind::Interval, &["delay"]);
    let b = producer("points", east.clone(), SelectionKind::Point, &["carrier"]);
    let c = producer(
        "brush",
        west.clone(),
        SelectionKind::Interval,
        &["distance"],
    );
    let s = state(Resolution::Intersect)
        .apply_all([
            SelectionUpdate::set(&a, between("delay", 10, 30)),
            SelectionUpdate::set(&b, values("carrier", ["AA".into()])),
            SelectionUpdate::set(&c, between("distance", 500, 1500)),
        ])
        .unwrap();
    assert_eq!(
        selected(flights(), cross(east).predicate(&s).unwrap()).await,
        vec![0, 1, 2, 3, 5, 6]
    );
    assert_eq!(
        selected(flights(), cross(west).predicate(&s).unwrap()).await,
        vec![1, 4]
    );
    assert_eq!(
        selected(flights(), cross(nested).predicate(&s).unwrap()).await,
        vec![1]
    );
}
#[test]
fn projection_expressions_reject_query_and_request_dependent_forms() {
    let id = ProjectionId::new("p").unwrap();
    let plan = LogicalPlanBuilder::empty(true)
        .project(vec![lit(1_i64)])
        .unwrap()
        .build()
        .unwrap();
    let volatile = create_udf(
        "unstable",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(|_| Ok(ColumnarValue::Scalar(1_i64.into()))),
    );
    for expr in [
        sum(col("x")),
        now(),
        volatile.call(vec![]),
        scalar_subquery(Arc::new(plan)),
        datafusion::logical_expr::Expr::Placeholder(
            datafusion::logical_expr::expr::Placeholder::new_with_field("$p".into(), None),
        ),
    ] {
        assert!(Projection::new(id.clone(), expr).is_err());
    }
    assert!(Projection::new(id, col("x") + lit(1_i64)).is_ok());
}
#[tokio::test]
async fn categorical_interval_sets_and_boolean_categories_are_supported() {
    let p = interval("band_brush", "carrier");
    let s = state(Resolution::Union)
        .set(
            &p,
            SelectionValue::tuple(vec![term(
                "carrier",
                ValueTest::OneOf(vec!["DL".into(), "AA".into()]),
            )]),
        )
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&s).unwrap()).await,
        vec![0, 1, 2, 3, 4]
    );
    let p = point("p", "flag");
    let s = state(Resolution::Union)
        .set(&p, values("flag", [ScalarValue::Boolean(None)]))
        .unwrap();
    let data = batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2]))),
        (
            "flag",
            Arc::new(BooleanArray::from(vec![Some(true), Some(false), None])),
        ),
    ]);
    assert_eq!(
        selected(data, membership().predicate(&s).unwrap()).await,
        vec![2]
    );
}
