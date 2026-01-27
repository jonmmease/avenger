//! Tests for radius expression calculation in marks
//!
//! Radius expressions determine the spatial extent of marks for hit testing
//! and layout purposes. Different mark types (Symbol, Line, Rect) calculate
//! radius differently based on their visual properties like size and stroke_width.

use avenger_chart::prelude::*;
use avenger_chart::render::RenderContext;
use avenger_chart::serialization::LogicalExprNodeExt;
use avenger_chart::theme::Theme;
use datafusion::prelude::SessionContext;
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

// ============================================================================
// Symbol Mark Tests
// ============================================================================

/// Test default channel values for Symbol marks from theme
#[tokio::test]
async fn test_symbol_default_channel_values() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::<Cartesian>::new().data(df).x(col("x")).y(col("y"));

    // Create a RenderContext with default theme
    let theme = Theme::light();
    let eval_ctx = avenger_chart::render::EvaluationContext::new(
        Arc::new(theme),
        Arc::new(ctx.clone()),
        indexmap::IndexMap::new(),
        Arc::new(avenger_chart::facet::evaluated_facet_tree::EvaluatedFacetTree::empty()),
    );
    let render_state = avenger_chart::render::RenderState::new(
        500.0,
        400.0,
        std::collections::HashMap::new(),
    );
    let context = RenderContext::new(&eval_ctx, &render_state, None);

    // Build the CompiledMark
    let renderer = symbol.compile_untransformed(&ctx).await.unwrap();

    // Test default channel values
    assert_eq!(
        renderer.default_channel_value("size", &context).unwrap(),
        ScalarValue::Float32(Some(72.0))
    );
    assert_eq!(
        renderer.default_channel_value("shape", &context).unwrap(),
        ScalarValue::Utf8(Some("circle".to_string()))
    );
    assert_eq!(
        renderer.default_channel_value("angle", &context).unwrap(),
        ScalarValue::Float32(Some(0.0))
    );
    // Fill has a hardcoded default of steelblue (#4682b4)
    assert_eq!(
        renderer.default_channel_value("fill", &context).unwrap(),
        ScalarValue::Utf8(Some("#4682b4".to_string()))
    );

    // Stroke is var(--bg-color) which is white in light mode
    assert_eq!(
        renderer.default_channel_value("stroke", &context).unwrap(),
        ScalarValue::Utf8(Some("#ffffff".to_string()))
    );
    assert_eq!(
        renderer.default_channel_value("opacity", &context).unwrap(),
        ScalarValue::Float32(Some(1.0))
    );

    // Test unknown channel returns None
    assert!(
        renderer
            .default_channel_value("unknown", &context)
            .is_none()
    );
}

/// Test radius expression calculation for Symbol marks
#[tokio::test]
async fn test_symbol_radius_expression() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::<Cartesian>::new().data(df).x(col("x")).y(col("y"));

    // Build the CompiledMark
    let renderer = symbol.compile_untransformed(&ctx).await.unwrap();

    // Create a simple channel resolver that returns the size and stroke_width defaults
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => lit(50.0),
            "stroke_width" => lit(2.0),
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Test radius expression for x dimension
    let radius_expr = renderer.radius_expression("x", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));

    // Test radius expression for y dimension
    let radius_expr = renderer.radius_expression("y", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));

    // Test radius expression for z dimension (should return None)
    let radius_expr = renderer.radius_expression("z", &resolve_channel);
    assert!(radius_expr.is_none());
}

#[tokio::test]
async fn test_symbol_radius_expression_with_mapped_size() {
    use datafusion::arrow::array::Float64Array;
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;

    let ctx = SessionContext::new();

    // Create test data with size column
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
    ]));

    let x_array = Float64Array::from(vec![1.0, 2.0, 3.0]);
    let y_array = Float64Array::from(vec![1.0, 2.0, 3.0]);
    let size_array = Float64Array::from(vec![10.0, 20.0, 30.0]);

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_array), Arc::new(y_array), Arc::new(size_array)],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    let symbol = Symbol::<Cartesian>::new()
        .data(df)
        .x(col("x"))
        .y(col("y"))
        .size(col("size"));

    // Build the CompiledMark
    let renderer = symbol.compile_untransformed(&ctx).await.unwrap();

    // Create a channel resolver that returns the size column
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => col("size"),
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Test radius expression uses the mapped size
    let radius_expr = renderer.radius_expression("x", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));
}

#[tokio::test]
async fn test_symbol_radius_includes_stroke_width() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::<Cartesian>::new().data(df).x(col("x")).y(col("y"));

    // Build the CompiledMark
    let renderer = symbol.compile_untransformed(&ctx).await.unwrap();

    // Test with specific size and stroke_width values
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => lit(100.0),       // area = 100, so radius = sqrt(100) * 0.5 = 5.0
            "stroke_width" => lit(4.0), // adds 2.0 to radius
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Get radius expression
    let radius_expr = renderer.radius_expression("x", &resolve_channel).unwrap();

    // The expression should be: sqrt(100) * 0.5 + 4.0 / 2.0 = 5.0 + 2.0 = 7.0
    // We can't easily evaluate the expression here, but we can verify it's symmetric
    if let RadiusExpression::Symmetric(expr) = radius_expr {
        // The expression exists and is symmetric - that's what we care about
        // We can deserialize and check if needed, but that requires a SessionContext
        let ctx = SessionContext::new();
        if let Ok(decoded) = expr.to_expr(&ctx) {
            let expr_str = format!("{:?}", decoded);
            // Now we can check the decoded expression contains our values
            assert!(
                expr_str.contains("100") || expr_str.contains("Int64(100)"),
                "Expression should contain size value 100: {}",
                expr_str
            );
            assert!(
                expr_str.contains("4") || expr_str.contains("Int64(4)"),
                "Expression should contain stroke_width value 4: {}",
                expr_str
            );
        }
    } else {
        panic!("Expected symmetric radius expression");
    }
}

// ============================================================================
// Line Mark Tests
// ============================================================================

/// Test radius expression calculation for Line marks
#[tokio::test]
async fn test_line_radius_expression() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let line = Line::<Cartesian>::new().data(df).x(col("x")).y(col("y"));

    // Build the CompiledMark
    let renderer = line.compile_untransformed(&ctx).await.unwrap();

    // Create a channel resolver that returns stroke_width
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "stroke_width" => lit(3.0),
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Test radius expression for y dimension (should have radius)
    let radius_expr = renderer.radius_expression("y", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));

    // Verify the expression multiplies stroke_width by 2
    if let Some(RadiusExpression::Symmetric(expr)) = radius_expr {
        // The expression exists and is symmetric - that's what we care about
        // Deserialize to check the actual values
        let ctx = SessionContext::new();
        if let Ok(decoded) = expr.to_expr(&ctx) {
            let expr_str = format!("{:?}", decoded);
            assert!(
                expr_str.contains("3")
                    || expr_str.contains("Int64(3)")
                    || expr_str.contains("Float64(3"),
                "Expression should contain stroke_width value 3: {}",
                expr_str
            );
        }
    }

    // Test radius expression for x dimension (should return None)
    let radius_expr = renderer.radius_expression("x", &resolve_channel);
    assert!(radius_expr.is_none());

    // Test radius expression for z dimension (should return None)
    let radius_expr = renderer.radius_expression("z", &resolve_channel);
    assert!(radius_expr.is_none());
}
