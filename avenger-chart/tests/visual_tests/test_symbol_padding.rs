//! Visual tests for symbol padding calculation

use super::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::col;
use datafusion::prelude::*;
use std::sync::Arc;

async fn create_simple_scatter_data() -> DataFrame {
    let ctx = SessionContext::new();
    
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));
    
    let x_data = Arc::new(Float32Array::from(vec![10.0, 50.0, 90.0]));
    let y_data = Arc::new(Float32Array::from(vec![10.0, 50.0, 90.0]));
    
    let batch = RecordBatch::try_new(schema.clone(), vec![x_data, y_data]).unwrap();
    
    ctx.register_batch("data", batch).unwrap();
    ctx.table("data").await.unwrap()
}

#[tokio::test]
async fn test_symbol_padding_no_nice() {
    let df = create_simple_scatter_data().await;
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 400.0)
        .data(df)
        .scale_x(|s| s.domain((0.0, 100.0)).nice(lit(false)))
        .scale_y(|s| s.domain((0.0, 100.0)).nice(lit(false)))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(lit(100.0))
                .shape(lit("circle"))
        );
    
    assert_visual_match_default(plot, "symbol", "test_symbol_padding_no_nice").await;
}

#[tokio::test]
async fn test_symbol_padding_with_nice() {
    let df = create_simple_scatter_data().await;
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 400.0)
        .data(df)
        .scale_x(|s| s.domain((0.0, 100.0)).nice(lit(true)))
        .scale_y(|s| s.domain((0.0, 100.0)).nice(lit(true)))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(lit(100.0))
                .shape(lit("circle"))
        );
    
    assert_visual_match_default(plot, "symbol", "test_symbol_padding_with_nice").await;
}

#[tokio::test]
async fn test_arrow_symbol_asymmetric_padding() {
    let ctx = SessionContext::new();
    
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("shape", DataType::Utf8, false),
    ]));
    
    // Create data with triangles pointing in different directions at edges
    let x_data = Arc::new(Float32Array::from(vec![5.0, 95.0, 50.0]));
    let y_data = Arc::new(Float32Array::from(vec![5.0, 95.0, 50.0]));
    let shape_data = Arc::new(StringArray::from(vec!["triangle-down", "triangle-up", "diamond"]));
    
    let batch = RecordBatch::try_new(
        schema.clone(), 
        vec![x_data, y_data, shape_data]
    ).unwrap();
    
    ctx.register_batch("data", batch).unwrap();
    let df = ctx.table("data").await.unwrap();
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| s.nice(lit(false)))  // No nice to see exact padding
        .scale_y(|s| s.nice(lit(false)))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(lit(500.0))  // Larger triangles to make asymmetry more visible
                .shape(col("shape"))
        );
    
    // The triangle-down at (5, 5) should require more padding at lower bounds
    // The triangle-up at (95, 95) should require more padding at upper bounds
    // The diamond at (50, 50) demonstrates rotation handling
    // Note: Currently using symmetric padding, so this test documents current behavior
    assert_visual_match_default(plot, "symbol", "test_arrow_symbol_asymmetric_padding").await;
}

#[tokio::test]
async fn test_exact_geometry_containment() {
    let ctx = SessionContext::new();
    
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));
    
    // Create several large shapes at different edges
    let x_data = Arc::new(Float32Array::from(vec![5.0, 50.0, 95.0]));
    let y_data = Arc::new(Float32Array::from(vec![5.0, 95.0, 50.0]));
    
    let batch = RecordBatch::try_new(
        schema.clone(), 
        vec![x_data, y_data]
    ).unwrap();
    
    ctx.register_batch("data", batch).unwrap();
    let df = ctx.table("data").await.unwrap();
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 400.0)
        .data(df)
        .scale_x(|s| s.domain((0.0, 100.0)).nice(lit(false)))
        .scale_y(|s| s.domain((0.0, 100.0)).nice(lit(false)))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(lit(400.0))  // Very large diamond
                .shape(lit("square"))  // Use square to test padding of rotated shapes
                .angle(lit(45.0))  // Rotate to test rotated geometry padding
        );
    
    // Verify that the rendered image contains exactly the diamond geometry
    // with no clipping and minimal padding
    assert_visual_match_default(plot, "symbol", "test_exact_geometry_containment").await;
}