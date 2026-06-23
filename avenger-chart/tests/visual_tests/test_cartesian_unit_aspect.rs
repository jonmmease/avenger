use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::prelude::*;
use std::{f64::consts::TAU, sync::Arc};

use super::helpers::assert_visual_match_default;

const CATEGORY: &str = "cartesian_unit_aspect";

fn read_batch(ctx: &SessionContext, fields: Vec<Field>, columns: Vec<ArrayRef>) -> DataFrame {
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .expect("create unit-aspect visual test data");
    ctx.read_batch(batch)
        .expect("read unit-aspect visual test data")
}

fn diagonal_data(ctx: &SessionContext) -> DataFrame {
    read_batch(
        ctx,
        vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("order", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![-5.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![-5.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, 1.0])) as ArrayRef,
        ],
    )
}

fn circle_data(ctx: &SessionContext) -> DataFrame {
    let steps = 160;
    let mut x = Vec::with_capacity(steps + 1);
    let mut y = Vec::with_capacity(steps + 1);
    let mut order = Vec::with_capacity(steps + 1);
    for index in 0..=steps {
        let angle = TAU * index as f64 / steps as f64;
        x.push(angle.cos());
        y.push(angle.sin());
        order.push(index as f64);
    }

    read_batch(
        ctx,
        vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("order", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(x)) as ArrayRef,
            Arc::new(Float64Array::from(y)) as ArrayRef,
            Arc::new(Float64Array::from(order)) as ArrayRef,
        ],
    )
}

fn guide_data(ctx: &SessionContext) -> DataFrame {
    read_batch(
        ctx,
        vec![
            Field::new("x_line", DataType::Float64, false),
            Field::new("y_line", DataType::Float64, false),
            Field::new("x_symbol", DataType::Float64, false),
            Field::new("y_symbol", DataType::Float64, false),
            Field::new("order", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(vec![-5.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![-5.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![-5.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![-4.0, 4.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, 1.0])) as ArrayRef,
        ],
    )
}

fn facet_circle_data(ctx: &SessionContext) -> DataFrame {
    let steps = 96;
    let mut panel = Vec::with_capacity((steps + 1) * 2);
    let mut x = Vec::with_capacity((steps + 1) * 2);
    let mut y = Vec::with_capacity((steps + 1) * 2);
    let mut order = Vec::with_capacity((steps + 1) * 2);

    for (panel_name, x_offset, y_offset, radius) in
        [("centered", 0.0, 0.0, 1.0), ("offset", 0.75, 0.25, 0.65)]
    {
        for index in 0..=steps {
            let angle = TAU * index as f64 / steps as f64;
            panel.push(panel_name);
            x.push(x_offset + radius * angle.cos());
            y.push(y_offset + radius * angle.sin());
            order.push(index as f64);
        }
    }

    read_batch(
        ctx,
        vec![
            Field::new("panel", DataType::Utf8, false),
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("order", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(panel)) as ArrayRef,
            Arc::new(Float64Array::from(x)) as ArrayRef,
            Arc::new(Float64Array::from(y)) as ArrayRef,
            Arc::new(Float64Array::from(order)) as ArrayRef,
        ],
    )
}

fn radius_symbol_data(ctx: &SessionContext) -> DataFrame {
    read_batch(
        ctx,
        vec![
            Field::new("panel", DataType::Utf8, false),
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec!["left", "left", "right", "right"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, 10.0, 0.0, 5.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, 10.0, 10.0, 0.0])) as ArrayRef,
        ],
    )
}

fn linear_domain(
    channel: CartesianPositionConfig,
    min: f64,
    max: f64,
    title: &str,
) -> CartesianPositionConfig {
    channel
        .scale_with::<Linear>(move |scale| scale.domain((min, max)).nice(false).zero(false))
        .axis(|axis| axis.title(title).grid(true))
}

fn symmetric_domain(channel: CartesianPositionConfig, title: &str) -> CartesianPositionConfig {
    linear_domain(channel, -5.0, 5.0, title)
}

fn unit_circle_domain(channel: CartesianPositionConfig, title: &str) -> CartesianPositionConfig {
    linear_domain(channel, -1.2, 1.2, title)
}

fn diagonal_mark() -> Line<Cartesian> {
    Line::new()
        .x_with(col("x"), |channel| symmetric_domain(channel, "x"))
        .y_with(col("y"), |channel| symmetric_domain(channel, "y"))
        .order(col("order"))
        .stroke("#2563eb")
        .stroke_width(4.0)
}

#[tokio::test]
async fn cartesian_unit_aspect_diagonal_default_distorted() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .plot_size(600.0, 300.0)
        .data(diagonal_data(&ctx))
        .mark(diagonal_mark());

    let compiled = plot.compile(&ctx).await.expect("compile default diagonal");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "diagonal_default_distorted",
    )
    .await;
}

#[tokio::test]
async fn cartesian_unit_aspect_diagonal_equal_units() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(600.0, 300.0)
        .data(diagonal_data(&ctx))
        .mark(diagonal_mark());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile unit-aspect diagonal");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "diagonal_equal_units").await;
}

#[tokio::test]
async fn cartesian_unit_aspect_circle_equal_units() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(600.0, 300.0)
        .data(circle_data(&ctx))
        .mark(
            Line::new()
                .x_with(col("x"), |channel| unit_circle_domain(channel, "x"))
                .y_with(col("y"), |channel| unit_circle_domain(channel, "y"))
                .order(col("order"))
                .stroke("#0f766e")
                .stroke_width(3.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile unit circle");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "circle_equal_units").await;
}

#[tokio::test]
async fn cartesian_unit_aspect_guide_expanded_domain() {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(600.0, 300.0)
        .data(guide_data(&ctx))
        .mark(
            Symbol::new()
                .x_with(col("x_symbol"), |channel| {
                    symmetric_domain(channel, "expanded x")
                })
                .y_with(col("y_symbol"), |channel| {
                    symmetric_domain(channel, "original y")
                })
                .fill("#dc2626")
                .stroke("#ffffff")
                .stroke_width(1.5)
                .size(220.0),
        )
        .mark(
            Line::new()
                .x_with(col("x_line"), |channel| {
                    symmetric_domain(channel, "expanded x")
                })
                .y_with(col("y_line"), |channel| {
                    symmetric_domain(channel, "original y")
                })
                .order(col("order"))
                .stroke("#334155")
                .stroke_width(2.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile guide expansion");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "guide_expanded_domain").await;
}

#[tokio::test]
async fn cartesian_unit_aspect_facet_shared_equal_units() {
    let ctx = SessionContext::new();
    let child = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Line::new()
            .x_with(col("x"), |channel| {
                channel
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared x").grid(true))
            })
            .y_with(col("y"), |channel| {
                channel
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared y").grid(true))
            })
            .order(col("order"))
            .stroke("#7c3aed")
            .stroke_width(3.0),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(620.0, 240.0)
        .data(facet_circle_data(&ctx))
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot.compile(&ctx).await.expect("compile shared facet");
    assert_visual_match_default(&compiled, &ctx, None, CATEGORY, "facet_shared_equal_units").await;
}

#[tokio::test]
async fn cartesian_unit_aspect_radius_aware_symbols_shared() {
    let ctx = SessionContext::new();
    let child = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Symbol::new()
            .x_with(col("x"), |channel| {
                channel
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared x").grid(true))
            })
            .y_with(col("y"), |channel| {
                channel
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared y").grid(true))
            })
            .fill("#f97316")
            .stroke("#111827")
            .stroke_width(1.5)
            .size(2500.0),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(620.0, 220.0)
        .data(radius_symbol_data(&ctx))
        .mark(Subplot::new(child).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile radius-aware facet");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "radius_aware_symbols_shared",
    )
    .await;
}
