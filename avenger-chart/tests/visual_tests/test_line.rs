//! Visual tests for line charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::line::Line;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

/// Create a simple line chart dataset
fn create_line_data() -> DataFrame {
    let x_values = Int32Array::from(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let y_values = Float64Array::from(vec![
        10.0, 25.0, 35.0, 30.0, 45.0, 60.0, 55.0, 70.0, 65.0, 80.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

#[tokio::test]
async fn test_simple_line_chart() {
    let df = create_line_data();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#4682b4")
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "line", "simple_line_chart").await;
}

#[tokio::test]
async fn test_line_with_dashed_stroke() {
    let df = create_line_data();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#dc143c")
                .stroke_width(3.0)
                .stroke_dash("dashed"),
        );

    assert_visual_match_default(plot, "line", "line_dashed_stroke").await;
}

#[tokio::test]
async fn test_line_with_gaps() {
    // Create data with gaps (undefined values)
    let x_values = Int32Array::from(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let y_values = Float64Array::from(vec![
        10.0, 25.0, 35.0, 30.0, 45.0, 60.0, 55.0, 70.0, 65.0, 80.0,
    ]);
    // Define which points are connected (0 creates gaps)
    let defined_values = Int32Array::from(vec![1, 1, 1, 0, 0, 1, 1, 1, 0, 1]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
        Field::new("defined", DataType::Int32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(defined_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .defined(col("defined")) // Using numeric column that will be coerced to boolean
                .stroke("#2e8b57")
                .stroke_width(2.5),
        );

    assert_visual_match_default(plot, "line", "line_with_gaps").await;
}

#[tokio::test]
async fn test_multiple_lines() {
    // Create data for multiple lines
    let x_values = Int32Array::from(vec![
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, // Line A
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, // Line B
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, // Line C
    ]);
    let y_values = Float64Array::from(vec![
        // Line A
        10.0, 15.0, 13.0, 18.0, 20.0, 25.0, 22.0, 28.0, 30.0, 35.0, // Line B
        20.0, 25.0, 30.0, 28.0, 35.0, 40.0, 45.0, 43.0, 50.0, 55.0, // Line C
        30.0, 32.0, 35.0, 40.0, 38.0, 45.0, 50.0, 55.0, 60.0, 65.0,
    ]);
    let series = datafusion::arrow::array::StringArray::from(vec![
        "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "B", "B", "B",
        "B", "B", "C", "C", "C", "C", "C", "C", "C", "C", "C", "C",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(series)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Cartesian)
        .data(df.clone())
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Line::new()
                .data(
                    df.clone()
                        .filter(col("series").eq(datafusion::logical_expr::lit("A")))
                        .unwrap(),
                )
                .x(col("x"))
                .y(col("y"))
                .stroke("#e74c3c")
                .stroke_width(2.0),
        )
        .mark(
            Line::new()
                .data(
                    df.clone()
                        .filter(col("series").eq(datafusion::logical_expr::lit("B")))
                        .unwrap(),
                )
                .x(col("x"))
                .y(col("y"))
                .stroke("#3498db")
                .stroke_width(2.0),
        )
        .mark(
            Line::new()
                .data(
                    df.filter(col("series").eq(datafusion::logical_expr::lit("C")))
                        .unwrap(),
                )
                .x(col("x"))
                .y(col("y"))
                .stroke("#f39c12")
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "line", "multiple_lines").await;
}

#[tokio::test]
async fn test_line_dash_patterns() {
    let df = create_line_data();

    let plot = Plot::new(Cartesian)
        .data(df.clone())
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear").domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        // Solid line
        .mark(
            Line::new()
                .data(df.clone())
                .x(col("x"))
                .y(col("y"))
                .stroke("#2c3e50")
                .stroke_width(2.0)
                .stroke_dash("solid"),
        )
        // Dashed line
        .mark(
            Line::new()
                .data(df.clone())
                .x(col("x"))
                .y(col("y") + datafusion::logical_expr::lit(10.0))
                .stroke("#e74c3c")
                .stroke_width(2.0)
                .stroke_dash("dashed"),
        )
        // Dotted line
        .mark(
            Line::new()
                .data(df.clone())
                .x(col("x"))
                .y(col("y") + datafusion::logical_expr::lit(20.0))
                .stroke("#27ae60")
                .stroke_width(2.0)
                .stroke_dash("dotted"),
        )
        // Dashdot line
        .mark(
            Line::new()
                .data(df)
                .x(col("x"))
                .y(col("y") + datafusion::logical_expr::lit(30.0))
                .stroke("#8e44ad")
                .stroke_width(2.0)
                .stroke_dash("dashdot"),
        );

    assert_visual_match_default(plot, "line", "line_dash_patterns").await;
}

#[tokio::test]
async fn test_line_vertical_padding_no_nice() {
    // Create data that goes right to the edges
    let x_values = Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    let y_values = Float64Array::from(vec![0.0, 50.0, 25.0, 75.0, 100.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear").option("nice", lit(false)))
        .scale_y(|scale| {
            scale.scale_type("linear").option("nice", lit(false))
            // Without radius padding, the line would be clipped at y=0 and y=100
        })
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#e74c3c")
                .stroke_width(10.0), // Large stroke width to make the effect visible
        );

    assert_visual_match_default(plot, "line", "line_vertical_padding_no_nice").await;
}
