// Test that scale defaults work correctly for bar charts

use super::datasets;
use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::SessionContext;

#[tokio::test]
async fn test_bar_chart_y_scale_auto_zero() {
    let ctx = SessionContext::new();
    let df = datasets::simple_categories();

    let plot = Chart::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s).axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#4682b4")
            .stroke("#000000")
            .stroke_width(1.0),
    );

    // This should produce the same result as bar_chart_inferred_domains
    // since the zero option is now applied by default
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "bar", "bar_chart_y_scale_auto_zero").await;
}

#[tokio::test]
async fn test_bar_chart_y_scale_no_nice() {
    let ctx = SessionContext::new();
    let df = datasets::simple_categories();

    let plot = Chart::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale(|s| s).axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))
                    .axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#e74c3c")
            .stroke("#c0392b")
            .stroke_width(1.0),
    );

    // Y-axis should start near the data minimum, not at zero
    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "bar", "bar_chart_y_scale_no_nice").await;
}
