//! Test that scale defaults work correctly for bar charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::ChannelValue;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::{col, lit};

use super::datasets;
use super::helpers::assert_visual_match_default;

#[tokio::test]
async fn test_bar_chart_y_scale_auto_zero() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .data(df)
        // No explicit options:
        // - y scale should get nice=true,zero=true by default
        // - x scale should get nice=true by default
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill("#4682b4".identity())
                .stroke("#000000".identity())
                .stroke_width(1.0),
        );

    // This should produce the same result as bar_chart_inferred_domains
    // since the zero option is now applied by default
    assert_visual_match_default(plot, "bar", "bar_chart_y_scale_auto_zero").await;
}

#[tokio::test]
async fn test_bar_chart_y_scale_no_zero() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .data(df)
        // Explicitly disable zero option - should show data range only
        .scale_x(|s| s)
        .scale_y(|s| s.option("zero", lit(false))) // Override default
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill("#e74c3c".identity())
                .stroke("#c0392b".identity())
                .stroke_width(1.0),
        );

    // Y-axis should start near the data minimum, not at zero
    assert_visual_match_default(plot, "bar", "bar_chart_y_scale_no_zero").await;
}
