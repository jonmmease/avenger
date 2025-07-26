# Visual Test Refactoring Comparison

## Before (Original Implementation)

```rust
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::lit;
use datafusion::prelude::*;
use std::sync::Arc;

/// Helper to create test data for bar charts
fn create_test_data() -> Result<DataFrame, Box<dyn std::error::Error>> {
    let categories = StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    let values = Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(categories), Arc::new(values)])?;

    let ctx = SessionContext::new();
    Ok(ctx.read_batch(batch)?)
}

async fn render_simple_bar_chart() -> image::RgbaImage {
    let df = create_test_data().expect("Failed to create test data");

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df.clone())
        .scale_x(|scale| {
            scale.domain_discrete(vec![
                lit("A"), lit("B"), lit("C"), lit("D"), lit("E"),
                lit("F"), lit("G"), lit("H"), lit("I"),
            ])
        })
        .scale_y(|scale| scale.domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2("category")
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4"))
                .stroke(lit("#000000"))
                .stroke_width(lit(1.0)),
        );

    // Create PNG canvas with dimensions
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 2.0,
    };
    let config = CanvasConfig::default();

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create canvas");

    // Render the plot to the canvas
    canvas
        .render_plot(&plot)
        .await
        .expect("Failed to render plot");

    // Render to image (already returns RgbaImage)
    canvas.render().await.expect("Failed to render image")
}

#[tokio::test]
async fn test_simple_bar_chart() {
    let baseline_path = get_baseline_path("simple_bar_chart");
    let rendered = render_simple_bar_chart().await;

    compare_images(&baseline_path, rendered, &VisualTestConfig::default())
        .expect("Visual test 'simple_bar_chart' failed");
}
```

**Total lines: ~80**

## After (With Helpers)

```rust
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::lit;

use super::{compare_images, get_baseline_path, VisualTestConfig};
use super::helpers::PlotTestExt;
use super::test_data;

fn simple_bar_chart() -> Plot<Cartesian> {
    let df = test_data::simple_categories().expect("Failed to create test data");

    Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|scale| {
            scale.domain_discrete(vec![
                lit("A"), lit("B"), lit("C"), lit("D"), lit("E"),
                lit("F"), lit("G"), lit("H"), lit("I"),
            ])
        })
        .scale_y(|scale| scale.domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2("category")
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4"))
                .stroke(lit("#000000"))
                .stroke_width(lit(1.0)),
        )
}

#[tokio::test]
async fn test_simple_bar_chart() {
    let baseline_path = get_baseline_path("simple_bar_chart");
    let rendered = simple_bar_chart().to_image().await;

    compare_images(&baseline_path, rendered, &VisualTestConfig::default())
        .expect("Visual test 'simple_bar_chart' failed");
}
```

**Total lines: ~40 (50% reduction!)**

## Key Improvements

1. **Data Generation**: Extracted to reusable `test_data` module
2. **Canvas Boilerplate**: Replaced 17 lines with single `.to_image()` call
3. **Import Reduction**: From 12 imports to 7
4. **Better Separation**: Plot creation separated from rendering
5. **Reusability**: Common patterns extracted to helpers

## Adding New Tests Is Now Trivial

```rust
#[tokio::test]
async fn test_bar_chart_with_custom_colors() {
    let df = test_data::simple_categories().expect("Failed to create test data");
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| s.domain_discrete(/* ... */))
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category"))
        .axis_y(|a| a.title("Value"))
        .mark(/* ... */);
    
    let baseline_path = get_baseline_path("bar_chart_custom_colors");
    let rendered = plot.to_image().await;
    
    compare_images(&baseline_path, rendered, &VisualTestConfig::default())
        .expect("Visual test failed");
}
```