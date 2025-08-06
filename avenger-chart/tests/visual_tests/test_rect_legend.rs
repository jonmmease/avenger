use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_rect_discrete_fill_legend() {
    // Create test data for a simple bar chart
    let categories = StringArray::from(vec!["Product A", "Product B", "Product C", "Product D"]);
    let values = Float32Array::from(vec![45.0, 38.0, 52.0, 41.0]);
    let colors = StringArray::from(vec!["Category 1", "Category 2", "Category 1", "Category 3"]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("product", DataType::Utf8, false),
        Field::new("value", DataType::Float32, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories) as ArrayRef,
            Arc::new(values) as ArrayRef,
            Arc::new(colors) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a bar chart with fill legend
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("band").option("padding_inner", lit(0.1)))
        .scale_y(|scale| scale.domain((0.0, 60.0)))
        .legend_fill(|legend| legend.title("Category"))
        .mark(
            Rect::new()
                .x(col("product").band(0.0))
                .x2(col("product").band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill(col("category"))
                .stroke(lit("#333333").identity())
                .stroke_width(lit(1.0).identity()),
        );

    assert_visual_match_default(plot, "legend", "rect_discrete_fill_legend").await;
}

#[tokio::test]
async fn test_rect_continuous_fill_legend() {
    // Create test data with continuous values for color
    let categories = StringArray::from(vec!["Q1", "Q2", "Q3", "Q4"]);
    let values = Float32Array::from(vec![25.0, 45.0, 60.0, 35.0]);
    let temperatures = Float32Array::from(vec![10.0, 25.0, 35.0, 18.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("quarter", DataType::Utf8, false),
        Field::new("sales", DataType::Float32, false),
        Field::new("temperature", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories) as ArrayRef,
            Arc::new(values) as ArrayRef,
            Arc::new(temperatures) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a bar chart with continuous color legend
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("band").option("padding_inner", lit(0.15)))
        .scale_y(|scale| scale.domain((0.0, 70.0)))
        .scale_fill(|scale| scale.domain((0.0, 40.0)))
        .legend_fill(|legend| legend.title("Temperature (°C)"))
        .mark(
            Rect::new()
                .x(col("quarter").band(0.0))
                .x2(col("quarter").band(1.0))
                .y(lit(0.0))
                .y2(col("sales"))
                .fill(col("temperature"))
                .stroke(lit("#000000").identity())
                .stroke_width(lit(0.5).identity()),
        );

    assert_visual_match_default(plot, "legend", "rect_continuous_fill_legend").await;
}

#[tokio::test]
async fn test_rect_stroke_legend() {
    // Create test data with stroke categories
    let products = StringArray::from(vec!["Widget A", "Widget B", "Widget C", "Widget D"]);
    let values = Float32Array::from(vec![35.0, 42.0, 28.0, 51.0]);
    let stroke_categories = StringArray::from(vec!["Premium", "Standard", "Premium", "Budget"]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("product", DataType::Utf8, false),
        Field::new("value", DataType::Float32, false),
        Field::new("quality", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(products) as ArrayRef,
            Arc::new(values) as ArrayRef,
            Arc::new(stroke_categories) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a bar chart with stroke legend
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("band").option("padding_inner", lit(0.1)))
        .scale_y(|scale| scale.domain((0.0, 60.0)))
        .scale_stroke(|scale| {
            scale
                .range_discrete(vec![lit("#d62728"), lit("#2ca02c"), lit("#ff7f0e")])
                .domain(vec![lit("Premium"), lit("Standard"), lit("Budget")])
        })
        .legend_stroke(|legend| legend.title("Quality Tier"))
        .mark(
            Rect::new()
                .x(col("product").band(0.0))
                .x2(col("product").band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill(lit("#1f77b4").identity())
                .stroke(col("quality"))
                .stroke_width(lit(3.0).identity()),
        );

    assert_visual_match_default(plot, "legend", "rect_stroke_legend").await;
}
