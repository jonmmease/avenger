//! Simple bar chart example demonstrating the avenger-chart API
//!
//! This example shows how to create a basic bar chart using the high-level
//! avenger-chart API and render it to a PNG file using PngCanvas.

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
        // Configure scales
        .scale_x(|scale| {
            scale.domain_discrete(vec![
                lit("A"),
                lit("B"),
                lit("C"),
                lit("D"),
                lit("E"),
                lit("F"),
                lit("G"),
                lit("H"),
                lit("I"),
            ])
        })
        .scale_y(|scale| scale.domain((0.0, 100.0)))
        // Configure axes
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        // Add bar mark
        .mark(
            Rect::new()
                .x("category")
                .x2("category") // Band scale will automatically expand to x2
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4")) // Steel blue
                .stroke(lit("#000000"))
                .stroke_width(lit(1.0)),
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

    // Save the PNG file
    let output_path = "simple_bar_chart.png";
    println!("Saving PNG to {}...", output_path);
    image.save(output_path)?;

    println!("Bar chart successfully rendered to {}", output_path);

    // Also show the data that was rendered
    println!("\nData rendered:");
    df.show().await?;

    Ok(())
}
