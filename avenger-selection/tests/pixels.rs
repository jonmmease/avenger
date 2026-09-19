mod common;

use avenger_scales_datafusion::{
    avenger_scales::scalar::Scalar, list_literal, options_literal, scale_expr, BuiltinScale,
};
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, Date32Array, Date64Array, Float32Array, Float64Array, Int64Array,
            StringArray, TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
        },
        datatypes::DataType,
    },
    common::ScalarValue,
    logical_expr::{col, LogicalPlanBuilder},
    prelude::SessionContext,
};
use std::{
    collections::HashMap,
    ops::Bound::{self, Excluded, Included, Unbounded},
    sync::Arc,
};

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}
fn linear(domain: [f64; 2], range: [f64; 2], origin: f64, size: f64) -> PixelGrid {
    PixelGrid::new(
        BuiltinScale::Linear,
        numbers(&domain),
        numbers(&range),
        HashMap::new(),
        origin,
        size,
    )
    .unwrap()
}
fn projection(name: &str) -> ProjectionId {
    ProjectionId::new(name).unwrap()
}
fn pixel_producer(grid: PixelGrid) -> ProducerDefinition {
    interval("brush", "x")
        .with_pixel_grids([(projection("x"), grid)])
        .unwrap()
}
fn pixel_state(
    p: &ProducerDefinition,
    lower: Bound<ScalarValue>,
    upper: Bound<ScalarValue>,
) -> SelectionSet {
    state(Resolution::Intersect)
        .set(p, range("x", lower, upper))
        .unwrap()
}
fn data(values: Vec<Option<f64>>) -> datafusion::arrow::record_batch::RecordBatch {
    batch(vec![
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..values.len() as i64)),
        ),
        ("x", Arc::new(Float64Array::from(values))),
    ])
}
async fn cells(grid: &PixelGrid, values: ArrayRef) -> Vec<Option<i64>> {
    let input = batch(vec![("x", values)]);
    let rows = SessionContext::new()
        .read_batch(input)
        .unwrap()
        .select(vec![grid.cell_expr(col("x")).alias("cell")])
        .unwrap()
        .collect()
        .await
        .unwrap();
    rows.iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .iter()
        })
        .collect()
}

#[tokio::test]
async fn scalar_batch_and_scale_udf_agree_at_fractional_negative_and_float32_boundaries() {
    let grid = linear([0.0, 2.0], [0.0, 2.0], 0.0, 1.0);
    let values = numbers(&[
        -2.1,
        -1.0,
        -1e-46,
        0.0,
        0.99,
        1.0 - 2_f64.powi(-25),
        1.0,
        1.99,
        2.0,
        3.0,
    ]);
    let result = cells(&grid, values.clone()).await;
    assert_eq!(
        result,
        vec![
            Some(-3),
            Some(-1),
            Some(0),
            Some(0),
            Some(0),
            Some(1),
            Some(1),
            Some(1),
            Some(2),
            Some(3)
        ]
    );
    for (index, cell) in result.iter().enumerate() {
        assert_eq!(
            *cell,
            grid.cell(&ScalarValue::try_from_array(&values, index).unwrap())
                .unwrap()
        );
    }
    let scaled = scale_expr(
        BuiltinScale::Linear,
        list_literal(grid.domain().clone()).unwrap(),
        list_literal(grid.range().clone()).unwrap(),
        options_literal(grid.options()).unwrap(),
        col("x"),
    )
    .unwrap();
    let rows = SessionContext::new()
        .read_batch(batch(vec![("x", values)]))
        .unwrap()
        .select(vec![scaled])
        .unwrap()
        .collect()
        .await
        .unwrap();
    let reference: Vec<_> = rows
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .iter()
                .map(|v| v.map(|v| ((f64::from(v) - grid.origin()) / grid.size()).floor() as i64))
        })
        .collect();
    assert_eq!(result, reference);

    let grid = linear([-1.0, 1.0], [-10.0, 10.0], 0.5, 2.0);
    assert_eq!(
        cells(&grid, numbers(&[-1.0, 0.0, 0.1, 1.0])).await,
        vec![Some(-6), Some(-1), Some(0), Some(4)]
    );
    assert!(
        cells(&grid, Arc::new(Float64Array::from(Vec::<f64>::new())))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn endpoint_flags_apply_to_whole_cells_including_collapsed_intervals() {
    let p = pixel_producer(linear([0.0, 200.0], [0.0, 600.0], 0.0, 2.0));
    let rows = data(vec![
        Some(9.9),
        Some(10.0),
        Some(10.5),
        Some(11.0),
        Some(29.9),
        Some(30.0),
        Some(30.5),
        None,
    ]);
    for (lo, hi, expected) in [
        (
            Included(10.0.into()),
            Excluded(30.0.into()),
            vec![1, 2, 3, 4],
        ),
        (
            Excluded(10.0.into()),
            Included(30.0.into()),
            vec![3, 4, 5, 6],
        ),
        (Included(10.1.into()), Included(10.2.into()), vec![1, 2]),
        (Excluded(10.1.into()), Included(10.2.into()), vec![]),
        (Included(10.1.into()), Excluded(10.2.into()), vec![]),
        (Unbounded, Unbounded, vec![0, 1, 2, 3, 4, 5, 6]),
    ] {
        let s = pixel_state(&p, lo, hi);
        assert_eq!(
            selected(rows.clone(), membership().predicate(&s).unwrap()).await,
            expected
        );
    }
    let s = pixel_state(&p, Included(10.0.into()), Excluded(30.0.into()));
    let c = s.contributions(&id()).unwrap().next().unwrap();
    assert_eq!(
        c.value(),
        &range("x", Included(10.0.into()), Excluded(30.0.into()))
    );
    assert_eq!(
        c.effective_value(),
        &range("x", Included(15_i64.into()), Excluded(45_i64.into()))
    );
    assert!(state(Resolution::Intersect)
        .set(&p, range("x", Included(10.2.into()), Included(10.1.into())))
        .is_err());
}

#[tokio::test]
async fn both_scale_directions_swap_bounds_and_unbounded_ends() {
    let rows = data(vec![
        Some(0.0),
        Some(10.0),
        Some(20.0),
        Some(30.0),
        Some(40.0),
    ]);
    for domain in [[0.0, 200.0], [200.0, 0.0]] {
        for output in [[0.0, 600.0], [600.0, 0.0]] {
            let grid = linear(domain, output, 0.0, 2.0);
            let decreasing = (domain[0] > domain[1]) ^ (output[0] > output[1]);
            assert_eq!(grid.is_decreasing(), decreasing);
            let p = pixel_producer(grid);
            for (lo, hi, expected) in [
                (Included(10.0.into()), Excluded(30.0.into()), vec![1, 2]),
                (Unbounded, Included(20.0.into()), vec![0, 1, 2]),
                (Excluded(20.0.into()), Unbounded, vec![3, 4]),
            ] {
                assert_eq!(
                    selected(
                        rows.clone(),
                        membership().predicate(&pixel_state(&p, lo, hi)).unwrap()
                    )
                    .await,
                    expected
                );
            }
            let s = pixel_state(&p, Included(10.0.into()), Excluded(30.0.into()));
            let c = s.contributions(&id()).unwrap().next().unwrap();
            let effective = if decreasing {
                range("x", Excluded(255_i64.into()), Included(285_i64.into()))
            } else {
                range("x", Included(15_i64.into()), Excluded(45_i64.into()))
            };
            assert_eq!(c.effective_value(), &effective);
        }
    }
}

#[tokio::test]
async fn clamp_offset_and_invalid_coordinates_follow_the_scale_kernel() {
    let values: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(-2.0),
        Some(-1.0),
        Some(0.0),
        Some(5.0),
        Some(10.0),
        Some(11.0),
        Some(f64::NEG_INFINITY),
        Some(f64::INFINITY),
        Some(f64::NAN),
        None,
    ]));
    for clamp in [false, true] {
        let grid = PixelGrid::new(
            BuiltinScale::Linear,
            numbers(&[0.0, 10.0]),
            numbers(&[0.0, 100.0]),
            HashMap::from([
                ("clamp".into(), Scalar::from_bool(clamp)),
                ("range_offset".into(), Scalar::from_f32(10.0)),
                ("round".into(), Scalar::from_bool(false)),
            ]),
            0.0,
            10.0,
        )
        .unwrap();
        let expected = if clamp {
            vec![
                Some(0),
                Some(0),
                Some(1),
                Some(6),
                Some(10),
                Some(10),
                Some(0),
                Some(10),
                None,
                None,
            ]
        } else {
            vec![
                Some(-1),
                Some(0),
                Some(1),
                Some(6),
                Some(11),
                Some(12),
                None,
                None,
                None,
                None,
            ]
        };
        assert_eq!(cells(&grid, values.clone()).await, expected);
        for (index, cell) in expected.iter().enumerate() {
            assert_eq!(
                grid.cell(&ScalarValue::try_from_array(&values, index).unwrap())
                    .unwrap(),
                *cell
            );
        }
        let p = pixel_producer(grid);
        assert!(state(Resolution::Union)
            .set(
                &p,
                range("x", Included(f64::NEG_INFINITY.into()), Unbounded)
            )
            .is_err());
    }
}

#[tokio::test]
async fn invalid_cells_are_null_and_clear_recovers_the_rows() {
    let grid = linear([0.0, 1.0], [0.0, 1.0], 0.0, 1.0);
    let min = i64::MIN as f64;
    let values = numbers(&[min, min - 2_f64.powi(41), 0.0, -min, f64::MAX, f64::NAN]);
    assert_eq!(
        cells(&grid, values.clone()).await,
        vec![Some(i64::MIN), None, Some(0), None, None, None]
    );
    for i in [1, 3, 4, 5] {
        assert_eq!(
            grid.cell(&ScalarValue::try_from_array(&values, i).unwrap())
                .unwrap(),
            None
        );
    }
    let p = pixel_producer(grid);
    let active = pixel_state(&p, Unbounded, Unbounded);
    let rows = batch(vec![
        ("id", Arc::new(Int64Array::from_iter_values(0..6))),
        ("x", values),
    ]);
    assert_eq!(
        selected(rows.clone(), membership().predicate(&active).unwrap()).await,
        vec![0, 2]
    );
    let cleared = active.clear(&p).unwrap();
    assert_eq!(
        selected(rows, membership().predicate(&cleared).unwrap()).await,
        vec![0, 1, 2, 3, 4, 5]
    );
    assert!(state(Resolution::Union)
        .set(&p, range("x", Included((-min).into()), Unbounded))
        .is_err());
    let tiny = linear([0.0, 1.0], [0.0, 1.0], 0.0, f64::from_bits(1));
    assert_eq!(tiny.cell(&1.0.into()).unwrap(), None);
}

#[tokio::test]
async fn utc_time_uses_existing_millisecond_and_float32_precision() {
    let cases: Vec<(ArrayRef, ArrayRef, Vec<Option<i64>>)> = vec![
        (
            Arc::new(Date32Array::from(vec![0, 10])),
            Arc::new(Date32Array::from(vec![
                Some(-1),
                Some(0),
                Some(5),
                Some(10),
                None,
            ])),
            vec![Some(-5), Some(0), Some(25), Some(50), None],
        ),
        (
            Arc::new(Date64Array::from(vec![0, 10_000])),
            Arc::new(Date64Array::from(vec![0, 5000, 10000])),
            vec![Some(0), Some(25), Some(50)],
        ),
        (
            Arc::new(TimestampMillisecondArray::from(vec![0, 1000]).with_timezone("UTC")),
            Arc::new(TimestampMillisecondArray::from(vec![0, 500, 1000]).with_timezone("UTC")),
            vec![Some(0), Some(25), Some(50)],
        ),
        (
            Arc::new(TimestampNanosecondArray::from(vec![0, 1_000_000_000]).with_timezone("UTC")),
            Arc::new(
                TimestampNanosecondArray::from(vec![0, 999_999, 500_000_000, 1_000_000_000])
                    .with_timezone("UTC"),
            ),
            vec![Some(0), Some(0), Some(25), Some(50)],
        ),
    ];
    for (domain, values, expected) in cases {
        let grid = PixelGrid::new(
            BuiltinScale::Time,
            domain,
            numbers(&[0.0, 100.0]),
            HashMap::from([("timezone".into(), Scalar::from("UTC"))]),
            0.0,
            2.0,
        )
        .unwrap();
        assert_eq!(cells(&grid, values.clone()).await, expected);
        for (i, cell) in expected.iter().enumerate() {
            assert_eq!(
                grid.cell(&ScalarValue::try_from_array(&values, i).unwrap())
                    .unwrap(),
                *cell
            );
        }
        let upper = ScalarValue::try_from_array(grid.domain(), 1).unwrap();
        let p = pixel_producer(grid.clone());
        let s = pixel_state(&p, Unbounded, Excluded(upper));
        let rows = batch(vec![
            (
                "id",
                Arc::new(Int64Array::from_iter_values(0..values.len() as i64)),
            ),
            ("x", values.clone()),
        ]);
        let expected: Vec<_> = expected
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some_and(|c| c < 50))
            .map(|(i, _)| i as i64)
            .collect();
        assert_eq!(
            selected(rows, membership().predicate(&s).unwrap()).await,
            expected
        );
    }
}

#[tokio::test]
async fn utc_temporal_overflows_are_invalid_cells_instead_of_panics() {
    let domain: ArrayRef = Arc::new(TimestampSecondArray::from(vec![0, 10]));
    let grid = PixelGrid::new(
        BuiltinScale::Time,
        domain,
        numbers(&[0.0, 100.0]),
        HashMap::new(),
        0.0,
        2.0,
    )
    .unwrap();
    let values: ArrayRef = Arc::new(TimestampSecondArray::from(vec![
        Some(i64::MIN),
        Some(0),
        Some(5),
        Some(i64::MAX),
        None,
    ]));
    assert_eq!(
        cells(&grid, values.clone()).await,
        vec![None, Some(0), Some(25), None, None]
    );
    assert_eq!(
        grid.cell(&ScalarValue::TimestampSecond(Some(i64::MAX), None))
            .unwrap(),
        None
    );
    let p = pixel_producer(grid.clone());
    assert!(state(Resolution::Union)
        .set(
            &p,
            range(
                "x",
                Unbounded,
                Included(ScalarValue::TimestampSecond(Some(i64::MAX), None))
            )
        )
        .is_err());
    assert!(grid
        .cell(&ScalarValue::TimestampMillisecond(Some(1), None))
        .is_err());
    let input = batch(vec![(
        "x",
        Arc::new(TimestampMillisecondArray::from(vec![1])),
    )]);
    assert!(SessionContext::new()
        .read_batch(input)
        .unwrap()
        .select(vec![grid.cell_expr(col("x"))])
        .is_err());
}

#[test]
fn configuration_validation_rejects_unsupported_and_degenerate_mappings() {
    for builtin in [BuiltinScale::Log, BuiltinScale::Pow, BuiltinScale::Band] {
        assert!(PixelGrid::new(
            builtin,
            numbers(&[0.0, 1.0]),
            numbers(&[0.0, 100.0]),
            HashMap::new(),
            0.0,
            2.0
        )
        .is_err());
    }
    for domain in [
        vec![1.0],
        vec![1.0, 1.0],
        vec![f64::NAN, 1.0],
        vec![f64::NEG_INFINITY, 1.0],
        vec![1.0, 1.0 + 1e-10],
        vec![-f32::MAX as f64, f32::MAX as f64],
    ] {
        assert!(PixelGrid::new(
            BuiltinScale::Linear,
            numbers(&domain),
            numbers(&[0.0, 100.0]),
            HashMap::new(),
            0.0,
            2.0
        )
        .is_err());
    }
    for range in [vec![0.0], vec![0.0, 0.0], vec![0.0, f64::INFINITY]] {
        assert!(PixelGrid::new(
            BuiltinScale::Linear,
            numbers(&[0.0, 1.0]),
            numbers(&range),
            HashMap::new(),
            0.0,
            2.0
        )
        .is_err());
    }
    for (origin, size) in [
        (f64::NAN, 2.0),
        (f64::INFINITY, 2.0),
        (0.0, 0.0),
        (0.0, -1.0),
        (0.0, f64::NAN),
        (0.0, f64::INFINITY),
    ] {
        assert!(PixelGrid::new(
            BuiltinScale::Linear,
            numbers(&[0.0, 1.0]),
            numbers(&[0.0, 100.0]),
            HashMap::new(),
            origin,
            size
        )
        .is_err());
    }
    for (name, value) in [
        ("round", Scalar::from_bool(true)),
        ("nice", Scalar::from_bool(true)),
        ("clamp", Scalar::from_f32(1.0)),
        ("f64_precision", Scalar::from_bool(true)),
        ("range_offset", Scalar::from_f32(f32::INFINITY)),
    ] {
        assert!(PixelGrid::new(
            BuiltinScale::Linear,
            numbers(&[0.0, 1.0]),
            numbers(&[0.0, 100.0]),
            HashMap::from([(name.into(), value)]),
            0.0,
            2.0
        )
        .is_err());
    }
    for (name, value) in [
        ("timezone", Scalar::from("America/New_York")),
        ("clamp", Scalar::from_bool(true)),
        ("range_offset", Scalar::from_f32(1.0)),
    ] {
        assert!(PixelGrid::new(
            BuiltinScale::Time,
            Arc::new(Date32Array::from(vec![0, 10])),
            numbers(&[0.0, 100.0]),
            HashMap::from([(name.into(), value)]),
            0.0,
            2.0
        )
        .is_err());
    }
    for domain in [
        Arc::new(TimestampNanosecondArray::from(vec![0, 1])) as ArrayRef,
        Arc::new(TimestampSecondArray::from(vec![0, i64::MAX])),
        Arc::new(TimestampMillisecondArray::from(vec![i64::MIN, i64::MAX])),
        Arc::new(TimestampSecondArray::from(vec![0, 10]).with_timezone("America/New_York")),
    ] {
        assert!(PixelGrid::new(
            BuiltinScale::Time,
            domain,
            numbers(&[0.0, 100.0]),
            HashMap::new(),
            0.0,
            2.0
        )
        .is_err());
    }
    assert!(PixelGrid::new(
        BuiltinScale::Linear,
        Arc::new(Float64Array::from(vec![Some(0.0), None])),
        numbers(&[0.0, 100.0]),
        HashMap::new(),
        0.0,
        2.0
    )
    .is_err());
}

#[test]
fn producer_validation_checks_grid_shape_and_update_terms_atomically() {
    let grid = linear([0.0, 200.0], [0.0, 600.0], 0.0, 2.0);
    assert!(point("p", "x")
        .with_pixel_grids([(projection("x"), grid.clone())])
        .is_err());
    let exact = producer("brush", view("brush"), SelectionKind::Interval, &["x", "y"]);
    assert!(exact.with_pixel_grids([]).is_err());
    assert!(exact
        .with_pixel_grids([(projection("missing"), grid.clone())])
        .is_err());
    assert!(exact
        .with_pixel_grids([
            (projection("x"), grid.clone()),
            (projection("x"), grid.clone())
        ])
        .is_err());
    assert!(exact
        .with_pixel_grids([
            (projection("x"), grid.clone()),
            (
                projection("y"),
                linear([0.0, 200.0], [0.0, 600.0], 0.0, 1.0)
            )
        ])
        .is_err());
    let pixel = exact.with_pixel_grids([(projection("x"), grid)]).unwrap();
    assert_eq!(pixel.precision(), IntervalPrecision::Pixels { size: 2.0 });
    assert_eq!(exact.precision(), IntervalPrecision::Exact);
    let s = state(Resolution::Intersect);
    for (x, y) in [
        (
            ValueTest::Equal(10_i64.into()),
            ValueTest::Equal("A".into()),
        ),
        (
            ValueTest::Range {
                lower: Included(10_i64.into()),
                upper: Excluded(30_i64.into()),
            },
            ValueTest::Range {
                lower: Unbounded,
                upper: Unbounded,
            },
        ),
        (
            ValueTest::Range {
                lower: Included(f64::INFINITY.into()),
                upper: Unbounded,
            },
            ValueTest::Equal("A".into()),
        ),
    ] {
        let values = SelectionValue::tuple(vec![term("x", x), term("y", y)]);
        assert!(s.set(&pixel, values).is_err());
        assert_eq!(s.contributions(&id()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn two_dimensional_tuples_and_categorical_dimensions_keep_correlation() {
    let exact = producer(
        "brush",
        view("brush"),
        SelectionKind::Interval,
        &["x", "y", "carrier"],
    );
    let grid = linear([0.0, 100.0], [0.0, 100.0], 0.0, 10.0);
    let pixel = exact
        .with_pixel_grids([(projection("x"), grid.clone()), (projection("y"), grid)])
        .unwrap();
    let tuple = |x0, x1, y0, y1, carrier: &str| {
        vec![
            term(
                "x",
                ValueTest::Range {
                    lower: Included(ScalarValue::from(x0)),
                    upper: Excluded(ScalarValue::from(x1)),
                },
            ),
            term(
                "y",
                ValueTest::Range {
                    lower: Included(ScalarValue::from(y0)),
                    upper: Excluded(ScalarValue::from(y1)),
                },
            ),
            term("carrier", ValueTest::OneOf(vec![carrier.into()])),
        ]
    };
    let s = state(Resolution::Intersect)
        .set(
            &pixel,
            SelectionValue::Tuples(vec![
                tuple(10_i64, 20_i64, 70_i64, 80_i64, "A"),
                tuple(70, 80, 10, 20, "B"),
            ]),
        )
        .unwrap();
    let rows = batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4]))),
        ("x", numbers(&[15.0, 75.0, 15.0, 75.0, 15.0])),
        ("y", numbers(&[75.0, 15.0, 15.0, 75.0, 75.0])),
        (
            "carrier",
            Arc::new(StringArray::from(vec!["A", "B", "A", "B", "B"])),
        ),
    ]);
    assert_eq!(
        selected(rows.clone(), membership().predicate(&s).unwrap()).await,
        vec![0, 1]
    );
    assert_eq!(
        selected(rows, cross(view("brush")).predicate(&s).unwrap()).await,
        vec![0, 1, 2, 3, 4]
    );
}

#[tokio::test]
async fn resizing_preserves_old_snapshots_and_uses_consumer_projection_mappings() {
    let old_grid = linear([0.0, 200.0], [0.0, 600.0], 0.0, 2.0);
    let old_p = pixel_producer(old_grid.clone());
    let before = pixel_state(&old_p, Included(10.6.into()), Excluded(30.0.into()));
    let c = before.contributions(&id()).unwrap().next().unwrap();
    let new_p = old_p
        .with_pixel_grids([(
            projection("x"),
            linear([0.0, 200.0], [0.0, 800.0], 0.0, 2.0),
        )])
        .unwrap();
    let after = before.set(&new_p, c.value().clone()).unwrap();
    let rows = batch(vec![
        ("id", Arc::new(Int64Array::from(vec![0, 1, 2]))),
        ("renamed", numbers(&[10.0, 10.6, 30.0])),
    ]);
    let filter = ConsumerFilter::new(
        view("target"),
        SelectionFilter::membership(&id(), EmptySelection::MatchAll),
    )
    .with_projection(old_p.address(), &projection("x"), col("renamed"))
    .unwrap();

    assert_eq!(
        selected(rows.clone(), filter.predicate(&before).unwrap()).await,
        vec![0, 1]
    );
    assert_eq!(
        selected(rows.clone(), filter.predicate(&after).unwrap()).await,
        vec![1]
    );
    assert_eq!(
        selected(rows, filter.predicate(&before).unwrap()).await,
        vec![0, 1]
    );
    assert_eq!(c.producer().pixel_grid(&projection("x")), Some(&old_grid));
    assert_eq!(
        c.value(),
        after.contributions(&id()).unwrap().next().unwrap().value()
    );
}

#[tokio::test]
async fn dataflow_reuses_equal_cells_and_invalidates_changed_grid_configuration(
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use avenger_datafusion_dataflow::{DataflowBuilder, Runtime, TableSnapshot};
    let old_p = pixel_producer(linear([0.0, 200.0], [0.0, 600.0], 0.0, 2.0));
    let a = pixel_state(&old_p, Included(10.4.into()), Excluded(30.0.into()));
    let b = pixel_state(&old_p, Included(10.6.into()), Excluded(30.0.into()));
    let new_p = old_p.with_pixel_grids([(
        projection("x"),
        linear([0.0, 200.0], [0.0, 800.0], 0.0, 2.0),
    )])?;
    let c = pixel_state(&new_p, Included(10.6.into()), Excluded(30.0.into()));
    assert_eq!(membership().predicate(&a)?, membership().predicate(&b)?);
    assert_ne!(membership().predicate(&b)?, membership().predicate(&c)?);
    let mut builder = DataflowBuilder::new();
    let batch = data(vec![Some(10.0), Some(10.6), Some(30.0), None]);
    let source = builder.table_snapshot(
        "data",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let input = builder.expr_input("selection", DataType::Boolean)?;
    let filtered = builder.add_plan(
        "filtered",
        LogicalPlanBuilder::from(source.plan_ref())
            .filter(input.expr_ref())?
            .build()?,
    )?;
    let output = builder.table_output("rows", &filtered)?;
    let prepared = Runtime::new(Default::default())?
        .prepare(&builder.finish()?)
        .await?;
    for (state, expected, plans) in [
        (&a, vec![0, 1], 1),
        (&b, vec![0, 1], 0),
        (&c, vec![1], 1),
        (&a, vec![0, 1], 0),
    ] {
        let inputs = prepared
            .inputs()
            .expr(&input, membership().predicate(state)?)?
            .finish()?;
        let result = prepared.query(&[output], &[], &inputs).await?;
        assert_eq!(ids(result.table(&output)?.batches()), expected);
        assert_eq!(result.report().physical_plans, plans);
    }
    Ok(())
}

#[tokio::test]
async fn utc_time_reversed_axes_and_timezone_options_keep_one_mapping() {
    for domain in [[0_i64, 4000], [4000, 0]] {
        for output in [[0.0, 400.0], [400.0, 0.0]] {
            let grid = PixelGrid::new(
                BuiltinScale::Time,
                Arc::new(Date64Array::from(domain.to_vec())),
                numbers(&output),
                HashMap::from([("timezone".into(), Scalar::from("uTc"))]),
                0.0,
                2.0,
            )
            .unwrap();
            let same = PixelGrid::new(
                BuiltinScale::Time,
                grid.domain().clone(),
                grid.range().clone(),
                HashMap::from([("timezone".into(), Scalar::from("UTC"))]),
                0.0,
                2.0,
            )
            .unwrap();
            assert_eq!(grid, same);
            let p = pixel_producer(grid);
            let s = pixel_state(
                &p,
                Included(ScalarValue::Date64(Some(1000))),
                Excluded(ScalarValue::Date64(Some(3000))),
            );
            let rows = batch(vec![
                ("id", Arc::new(Int64Array::from(vec![0, 1, 2, 3, 4]))),
                (
                    "x",
                    Arc::new(Date64Array::from(vec![0, 1000, 2000, 3000, 4000])),
                ),
            ]);
            assert_eq!(
                selected(rows, membership().predicate(&s).unwrap()).await,
                vec![1, 2]
            );
        }
    }
}

#[tokio::test]
async fn global_toggle_keeps_distinct_exact_and_pixel_range_meanings() {
    let pixel = pixel_producer(linear([0.0, 200.0], [0.0, 600.0], 0.0, 2.0));
    let exact = point("bin", "x");
    let raw = range("x", Included(10.6.into()), Excluded(30.0.into()));
    let SelectionValue::Tuples(tuples) = raw.clone() else {
        panic!()
    };
    let s = state(Resolution::Global)
        .set(&pixel, raw)
        .unwrap()
        .toggle(&exact, SelectionValue::Tuples(tuples))
        .unwrap();
    assert_eq!(s.contributions(&id()).unwrap().count(), 2);
    let rows = data(vec![Some(10.0), Some(10.6), Some(30.0)]);
    assert_eq!(
        selected(
            rows.clone(),
            cross(pixel.address().origin.clone()).predicate(&s).unwrap()
        )
        .await,
        vec![1]
    );
    assert_eq!(
        selected(
            rows,
            cross(exact.address().origin.clone()).predicate(&s).unwrap()
        )
        .await,
        vec![0, 1]
    );
}
