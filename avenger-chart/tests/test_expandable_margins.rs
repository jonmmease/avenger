use avenger_chart::layout::{CanvasConstraint, Margins, PlotConstraint};
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn test_margins_expand_with_fixed_canvas_and_plot() -> Result<(), Box<dyn std::error::Error>>
{
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 1 as x, 2 as y").await?;

    // Create a plot with:
    // - Fixed canvas: 500x400
    // - Fixed plot: 200x150
    // - Minimum margins: 10px
    // Expected: margins should expand to center the plot
    // Horizontal: (500 - 200) / 2 = 150px each side (minus axes/guides)
    // Vertical: (400 - 150) / 2 = 125px each side (minus axes/guides)
    let plot = Plot::<Cartesian>::new()
        .canvas_size(500.0, 400.0)
        .plot_size(200.0, 150.0)
        .margins(Margins::uniform(10.0))
        .mark(Symbol::new().data(df).x(col("x")).y(col("y")).size(100.0));

    // Render the plot
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [500.0, 400.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // This should succeed with the plot centered in the canvas
    let built_plot = plot.build();
    canvas.render_plot(&built_plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_margins_with_canvas_and_plot_width() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 1 as x, 2 as y").await?;

    // Create a plot with:
    // - Canvas width: 400
    // - Plot width: 250
    // - Minimum margins: 10px
    // Expected: horizontal margins expand, vertical size determined by content
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::Width(400.0))
        .plot_constraint(PlotConstraint::Width(250.0))
        .margins(Margins::uniform(10.0))
        .mark(Symbol::new().data(df).x(col("x")).y(col("y")).size(100.0));

    // Render the plot
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0], // Height will be determined by layout
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // This should succeed with horizontal margins expanded
    let built_plot = plot.build();
    canvas.render_plot(&built_plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_margins_with_canvas_and_plot_height() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 1 as x, 2 as y").await?;

    // Create a plot with:
    // - Canvas height: 350
    // - Plot height: 200
    // - Minimum margins: 15px
    // Expected: vertical margins expand, width determined by content
    let plot = Plot::<Cartesian>::new()
        .canvas_constraint(CanvasConstraint::Height(350.0))
        .plot_constraint(PlotConstraint::Height(200.0))
        .margins(Margins::uniform(15.0))
        .mark(Symbol::new().data(df).x(col("x")).y(col("y")).size(100.0));

    // Render the plot
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 350.0], // Width will be determined by layout
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // This should succeed with vertical margins expanded
    let built_plot = plot.build();
    canvas.render_plot(&built_plot).await?;

    Ok(())
}
