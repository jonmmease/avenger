//! Visual tests for data domain inference

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::ChannelValue;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::prelude::*;

use super::helpers::assert_visual_match;

#[tokio::test]
async fn test_bar_chart_inferred_domain() {
    // Create sample data
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES 
            ('Category A', 28.0),
            ('Category B', 55.0),
            ('Category C', 43.0),
            ('Category D', 91.0),
            ('Category E', 81.0)
        ) AS t(category, value)",
        )
        .await
        .unwrap();

    // Create a bar chart without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        // No explicit domain specifications - should be inferred
        .scale_x(|s| s)
        .scale_y(|s| s.option("zero", lit(false)).option("nice", lit(false)))
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill("#3498db".identity())
                .stroke("crimson".identity())
                .stroke_width(1.0),
        );

    // Apply domain inference
    plot.apply_default_domain("x");
    plot.apply_default_domain("y");

    assert_visual_match(plot, "data_domain", "bar_chart_inferred", 0.99).await;
}

#[tokio::test]
async fn test_scatter_plot_inferred_domain() {
    // Create scatter plot data
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES 
            (15.5, 22.3),
            (25.2, 38.7),
            (35.8, 18.9),
            (45.1, 42.6),
            (30.0, 30.0)
        ) AS t(x, y)",
        )
        .await
        .unwrap();

    // Create a scatter plot without explicit domains
    let mut plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        // No explicit domain specifications - should compute min/max from expressions
        // The issue was that nice=true was rounding 13.5 down to 10
        .scale_x(|s| s)
        .scale_y(|s| s)
        .axis_x(|a| a.title("X Value"))
        .axis_y(|a| a.title("Y Value"))
        .mark(
            Rect::new()
                .x(col("x").sub(lit(2.0)))
                .x2(col("x").add(lit(2.0)))
                .y(col("y").sub(lit(2.0)))
                .y2(col("y").add(lit(2.0)))
                .fill("#e74c3c".identity())
                .opacity(0.7),
        );

    // Apply domain inference from expressions
    plot.apply_default_domain("x");
    plot.apply_default_domain("y");

    assert_visual_match(plot, "data_domain", "scatter_inferred", 0.99).await;
}
