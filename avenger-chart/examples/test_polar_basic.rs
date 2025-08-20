use avenger_chart::coords::Polar;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use datafusion::prelude::*;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create sample data with polar coordinates
    let ctx = SessionContext::new();
    
    // Create test data with various angles and radii
    let df = ctx
        .sql("SELECT 
            angle * 0.0174533 as theta,  -- Convert degrees to radians
            radius,
            category
        FROM (
            VALUES 
            (0.0, 50.0, 'A'),
            (45.0, 75.0, 'B'),
            (90.0, 100.0, 'A'),
            (135.0, 60.0, 'B'),
            (180.0, 80.0, 'A'),
            (225.0, 90.0, 'B'),
            (270.0, 70.0, 'A'),
            (315.0, 85.0, 'B'),
            (30.0, 40.0, 'C'),
            (120.0, 95.0, 'C'),
            (210.0, 55.0, 'C'),
            (300.0, 65.0, 'C')
        ) AS t(angle, radius, category)")
        .await?;

    // Create polar plot
    let plot = Plot::new(Polar::new())
        .data(df)
        .scale_r(|s| s.domain_interval(lit(0.0), lit(120.0)))
        .scale_theta(|s| s.domain_interval(lit(0.0), lit(2.0 * std::f64::consts::PI)))
        .scale_fill(|s| s.scale_type("ordinal"))
        .mark(
            Symbol::new()
                .r(col("radius"))
                .theta(col("theta"))
                .fill(col("category"))
                .size(lit(100.0))
        )
        .with_size(500.0, 500.0);

    // Create canvas
    let dimensions = CanvasDimensions {
        size: [500.0, 500.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // Render the plot to the canvas
    canvas.render_plot(&plot).await?;

    // Render to PNG image
    let image = canvas.render().await?;

    // Save the PNG file
    let output_path = "polar_test.png";
    image.save(output_path)?;
    
    println!("Polar plot saved to {}", output_path);
    
    Ok(())
}