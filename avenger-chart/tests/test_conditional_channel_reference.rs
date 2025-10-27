use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn test_reference_to_conditional_channel() -> Result<(), Box<dyn std::error::Error>> {
    // Test that references to conditional channels resolve to their 'otherwise' expression
    // This test uses numeric conditionals to avoid DataFusion type coercion issues
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT
                10.0 as x_val,
                20.0 as y_val,
                5.0 as size_val,
                true as flag",
        )
        .await?;

    // Create a plot where one channel references a conditional channel
    // NEW BEHAVIOR: References to conditional channels should resolve to the 'otherwise' expression
    let plot = Plot::<Cartesian>::new().canvas_size(400.0, 300.0).mark(
        Symbol::new()
            .data(df.clone())
            .x(col("x_val"))
            .y(col("y_val"))
            // Create a conditional size channel with 'size_val' as the otherwise value
            .size_with(col("size_val"), |c| {
                c.when_value(col("flag").eq(lit(false)), lit(10.0))
            })
            // stroke_width references the conditional size channel
            // This should resolve to col("size_val") (the 'otherwise' expression)
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

    // NEW BEHAVIOR: This should succeed because :size resolves to col("size_val")
    let compiled = plot.compile(&ctx).await?;
    let result = canvas.render_plot(&compiled, &ctx, None).await;

    // The render should succeed
    assert!(
        result.is_ok(),
        "Expected successful render when referencing conditional channel (should resolve to 'otherwise' expression), got error: {:?}",
        result.err()
    );

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
