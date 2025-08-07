use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_symbol_legend_with_scalar_expressions() {
    // Create test data
    let categories = StringArray::from(vec!["A", "B", "A", "C", "B"]);
    let x_values = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create plot with scalar expressions for various channels
    let plot = Plot::new(Cartesian)
        .data(df)
        .legend_shape(|legend| legend.title("Category"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("category")) // This is the legend channel
                .size(lit(100.0).identity()) // Scalar expression - should use this value
                .fill(lit("#ff0000").identity()) // Scalar expression - should use red
                .stroke(lit("#0000ff").identity()) // Scalar expression - should use blue
                .stroke_width(lit(2.0).identity()) // Scalar expression - should use 2.0
                .angle(lit(45.0).identity()), // Scalar expression - should use 45 degrees
        );

    assert_visual_match_default(plot, "legend", "symbol_scalar_expressions").await;
}

#[tokio::test]
async fn test_symbol_legend_with_column_dependencies() {
    // Create test data
    let categories = StringArray::from(vec!["A", "B", "A", "C", "B"]);
    let x_values = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 1.0]);
    let size_values = Float32Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("size_col", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories),
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(size_values),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create plot where size depends on a column (not the legend channel)
    let plot = Plot::new(Cartesian)
        .data(df)
        .legend_fill(|legend| legend.title("Category"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category")) // This is the legend channel
                .size(col("size_col")), // Depends on column but not legend channel - should use default
        );

    assert_visual_match_default(plot, "legend", "symbol_column_dependencies").await;
}

#[tokio::test]
async fn test_ordinal_size_legend() {
    // Create test data with categories for size mapping
    let categories =
        StringArray::from(vec!["Small", "Medium", "Large", "Small", "Large", "Medium"]);
    let x_values = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("size_category", DataType::Utf8, false),
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create plot with ordinal size scale
    let plot = Plot::new(Cartesian)
        .data(df)
        .legend_size(|legend| legend.title("Size Category"))
        .scale_size(|scale| {
            scale
                .range_discrete(vec![lit(50.0), lit(150.0), lit(300.0)])
                .domain(vec![lit("Small"), lit("Medium"), lit("Large")])
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(col("size_category"))
                .fill(lit("#1f77b4").identity())
                .shape(lit("circle").identity()),
        );

    assert_visual_match_default(plot, "legend", "ordinal_size_legend").await;
}

#[tokio::test]
async fn test_combined_size_color_shape_legend() {
    // Set environment variable to save layout SVG for debugging
    unsafe {
        std::env::set_var("AVENGER_DEBUG_LAYOUT", "tests/failures/legend");
    }

    // Create test data with a single category column that will drive size, color, and shape
    let categories = StringArray::from(vec![
        "Type A", "Type B", "Type C", "Type A", "Type B", "Type C",
    ]);
    let x_values = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories) as ArrayRef,
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .data(df)
        // Configure scales for the shared category column
        .scale_size(|scale| {
            scale
                .range_discrete(vec![lit(30.0), lit(120.0), lit(480.0)])
                .domain(vec![lit("Type A"), lit("Type B"), lit("Type C")])
        })
        .scale_fill(|scale| {
            scale
                .range_discrete(vec![lit("#e41a1c"), lit("#377eb8"), lit("#4daf4a")])
                .domain(vec![lit("Type A"), lit("Type B"), lit("Type C")])
        })
        .scale_shape(|scale| {
            scale
                .range_discrete(vec![lit("circle"), lit("square"), lit("triangle-up")])
                .domain(vec![lit("Type A"), lit("Type B"), lit("Type C")])
        })
        // Configure the legend to show all three varying properties
        .legend_fill(|legend| legend.title("Type"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                // All three channels use the same column
                .size(col("category"))
                .fill(col("category"))
                .shape(col("category"))
                .stroke(lit("#000000").identity())
                .stroke_width(lit(1.0).identity()),
        );

    assert_visual_match_default(plot, "legend", "combined_size_color_shape_legend").await;
}
