use avenger_chart::prelude::*;
// Visual tests for data domain inference

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
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.option("zero", lit(false)).option("nice", lit(false)))
                    .axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#3498db")
            .stroke("crimson")
            .stroke_width(1.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "data_domain",
        "bar_chart_inferred",
        0.9999,
    )
    .await;
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
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("x").sub(lit(2.0)), |c| {
                c.scale(|s| s).axis(|a| a.title("X Value"))
            })
            .x2(col("x").add(lit(2.0)))
            .y_with(col("y").sub(lit(2.0)), |c| {
                c.scale(|s| s).axis(|a| a.title("Y Value"))
            })
            .y2(col("y").add(lit(2.0)))
            .fill("#e74c3c")
            .opacity(0.7),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "data_domain",
        "scatter_inferred",
        0.9999,
    )
    .await;
}
