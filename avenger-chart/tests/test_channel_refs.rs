//! Test channel reference functionality

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::Mark;
use avenger_chart::marks::line::Line;
use datafusion::logical_expr::ident;
use datafusion::prelude::*;

#[test]
fn test_channel_reference_basic() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with channel references
    let mark = Line::<Cartesian>::new().data(df).x(ident("month")).y(":x"); // This should reference the :x channel

    // Get the mark config to inspect
    let config = mark.into_config();

    // Verify the data context has the :x column
    let data_ctx = &config.data;

    // Check that encoding was tracked
    assert_eq!(data_ctx.encoding("x"), Some("month".to_string()));
    // With the new API, channel references stay as ":y" until render time
    assert_eq!(data_ctx.encoding("y"), Some(":y".to_string())); // Channel ref stored as-is

    // Check that expression was stored and resolved
    assert_eq!(
        data_ctx.encoding_expr_string("x"),
        Some("month".to_string())
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

    // Get the mark config to inspect
    let config = mark.into_config();

    // Verify the data context
    let data_ctx = &config.data;

    // Check encodings
    assert_eq!(data_ctx.encoding("x"), Some("month".to_string()));
    assert_eq!(data_ctx.encoding("y"), Some(":y".to_string())); // Complex expr gets channel name
    assert_eq!(data_ctx.encoding("stroke"), Some(":stroke".to_string())); // stroke is complex expr (resolved :y)

    // Check expression strings
    assert_eq!(
        data_ctx.encoding_expr_string("x"),
        Some("month".to_string())
    );
    assert!(data_ctx.encoding_expr_string("y").unwrap().contains("sum"));
    assert!(
        data_ctx
            .encoding_expr_string("stroke")
            .unwrap()
            .contains("sum")
    ); // :y resolved to sum(sales)
}
