//! Tests for facet measurement edge cases
//!
//! These tests verify facet behavior with:
//! - Single cell facets (one domain value)
//! - Multiple cells with consistent spacing
//! - Empty cells (filtered data resulting in no data for some cells)

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a simple dataset with a single category for single-cell facet tests
fn single_category_dataset(ctx: &SessionContext) -> DataFrame {
    let category = StringArray::from(vec!["A", "A", "A", "A", "A"]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_vals = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(category), Arc::new(x_vals), Arc::new(y_vals)],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

/// Create dataset with two categories for basic facet tests
fn two_category_dataset(ctx: &SessionContext) -> DataFrame {
    let categories = StringArray::from(vec!["A", "A", "A", "B", "B", "B"]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 3.0, 1.5, 2.5, 3.5]);
    let y_vals = Float64Array::from(vec![2.0, 4.0, 3.0, 3.0, 5.0, 2.5]);

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

/// Create dataset with many categories for stress testing
fn many_category_dataset(ctx: &SessionContext, num_categories: usize) -> DataFrame {
    let mut categories = Vec::new();
    let mut x_vals = Vec::new();
    let mut y_vals = Vec::new();

    for i in 0..num_categories {
        let cat_name = format!("Cat{:02}", i);
        // Add 3 data points per category
        for j in 0..3 {
            categories.push(cat_name.clone());
            x_vals.push((j as f64) + (i as f64) * 0.1);
            y_vals.push((j as f64) * 2.0 + (i as f64) * 0.5);
        }
    }

    let category_array = StringArray::from(categories);
    let x_array = Float64Array::from(x_vals);
    let y_array = Float64Array::from(y_vals);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(category_array),
            Arc::new(x_array),
            Arc::new(y_array),
        ],
    )
    .expect("create batch");

    ctx.read_batch(batch).expect("read batch")
}

/// Create dataset with extreme numeric values for edge case testing
fn extreme_values_dataset(ctx: &SessionContext) -> DataFrame {
    let categories =
        StringArray::from(vec!["Normal", "Normal", "Large", "Large", "Small", "Small"]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 1000000.0, 2000000.0, 0.000001, 0.000002]);
    let y_vals = Float64Array::from(vec![1.0, 2.0, 1e6, 2e6, 1e-6, 2e-6]);

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

/// Create dataset with integer facet values
fn integer_facet_dataset(ctx: &SessionContext) -> DataFrame {
    let categories = Int32Array::from(vec![1, 1, 1, 2, 2, 2, 3, 3, 3]);
    let x_vals = Float64Array::from(vec![1.0, 2.0, 3.0, 1.5, 2.5, 3.5, 2.0, 3.0, 4.0]);
    let y_vals = Float64Array::from(vec![2.0, 4.0, 3.0, 3.0, 5.0, 2.5, 1.0, 3.0, 5.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Int32, false),
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
// Single Cell Tests
// ============================================================================

#[tokio::test]
async fn facet_row_single_cell() {
    let ctx = SessionContext::new();
    let df = single_category_dataset(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#4682b4"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(400.0, 300.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_single_cell").await;
}

#[tokio::test]
async fn facet_column_single_cell() {
    let ctx = SessionContext::new();
    let df = single_category_dataset(&ctx);

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#228b22"),
                ),
            )
            .column(col("category")),
        )
        .canvas_size(400.0, 300.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_column_single_cell").await;
}

// ============================================================================
// Two Cell Tests (Basic Measurement)
// ============================================================================

#[tokio::test]
async fn facet_row_two_cells() {
    let ctx = SessionContext::new();
    let df = two_category_dataset(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#dc143c"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(400.0, 350.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_two_cells").await;
}

#[tokio::test]
async fn facet_column_two_cells() {
    let ctx = SessionContext::new();
    let df = two_category_dataset(&ctx);

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#9932cc"),
                ),
            )
            .column(col("category")),
        )
        .canvas_size(500.0, 300.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_column_two_cells").await;
}

// ============================================================================
// Many Cells Tests (Stress Testing)
// ============================================================================

#[tokio::test]
async fn facet_row_many_cells() {
    let ctx = SessionContext::new();
    let df = many_category_dataset(&ctx, 8);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(20.0)
                        .fill("#4169e1"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(450.0, 800.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_many_cells").await;
}

#[tokio::test]
async fn facet_column_many_cells() {
    let ctx = SessionContext::new();
    let df = many_category_dataset(&ctx, 6);

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(20.0)
                        .fill("#20b2aa"),
                ),
            )
            .column(col("category")),
        )
        .canvas_size(900.0, 350.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_column_many_cells").await;
}

// ============================================================================
// Integer Facet Value Tests
// ============================================================================

#[tokio::test]
async fn facet_row_integer_values() {
    let ctx = SessionContext::new();
    let df = integer_facet_dataset(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("x"))
                        .y(col("y"))
                        .size(36.0)
                        .fill("#ff6347"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(400.0, 450.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_integer_values").await;
}

// ============================================================================
// Extreme Value Tests
// ============================================================================

#[tokio::test]
async fn facet_row_extreme_values_free() {
    let ctx = SessionContext::new();
    let df = extreme_values_dataset(&ctx);

    // Free scaling should handle extreme value ranges per-cell
    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("x"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                        })
                        .y_with(col("y"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                        })
                        .size(36.0)
                        .fill("#8b4513"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(400.0, 450.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "facet_row_extreme_values_free",
    )
    .await;
}

// ============================================================================
// Free Scaling Single Cell Tests
// ============================================================================

#[tokio::test]
async fn facet_row_single_cell_free() {
    let ctx = SessionContext::new();
    let df = single_category_dataset(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("x"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                                .axis(|a| a.title("X Axis"))
                        })
                        .y_with(col("y"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                                .axis(|a| a.title("Y Axis"))
                        })
                        .size(36.0)
                        .fill("#2e8b57"),
                ),
            )
            .row(col("category")),
        )
        .canvas_size(400.0, 300.0);

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_single_cell_free").await;
}
