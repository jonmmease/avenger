use arrow::array::Float64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use avenger_chart::prelude::*;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

/// Create test data with continuous values for colorbar
fn create_colorbar_data() -> RecordBatch {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0, 11.0]);
    let temp_values = Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 28.0, 32.0, 35.0, 38.0, 42.0, 45.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(temp_values)],
    )
    .unwrap()
}

#[tokio::test]
async fn test_colorbar_right_position() {
    let ctx = SessionContext::new();
    let batch = create_colorbar_data();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("temperature"), |c| {
                    c.scale(|s| s)
                        .legend(|l| l.title("Temperature (°C)").position(LegendPosition::Right))
                })
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_positions",
        "colorbar_right_position",
    )
    .await;
}

#[tokio::test]
async fn test_colorbar_left_position() {
    let ctx = SessionContext::new();
    let batch = create_colorbar_data();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("temperature"), |c| {
                    c.scale(|s| s)
                        .legend(|l| l.title("Temperature (°C)").position(LegendPosition::Left))
                })
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_positions",
        "colorbar_left_position",
    )
    .await;
}

#[tokio::test]
async fn test_colorbar_top_position() {
    let ctx = SessionContext::new();
    let batch = create_colorbar_data();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("temperature"), |c| {
                    c.scale(|s| s)
                        .legend(|l| l.title("Temperature (°C)").position(LegendPosition::Top))
                })
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_positions",
        "colorbar_top_position",
    )
    .await;
}

#[tokio::test]
async fn test_colorbar_bottom_position() {
    let ctx = SessionContext::new();
    let batch = create_colorbar_data();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("temperature"), |c| {
                    c.scale(|s| s)
                        .legend(|l| l.title("Temperature (°C)").position(LegendPosition::Bottom))
                })
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_positions",
        "colorbar_bottom_position",
    )
    .await;
}

#[tokio::test]
async fn test_colorbar_with_background() {
    // Test colorbar with background styling at different positions
    let ctx = SessionContext::new();
    let batch = create_colorbar_data();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("temperature"), |c| {
                    c.scale(|s| s)
                        .legend(|l| {
                            l.title("Temp (°C)")
                                .position(LegendPosition::Bottom)
                                .background_fill("#f0f0f0")
                                .background_stroke("#333333")
                                .background_padding(8.0)
                                .background_corner_radius(4.0)
                        })
                })
                .size(80.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_positions",
        "colorbar_with_background",
    )
    .await;
}
