//! Diagnostic test for CASE expression titles without media queries
//!
//! This test isolates whether performance issues are caused by:
//! - CASE expression evaluation in titles
//! - Media queries in CSS themes
//! - The combination of both

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

#[tokio::test]
async fn test_case_expression_title_no_media_query() {
    let ctx = SessionContext::new();

    // Same test data as media query test
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");
    let df = ctx.read_batch(batch).expect("Failed to read batch");

    // Width param for CASE expression
    let width_param = Param::new("width", ScalarValue::Float32(Some(400.0)));

    // CASE expression for title (same as media query test)
    let title_expr = when(width_param.expr().lt(lit(600)), lit("Small Screen (400px)"))
        .when(
            width_param.expr().lt(lit(1200)),
            lit("Medium Screen (800px)"),
        )
        .otherwise(lit("Large Screen (1400px)"))
        .unwrap();

    let subtitle_expr = when(
        width_param.expr().lt(lit(600)),
        lit("Testing CASE expression without media queries (Small)"),
    )
    .when(
        width_param.expr().lt(lit(1200)),
        lit("Testing CASE expression without media queries (Medium)"),
    )
    .otherwise(lit("Testing CASE expression without media queries (Large)"))
    .unwrap();

    let plot = Plot::<Cartesian>::new()
        .canvas_size(400.0, 300.0) // Fixed size
        .title(title_expr)
        .subtitle(subtitle_expr)
        .data(df)
        .add_param(width_param)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.grid(true).title("X Axis")))
                .y_with(col("y"), |c| c.axis(|a| a.grid(true).title("Y Axis"))),
        );
    // NO theme with media queries - using default theme

    eprintln!("=== DIAGNOSTIC TEST: CASE expressions without media queries ===");
    eprintln!("Compiling...");
    let compile_start = std::time::Instant::now();
    let compiled = plot.compile(&ctx).await.expect("Failed to compile");
    let compile_duration = compile_start.elapsed();
    eprintln!("Compilation took: {:?}", compile_duration);

    // Render with param (small width)
    eprintln!("\nRendering with width=400...");
    let mut params_small = IndexMap::new();
    params_small.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));

    let evaluate_start = std::time::Instant::now();
    let _result = compiled
        .evaluate(&ctx, Some(params_small))
        .await
        .expect("Failed to evaluate");
    let evaluate_duration = evaluate_start.elapsed();
    eprintln!("Evaluate (400px) took: {:?}", evaluate_duration);

    // Render with different param (medium width)
    eprintln!("\nRendering with width=800...");
    let mut params_medium = IndexMap::new();
    params_medium.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));

    let evaluate_start = std::time::Instant::now();
    let _result = compiled
        .evaluate(&ctx, Some(params_medium))
        .await
        .expect("Failed to evaluate");
    let evaluate_duration = evaluate_start.elapsed();
    eprintln!("Evaluate (800px) took: {:?}", evaluate_duration);

    // Render with different param (large width)
    eprintln!("\nRendering with width=1400...");
    let mut params_large = IndexMap::new();
    params_large.insert("width".to_string(), ScalarValue::Float32(Some(1400.0)));

    let evaluate_start = std::time::Instant::now();
    let _result = compiled
        .evaluate(&ctx, Some(params_large))
        .await
        .expect("Failed to evaluate");
    let evaluate_duration = evaluate_start.elapsed();
    eprintln!("Evaluate (1400px) took: {:?}", evaluate_duration);

    eprintln!("\n=== TEST COMPLETE ===");
    eprintln!("If each render took < 10 seconds: Media queries are the bottleneck");
    eprintln!("If each render took > 30 seconds: CASE expressions are the bottleneck");
    eprintln!("If each render took 10-20 seconds: Both contribute to the issue");

    // Assert evaluations complete in reasonable time
    // We'll be lenient here since this is diagnostic
    // Normal visual tests take 5-7 seconds, so 20 seconds is already concerning
    assert!(
        evaluate_duration.as_secs() < 20,
        "Evaluate took too long: {:?}",
        evaluate_duration
    );
}
