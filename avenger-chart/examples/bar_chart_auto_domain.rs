//! Example demonstrating automatic data domain inference for bar charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::ChannelValue;
use avenger_chart::plot::Plot;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx.sql(
        "SELECT * FROM (VALUES 
            ('A', 28.0),
            ('B', 55.0),
            ('C', 43.0),
            ('D', 91.0),
            ('E', 81.0),
            ('F', 53.0),
            ('G', 19.0),
            ('H', 87.0),
            ('I', 52.0)
        ) AS t(category, value)"
    ).await?;

    // Create a bar chart without explicit domain specifications
    // The domains should be inferred from the data
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        // Note: No explicit domain for x scale - should infer from data
        .scale_x(|s| s)  // Will automatically switch to band scale for discrete data
        // Note: No explicit domain for y scale - should infer from data  
        .scale_y(|s| s)  // Will automatically compute min/max from data
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4"))
                .stroke(lit("#000000"))
                .stroke_width(lit(1.0))
        );

    // Render the chart
    let renderer = avenger_chart::render::VegaSceneRenderer::new();
    let scene = renderer.render(&plot, 400, 300).await?;
    
    println!("Scene rendered with {} marks", scene.len());
    
    // Save to PNG
    use avenger_wgpu::canvas::CanvasConfig;
    let config = CanvasConfig::new(400, 300).with_scale(2.0);
    let mut canvas = config.create_canvas().await?;
    canvas.set_scene(&scene);
    let img = canvas.snapshot().await?;
    img.save("bar_chart_auto_domain.png")?;
    
    println!("Chart saved to bar_chart_auto_domain.png");
    
    Ok(())
}