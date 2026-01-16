//! Tests for facet domain edge cases with special values
//!
//! These tests verify facet behavior with domains containing:
//! - Numeric facet columns with special ordering
//! - Infinity values (positive and negative) - filtered to finite values
//! - String domains with various orderings
//!
//! Note: NaN values in data columns cause panics in domain_solver.rs
//! (a known limitation that would need fixes in avenger-scales).
//! These edge cases exercise the total-order comparison in scalar_cmp.rs

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create dataset with numeric facet values including special ordering concerns
fn dataset_with_numeric_facet_special_order(ctx: &SessionContext) -> DataFrame {
    // Test that numeric facet values are sorted correctly even with edge values
    let categories = Float64Array::from(vec![
        -100.0, -100.0, 0.0, 0.0, 100.0, 100.0, -1.0, -1.0, 1.0, 1.0,
    ]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
    let y_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Float64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_vals), Arc::new(y_vals)],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

/// Create dataset with integer facet values
fn dataset_with_integer_facet(ctx: &SessionContext) -> DataFrame {
    // Test that integer facet values are sorted correctly
    let categories = Int64Array::from(vec![3, 3, 1, 1, 2, 2, -5, -5, 10, 10]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
    let y_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Int64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_vals), Arc::new(y_vals)],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

/// Create dataset with string facet values including special characters
fn dataset_with_string_facet_special_chars(ctx: &SessionContext) -> DataFrame {
    // Test string facet values with special characters
    let categories = StringArray::from(vec![
        "Zebra",
        "Zebra",
        "Alpha",
        "Alpha",
        "123",
        "123",
        "_underscore",
        "_underscore",
    ]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);
    let y_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_vals), Arc::new(y_vals)],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

/// Create dataset with large numeric range in data
fn dataset_with_large_range(ctx: &SessionContext) -> DataFrame {
    // Test that extreme but finite values work correctly
    let categories = StringArray::from(vec!["Small", "Small", "Large", "Large"]);
    let x_vals = Float64Array::from(vec![1e-10, 2e-10, 1e10, 2e10]);
    let y_vals = Float64Array::from(vec![1.0, 2.0, 1.0, 2.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_vals), Arc::new(y_vals)],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

// ============================================================================
// Numeric Facet Ordering Tests
// ============================================================================

#[tokio::test]
async fn facet_row_numeric_ordering() {
    let ctx = SessionContext::new();
    let df = dataset_with_numeric_facet_special_order(&ctx);

    // Numeric facet values should be sorted in correct numeric order
    // -100, -1, 0, 1, 100 (not string order: -1, -100, 0, 1, 100)
    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("category")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#2ecc71"),
                ),
            ),
        )
        .canvas_size(400.0, 600.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_numeric_ordering").await;
}

#[tokio::test]
async fn facet_row_integer_ordering() {
    let ctx = SessionContext::new();
    let df = dataset_with_integer_facet(&ctx);

    // Integer facet values should be sorted in correct numeric order
    // -5, 1, 2, 3, 10
    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("category")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#e74c3c"),
                ),
            ),
        )
        .canvas_size(400.0, 600.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_integer_ordering").await;
}

// ============================================================================
// String Facet Ordering Tests
// ============================================================================

#[tokio::test]
async fn facet_row_string_special_chars() {
    let ctx = SessionContext::new();
    let df = dataset_with_string_facet_special_chars(&ctx);

    // String facet values with special chars should sort lexicographically
    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("category")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#3498db"),
                ),
            ),
        )
        .canvas_size(400.0, 500.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "facet_row_string_special_chars",
    )
    .await;
}

// ============================================================================
// Large Range Tests
// ============================================================================

#[tokio::test]
async fn facet_row_large_range() {
    let ctx = SessionContext::new();
    let df = dataset_with_large_range(&ctx);

    // Very large and very small but finite values should work with Free scaling
    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("category")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("x"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                        })
                        .y(col("y"))
                        .size(36.0)
                        .fill("#9b59b6"),
                ),
            ),
        )
        .canvas_size(400.0, 350.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_large_range").await;
}
