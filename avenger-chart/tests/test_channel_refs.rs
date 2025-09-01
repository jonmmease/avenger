//! Test channel reference functionality

use avenger_chart::prelude::*;
use datafusion::logical_expr::ident;
use datafusion::prelude::*;

#[test]
fn test_channel_reference_basic() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with channel references
    let mark = Line::<Cartesian>::new().data(df).x(ident("month")).y(":x"); // This should reference the :x channel

    let data_ctx = mark.data_context();

    // Check that encoding was tracked
    assert_eq!(data_ctx.encoding("x"), Some("month".to_string()));
    // When using a channel reference like ":x", it's not a simple column name
    assert_eq!(data_ctx.encoding("y"), None); // Channel ref is not a simple column

    // Check that expression was stored
    // The encoding_expr_string returns a debug representation of the Expr
    assert!(
        data_ctx
            .encoding_expr_string("x")
            .unwrap()
            .contains("month")
    );
    // The expression string will show the channel reference
    assert!(data_ctx.encoding_expr_string("y").unwrap().contains(":x")); // Shows the channel ref
}

#[test]
fn test_channel_reference_with_expression() {
    use datafusion::functions_aggregate::expr_fn::sum;

    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with an expression
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month"))
        .y(sum(col("sales"))) // Expression
        .stroke(col(":y")); // Reference the y channel

    let data_ctx = mark.data_context();

    // Check encodings
    assert_eq!(data_ctx.encoding("x"), Some("month".to_string()));
    assert_eq!(data_ctx.encoding("y"), None); // sum(col("sales")) is not a simple column reference
    assert_eq!(data_ctx.encoding("stroke"), Some(":y".to_string())); // col(":y") creates a column with name ":y"

    // Check expression strings contain the expected values
    assert!(
        data_ctx
            .encoding_expr_string("x")
            .unwrap()
            .contains("month")
    );
    // sum(col("sales")) should produce an expression string containing "SUM"
    assert!(
        data_ctx
            .encoding_expr_string("y")
            .unwrap()
            .to_uppercase()
            .contains("SUM")
    );
    assert!(
        data_ctx
            .encoding_expr_string("stroke")
            .unwrap()
            .contains(":y")
    ); // Channel reference stays as ":y" until render time
}
