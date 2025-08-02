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

    // Create a simple channel resolver that returns the size default
    let resolve_channel = |channel: &str| -> datafusion::logical_expr::Expr {
        match channel {
            "size" => lit(50.0),
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
