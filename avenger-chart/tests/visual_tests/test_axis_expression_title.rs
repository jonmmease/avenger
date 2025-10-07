//! Visual test for expression-based axis titles
//!
//! This test demonstrates that axis titles can use DataFusion expressions
//! to dynamically compute their text based on parameters.

use super::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

/// Create test data for axis expression tests
fn create_test_data(ctx: &SessionContext) -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    ctx.read_batch(batch).expect("Failed to read batch")
}

#[tokio::test]
async fn test_axis_title_with_parameter_expression() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);

    // Create a parameter for the axis unit
    let unit_param = Param::new("unit", ScalarValue::Utf8(Some("meters".to_string())));

    // Create CASE expression for dynamic x-axis title based on unit parameter
    let x_axis_title = when(unit_param.expr().eq(lit("meters")), lit("Distance (m)"))
        .when(unit_param.expr().eq(lit("feet")), lit("Distance (ft)"))
        .otherwise(lit("Distance"))
        .unwrap();

    // Create plot with expression-based axis title
    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0)
        .title("Axis Titles with Expressions")
        .data(df)
        .add_param(unit_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.grid(true).title(x_axis_title)))
                .y_with(col("y"), |c| {
                    c.axis(|a| a.grid(true).title("Value (units)"))
                }),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: Render with unit="meters"
    let mut params_meters = IndexMap::new();
    params_meters.insert(
        "unit".to_string(),
        ScalarValue::Utf8(Some("meters".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_meters),
        "axis_expression",
        "axis_title_meters",
    )
    .await;

    // Test 2: Render with unit="feet"
    let mut params_feet = IndexMap::new();
    params_feet.insert(
        "unit".to_string(),
        ScalarValue::Utf8(Some("feet".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_feet),
        "axis_expression",
        "axis_title_feet",
    )
    .await;

    // Test 3: Render with unit="unknown" (should use otherwise clause)
    let mut params_other = IndexMap::new();
    params_other.insert(
        "unit".to_string(),
        ScalarValue::Utf8(Some("km".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_other),
        "axis_expression",
        "axis_title_default",
    )
    .await;
}

#[tokio::test]
async fn test_axis_title_string_literal() {
    // Test that string literals still work (backward compatibility)
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0)
        .title("Axis with String Literal Title")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| a.grid(true).title("X Axis (literal)"))
                })
                .y_with(col("y"), |c| {
                    c.axis(|a| a.grid(true).title("Y Axis (literal)"))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_expression",
        "axis_title_string_literal",
    )
    .await;
}
