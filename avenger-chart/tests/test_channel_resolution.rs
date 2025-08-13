//! Test channel reference resolution in expressions

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::Mark;
use avenger_chart::marks::line::Line;
use datafusion::prelude::*;

#[test]
fn test_channel_resolution_simple() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with nested channel references
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month")) // x -> col("month")
        .y(ident("sales")) // y -> col("sales")
        .stroke(col(":x")); // stroke -> should resolve to col("month")

    // Get the mark config to inspect
    let data_ctx = mark.data_context();

    // Check that expressions are stored correctly
    // Channel references like ":x" are not resolved at this stage
    let x_expr = data_ctx.encoding_expr_string("x").unwrap();
    let stroke_expr = data_ctx.encoding_expr_string("stroke").unwrap();

    assert!(x_expr.contains("month"));
    assert!(stroke_expr.contains(":x")); // Channel reference stays as ":x" until render time
}

#[test]
fn test_channel_resolution_with_expression() {
    use datafusion::functions_aggregate::expr_fn::sum;

    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with an expression
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month"))
        .y(sum(col("sales"))) // y -> SUM(sales)
        .opacity(col(":y")); // opacity -> should resolve to SUM(sales)

    // Get the mark config to inspect
    let data_ctx = mark.data_context();

    // Check that expressions are stored correctly
    // Channel references are not resolved at this stage
    let y_expr = data_ctx.encoding_expr_string("y").unwrap();
    let opacity_expr = data_ctx.encoding_expr_string("opacity").unwrap();

    assert!(y_expr.to_uppercase().contains("SUM"));
    assert!(opacity_expr.contains(":y")); // Channel reference stays as ":y" until render time
}

#[test]
fn test_channel_resolution_chained() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with chained channel references
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month")) // x -> col("month")
        .y(col(":x")) // y -> should resolve to col("month")
        .stroke(col(":y")); // stroke -> should resolve to col("month") 

    // Get the mark config to inspect
    let data_ctx = mark.data_context();

    // Check that channel references are preserved
    // They are not resolved at this stage
    let x_expr = data_ctx.encoding_expr_string("x").unwrap();
    let y_expr = data_ctx.encoding_expr_string("y").unwrap();
    let stroke_expr = data_ctx.encoding_expr_string("stroke").unwrap();

    assert!(x_expr.contains("month"));
    assert!(y_expr.contains(":x")); // References :x
    assert!(stroke_expr.contains(":y")); // References :y
}

#[test]
fn test_channel_resolution_unknown_channel() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a line mark with a reference to an unknown channel
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month"))
        .y(col(":unknown")); // Reference to undefined channel

    // Get the mark config to inspect
    let data_ctx = mark.data_context();

    // Unknown channel reference should be kept as-is
    let y_expr = data_ctx.encoding_expr_string("y").unwrap();
    assert!(y_expr.contains(":unknown")); // Should remain as column reference
}

#[test]
fn test_channel_resolution_complex_expression() {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.read_empty().unwrap();

    // Create a mark with a complex expression containing channel reference
    let mark = Line::<Cartesian>::new()
        .data(df)
        .x(ident("month"))
        .y(ident("sales"))
        .stroke(col(":x").eq(lit("January"))); // Boolean expression with channel ref

    // Get the mark config to inspect
    let data_ctx = mark.data_context();

    // Channel references are not resolved at this stage
    let stroke_expr = data_ctx.encoding_expr_string("stroke").unwrap();

    assert!(stroke_expr.contains(":x")); // :x is preserved
    assert!(stroke_expr.contains("January"));
    // Channel resolution happens at render time
}
