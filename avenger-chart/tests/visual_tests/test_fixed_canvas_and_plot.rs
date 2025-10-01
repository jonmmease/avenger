use avenger_chart::layout::{CanvasConstraint, Margins, PlotConstraint};
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;

fn create_test_data() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0]);
    let y_values = Float64Array::from(vec![10.0, 20.0, 15.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn test_fixed_canvas_500x400_fixed_plot_200x150() {
    let df = create_test_data();

    // Create a plot with:
    // - Fixed canvas: 500x400
    // - Fixed plot: 200x150
    // - Uniform margins: 10px
    // This tests whether margins expand or plot area position adjusts
    let plot = Plot::<Cartesian>::new()
        .canvas_size(500.0, 400.0)
        .plot_size(200.0, 150.0)
        .margins(Margins::uniform(10.0))
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill("#3498db"),
        );

    assert_visual_match_default(
        plot,
        "fixed_canvas_and_plot",
        "canvas_500x400_plot_200x150",
    )
    .await;
}

#[tokio::test]
async fn test_canvas_width_400_plot_width_250() {
    let df = create_test_data();

    // Create a plot with:
    // - Canvas width: 400
    // - Plot width: 250
    // - Uniform margins: 10px
    // Tests horizontal space distribution with fixed canvas and plot widths
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::Width(400.0))
        .plot_constraint(PlotConstraint::Width(250.0))
        .margins(Margins::uniform(10.0))
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(80.0)
                .fill("#e74c3c"),
        );

    assert_visual_match_default(plot, "fixed_canvas_and_plot", "canvas_width_400_plot_width_250")
        .await;
}

#[tokio::test]
async fn test_canvas_height_350_plot_height_200() {
    let df = create_test_data();

    // Create a plot with:
    // - Canvas height: 350
    // - Plot height: 200
    // - Uniform margins: 15px
    // Tests vertical space distribution with fixed canvas and plot heights
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::Height(350.0))
        .plot_constraint(PlotConstraint::Height(200.0))
        .margins(Margins::uniform(15.0))
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(90.0)
                .fill("#2ecc71"),
        );

    assert_visual_match_default(
        plot,
        "fixed_canvas_and_plot",
        "canvas_height_350_plot_height_200",
    )
    .await;
}
