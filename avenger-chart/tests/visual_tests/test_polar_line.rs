use super::helpers::assert_visual_match_default;
use avenger_chart::polar::PolarPositionConfig;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{ArrayRef, Float64Array, Int32Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::prelude::*;
use std::{
    f64::consts::{FRAC_PI_2, PI, TAU},
    sync::Arc,
};

fn read_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> DataFrame {
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .expect("create polar line visual test data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read polar line visual test data")
}

fn full_circle_theta(c: PolarPositionConfig) -> PolarPositionConfig {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(TAU))))
}

#[tokio::test]
async fn polar_line_arc_vs_chord() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![Field::new("theta", DataType::Float64, false)],
        vec![Arc::new(Float64Array::from(vec![0.0, PI])) as ArrayRef],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Coordinate Arc vs Display Chord")
        .mark(
            Line::<Polar>::new()
                .r(70.0)
                .theta_with(col("theta"), full_circle_theta)
                .geometry_space(GeometrySpace::Coordinate)
                .stroke("#2563eb")
                .stroke_width(5.0),
        )
        .mark(
            Line::<Polar>::new()
                .r(70.0)
                .theta_with(col("theta"), full_circle_theta)
                .geometry_space(GeometrySpace::Display)
                .stroke("#dc2626")
                .stroke_width(3.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "arc_vs_chord").await;
}

#[tokio::test]
async fn polar_line_radial_and_spiral() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("series", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![
                10.0, 40.0, 70.0, 100.0, 14.0, 34.0, 54.0, 74.0, 94.0,
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![
                FRAC_PI_2, FRAC_PI_2, FRAC_PI_2, FRAC_PI_2, 0.15, 1.35, 2.55, 3.75, 4.95,
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "radial", "radial", "radial", "radial", "spiral", "spiral", "spiral", "spiral",
                "spiral",
            ])) as ArrayRef,
        ],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Radial and Spiral Polar Lines")
        .legend("stroke", |legend| legend.title("Series"))
        .mark(
            Line::<Polar>::new()
                .r_with(col("r"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(std::f64::consts::TAU))))
                })
                .geometry_space(GeometrySpace::Coordinate)
                .details(["series"])
                .stroke(col("series"))
                .stroke_width(3.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "radial_and_spiral").await;
}

#[tokio::test]
async fn polar_line_dashed_gaps() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("defined", DataType::Int32, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![
                82.0, 82.0, 82.0, 82.0, 82.0, 82.0, 82.0, 82.0, 82.0,
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![
                0.0, 0.7, 1.4, 2.1, 2.8, 3.5, 4.2, 4.9, 5.6,
            ])) as ArrayRef,
            Arc::new(Int32Array::from(vec![1, 1, 1, 0, 1, 1, 1, 0, 1])) as ArrayRef,
        ],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Dashed Line With Gaps")
        .mark(
            Line::<Polar>::new()
                .r_with(col("r"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(std::f64::consts::TAU))))
                })
                .geometry_space(GeometrySpace::Coordinate)
                .defined(col("defined"))
                .stroke("#0891b2")
                .stroke_width(4.0)
                .stroke_dash("dashed"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "dashed_gaps").await;
}

#[tokio::test]
async fn polar_line_multi_series_details() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("series", DataType::Utf8, false),
            Field::new("dash", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![
                34.0, 34.0, 34.0, 34.0, 34.0, 60.0, 60.0, 60.0, 60.0, 60.0, 86.0, 86.0, 86.0, 86.0,
                86.0,
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![
                0.15, 0.95, 1.75, 2.55, 3.35, 0.45, 1.35, 2.25, 3.15, 4.05, 0.75, 1.75, 2.75, 3.75,
                4.75,
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "solid", "solid", "solid", "solid", "solid", "dashed", "dashed", "dashed",
                "dashed", "dashed", "dotted", "dotted", "dotted", "dotted", "dotted",
            ])) as ArrayRef,
        ],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Multi-Series Polar Lines")
        .legend("stroke", |legend| legend.title("Series"))
        .mark(
            Line::<Polar>::new()
                .r_with(col("r"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(std::f64::consts::TAU))))
                })
                .geometry_space(GeometrySpace::Coordinate)
                .details(["series"])
                .stroke(col("series"))
                .stroke_width(3.0)
                .stroke_dash(col("dash")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "multi_series_details").await;
}

#[tokio::test]
async fn polar_line_categorical_theta() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![30.0, 78.0, 46.0, 90.0, 54.0, 70.0])) as ArrayRef,
            Arc::new(StringArray::from(vec!["N", "NE", "E", "SE", "S", "SW"])) as ArrayRef,
        ],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Categorical Theta")
        .mark(
            Line::<Polar>::new()
                .r_with(col("r"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .theta_with(col("theta"), |c| c.axis(|a| a.visible(false)))
                .geometry_space(GeometrySpace::Coordinate)
                .stroke("#7c3aed")
                .stroke_width(4.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "categorical_theta").await;
}

#[tokio::test]
async fn polar_line_clipping() {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![45.0, 115.0, 55.0, 125.0, 60.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.15, 1.05, 1.95, 2.85, 3.75])) as ArrayRef,
        ],
    );

    let plot = Chart::<Polar>::new()
        .data(df)
        .title("Clipped Polar Line")
        .mark(
            Line::<Polar>::new()
                .r_with(col("r"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(std::f64::consts::TAU))))
                })
                .geometry_space(GeometrySpace::Coordinate)
                .stroke("#f97316")
                .stroke_width(5.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    assert_visual_match_default(&compiled, &ctx, None, "polar_line", "clipping").await;
}
