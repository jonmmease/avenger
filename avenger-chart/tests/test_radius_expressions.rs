use avenger_chart::marks::symbol::Symbol;
use avenger_chart::marks::{Mark, RadiusExpression};
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::SessionContext;
use std::sync::Arc;

#[tokio::test]
async fn test_symbol_default_channel_values() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::new().data(df).x(col("x")).y(col("y"));

    // Test default channel values
    assert_eq!(symbol.default_channel_value("size").unwrap(), lit(64.0));
    assert_eq!(
        symbol.default_channel_value("shape").unwrap(),
        lit("circle")
    );
    assert_eq!(symbol.default_channel_value("angle").unwrap(), lit(0.0));
    assert_eq!(
        symbol.default_channel_value("fill").unwrap(),
        lit("#4682b4")
    );
    assert_eq!(
        symbol.default_channel_value("stroke").unwrap(),
        lit("#000000")
    );
    assert_eq!(symbol.default_channel_value("opacity").unwrap(), lit(1.0));

    // Test unknown channel returns None
    assert!(symbol.default_channel_value("unknown").is_none());
}

#[tokio::test]
async fn test_symbol_radius_expression() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::new().data(df).x(col("x")).y(col("y"));

    // Create a simple channel resolver that returns the size and stroke_width defaults
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => lit(50.0),
            "stroke_width" => lit(2.0),
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Test radius expression for x dimension
    let radius_expr = symbol.radius_expression("x", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));

    // Test radius expression for y dimension
    let radius_expr = symbol.radius_expression("y", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));

    // Test radius expression for z dimension (should return None)
    let radius_expr = symbol.radius_expression("z", &resolve_channel);
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

    let symbol = Symbol::new()
        .data(df)
        .x(col("x"))
        .y(col("y"))
        .size(col("size"));

    // Create a channel resolver that returns the size column
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => col("size"),
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Test radius expression uses the mapped size
    let radius_expr = symbol.radius_expression("x", &resolve_channel);
    assert!(matches!(radius_expr, Some(RadiusExpression::Symmetric(_))));
}

#[tokio::test]
async fn test_symbol_radius_includes_stroke_width() {
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    let symbol = Symbol::new().data(df).x(col("x")).y(col("y"));

    // Test with specific size and stroke_width values
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => lit(100.0),       // area = 100, so radius = sqrt(100) * 0.5 = 5.0
            "stroke_width" => lit(4.0), // adds 2.0 to radius
            _ => lit(datafusion::scalar::ScalarValue::Null),
        }
    };

    // Get radius expression
    let radius_expr = symbol.radius_expression("x", &resolve_channel).unwrap();

    // The expression should be: sqrt(100) * 0.5 + 4.0 / 2.0 = 5.0 + 2.0 = 7.0
    // We can't easily evaluate the expression here, but we can verify it includes both components
    if let RadiusExpression::Symmetric(expr) = radius_expr {
        // Convert to string to check the expression includes both size and stroke_width
        let expr_str = format!("{:?}", expr);

        // The expression should contain references to both values
        assert!(expr_str.contains("100")); // our size value
        assert!(expr_str.contains("4")); // our stroke_width value
        assert!(expr_str.contains("0.5")); // the size multiplier
        assert!(expr_str.contains("2")); // the stroke_width divisor
    } else {
        panic!("Expected symmetric radius expression");
    }
}
