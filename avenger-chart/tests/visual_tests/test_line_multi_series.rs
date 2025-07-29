//! Visual tests for multi-series line charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::line::Line;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

/// Create multi-series line data
fn create_multi_series_data() -> DataFrame {
    // Create data for 3 different series (A, B, C)
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Series A
        1.0, 2.0, 3.0, 4.0, 5.0, // Series B
        1.0, 2.0, 3.0, 4.0, 5.0, // Series C
    ]);

    let y_values = Float64Array::from(vec![
        10.0, 20.0, 15.0, 25.0, 30.0, // Series A
        5.0, 15.0, 20.0, 18.0, 22.0, // Series B
        8.0, 12.0, 18.0, 20.0, 26.0, // Series C
    ]);

    let series = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
    ]);

    let order = Int32Array::from(vec![
        1, 2, 3, 4, 5, // Series A in order
        1, 2, 3, 4, 5, // Series B in order
        1, 2, 3, 4, 5, // Series C in order
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("order", DataType::Int32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(series),
            Arc::new(order),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

/// Create mixed order data to test order channel
fn create_mixed_order_data() -> DataFrame {
    // Create data with mixed ordering to test order channel
    let x_values = Float64Array::from(vec![
        5.0, 1.0, 3.0, 2.0, 4.0, // Series A out of order
        3.0, 5.0, 1.0, 4.0, 2.0, // Series B out of order
    ]);

    let y_values = Float64Array::from(vec![
        30.0, 10.0, 15.0, 20.0, 25.0, // Series A values
        18.0, 22.0, 5.0, 20.0, 15.0, // Series B values
    ]);

    let series = StringArray::from(vec!["A", "A", "A", "A", "A", "B", "B", "B", "B", "B"]);

    // Order values to sort points correctly
    let order = Int32Array::from(vec![
        5, 1, 3, 2, 4, // Series A order
        3, 5, 1, 4, 2, // Series B order
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("order", DataType::Int32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(series),
            Arc::new(order),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

/// Create data with different width values per series
fn create_multi_series_with_widths() -> DataFrame {
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Series A
        1.0, 2.0, 3.0, 4.0, 5.0, // Series B
        1.0, 2.0, 3.0, 4.0, 5.0, // Series C
    ]);

    let y_values = Float64Array::from(vec![
        10.0, 20.0, 15.0, 25.0, 30.0, // Series A
        5.0, 15.0, 20.0, 18.0, 22.0, // Series B
        8.0, 12.0, 18.0, 20.0, 26.0, // Series C
    ]);

    let series = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
    ]);

    // Width values that vary by series
    let widths = Float64Array::from(vec![
        1.0, 1.0, 1.0, 1.0, 1.0, // Series A - thin
        3.0, 3.0, 3.0, 3.0, 3.0, // Series B - medium
        5.0, 5.0, 5.0, 5.0, 5.0, // Series C - thick
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("width", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(series),
            Arc::new(widths),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn test_multi_series_line_with_color() {
    let df = create_multi_series_data();

    // Create a plot with lines colored by series
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke(|s| s.scale_type("ordinal"))
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series")) // Color varies by series
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "line", "multi_series_color").await;
}

#[tokio::test]
async fn test_multi_series_line_with_width() {
    let df = create_multi_series_with_widths();

    // Create a plot with lines having different widths per series
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#4682b4")
                .stroke_width(col("width")), // Width varies by series
        );

    assert_visual_match_default(plot, "line", "multi_series_width").await;
}

#[tokio::test]
async fn test_multi_series_with_color_and_width() {
    let df = create_multi_series_with_widths();

    // Create a plot where color and width vary by series
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke(|s| s.scale_type("ordinal"))
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series")) // Color varies by series
                .stroke_width(col("width")), // Width varies by series
        );

    assert_visual_match_default(plot, "line", "multi_series_color_width").await;
}

#[tokio::test]
async fn test_line_with_order_channel() {
    let df = create_mixed_order_data();

    // Create a plot using order channel to sort points
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke(|s| s.scale_type("ordinal"))
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .order(col("order")), // Use order channel
        );

    assert_visual_match_default(plot, "line", "multi_series_order").await;
}

#[tokio::test]
async fn test_multi_series_line_with_dash() {
    // Create data with different series indicated by dash pattern
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 4.0, 3.0, 5.0, 4.5, 1.5, 3.0, 2.5, 4.0, 3.5, 2.5, 3.5, 4.0, 4.5, 5.0,
    ]);
    let dash_type = StringArray::from(vec![
        "solid", "solid", "solid", "solid", "solid", "dashed", "dashed", "dashed", "dashed",
        "dashed", "dotted", "dotted", "dotted", "dotted", "dotted",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("dash_type", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(dash_type)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke_dash(|s| {
            s.scale_type("ordinal")
                .range_discrete(vec![lit("solid"), lit("dashed"), lit("dotted")])
        })
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke_dash(col("dash_type"))
                .stroke_width(2.0)
                .stroke("#4472C4"),
        );

    assert_visual_match_default(plot, "line", "multi_series_dash").await;
}

#[tokio::test]
async fn test_multi_series_line_with_color_and_dash() {
    // Create data with different series indicated by both color and dash
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0,
        4.0, 5.0,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 4.0, 3.0, 5.0, 4.5, 1.5, 3.0, 2.5, 4.0, 3.5, 2.5, 3.5, 4.0, 4.5, 5.0, 3.0, 4.5, 5.0,
        5.5, 6.0,
    ]);
    let series = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "A", "A", "A", "A", "A", "B", "B", "B",
        "B", "B",
    ]);
    let line_style = StringArray::from(vec![
        "solid", "solid", "solid", "solid", "solid", "solid", "solid", "solid", "solid", "solid",
        "dashed", "dashed", "dashed", "dashed", "dashed", "dashed", "dashed", "dashed", "dashed",
        "dashed",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("line_style", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(series),
            Arc::new(line_style),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke(|s| s.scale_type("ordinal"))
        .scale_stroke_dash(|s| {
            s.scale_type("ordinal")
                .range_discrete(vec![lit("solid"), lit("dashed")])
        })
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .stroke_dash(col("line_style"))
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "line", "multi_series_color_dash").await;
}

#[tokio::test]
async fn test_multi_series_line_all_encodings() {
    // Create data with series indicated by color, width, and dash
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 4.0, 3.0, 5.0, 4.5, 1.5, 3.0, 2.5, 4.0, 3.5, 3.0, 4.5, 5.0, 5.5, 6.0,
    ]);
    let series = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
    ]);
    let size = Float64Array::from(vec![
        1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 3.0,
    ]);
    let dash = StringArray::from(vec![
        "solid", "solid", "solid", "solid", "solid", "dashed", "dashed", "dashed", "dashed",
        "dashed", "dotted", "dotted", "dotted", "dotted", "dotted",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("size", DataType::Float64, false),
        Field::new("dash", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(series),
            Arc::new(size),
            Arc::new(dash),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_stroke(|s| s.scale_type("ordinal"))
        .scale_stroke_width(|s| s.scale_type("linear").range_numeric(lit(1.0), lit(4.0)))
        .scale_stroke_dash(|s| {
            s.scale_type("ordinal")
                .range_discrete(vec![lit("solid"), lit("dashed"), lit("dotted")])
        })
        .axis_x(|axis| axis.title("X").grid(true))
        .axis_y(|axis| axis.title("Y").grid(true))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .stroke_width(col("size"))
                .stroke_dash(col("dash")),
        );

    assert_visual_match_default(plot, "line", "multi_series_all_encodings").await;
}
