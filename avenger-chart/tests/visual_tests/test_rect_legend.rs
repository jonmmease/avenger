use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;

use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::scales::Band;
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
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| legend.title("Category"))
        .mark(
            Rect::new()
                .x_with(col("product"), |c| {
                    c.scale_with::<Band>(|scale| scale.padding_inner(0.1))
                        .band(0.0)
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| c.scale(|scale| scale.domain((0.0, 60.0))))
                .y2_with(col("value"), |c| c)
                .fill_with(col("category"), |c| c)
                .stroke_with(lit("#333333"), |c| c.no_scale())
                .stroke_width_with(lit(1.0), |c| c.no_scale()),
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
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| legend.title("Temperature (°C)"))
        .mark(
            Rect::new()
                .x_with(col("quarter"), |c| {
                    c.scale_with::<Band>(|scale| scale.padding_inner(0.15))
                        .band(0.0)
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| c.scale(|scale| scale.domain((0.0, 70.0))))
                .y2_with(col("sales"), |c| c)
                .fill_with(col("temperature"), |c| {
                    c.scale(|scale| scale.domain((0.0, 40.0)))
                })
                .stroke_with(lit("#000000"), |c| c.no_scale())
                .stroke_width_with(lit(0.5), |c| c.no_scale()),
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
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("stroke", |legend| legend.title("Quality Tier"))
        .mark(
            Rect::new()
                .x_with(col("product"), |c| {
                    c.scale_with::<Band>(|scale| scale.padding_inner(0.1))
                        .band(0.0)
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| c.scale(|scale| scale.domain((0.0, 60.0))))
                .y2_with(col("value"), |c| c)
                .fill_with(lit("#1f77b4"), |c| c.no_scale())
                .stroke_with(col("quality"), |c| {
                    c.scale(|scale| {
                        scale
                            .range_discrete(vec!["#d62728", "#2ca02c", "#ff7f0e"])
                            .domain(vec![lit("Premium"), lit("Standard"), lit("Budget")])
                    })
                })
                .stroke_width_with(lit(3.0), |c| c.no_scale()),
        );

    assert_visual_match_default(plot, "legend", "rect_stroke_legend").await;
}
