use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;

use avenger_chart::marks::line::Line;
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::scales::{Band, Ordinal};
use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

/// Test symbol legend with title
#[tokio::test]
async fn test_symbol_legend_with_title() {
    // Create sample data with categories
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let x_data = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_data = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0]);
    let category_data = StringArray::from(vec!["A", "B", "A", "B", "C", "C"]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(x_data), Arc::new(y_data), Arc::new(category_data)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 7.0)))
        .scale_y(|s| s.domain((0.0, 7.0)))
        ._scale_with::<Ordinal>("fill", |s| s)
        .legend("fill", |legend| legend.title("Category"))
        .mark(Symbol::new().x(col("x")).y(col("y")).fill(col("category")));

    assert_visual_match_default(plot, "legend", "symbol_legend_with_title").await;
}

/// Test line legend with title
#[tokio::test]
async fn test_line_legend_with_title() {
    // Create sample data with series
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("series", DataType::Utf8, false),
    ]));

    let x_data = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0]);
    let y_data = Float32Array::from(vec![2.0, 3.5, 3.0, 4.5, 1.5, 2.8, 3.8, 3.2]);
    let series_data = StringArray::from(vec![
        "Series A", "Series A", "Series A", "Series A", "Series B", "Series B", "Series B",
        "Series B",
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(x_data), Arc::new(y_data), Arc::new(series_data)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.5, 4.5)))
        .scale_y(|s| s.domain((0.0, 5.0)))
        ._scale_with::<Ordinal>("stroke", |s| s)
        .legend("stroke", |legend| legend.title("Line Series"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "legend", "line_legend_with_title").await;
}

/// Test rect stroke legend with title (should use symbol legend)
#[tokio::test]
async fn test_rect_stroke_legend_with_title() {
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
        .scale_x_with::<Band>(|scale| scale.padding_inner(0.1))
        .scale_y(|scale| scale.domain((0.0, 60.0)))
        ._scale("stroke", |scale| {
            scale
                .range_discrete(vec!["#d62728", "#2ca02c", "#ff7f0e"])
                .domain(vec![lit("Premium"), lit("Standard"), lit("Budget")])
        })
        .legend("stroke", |legend| legend.title("Quality Tier"))
        .mark(
            Rect::new()
                .x_with(col("product"), |c| c.band(0.0))
                .x2_with(col(":x"), |c| c.band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill_with(lit("#1f77b4"), |c| c.no_scale())
                .stroke(col("quality"))
                .stroke_width_with(lit(3.0), |c| c.no_scale()),
        );

    assert_visual_match_default(plot, "legend", "rect_stroke_legend_with_title").await;
}

/// Test shape legend with title
#[tokio::test]
async fn test_shape_legend_with_title() {
    // Create sample data with shapes
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("shape_type", DataType::Utf8, false),
    ]));

    let x_data = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_data = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0]);
    let shape_data = StringArray::from(vec![
        "circle", "square", "triangle", "circle", "square", "triangle",
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(x_data), Arc::new(y_data), Arc::new(shape_data)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 7.0)))
        .scale_y(|s| s.domain((0.0, 7.0)))
        ._scale_with::<Ordinal>("shape", |s| s)
        .legend("shape", |legend| legend.title("Shape Type"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("shape_type"))
                .fill_with(lit("#1f77b4"), |c| c.no_scale()),
        );

    assert_visual_match_default(plot, "legend", "shape_legend_with_title").await;
}

/// Test legend with background and title
#[tokio::test]
async fn test_legend_with_title_and_background() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let x_data = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_data = Float32Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 1.0]);
    let category_data = StringArray::from(vec![
        "Type A", "Type B", "Type A", "Type B", "Type C", "Type C",
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(x_data), Arc::new(y_data), Arc::new(category_data)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 7.0)))
        .scale_y(|s| s.domain((0.0, 7.0)))
        ._scale_with::<Ordinal>("fill", |s| s)
        .legend("fill", |legend| {
            legend
                .title("Data Categories")
                .background_fill("#f0f0f0")
                .background_stroke("#888888")
                .background_corner_radius(4.0)
                .background_padding(8.0)
        })
        .mark(Symbol::new().x(col("x")).y(col("y")).fill(col("category")));

    assert_visual_match_default(plot, "legend", "legend_with_title_and_background").await;
}
