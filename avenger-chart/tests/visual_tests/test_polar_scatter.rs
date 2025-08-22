//! Visual tests for polar scatter plots

use super::helpers::assert_visual_match_default;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::polar::Polar;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::col;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a simple polar scatter plot dataset
fn create_polar_data() -> DataFrame {
    use datafusion::arrow::array::StringArray;

    // Create some sample data with radius and angle values
    let radius_values = Float64Array::from(vec![
        30.0, 45.0, 60.0, 75.0, 90.0, 40.0, 55.0, 70.0, 85.0, 100.0, 35.0, 50.0, 65.0, 80.0, 95.0,
    ]);

    // Angles in radians (spread around the circle)
    let theta_values = Float64Array::from(vec![
        0.0, 0.4, 0.8, 1.2, 1.6, 2.0, 2.4, 2.8, 3.2, 3.6, 4.0, 4.4, 4.8, 5.2, 5.6,
    ]);

    // Categories for coloring
    let category_values = StringArray::from(vec![
        "Type A", "Type A", "Type A", "Type A", "Type A", "Type B", "Type B", "Type B", "Type B",
        "Type B", "Type C", "Type C", "Type C", "Type C", "Type C",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("radius", DataType::Float64, false),
        Field::new("theta", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(radius_values),
            Arc::new(theta_values),
            Arc::new(category_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

#[tokio::test]
async fn test_polar_scatter_plot() {
    let df = create_polar_data();

    let plot = Plot::new(Polar::new())
        .data(df)
        .title("Polar Scatter Plot")
        .subtitle("Demonstrating categorical colors and legends")
        .scale_r(|scale| {
            use datafusion::logical_expr::lit;
            scale
                .scale_type("linear")
                .domain(avenger_chart::scales::ScaleDomain::new_interval(
                    lit(0.0),
                    lit(120.0),
                ))
        })
        .scale_theta(|scale| scale.scale_type("linear"))
        .scale_fill(|scale| scale.scale_type("ordinal"))
        .axis_r(|axis| axis.tick_count(6))
        .axis_theta(|axis| axis.visible(true))
        .legend_fill(|legend| legend.title("Category"))
        .mark(
            Symbol::new()
                .r(col("radius"))
                .theta(col("theta"))
                .size(120.0)
                .fill(col("category"))
                .stroke("#333333")
                .stroke_width(1.0),
        );

    assert_visual_match_default(plot, "polar", "scatter_plot").await;
}

#[tokio::test]
async fn test_polar_scatter_with_clipping() {
    use datafusion::arrow::array::StringArray;

    // Create data with radius values that will exceed the scale domain
    // Some points will be at 110-120 radius, but we'll set scale max to 80
    let radius_values = Float64Array::from(vec![
        20.0, 40.0, 60.0, 75.0, 80.0, // Within bounds
        85.0, 90.0, 95.0, 100.0, 110.0, 120.0, // Will be clipped
        30.0, 50.0, 70.0, 75.0, // More within bounds
        88.0, 92.0, 105.0, 115.0, 125.0, // More clipped
    ]);

    // Spread angles around the circle
    let theta_values = Float64Array::from(vec![
        0.0, 0.3, 0.6, 0.9, 1.2, 1.5, 1.8, 2.1, 2.4, 2.7, 3.0, 3.3, 3.6, 3.9, 4.2, 4.5, 4.8, 5.1,
        5.4, 5.7,
    ]);

    // Categories for visual distinction
    let category_values = StringArray::from(vec![
        "Within", "Within", "Within", "Within", "Within", "Clipped", "Clipped", "Clipped",
        "Clipped", "Clipped", "Clipped", "Within", "Within", "Within", "Within", "Clipped",
        "Clipped", "Clipped", "Clipped", "Clipped",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("radius", DataType::Float64, false),
        Field::new("theta", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(radius_values),
            Arc::new(theta_values),
            Arc::new(category_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Polar::new())
        .data(df)
        .title("Polar Plot with Clipping")
        .subtitle("Points beyond r=80 are clipped at the boundary")
        .scale_r(|scale| {
            use datafusion::logical_expr::lit;
            scale
                .scale_type("linear")
                .domain(avenger_chart::scales::ScaleDomain::new_interval(
                    lit(0.0),
                    lit(80.0), // Set max to 80, but data goes to 125
                ))
        })
        .scale_theta(|scale| scale.scale_type("linear"))
        .scale_fill(|scale| {
            use palette::Srgba;
            scale
                .scale_type("ordinal")
                .range(avenger_chart::scales::ScaleRange::Color(vec![
                    Srgba::new(0.2, 0.6, 1.0, 1.0), // Blue for within bounds
                    Srgba::new(1.0, 0.4, 0.2, 1.0), // Red/orange for clipped
                ]))
        })
        .axis_r(|axis| axis.tick_count(5).title("Radius (max: 80)"))
        .axis_theta(|axis| axis.visible(true))
        .legend_fill(|legend| legend.title("Data Range"))
        .mark(
            Symbol::new()
                .r(col("radius"))
                .theta(col("theta"))
                .size(200.0) // Large symbols to make clipping obvious
                .fill(col("category"))
                .stroke("#000000")
                .stroke_width(2.0)
                .shape("circle"),
        );

    assert_visual_match_default(plot, "polar", "scatter_with_clipping").await;
}

#[tokio::test]
async fn test_polar_scatter_with_size_color() {
    // Create data with additional dimensions for size and color
    let radius_values = Float64Array::from(vec![
        30.0, 45.0, 60.0, 75.0, 90.0, 40.0, 55.0, 70.0, 85.0, 100.0, 35.0, 50.0, 65.0, 80.0, 95.0,
        25.0, 42.0, 58.0, 73.0, 88.0,
    ]);

    let theta_values = Float64Array::from(vec![
        0.0, 0.3, 0.6, 0.9, 1.2, 1.5, 1.8, 2.1, 2.4, 2.7, 3.0, 3.3, 3.6, 3.9, 4.2, 4.5, 4.8, 5.1,
        5.4, 5.7,
    ]);

    // Size values
    let size_values = Float64Array::from(vec![
        50.0, 100.0, 150.0, 200.0, 250.0, 75.0, 125.0, 175.0, 225.0, 275.0, 60.0, 110.0, 160.0,
        210.0, 260.0, 80.0, 130.0, 180.0, 230.0, 280.0,
    ]);

    // Color values (continuous)
    let color_values = Float64Array::from(vec![
        0.0, 1.0, 2.0, 3.0, 4.0, 0.5, 1.5, 2.5, 3.5, 4.5, 0.2, 1.2, 2.2, 3.2, 4.2, 0.8, 1.8, 2.8,
        3.8, 4.8,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("radius", DataType::Float64, false),
        Field::new("theta", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
        Field::new("color", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(radius_values),
            Arc::new(theta_values),
            Arc::new(size_values),
            Arc::new(color_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Polar::new())
        .data(df)
        .scale_r(|scale| {
            use datafusion::logical_expr::lit;
            scale
                .scale_type("linear")
                .domain(avenger_chart::scales::ScaleDomain::new_interval(
                    lit(0.0),
                    lit(120.0),
                ))
        })
        .scale_theta(|scale| scale.scale_type("linear"))
        .scale_size(|scale| {
            use datafusion::logical_expr::lit;
            scale
                .scale_type("linear")
                .range(avenger_chart::scales::ScaleRange::new_interval(
                    lit(50.0),
                    lit(300.0),
                ))
        })
        .scale_fill(|scale| {
            use palette::Srgba;
            scale
                .scale_type("linear")
                .range(avenger_chart::scales::ScaleRange::Color(vec![
                    Srgba::new(
                        0x44 as f32 / 255.0,
                        0x01 as f32 / 255.0,
                        0x54 as f32 / 255.0,
                        1.0,
                    ),
                    Srgba::new(
                        0x21 as f32 / 255.0,
                        0x90 as f32 / 255.0,
                        0x8c as f32 / 255.0,
                        1.0,
                    ),
                    Srgba::new(
                        0xfd as f32 / 255.0,
                        0xe7 as f32 / 255.0,
                        0x25 as f32 / 255.0,
                        1.0,
                    ),
                ]))
        })
        .axis_r(|axis| axis.title("Radius").tick_count(6))
        .axis_theta(|axis| axis.title("Angle"))
        .legend_size(|legend| legend.title("Size"))
        .legend_fill(|legend| legend.title("Color Value"))
        .mark(
            Symbol::new()
                .r(col("radius"))
                .theta(col("theta"))
                .size(col("size"))
                .fill(col("color"))
                .stroke("#333333")
                .stroke_width(0.5),
        );

    assert_visual_match_default(plot, "polar", "scatter_size_color").await;
}
