//! Simple bar chart example demonstrating the avenger-chart API
//!
//! This example shows how to create a basic bar chart using the high-level
//! avenger-chart API and render it to a PNG file using PngCanvas.

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data for bar chart
    let categories = StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    let values = Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(categories), Arc::new(values)])?;

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch)?;

    // Create bar chart using avenger-chart API
    let plot = Plot::new(Cartesian)
        .data(df.clone())
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        // Add bar mark
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(col(":x").band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(1.0),
        );

    // Create PNG canvas with dimensions
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 2.0,
    };
    let config = CanvasConfig::default();

    println!("Creating PNG canvas...");
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // Render the plot to the canvas
    println!("Rendering plot to canvas...");
    canvas.render_plot(&plot).await?;

    // Render to PNG image
    println!("Rendering to PNG...");
    let image = canvas.render().await?;

    // Create output directory relative to the cargo manifest directory
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let output_dir = std::path::Path::new(&manifest_dir)
        .join("examples")
        .join("output");
    std::fs::create_dir_all(&output_dir)?;

    // Save the PNG file
    let output_path = output_dir.join("simple_bar_chart.png");
    println!("Saving PNG to {}...", output_path.display());
    image.save(&output_path)?;

    println!(
        "Bar chart successfully rendered to {}",
        output_path.display()
    );

    // Also show the data that was rendered
    println!("\nData rendered:");
    df.show().await?;

    Ok(())
}
