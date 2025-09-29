use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn test_reference_to_conditional_channel() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value,
                true as flag
            UNION ALL
            SELECT 'B', 20.0, false
            UNION ALL  
            SELECT 'C', 15.0, true",
        )
        .await?;

    // Create a plot with a conditional channel that is referenced
    let plot = Plot::<Cartesian>::new().canvas_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df.clone())
            .x(col("value"))
            .y(col("value"))
            // Create a conditional fill channel
            .fill_with(col("category"), |c| {
                c.when_value(col("flag").eq(lit(false)), lit("red"))
            })
            // stroke references the conditional fill channel - this should error
            .stroke(col(":fill")),
    );

    // Try to render - this should handle the unresolved reference gracefully
    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // This should fail with an error about conditional channel reference
    let compiled = plot.compile(&ctx).await.unwrap();
    let result = canvas.render_plot(&compiled, &ctx, None).await;

    // We expect an error
    assert!(
        result.is_err(),
        "Expected error when referencing conditional channel"
    );

    if let Err(e) = result {
        let error_str = e.to_string();
        println!("Error: {}", error_str);
        assert!(
            error_str.contains("conditional") || error_str.contains("Cannot reference"),
            "Expected error about conditional channel reference, got: {}",
            error_str
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_rect_with_conditional_x_reference() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value,
                true as flag
            UNION ALL
            SELECT 'B', 20.0, false",
        )
        .await?;

    // Create a plot where x2 references a conditional x
    let plot = Plot::<Cartesian>::new().canvas_size(400.0, 300.0).mark(
        Rect::new()
            .data(df)
            .x(col("value"))
            .y(lit(0.0))
            .y2(col("value"))
            // fill is conditional
            .fill_with(col("category"), |c| {
                c.when_value(col("flag").eq(lit(false)), lit("blue"))
            })
            // stroke tries to reference the conditional fill channel
            .stroke(col(":fill")),
    );

    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    // This should handle the unresolved conditional reference
    let compiled = plot.compile(&ctx).await.unwrap();
    let result = canvas.render_plot(&compiled, &ctx, None).await;
    println!("Rect render result: {:?}", result.is_ok());

    Ok(())
}

#[tokio::test]
async fn test_chain_through_conditional() -> Result<(), Box<dyn std::error::Error>> {
    // Test what happens when chaining through a conditional
    // fill -> :x (where x is conditional)
    // stroke -> :fill (should this work?)

    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 
                'A' as category, 
                10.0 as value,
                true as flag",
        )
        .await?;

    let plot = Plot::<Cartesian>::new().canvas_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df)
            .x(col("value"))
            .y(col("value"))
            // size is conditional
            .size_with(lit(100.0), |c| {
                c.when_value(col("flag").eq(lit(false)), lit(50.0))
            })
            // stroke_width references conditional size - this should error
            .stroke_width(col(":size")),
    );

    use avenger_common::canvas::CanvasDimensions;
    use avenger_wgpu::canvas::{CanvasConfig, PngCanvas};

    let dimensions = CanvasDimensions {
        size: [400.0, 300.0],
        scale: 1.0,
    };
    let config = CanvasConfig::default();
    let mut canvas = PngCanvas::new(dimensions, config).await?;

    let compiled = plot.compile(&ctx).await.unwrap();
    let result = canvas.render_plot(&compiled, &ctx, None).await;
    println!("Chain through conditional result: {:?}", result.is_ok());

    Ok(())
}
