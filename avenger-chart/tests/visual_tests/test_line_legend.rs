use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;
// Visual tests for line mark legends

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
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 35.0))))
            .stroke_with(col("series"), |c| {
                c.scale(|s| s).legend(|legend| legend.title("Series"))
            })
            .order(col("order")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "legend",
        "line_discrete_stroke_legend",
    )
    .await;
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
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 15.0))))
            .stroke_with(lit("#1f77b4"), |c| c.no_scale())
            .stroke_width_with(col("importance"), |c| {
                c.scale_with(|scale: Scale<Ordinal>| {
                    scale.range_discrete(vec![1.0, 3.0, 6.0]).domain(vec![
                        lit("Low"),
                        lit("Medium"),
                        lit("High"),
                    ])
                })
                .legend(|legend| legend.title("Importance"))
            })
            .order(col("order")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "legend", "line_stroke_width_legend").await;
}

#[tokio::test]
async fn test_line_stroke_dash_legend() {
    // Create test data with all 8 dash patterns
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 1
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 2
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 3
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 4
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 5
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 6
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 7
        1.0, 2.0, 3.0, 4.0, 5.0, // Line 8
    ]);

    let y_values = Float32Array::from(vec![
        22.0, 23.0, 21.0, 24.0, 25.0, // Line 1 (Type A)
        19.0, 20.0, 18.0, 21.0, 22.0, // Line 2 (Type B)
        16.0, 17.0, 15.0, 18.0, 19.0, // Line 3 (Type C)
        13.0, 14.0, 12.0, 15.0, 16.0, // Line 4 (Type D)
        10.0, 11.0, 9.0, 12.0, 13.0, // Line 5 (Type E)
        7.0, 8.0, 6.0, 9.0, 10.0, // Line 6 (Type F)
        4.0, 5.0, 3.0, 6.0, 7.0, // Line 7 (Type G)
        1.0, 2.0, 0.0, 3.0, 4.0, // Line 8 (Type H)
    ]);

    let line_type = StringArray::from(vec![
        "Type A", "Type A", "Type A", "Type A", "Type A", "Type B", "Type B", "Type B", "Type B",
        "Type B", "Type C", "Type C", "Type C", "Type C", "Type C", "Type D", "Type D", "Type D",
        "Type D", "Type D", "Type E", "Type E", "Type E", "Type E", "Type E", "Type F", "Type F",
        "Type F", "Type F", "Type F", "Type G", "Type G", "Type G", "Type G", "Type G", "Type H",
        "Type H", "Type H", "Type H", "Type H",
    ]);

    let order = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0,
        4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0,
        5.0, // Type G
        1.0, 2.0, 3.0, 4.0, 5.0, // Type H
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

    // Create a line chart with both stroke color and dash pattern varying by line_type
    // This showcases the default Okabe-Ito color palette along with dash patterns
    let plot = Chart::<Cartesian>::new()
        .data(df)
        .title("Multi-Series Time Series Analysis")
        .subtitle("Eight distinct patterns with colorblind-safe palette")
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s)
                        .axis(|axis| axis.title("Sample Index").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s)
                        .axis(|axis| axis.title("Performance Metric (%)").grid(true))
                })
                .stroke(col("line_type"))
                .stroke_dash_with(col("line_type"), |c| {
                    c.scale(|s| s).legend(|legend| legend.title("Line Pattern"))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "legend", "line_stroke_dash_legend").await;
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
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 25.0))))
            .stroke_with(col("priority"), |c| {
                c.scale_with::<Ordinal>(|scale| {
                    scale
                        .range_discrete(vec!["#d62728", "#ff7f0e", "#2ca02c"])
                        .domain(vec![lit("High"), lit("Medium"), lit("Low")])
                })
                .legend(|legend| legend.title("Priority"))
            })
            .stroke_width_with(col("priority"), |c| {
                c.scale_with(|scale: Scale<Ordinal>| {
                    scale.range_discrete(vec![4.0, 2.5, 1.0]).domain(vec![
                        lit("High"),
                        lit("Medium"),
                        lit("Low"),
                    ])
                })
            })
            .order(col("order")),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "legend",
        "line_combined_stroke_width_legend",
    )
    .await;
}
