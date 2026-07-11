use avenger_chart::prelude::*;
// Test file for scale and legend configuration
// Updated to use channel-level API instead of removed plot-level API

use crate::visual_tests::helpers::assert_visual_match_default;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_channel_level_scale_config() {
    // Test channel-level scale configuration
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Using channel-level scale configuration
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 8.0))))
            .size(50.0)
            .fill_with("#3498db", |c| c.no_scale()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "plot_level_config",
        "plot_level_scales",
    )
    .await;
}

#[tokio::test]
async fn test_channel_level_legend_config() {
    // Test channel-level legend configuration
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

    // Using channel-level legend configuration
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c)
            .y_with(col("y"), |c| c)
            .fill_with(col("category"), |c| {
                c.legend(|legend| legend.title("Channel-Level Category"))
            })
            .size(100.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "plot_level_config",
        "plot_level_legend",
    )
    .await;
}

#[tokio::test]
async fn test_channel_level_mixed_config() {
    // Test multiple channel-level configurations
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

    // All configuration is now at channel-level
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|scale| scale.domain((0.0, 7.0)))) // Channel-level
            .y_with(col("y"), |c| c.scale(|scale| scale.domain((0.0, 8.0)))) // Channel-level
            .fill_with(col("category"), |c| {
                c.legend(|legend| legend.title("Category"))
            }) // Channel-level
            .size_with(col("size_val"), |c| {
                c.scale(|scale| scale.domain((0.0, 40.0))) // Channel-level scale
                    .legend(|legend| legend.title("Size")) // Channel-level legend
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "plot_level_config", "mixed_config").await;
}
