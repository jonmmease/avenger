//! Tests for default axis creation

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::line::Line;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

fn create_test_data() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0]);
    let categories = StringArray::from(vec!["A", "B", "C", "D", "E"]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x_val", DataType::Float64, false),
        Field::new("y_val", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(categories)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn test_default_axes_with_numeric_scales() {
    let df = create_test_data();

    // Create a plot without explicit axes
    let plot = Plot::new(Cartesian).data(df).mark(
        Line::new()
            .x(col("x_val"))
            .y(col("y_val"))
            .stroke("#4682b4"),
    );

    // Render to trigger default axis creation
    use avenger_chart::render::CanvasExt;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // This should create default axes with:
    // - x axis titled "x_val" with grid enabled (numeric scale)
    // - y axis titled "y_val" with grid enabled (numeric scale)
    canvas
        .render_plot(&plot)
        .await
        .expect("Failed to render plot");
}

#[tokio::test]
async fn test_default_axes_with_band_scale() {
    let df = create_test_data();

    // Create a plot with band scale on x
    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.scale_type("band"))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(col("category").scaled().with_band(1.0))
                .y(lit(0.0))
                .y2(col("y_val"))
                .fill("#4682b4"),
        );

    // Render to trigger default axis creation
    use avenger_chart::render::CanvasExt;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // This should create default axes with:
    // - x axis titled "category" with grid disabled (band scale)
    // - y axis titled "y_val" with grid enabled (numeric scale)
    canvas
        .render_plot(&plot)
        .await
        .expect("Failed to render plot");
}

#[tokio::test]
async fn test_disable_axis_with_visible_false() {
    let df = create_test_data();

    // Create a plot and explicitly disable x axis
    let plot = Plot::new(Cartesian)
        .data(df)
        .axis_x(|axis| axis.visible(false))
        .mark(
            Line::new()
                .x(col("x_val"))
                .y(col("y_val"))
                .stroke("#4682b4"),
        );

    // Render
    use avenger_chart::render::CanvasExt;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // X axis should be invisible, Y axis should be created with defaults
    canvas
        .render_plot(&plot)
        .await
        .expect("Failed to render plot");
}

#[tokio::test]
async fn test_override_default_axis_properties() {
    let df = create_test_data();

    // Create a plot and override some default axis properties
    let plot = Plot::new(Cartesian)
        .data(df)
        .axis_x(|axis| axis.title("Custom X Title").grid(false))
        .mark(
            Line::new()
                .x(col("x_val"))
                .y(col("y_val"))
                .stroke("#4682b4"),
        );

    // Render
    use avenger_chart::render::CanvasExt;
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // X axis should have custom title and no grid
    // Y axis should be created with defaults
    canvas
        .render_plot(&plot)
        .await
        .expect("Failed to render plot");
}
