use super::helpers::assert_visual_match_default;
use avenger_chart::polar::PolarPositionConfig;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::prelude::*;
use std::{f64::consts::TAU, sync::Arc};

use crate::mark_effects_support::KeepUprightText;

const LABEL_THETA: [f64; 8] = [0.2, 0.9, 1.7, 2.6, 3.4, 4.3, 5.1, 5.9];

fn read_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> DataFrame {
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .expect("create polar text visual test data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read polar text visual test data")
}

fn label_data(r: f64, labels: &[&str], theta: &[f64]) -> DataFrame {
    read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![r; labels.len()])) as ArrayRef,
            Arc::new(Float64Array::from(theta.to_vec())) as ArrayRef,
            Arc::new(StringArray::from(labels.to_vec())) as ArrayRef,
        ],
    )
}

fn text_label_data(r: f64) -> DataFrame {
    label_data(r, &["TEXT"; 8], &LABEL_THETA)
}

fn ring_data() -> DataFrame {
    let theta = (0..=96)
        .map(|index| index as f64 / 96.0 * TAU)
        .collect::<Vec<_>>();
    read_batch(
        vec![Field::new("theta", DataType::Float64, false)],
        vec![Arc::new(Float64Array::from(theta)) as ArrayRef],
    )
}

fn spoke_data() -> DataFrame {
    let mut r = Vec::new();
    let mut theta = Vec::new();
    let mut spoke = Vec::new();
    for (index, angle) in LABEL_THETA.iter().enumerate() {
        r.push(0.0);
        r.push(92.0);
        theta.push(*angle);
        theta.push(*angle);
        spoke.push(format!("s{index}"));
        spoke.push(format!("s{index}"));
    }
    read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("spoke", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(r)) as ArrayRef,
            Arc::new(Float64Array::from(theta)) as ArrayRef,
            Arc::new(StringArray::from(spoke)) as ArrayRef,
        ],
    )
}

fn categorical_label_data() -> DataFrame {
    read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Utf8, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![75.0; 8])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "N", "NE", "E", "SE", "S", "SW", "W", "NW",
            ])) as ArrayRef,
        ],
    )
}

fn leader_label_data() -> DataFrame {
    read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("theta", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
            Field::new("dx", DataType::Float64, false),
            Field::new("dy", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![54.0, 62.0, 58.0, 64.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.35, 1.8, 3.25, 4.9])) as ArrayRef,
            Arc::new(StringArray::from(vec!["one", "two", "three", "four"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![28.0, -24.0, -30.0, 24.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![-18.0, -18.0, 20.0, 20.0])) as ArrayRef,
        ],
    )
}

fn full_circle_theta(c: PolarPositionConfig) -> PolarPositionConfig {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(TAU))))
}

fn full_circle_theta_without_axis(c: PolarPositionConfig) -> PolarPositionConfig {
    full_circle_theta(c).axis(|a| a.visible(false))
}

fn categorical_theta_without_axis(c: PolarPositionConfig) -> PolarPositionConfig {
    c.scale_with::<Point>(|s| s.round(false))
        .axis(|a| a.visible(false))
}

fn reference_plot() -> Chart<Polar> {
    Chart::<Polar>::new()
        .plot_size(260.0, 260.0)
        .mark(
            Line::<Polar>::new()
                .data(ring_data())
                .r(92.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .stroke("#d1d5db")
                .stroke_width(1.0),
        )
        .mark(
            Line::<Polar>::new()
                .data(spoke_data())
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), |c| c.no_scale())
                .geometry_space(GeometrySpace::Display)
                .details(["spoke"])
                .stroke("#e5e7eb")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::<Polar>::new()
                .data(text_label_data(74.0))
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), |c| c.no_scale())
                .size(20.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
}

fn centered_text(data: DataFrame) -> Text<Polar> {
    Text::<Polar>::new()
        .data(data)
        .r_with(col("r"), |c| c.no_scale())
        .theta_with(col("theta"), |c| c.no_scale())
        .text(col("label"))
        .align("center")
        .baseline("middle")
        .font_size(14.0)
        .font_weight("bold")
}

#[tokio::test]
async fn radial_coordinate_default_angles() {
    let ctx = SessionContext::new();
    let plot = reference_plot()
        .title("Default Coordinate Radial Text")
        .mark(
            centered_text(text_label_data(74.0))
                .angle(0.0)
                .color("#1d4ed8"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "radial_coordinate_default_angles",
    )
    .await;
}

#[tokio::test]
async fn radial_coordinate_explicit_angles() {
    let ctx = SessionContext::new();
    let plot = reference_plot()
        .title("Explicit Coordinate Radial Text")
        .mark(
            centered_text(text_label_data(74.0))
                .geometry_space(GeometrySpace::Coordinate)
                .angle(0.0)
                .color("#2563eb"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "radial_coordinate_explicit_angles",
    )
    .await;
}

#[tokio::test]
async fn tangential_coordinate_angles() {
    let ctx = SessionContext::new();
    let plot = reference_plot().title("Coordinate Tangential Text").mark(
        centered_text(text_label_data(74.0))
            .geometry_space(GeometrySpace::Coordinate)
            .angle(90.0)
            .color("#047857"),
    );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "tangential_coordinate_angles",
    )
    .await;
}

#[tokio::test]
async fn display_space_horizontal_angles() {
    let ctx = SessionContext::new();
    let plot = reference_plot()
        .title("Display Space Horizontal Text")
        .mark(
            centered_text(text_label_data(74.0))
                .geometry_space(GeometrySpace::Display)
                .angle(0.0)
                .color("#dc2626"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "display_space_horizontal_angles",
    )
    .await;
}

#[tokio::test]
async fn coordinate_vs_display_overlay() {
    let ctx = SessionContext::new();
    let plot = reference_plot()
        .title("Coordinate vs Display Text")
        .mark(
            centered_text(text_label_data(84.0))
                .geometry_space(GeometrySpace::Coordinate)
                .angle(0.0)
                .color("#2563eb"),
        )
        .mark(
            centered_text(text_label_data(58.0))
                .geometry_space(GeometrySpace::Display)
                .angle(0.0)
                .color("#dc2626"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "coordinate_vs_display_overlay",
    )
    .await;
}

#[tokio::test]
async fn keep_upright_adjustment_radial() {
    let ctx = SessionContext::new();
    let plot = reference_plot().title("Keep Upright Radial Text").mark(
        centered_text(text_label_data(74.0))
            .geometry_space(GeometrySpace::Coordinate)
            .angle(0.0)
            .color("#7c2d12")
            .adjust_transform(KeepUprightText::new(), |text, upright| {
                text.angle(upright.angle())
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "keep_upright_adjustment_radial",
    )
    .await;
}

#[tokio::test]
async fn keep_upright_adjustment_tangential() {
    let ctx = SessionContext::new();
    let plot = reference_plot().title("Keep Upright Tangential Text").mark(
        centered_text(text_label_data(74.0))
            .geometry_space(GeometrySpace::Coordinate)
            .angle(90.0)
            .color("#581c87")
            .adjust_transform(KeepUprightText::new(), |text, upright| {
                text.angle(upright.angle())
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "keep_upright_adjustment_tangential",
    )
    .await;
}

#[tokio::test]
async fn leader_text_coordinate_angle() {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(260.0, 260.0)
        .title("Coordinate Text With Leaders")
        .mark(
            Line::<Polar>::new()
                .data(ring_data())
                .r(80.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .stroke("#d1d5db")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::<Polar>::new()
                .data(leader_label_data())
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), |c| c.no_scale())
                .size(36.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
        .mark(
            Text::<Polar>::new()
                .data(leader_label_data())
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), |c| c.no_scale())
                .geometry_space(GeometrySpace::Coordinate)
                .text(col("label"))
                .angle(0.0)
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .font_weight("bold")
                .color("#0f766e")
                .leader(true)
                .leader_offset_x(col("dx"))
                .leader_offset_y(col("dy"))
                .leader_stroke("#334155")
                .leader_stroke_width(1.2),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "leader_text_coordinate_angle",
    )
    .await;
}

#[tokio::test]
async fn categorical_theta_coordinate_angles() {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(260.0, 260.0)
        .data(categorical_label_data())
        .title("Categorical Coordinate Text")
        .mark(
            Line::<Polar>::new()
                .data(ring_data())
                .r(92.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .stroke("#d1d5db")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::<Polar>::new()
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), categorical_theta_without_axis)
                .size(24.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
        .mark(
            Text::<Polar>::new()
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), categorical_theta_without_axis)
                .geometry_space(GeometrySpace::Coordinate)
                .text(col("theta"))
                .angle(0.0)
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .font_weight("bold")
                .color("#7e22ce"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "categorical_theta_coordinate_angles",
    )
    .await;
}

#[tokio::test]
async fn scaled_theta_coordinate_angles() {
    let ctx = SessionContext::new();
    let df = label_data(74.0, &["0", "25", "50", "75"], &[0.0, 25.0, 50.0, 75.0]);
    let plot = Chart::<Polar>::new()
        .plot_size(260.0, 260.0)
        .title("Scaled Theta Coordinate Text")
        .mark(
            Line::<Polar>::new()
                .data(ring_data())
                .r(92.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .stroke("#d1d5db")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::<Polar>::new()
                .data(df.clone())
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), full_circle_theta_without_axis)
                .size(24.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
        .mark(
            Text::<Polar>::new()
                .data(df)
                .r_with(col("r"), |c| c.no_scale())
                .theta_with(col("theta"), full_circle_theta_without_axis)
                .geometry_space(GeometrySpace::Coordinate)
                .text(col("label"))
                .angle(0.0)
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .font_weight("bold")
                .color("#0f766e"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile polar text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "polar_text",
        "scaled_theta_coordinate_angles",
    )
    .await;
}
