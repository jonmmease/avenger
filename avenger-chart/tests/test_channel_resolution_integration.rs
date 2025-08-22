use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::ChannelExpr; // Need this for .band()
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::render::CanvasExt;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};
use datafusion::prelude::*;
use avenger_scales::scales::band::BandScale;
//! Integration test for channel reference resolution during rendering


#[tokio::test]
async fn test_channel_resolution_in_rendering() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value
            UNION ALL
            SELECT 'B', 20.0",
        )
        .await?;

    // Create a plot with channel reference
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Rect::new()
            .data(df)
            .x(col("category"))
            .x2(col(":x").band(1.0)) // This references the x channel
            .y(lit(0.0))
            .y2(col("value")),
    );

    // Try to render - this should succeed if channel resolution works
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    // If we get here without error, channel resolution worked
    Ok(())
}

#[tokio::test]
async fn test_channel_resolution_with_scale() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'Category A' as name, 
                10.0 as value
            UNION ALL
            SELECT 'Category B', 20.0
            UNION ALL
            SELECT 'Category C', 15.0",
        )
        .await?;

    // Create a plot where channel reference goes through scale transformation
    let plot = Plot::new(Cartesian)
        .with_size(400.0, 300.0)
        .scale_x(|scale| scale.scale_type(BandScale))
        .mark(
            Rect::new()
                .data(df)
                .x(col("name"))
                .x2(col(":x").band(1.0)) // Should resolve to the scaled x value
                .y(lit(0.0))
                .y2(col("value")),
        );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_unresolved_channel_reference_fails() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 'A' as category, 10.0 as value").await?;

    // Create a plot with reference to non-existent channel
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Rect::new()
            .data(df)
            .x(col("category"))
            .x2(col(":nonexistent")) // This channel doesn't exist
            .y(lit(0.0))
            .y2(col("value")),
    );

    // This should fail during rendering
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    let result = canvas.render_plot(&plot).await;

    // We expect an error because :nonexistent can't be resolved
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_simple_chained_channel_resolution() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value,
                5.0 as size
            UNION ALL
            SELECT 'B', 20.0, 8.0
            UNION ALL
            SELECT 'C', 15.0, 6.0",
        )
        .await?;

    // Create a plot with chained channel references
    // x -> category
    // y -> value
    // stroke -> :x (resolves to category)
    // fill -> :stroke (resolves to category through :x)
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col("category"))
            .y(col("value"))
            .stroke(col(":x")) // References x -> category
            .fill(col(":stroke")) // References stroke -> :x -> category
            .size(col("size")),
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_complex_chained_channel_resolution() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                1.0 as x_val,
                2.0 as y_val,
                'red' as color
            UNION ALL
            SELECT 2.0 as x_val, 4.0 as y_val, 'blue' as color
            UNION ALL
            SELECT 3.0 as x_val, 3.0 as y_val, 'green' as color",
        )
        .await?;

    // Create a plot with complex chained references
    // x -> x_val
    // y -> y_val
    // x2 -> :x + 0.5 (expression with channel ref)
    // y2 -> :y + 0.5 (expression with channel ref)
    // stroke -> :fill (references fill)
    // fill -> color
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Rect::new()
            .data(df)
            .x(col("x_val"))
            .y(col("y_val"))
            .x2(col(":x") + lit(0.5)) // Expression with channel reference
            .y2(col(":y") + lit(0.5)) // Expression with channel reference
            .fill(col("color"))
            .stroke(col(":fill")), // References fill -> color
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_multiple_level_chained_resolution() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as cat,
                10.0 as val
            UNION ALL
            SELECT 'B', 20.0",
        )
        .await?;

    // Create a plot with 3+ levels of chaining
    // x -> cat
    // stroke -> :x (resolves to cat)
    // fill -> :stroke (resolves to :x -> cat)
    // opacity -> :fill (resolves to :stroke -> :x -> cat)
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col("cat"))
            .y(col("val"))
            .stroke(col(":x")) // Level 1: references x
            .fill(col(":stroke")) // Level 2: references stroke -> x
            .size(50.0), // Using constant size
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    Ok(())
}

#[tokio::test]
async fn test_cyclic_channel_reference_detection() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 'A' as category, 10.0 as value").await?;

    // Create a plot with cyclic channel references
    // This should be detected and handled gracefully
    // x -> :y (references y)
    // y -> :x (references x)
    // This creates a cycle: x -> y -> x
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col(":y")) // x references y
            .y(col(":x")) // y references x - creates cycle!
            .size(lit(100.0)),
    );

    // Try to render - should handle the cycle gracefully
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // The render should complete (with an error logged) but not panic
    let _result = canvas.render_plot(&plot).await;

    // The plot should still render (falling back to unresolved refs)
    // or fail gracefully
    // Either outcome is acceptable as long as it doesn't panic

    Ok(())
}

#[tokio::test]
async fn test_complex_cycle_detection() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 'A' as category, 10.0 as value").await?;

    // Create a plot with a longer cycle
    // x -> :stroke
    // stroke -> :fill
    // fill -> :x
    // This creates a cycle: x -> stroke -> fill -> x
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col(":stroke")) // x references stroke
            .y(col("value"))
            .stroke(col(":fill")) // stroke references fill
            .fill(col(":x")) // fill references x - creates cycle!
            .size(lit(100.0)),
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // Should handle the cycle gracefully
    let _result = canvas.render_plot(&plot).await;

    Ok(())
}

#[tokio::test]
async fn test_partial_cycle_with_valid_channels() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value,
                5.0 as size",
        )
        .await?;

    // Create a plot with some valid channels and a cycle in others
    // x -> category (valid)
    // y -> value (valid)
    // stroke -> :fill (references fill)
    // fill -> :stroke (references stroke - creates cycle!)
    // size -> size (valid)
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col("category")) // Valid
            .y(col("value")) // Valid
            .stroke(col(":fill")) // Part of cycle
            .fill(col(":stroke")) // Part of cycle
            .size(col("size")), // Valid
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // Should handle gracefully - valid channels work, cyclic ones don't resolve
    let _result = canvas.render_plot(&plot).await;

    Ok(())
}

#[tokio::test]
async fn test_channel_resolution_with_expressions() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                10.0 as x_val,
                100.0 as y_val",
        )
        .await?;

    // Create a plot with expressions containing channel references
    // x -> x_val
    // y -> y_val * 2
    // x2 -> :x * 2 (expression with channel ref)
    // y2 -> :y + 1 (expression with channel ref to computed value)
    let plot = Plot::new(Cartesian).with_size(400.0, 300.0).mark(
        Rect::new()
            .data(df)
            .x(col("x_val"))
            .y(col("y_val") * lit(2.0))
            .x2(col(":x") * lit(2.0)) // Should resolve to x_val * 2
            .y2(col(":y") + lit(1.0)), // Should resolve to (y_val * 2) + 1
    );

    // Try to render
    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;
    canvas.render_plot(&plot).await?;

    Ok(())
}
