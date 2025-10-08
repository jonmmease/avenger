use avenger_chart::layout::{CanvasConstraint, Margins, PlotConstraint};
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;

fn create_test_data() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![10.0, 25.0, 15.0, 30.0, 20.0, 35.0]);
    let categories = StringArray::from(vec![
        "Group A", "Group B", "Group A", "Group B", "Group A", "Group B",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
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
async fn test_fixed_plot_area_400x300_with_legend() {
    let ctx = SessionContext::new();
    let df = create_test_data();

    // Create a plot with fixed PLOT AREA of 400x300
    // The canvas will expand to fit this plus axes, legend, and margins
    let plot = Plot::<Cartesian>::new()
        .plot_size(400.0, 300.0)
        .margins(Margins::uniform(0.0).top(20.0).right(20.0).bottom(30.0).left(40.0))
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s).axis(|a| a.title("X Axis Label").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Y Axis Label").grid(true))
                })
                .stroke_with(col("category"), |c| {
                    c.legend(|l| l.title("Category Legend"))
                })
                .stroke_width(2.5),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "fixed_plot_area",
        "plot_400x300_with_legend",
    )
    .await;
}

#[tokio::test]
async fn test_fixed_plot_area_300x200_no_legend() {
    let ctx = SessionContext::new();
    let df = create_test_data();

    // Create a plot with fixed plot area but no legend
    let plot = Plot::<Cartesian>::new()
        .plot_size(300.0, 200.0)
        .margins(avenger_chart::layout::Margins::uniform(15.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s).axis(|a| a.title("X Values")))
                .y_with(col("y"), |c| c.scale(|s| s).axis(|a| a.title("Y Values")))
                .fill("#2ecc71")
                .size(40.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "fixed_plot_area",
        "plot_300x200_no_legend",
    )
    .await;
}

#[tokio::test]
async fn test_comparison_canvas_vs_plot_area() {
    let ctx = SessionContext::new();
    let df = create_test_data();

    // Traditional fixed canvas size
    let plot_canvas = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0) // Fixed canvas
        .data(df.clone())
        .mark(
            Line::new()
                .x_with(col("x"), |c| c.scale(|s| s).axis(|a| a.title("X Axis")))
                .y_with(col("y"), |c| c.scale(|s| s).axis(|a| a.title("Y Axis")))
                .stroke("#3498db")
                .stroke_width(2.0),
        );

    // Fixed plot area - canvas expands (build plot again)
    let plot_area = Plot::<Cartesian>::new()
        .plot_size(400.0, 300.0)
        .data(df.clone())
        .mark(
            Line::new()
                .x_with(col("x"), |c| c.scale(|s| s).axis(|a| a.title("X Axis")))
                .y_with(col("y"), |c| c.scale(|s| s).axis(|a| a.title("Y Axis")))
                .stroke("#3498db")
                .stroke_width(2.0),
        );

    // Canvas mode uses standard renderer (produces 800x600 at 2x scale)
    let compiled_canvas = plot_canvas
        .compile(&ctx)
        .await
        .expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled_canvas,
        &ctx,
        None,
        "fixed_plot_area",
        "canvas_mode_400x300",
    )
    .await;

    // Plot area mode uses computed canvas (produces larger image)
    let compiled_area = plot_area
        .compile(&ctx)
        .await
        .expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled_area,
        &ctx,
        None,
        "fixed_plot_area",
        "plot_area_mode_400x300",
    )
    .await;
}

#[tokio::test]
async fn test_plot_width_with_canvas_height() {
    let ctx = SessionContext::new();
    let df = create_test_data();

    // Create a plot with:
    // - Fixed plot area width of 350px
    // - Fixed canvas height of 400px
    // Plot area height and canvas width will be computed
    let plot = Plot::<Cartesian>::new()
        .plot_constraint(PlotConstraint::width(350.0)) // Fixed plot area width
        .canvas_constraint(CanvasConstraint::height(400.0)) // Fixed canvas height
        .margins(Margins::uniform(20.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Fixed Plot Width"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Fixed Canvas Height"))
                })
                .fill_with(col("category"), |c| c.legend(|l| l.title("Groups")))
                .size(60.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "fixed_plot_area",
        "plot_width_canvas_height",
    )
    .await;
}

#[tokio::test]
async fn test_fixed_plot_area_with_fixed_canvas() {
    let ctx = SessionContext::new();
    let df = create_test_data();

    // Create a plot with:
    // - Fixed plot area of 300x200
    // - Fixed canvas of 500x400
    // Margins will expand to center the plot
    let plot = Plot::<Cartesian>::new()
        .canvas_size(500.0, 400.0)
        .plot_size(300.0, 200.0)
        .margins(Margins::uniform(0.0).top(15.0).right(25.0).bottom(20.0).left(30.0))
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s).axis(|a| a.title("X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Y Axis").grid(true))
                })
                .stroke_with(col("category"), |c| c.legend(|l| l.title("Category")))
                .stroke_width(2.5),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "fixed_plot_area",
        "fixed_plot_with_fixed_canvas",
    )
    .await;
}
