use avenger_chart::coords::Cartesian;
use avenger_chart::error::AvengerChartError;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
use datafusion::arrow::array::Float32Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::{SessionContext, col, lit};
use std::sync::Arc;

#[tokio::test]
async fn test_literal_x_value_error() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "A", "B", "C",
            ])),
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with literal x value (incorrect usage)
    let plot = Plot::new(Cartesian).mark(Symbol::new().data(df).x("category").y(col("value")));

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should error with helpful message
    assert!(result.is_err());
    let err = result.unwrap_err();

    match err {
        AvengerChartError::PositionalScaleLiteralError {
            scale_name,
            coord_system,
            literal_value,
            suggestion,
        } => {
            assert_eq!(scale_name, "x");
            assert_eq!(coord_system, "Cartesian");
            assert_eq!(literal_value, "string literal");
            assert!(suggestion.contains("col(\""));
        }
        _ => panic!("Expected PositionalScaleLiteralError, got {:?}", err),
    }
}

#[tokio::test]
async fn test_literal_y_value_error() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0])),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with literal string y value (incorrect usage)
    let plot = Plot::new(Cartesian).mark(Symbol::new().data(df).x(col("x")).y("fixed")); // Literal string

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should error with helpful message
    assert!(result.is_err());
    let err = result.unwrap_err();

    match err {
        AvengerChartError::PositionalScaleLiteralError {
            scale_name,
            coord_system,
            literal_value,
            ..
        } => {
            assert_eq!(scale_name, "y");
            assert_eq!(coord_system, "Cartesian");
            assert_eq!(literal_value, "string literal");
        }
        _ => panic!("Expected PositionalScaleLiteralError, got {:?}", err),
    }
}

#[tokio::test]
async fn test_explicit_domain_allows_literals() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Float32,
        false,
    )]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0]))],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with numeric literal x value but explicit domain (should work)
    let plot = Plot::new(Cartesian)
        .mark(Symbol::new().data(df).x(50.0).y(col("value")))
        .scale_x(|s| s.domain_interval(lit(0), lit(100)));

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should NOT error because domain is explicit
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_column_reference_works() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0])),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with proper column references (correct usage)
    let plot = Plot::new(Cartesian).mark(Symbol::new().data(df).x(col("x")).y(col("y")));

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should work fine
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_expression_with_column_works() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0])),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with expressions that reference columns
    let plot = Plot::new(Cartesian).mark(
        Symbol::new()
            .data(df)
            .x(col("x") + lit(10)) // Expression with column
            .y(col("y") * lit(2)), // Expression with column
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should work fine because expressions reference columns
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_non_positional_scales_allow_literals() {
    // Create test data
    let ctx = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
            Arc::new(Float32Array::from(vec![10.0, 20.0, 30.0])),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    // Create a plot with literal values for non-positional channels (should work)
    let plot = Plot::new(Cartesian).mark(
        Symbol::new()
            .data(df)
            .x(col("x"))
            .y(col("y"))
            .fill("red") // Literal color - OK for non-positional
            .size(10.0), // Literal size - OK for non-positional
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    let result = canvas.render_plot(&plot).await;

    // Should work fine because fill and size are not positional
    assert!(result.is_ok());
}
