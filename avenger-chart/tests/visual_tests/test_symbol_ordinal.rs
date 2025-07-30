//! Visual tests for symbol charts with automatic ordinal scales

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::col;
use datafusion::prelude::*;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

#[tokio::test]
async fn test_symbol_automatic_shape_scale() {
    // Create data with different product categories
    let sales = Float64Array::from(vec![
        120.5, 85.2, 95.8, 110.3, 75.6, 88.9, 92.4, 105.7, 78.3, 98.6, 115.2, 82.7,
    ]);
    let profit = Float64Array::from(vec![
        22.3, 15.8, 18.9, 25.6, 12.4, 16.7, 17.8, 23.9, 13.2, 19.8, 26.1, 14.5,
    ]);
    let category = StringArray::from(vec![
        "Electronics",
        "Clothing",
        "Food",
        "Electronics",
        "Clothing",
        "Food",
        "Electronics",
        "Clothing",
        "Food",
        "Electronics",
        "Clothing",
        "Food",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("sales", DataType::Float64, false),
        Field::new("profit", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(sales), Arc::new(profit), Arc::new(category)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    // Create plot using col("category") for automatic shape mapping
    // This demonstrates that an ordinal scale is automatically created
    // with shape strings as the range values
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear").domain((60.0, 130.0)))
        .scale_y(|scale| scale.scale_type("linear").domain((10.0, 30.0)))
        .axis_x(|axis| axis.title("Sales ($k)"))
        .axis_y(|axis| axis.title("Profit ($k)"))
        .mark(
            Symbol::new()
                .x(col("sales"))
                .y(col("profit"))
                .shape(col("category")) // Automatic ordinal scale
                .fill(col("category")) // Also use for color
                .size(200.0)
                .stroke("#333333")
                .stroke_width(1.5),
        );

    assert_visual_match_default(plot, "symbol_ordinal", "automatic_shape_scale").await;
}

#[tokio::test]
async fn test_symbol_custom_enumeration() {
    // Create data with custom priority levels
    let tasks = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    ]);
    let completion = Float64Array::from(vec![
        95.0, 20.0, 75.0, 15.0, 85.0, 50.0, 90.0, 30.0, 65.0, 10.0, 80.0, 40.0, 70.0, 25.0, 60.0,
    ]);
    let priority = StringArray::from(vec![
        "critical", "low", "high", "low", "critical", "medium", "critical", "low", "high", "low",
        "high", "medium", "high", "low", "medium",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("task_id", DataType::Float64, false),
        Field::new("completion", DataType::Float64, false),
        Field::new("priority", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(tasks), Arc::new(completion), Arc::new(priority)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    // Create plot demonstrating automatic ordinal scale for custom enumeration
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear").domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("Task ID"))
        .axis_y(|axis| axis.title("Completion %"))
        .mark(
            Symbol::new()
                .x(col("task_id"))
                .y(col("completion"))
                .shape(col("priority")) // Maps priority levels to shapes
                .fill(col("priority")) // Also use for color
                .size(250.0)
                .stroke("#000000")
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "symbol_ordinal", "custom_enumeration").await;
}
