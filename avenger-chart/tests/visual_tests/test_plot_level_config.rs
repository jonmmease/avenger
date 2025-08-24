// Test file specifically for plot-level scale and legend configuration
// This ensures we maintain test coverage for the plot-level API

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_plot_level_scale_config() {
    // Test that plot-level scale configuration still works
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Using plot-level scale configuration
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale("x", |scale| scale.domain((0.0, 6.0)))
        .scale("y", |scale| scale.domain((0.0, 8.0)))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(50.0)
                .fill("#3498db"),
        );

    assert_visual_match_default(plot, "plot_level_config", "plot_level_scales").await;
}

#[tokio::test]
async fn test_plot_level_legend_config() {
    // Test that plot-level legend configuration still works
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Using plot-level legend configuration
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| legend.title("Plot-Level Category"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(100.0),
        );

    assert_visual_match_default(plot, "plot_level_config", "plot_level_legend").await;
}

#[tokio::test]
async fn test_plot_and_channel_level_mixed() {
    // Test that plot-level and channel-level configs can be mixed
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0]);
    let sizes = Float64Array::from(vec![10.0, 20.0, 30.0, 15.0, 25.0, 35.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size_val", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories),
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(sizes),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Mix plot-level and channel-level configuration
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale("x", |scale| scale.domain((0.0, 7.0))) // Plot-level
        .legend("fill", |legend| legend.title("Category (Plot)")) // Plot-level
        .mark(
            Symbol::new()
                .x(col("x")) // Uses plot-level scale
                .y(col("y").scale(|scale| scale.domain((0.0, 8.0)))) // Channel-level scale
                .fill(col("category")) // Uses plot-level legend
                .size(
                    col("size_val")
                        .scale(|scale| scale.domain((0.0, 40.0))) // Channel-level scale
                        .legend(|legend| legend.title("Size (Channel)")),
                ), // Channel-level legend
        );

    assert_visual_match_default(plot, "plot_level_config", "mixed_config").await;
}
