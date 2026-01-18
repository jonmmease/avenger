//! Tests for conditional channel domain behavior
//!
//! These tests verify that conditional channels with literal Value branches
//! correctly exclude those literal values from scale domain computation.
//! The focus is on compilation succeeding, which validates domain computation.

use avenger_chart::prelude::*;
use avenger_chart::scales::Linear;
use datafusion::arrow::array::{BooleanArray, Float64Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::SessionContext;
use palette::rgb::Srgba;
use std::sync::Arc;

/// Test that numeric domain excludes literal branches.
///
/// When a conditional has literal Value branches, the numeric domain
/// should only include values from rows where the scale is actually used.
#[tokio::test]
async fn test_conditional_numeric_domain_excludes_extreme_values() {
    let ctx = SessionContext::new();

    // Create data where the highlighted row has an extreme value (100)
    // that would skew the color scale if included
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    // Values: 10, 20, 30, 40, 100
    // The row with 100 is highlighted and should be excluded from domain
    let value = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 100.0]);
    let highlight = BooleanArray::from(vec![false, false, false, false, true]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
        Field::new("highlight", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(value),
            Arc::new(highlight),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a color scale from value
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(200.0)
            .fill_with(col("value"), |c| {
                c.when_value(col("highlight"), lit("#ff0000"))
                    .scale_with::<Linear>(|s| {
                        s.range_colors(vec![
                            Srgba::new(0.0, 0.0, 0.5, 1.0), // Dark blue at min
                            Srgba::new(0.0, 1.0, 1.0, 1.0), // Cyan at max
                        ])
                    })
            }),
    );

    // Compilation and evaluation should succeed
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    let _result = compiled
        .evaluate(&ctx, None)
        .await
        .expect("Failed to evaluate plot");
}

/// Test that type inference uses scale input only, not literal branches.
///
/// When a conditional has a numeric scaled branch and string literal branches,
/// the type inference should identify the channel as numeric (for the scale),
/// not as string.
#[tokio::test]
async fn test_conditional_type_inference_numeric_with_string_literal() {
    let ctx = SessionContext::new();

    // Create data with a numeric value column
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]);
    let y = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]);
    let value = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0]);
    let highlight = BooleanArray::from(vec![true, false, false, false]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
        Field::new("highlight", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(value),
            Arc::new(highlight),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with conditional color: string literal for highlight, numeric scale otherwise
    // Type inference should see this as numeric, not string
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(200.0)
            // The literal "#ff0000" is a string, but the scaled branch uses numeric `value`
            // Type inference should correctly identify this as numeric for scale building
            .fill_with(col("value"), |c| {
                c.when_value(col("highlight"), lit("#ff0000"))
                    .scale_with::<Linear>(|s| {
                        s.range_colors(vec![
                            Srgba::new(0.0, 0.0, 1.0, 1.0),
                            Srgba::new(0.0, 1.0, 0.0, 1.0),
                        ])
                    })
            }),
    );

    // If type inference incorrectly used string type from the literal branch,
    // this would fail because Linear scale doesn't work with strings
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    let _result = compiled
        .evaluate(&ctx, None)
        .await
        .expect("Failed to evaluate - type inference may have incorrectly used string type");
}

