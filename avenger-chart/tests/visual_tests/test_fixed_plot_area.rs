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
    let df = create_test_data();

    // Create a plot with fixed PLOT AREA of 400x300
    // The canvas will expand to fit this plus axes, legend, and margins
    let plot = Plot::<Cartesian>::new()
        .plot_size(400.0, 300.0)
        .margins(Margins {
            top: 20.0,
            right: 20.0,
            bottom: 30.0,
            left: 40.0,
        })
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

    assert_visual_match_default(plot, "fixed_plot_area", "plot_400x300_with_legend").await;
}

#[tokio::test]
async fn test_fixed_plot_area_300x200_no_legend() {
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

    assert_visual_match_default(plot, "fixed_plot_area", "plot_300x200_no_legend").await;
}

#[tokio::test]
async fn test_comparison_canvas_vs_plot_area() {
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
    assert_visual_match_default(plot_canvas, "fixed_plot_area", "canvas_mode_400x300").await;

    // Plot area mode uses computed canvas (produces larger image)
    assert_visual_match_default(plot_area, "fixed_plot_area", "plot_area_mode_400x300").await;
}

#[tokio::test]
async fn test_plot_aspect_ratio_with_canvas_width() {
    let df = create_test_data();

    // Create a plot with:
    // - Fixed canvas width of 500px
    // - Plot area aspect ratio of 2:1 (width:height)
    // Canvas height will be computed to accommodate the plot area with this ratio
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::Width(500.0)) // Fixed canvas width
        .plot_constraint(PlotConstraint::AspectRatio(2.0)) // Plot area width:height = 2:1
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s)
                        .axis(|a| a.title("X Axis with Fixed Width").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Y Axis").grid(true))
                })
                .stroke_with(col("category"), |c| c.legend(|l| l.title("Category")))
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "fixed_plot_area", "aspect_ratio_with_width").await;
}

#[tokio::test]
async fn test_plot_width_with_canvas_height() {
    let df = create_test_data();

    // Create a plot with:
    // - Fixed plot area width of 350px
    // - Fixed canvas height of 400px
    // Plot area height and canvas width will be computed
    let plot = Plot::<Cartesian>::new()
        .plot_constraint(PlotConstraint::Width(350.0)) // Fixed plot area width
        .canvas_constraint(CanvasConstraint::Height(400.0)) // Fixed canvas height
        .margins(Margins {
            top: 20.0,
            right: 20.0,
            bottom: 20.0,
            left: 20.0,
        })
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

    assert_visual_match_default(plot, "fixed_plot_area", "plot_width_canvas_height").await;
}

#[tokio::test]
async fn test_canvas_aspect_ratio() {
    let df = create_test_data();

    // Create a plot with canvas aspect ratio of 2:1 (wide)
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::PreferredAspectRatio(2.0)) // Width:Height = 2:1
        .margins(Margins::uniform(20.0))
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
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "fixed_plot_area", "canvas_aspect_ratio_2_1").await;
}

#[tokio::test]
async fn test_canvas_aspect_ratio_tall() {
    let df = create_test_data();

    // Create a plot with canvas aspect ratio of 0.5:1 (tall)
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::PreferredAspectRatio(0.5)) // Width:Height = 0.5:1 (tall)
        .margins(avenger_chart::layout::Margins::uniform(15.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s).axis(|a| a.title("X Axis")))
                .y_with(col("y"), |c| c.scale(|s| s).axis(|a| a.title("Y Axis")))
                .fill_with(col("category"), |c| c.legend(|l| l.title("Groups")))
                .size(50.0),
        );

    assert_visual_match_default(plot, "fixed_plot_area", "canvas_aspect_ratio_tall").await;
}

#[tokio::test]
async fn test_fixed_plot_area_with_canvas_aspect_ratio() {
    let df = create_test_data();

    // Create a plot with:
    // - Fixed plot area of 300x200
    // Note: Canvas aspect ratio with fixed plot area is not supported
    // in the new API due to Taffy limitations. Using fixed plot area only.
    let plot = Plot::<Cartesian>::new()
        .plot_size(300.0, 200.0)
        .margins(Margins {
            top: 15.0,
            right: 25.0,
            bottom: 20.0,
            left: 30.0,
        })
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

    assert_visual_match_default(plot, "fixed_plot_area", "fixed_plot_with_canvas_aspect").await;
}

#[tokio::test]
async fn test_fixed_canvas_with_plot_aspect_ratio() {
    let df = create_test_data();

    // Create a plot with:
    // - Fixed canvas: 600x400
    // - Plot area aspect ratio: 3:1 (wider than tall)
    // - Minimum margins: 20px
    // The plot area will expand to fill as much space as possible while maintaining 3:1 ratio,
    // then margins will expand to fill remaining space
    let plot = Plot::<Cartesian>::new()
        .canvas_size(600.0, 400.0)
        .plot_constraint(PlotConstraint::AspectRatio(3.0)) // 3:1 aspect ratio
        .margins(Margins::uniform(20.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s).axis(|a| a.title("X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s).axis(|a| a.title("Y Axis").grid(true))
                })
                .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
                .size(100.0)
                .shape("circle"),
        );

    assert_visual_match_default(plot, "fixed_plot_area", "fixed_canvas_plot_aspect_ratio").await;
}
