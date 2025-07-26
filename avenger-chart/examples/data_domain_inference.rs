//! Example demonstrating automatic data domain inference for scales

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx
        .read_csv(
            "https://raw.githubusercontent.com/vega/vega-datasets/main/data/cars.json",
            CsvReadOptions::new()
                .has_header(true)
                .delimiter(b',')
                .file_extension("csv"),
        )
        .await?
        .select_columns(&["Name", "Miles_per_Gallon", "Cylinders", "Origin"])?
        .filter(col("Miles_per_Gallon").is_not_null())?
        .limit(0, Some(20))?;

    // Create a bar chart without explicit domain specifications
    // The domains should be inferred from the data
    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        // Note: No explicit domain for x scale - should infer from data
        .scale_x(|s| s)  // Uses default linear scale, domain will be inferred
        // Note: No explicit domain for y scale - should infer from data  
        .scale_y(|s| s)  // Uses default linear scale, domain will be inferred
        .axis_x(|a| a.title("Miles per Gallon"))
        .axis_y(|a| a.title("Count"))
        .mark(
            Rect::new()
                .x("Miles_per_Gallon")
                .x2(col("Miles_per_Gallon").add(lit(1.0)))  // 1 unit width bars
                .y(lit(0.0))
                .y2(lit(1.0))  // Will be stacked/aggregated
                .fill(lit("#4682b4"))
        );

    // Render the chart
    let renderer = avenger_chart::render::VegaSceneRenderer::new();
    let scene = renderer.render(&plot, 600, 400).await?;
    
    println!("Scene rendered with {} marks", scene.len());
    
    // Save to PNG
    use avenger_wgpu::canvas::CanvasConfig;
    let config = CanvasConfig::new(600, 400).with_scale(2.0);
    let mut canvas = config.create_canvas().await?;
    canvas.set_scene(&scene);
    let img = canvas.snapshot().await?;
    img.save("data_domain_inference.png")?;
    
    println!("Chart saved to data_domain_inference.png");
    
    Ok(())
}