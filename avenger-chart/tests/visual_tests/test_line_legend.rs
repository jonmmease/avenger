//! Visual tests for line mark legends

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::line::Line;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_line_discrete_stroke_legend() {
    // Create test data for multi-series line chart
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Series A
        1.0, 2.0, 3.0, 4.0, 5.0, // Series B
        1.0, 2.0, 3.0, 4.0, 5.0, // Series C
    ]);

    let y_values = Float32Array::from(vec![
        10.0, 20.0, 15.0, 25.0, 30.0, // Series A
        5.0, 15.0, 20.0, 18.0, 22.0, // Series B
        8.0, 12.0, 18.0, 20.0, 26.0, // Series C
    ]);

    let series = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
    ]);

    let order = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Series A
        1.0, 2.0, 3.0, 4.0, 5.0, // Series B
        1.0, 2.0, 3.0, 4.0, 5.0, // Series C
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("order", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
            Arc::new(series) as ArrayRef,
            Arc::new(order) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a multi-series line chart with stroke legend
    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 6.0)))
        .scale_y(|scale| scale.domain((0.0, 35.0)))
        .legend_stroke(|legend| legend.title("Series"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .order(col("order")),
        );

    assert_visual_match_default(plot, "legend", "line_discrete_stroke_legend").await;
}

// Continuous stroke legend test removed - not supported yet

#[tokio::test]
async fn test_line_stroke_width_legend() {
    // Create test data with varying stroke widths
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Thin line
        1.0, 2.0, 3.0, 4.0, 5.0, // Medium line
        1.0, 2.0, 3.0, 4.0, 5.0, // Thick line
    ]);

    let y_values = Float32Array::from(vec![
        10.0, 12.0, 11.0, 13.0, 14.0, // Thin line
        7.0, 9.0, 8.0, 10.0, 11.0, // Medium line
        4.0, 6.0, 5.0, 7.0, 8.0, // Thick line
    ]);

    let importance = StringArray::from(vec![
        "Low", "Low", "Low", "Low", "Low", "Medium", "Medium", "Medium", "Medium", "Medium",
        "High", "High", "High", "High", "High",
    ]);

    let order = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("importance", DataType::Utf8, false),
        Field::new("order", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
            Arc::new(importance) as ArrayRef,
            Arc::new(order) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a line chart with stroke width legend
    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 6.0)))
        .scale_y(|scale| scale.domain((0.0, 15.0)))
        .scale_stroke_width(|scale| {
            scale
                .scale_type("ordinal")
                .range_discrete(vec![lit(1.0), lit(3.0), lit(6.0)])
                .domain(vec![lit("Low"), lit("Medium"), lit("High")])
        })
        .legend_stroke_width(|legend| legend.title("Importance"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(lit("#1f77b4").identity())
                .stroke_width(col("importance"))
                .order(col("order")),
        );

    assert_visual_match_default(plot, "legend", "line_stroke_width_legend").await;
}

#[tokio::test]
async fn test_line_stroke_dash_legend() {
    // Create test data with varying dash patterns
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Solid line
        1.0, 2.0, 3.0, 4.0, 5.0, // Dashed line
        1.0, 2.0, 3.0, 4.0, 5.0, // Dotted line
    ]);

    let y_values = Float32Array::from(vec![
        10.0, 14.0, 12.0, 16.0, 18.0, // Solid line
        8.0, 12.0, 10.0, 14.0, 16.0, // Dashed line
        6.0, 10.0, 8.0, 12.0, 14.0, // Dotted line
    ]);

    let line_type = StringArray::from(vec![
        "Actual",
        "Actual",
        "Actual",
        "Actual",
        "Actual",
        "Predicted",
        "Predicted",
        "Predicted",
        "Predicted",
        "Predicted",
        "Baseline",
        "Baseline",
        "Baseline",
        "Baseline",
        "Baseline",
    ]);

    let order = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("line_type", DataType::Utf8, false),
        Field::new("order", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
            Arc::new(line_type) as ArrayRef,
            Arc::new(order) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a line chart with stroke dash legend
    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 6.0)))
        .scale_y(|scale| scale.domain((0.0, 20.0)))
        .scale_stroke_dash(|scale| {
            scale
                .scale_type("ordinal")
                .range_discrete(vec![lit("solid"), lit("dashed"), lit("dotted")])
                .domain(vec![lit("Actual"), lit("Predicted"), lit("Baseline")])
        })
        .legend_stroke_dash(|legend| legend.title("Line Type"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(lit("#333333").identity())
                .stroke_dash(col("line_type"))
                .order(col("order")),
        );

    assert_visual_match_default(plot, "legend", "line_stroke_dash_legend").await;
}

#[tokio::test]
async fn test_line_combined_stroke_width_legend() {
    // Create test data where same column encodes both stroke and width
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Priority 1
        1.0, 2.0, 3.0, 4.0, 5.0, // Priority 2
        1.0, 2.0, 3.0, 4.0, 5.0, // Priority 3
    ]);

    let y_values = Float32Array::from(vec![
        15.0, 18.0, 16.0, 20.0, 22.0, // Priority 1
        10.0, 13.0, 11.0, 15.0, 17.0, // Priority 2
        5.0, 8.0, 6.0, 10.0, 12.0, // Priority 3
    ]);

    let priority = StringArray::from(vec![
        "High", "High", "High", "High", "High", "Medium", "Medium", "Medium", "Medium", "Medium",
        "Low", "Low", "Low", "Low", "Low",
    ]);

    let order = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("priority", DataType::Utf8, false),
        Field::new("order", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
            Arc::new(priority) as ArrayRef,
            Arc::new(order) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a line chart where priority encodes both stroke color and width
    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.domain((0.0, 6.0)))
        .scale_y(|scale| scale.domain((0.0, 25.0)))
        .scale_stroke(|scale| {
            scale
                .scale_type("ordinal")
                .range_discrete(vec![lit("#d62728"), lit("#ff7f0e"), lit("#2ca02c")])
                .domain(vec![lit("High"), lit("Medium"), lit("Low")])
        })
        .scale_stroke_width(|scale| {
            scale
                .scale_type("ordinal")
                .range_discrete(vec![lit(4.0), lit(2.5), lit(1.0)])
                .domain(vec![lit("High"), lit("Medium"), lit("Low")])
        })
        .legend_stroke(|legend| legend.title("Priority"))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("priority"))
                .stroke_width(col("priority"))
                .order(col("order")),
        );

    assert_visual_match_default(plot, "legend", "line_combined_stroke_width_legend").await;
}
