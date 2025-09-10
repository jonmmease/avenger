use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use avenger_chart::zerod::ZeroDCoord;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::col;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a dataset for ZeroD rendering (data is present but position is ignored)
fn create_zerod_data() -> DataFrame {
    // Create data with different categories and sizes
    let categories = StringArray::from(vec!["Type A", "Type B", "Type C", "Type D", "Type E"]);
    let sizes = Float64Array::from(vec![100.0, 150.0, 200.0, 250.0, 300.0]);
    let values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("size", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(sizes), Arc::new(values)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

#[tokio::test]
async fn test_zerod_symbol_basic() {
    let df = create_zerod_data();

    // Create a ZeroD plot - all symbols render at center
    let plot = Plot::<ZeroDCoord>::new()
        .title("Zero-Dimensional Symbol Plot")
        .subtitle("All points collapse to a single location")
        .data(df)
        .mark(
            Symbol::new()
                .size(200.0)
                .fill("#e74c3c")
                .stroke("#c0392b")
                .stroke_width(2.0)
                .shape("circle"),
        );

    assert_visual_match_default(plot, "zerod", "symbol_basic").await;
}

#[tokio::test]
async fn test_zerod_symbol_with_color_encoding() {
    let df = create_zerod_data();

    // ZeroD with color encoding - shows how data can still be encoded visually
    let plot = Plot::<ZeroDCoord>::new()
        .title("ZeroD with Color Encoding")
        .subtitle("Multiple data points at the same position")
        .data(df)
        .mark(
            Symbol::new()
                .size_with(col("size"), |c| c.legend(|l| l.title("Size")))
                .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
                .stroke("#333333")
                .stroke_width(1.0)
                .shape("circle"),
        );

    assert_visual_match_default(plot, "zerod", "symbol_color_encoding").await;
}

#[tokio::test]
async fn test_zerod_symbol_single_point() {
    // Create a single point dataset
    let categories = StringArray::from(vec!["Single Point"]);
    let values = Float64Array::from(vec![42.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(categories), Arc::new(values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    // Single point in ZeroD space
    let plot = Plot::<ZeroDCoord>::new()
        .title("Single Point in Zero Dimensions")
        .subtitle("Perfect for KPI or aggregate value display")
        .data(df)
        .mark(
            Symbol::new()
                .size(400.0)
                .fill("#3498db")
                .stroke("#2980b9")
                .stroke_width(3.0)
                .shape("square"),
        );

    assert_visual_match_default(plot, "zerod", "symbol_single_point").await;
}

#[tokio::test]
async fn test_zerod_symbol_shapes() {
    // Create data with different shapes
    let shapes = StringArray::from(vec!["circle", "square", "triangle", "diamond", "cross"]);
    let labels = StringArray::from(vec!["Circle", "Square", "Triangle", "Diamond", "Cross"]);
    let sizes = Float64Array::from(vec![150.0, 150.0, 150.0, 150.0, 150.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("shape", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("size", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(shapes), Arc::new(labels), Arc::new(sizes)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    // Different shapes all at center - useful for showing shape legend
    let plot = Plot::<ZeroDCoord>::new()
        .title("Shape Gallery in Zero-D")
        .subtitle("All shapes rendered at the same position")
        .data(df)
        .mark(
            Symbol::new()
                .size_with(col("size"), |c| c.no_legend())
                .fill("#9b59b6")
                .stroke("#8e44ad")
                .stroke_width(2.0)
                .shape_with(col("shape"), |c| c.legend(|l| l.title("Shape Type"))),
        );

    assert_visual_match_default(plot, "zerod", "symbol_shapes").await;
}

#[tokio::test]
async fn test_zerod_symbol_varied_sizes() {
    let df = create_zerod_data();

    // ZeroD with varied sizes
    let plot = Plot::<ZeroDCoord>::new()
        .title("ZeroD with Size Encoding")
        .subtitle("Different sizes at the same position")
        .data(df)
        .mark(
            Symbol::new()
                .size_with(col("value") * lit(10.0), |c| c.no_legend())
                .fill("#27ae60")
                .stroke("#1e8449")
                .stroke_width(1.5)
                .shape("circle"),
        );

    assert_visual_match_default(plot, "zerod", "symbol_varied_sizes").await;
}
