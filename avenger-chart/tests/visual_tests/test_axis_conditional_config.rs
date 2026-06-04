//! Visual test for conditional axis configuration using expressions
//!
//! This test demonstrates that axis configuration options can use DataFusion expressions
//! to dynamically change based on parameters.

use super::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{ArrayRef, Float64Array, StructArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::logical_expr::when;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

/// Create test data for axis configuration tests
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

fn tick_spacing_param() -> Param {
    Param::new(
        "y_tick_spacing",
        ScalarValue::Struct(Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("start", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![-2.0])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("step", DataType::Float64, false)),
                Arc::new(Float64Array::from(vec![2.0])) as ArrayRef,
            ),
        ]))),
    )
}

#[tokio::test]
async fn test_conditional_axis_grid() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);

    // Create a parameter to control grid visibility
    let show_grid_param = Param::new("show_grid", ScalarValue::Boolean(Some(true)));

    // Create plot with conditional grid (note: can pass Param directly, no need for .expr())
    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0)
        .title("Conditional Axis Grid")
        .data(df)
        .add_param(show_grid_param.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| a.grid(&show_grid_param).title("X Axis"))
                })
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: With grid enabled
    let mut params_grid_on = IndexMap::new();
    params_grid_on.insert("show_grid".to_string(), ScalarValue::Boolean(Some(true)));
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_grid_on),
        "axis_conditional",
        "grid_on",
    )
    .await;

    // Test 2: With grid disabled
    let mut params_grid_off = IndexMap::new();
    params_grid_off.insert("show_grid".to_string(), ScalarValue::Boolean(Some(false)));
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_grid_off),
        "axis_conditional",
        "grid_off",
    )
    .await;
}

#[tokio::test]
async fn test_conditional_axis_position() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);

    // Create a parameter to control axis position
    let axis_pos_param = Param::new("axis_pos", ScalarValue::Utf8(Some("bottom".to_string())));

    // Create CASE expression for axis position
    let position_expr = when(axis_pos_param.expr().eq(lit("top")), lit("top"))
        .otherwise(lit("bottom"))
        .unwrap();

    // Create plot with conditional axis position
    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0)
        .title("Conditional Axis Position")
        .data(df)
        .add_param(axis_pos_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| a.position(position_expr).grid(true).title("X Axis"))
                })
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: With axis at bottom
    let mut params_bottom = IndexMap::new();
    params_bottom.insert(
        "axis_pos".to_string(),
        ScalarValue::Utf8(Some("bottom".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_bottom),
        "axis_conditional",
        "position_bottom",
    )
    .await;

    // Test 2: With axis at top
    let mut params_top = IndexMap::new();
    params_top.insert(
        "axis_pos".to_string(),
        ScalarValue::Utf8(Some("top".to_string())),
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_top),
        "axis_conditional",
        "position_top",
    )
    .await;
}

#[tokio::test]
async fn test_conditional_axis_visibility() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);

    // Create a parameter to control axis visibility
    let show_axis_param = Param::new("show_x_axis", ScalarValue::Boolean(Some(true)));

    // Create plot with conditional axis visibility (note: can pass Param directly)
    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0)
        .title("Conditional Axis Visibility")
        .data(df)
        .add_param(show_axis_param.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| a.visible(&show_axis_param).grid(true).title("X Axis"))
                })
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        );

    // Compile once
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");

    // Test 1: With axis visible
    let mut params_visible = IndexMap::new();
    params_visible.insert("show_x_axis".to_string(), ScalarValue::Boolean(Some(true)));
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_visible),
        "axis_conditional",
        "visible_true",
    )
    .await;

    // Test 2: With axis hidden
    let mut params_hidden = IndexMap::new();
    params_hidden.insert("show_x_axis".to_string(), ScalarValue::Boolean(Some(false)));
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(params_hidden),
        "axis_conditional",
        "visible_false",
    )
    .await;
}

#[tokio::test]
async fn test_axis_tick_spacing_start_step() {
    let ctx = SessionContext::new();
    let df = create_test_data(&ctx);
    let y_tick_spacing = tick_spacing_param();

    let plot = Plot::<Cartesian>::new()
        .canvas_size(520.0, 360.0)
        .title("Axis Tick Spacing")
        .data(df)
        .add_param(y_tick_spacing.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)).nice(false).zero(false))
                        .axis(|a| {
                            a.title("ticks_start_step: 0 + 2.5n")
                                .ticks_start_step(0.0, 2.5)
                        })
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((-3.0, 9.0)).nice(false).zero(false))
                        .axis(|a| a.title("tick_spacing param").tick_spacing(&y_tick_spacing))
                })
                .fill("#2563eb")
                .stroke("#0f172a")
                .stroke_width(1.0)
                .size(120.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_conditional",
        "tick_spacing_start_step",
    )
    .await;
}
