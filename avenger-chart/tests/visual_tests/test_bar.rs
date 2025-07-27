//! Visual tests for bar charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelValue;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::lit;

use super::datasets;
use super::helpers::assert_visual_match_default;

#[tokio::test]
async fn test_simple_bar_chart() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|scale| {
            scale.domain_discrete(vec![
                lit("A"),
                lit("B"),
                lit("C"),
                lit("D"),
                lit("E"),
                lit("F"),
                lit("G"),
                lit("H"),
                lit("I"),
            ])
        })
        .scale_y(|scale| scale.domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#4682b4"))
                .stroke(lit("#000000"))
                .stroke_width(lit(1.0)),
        );

    assert_visual_match_default(plot, "bar", "simple_bar_chart").await;
}

#[tokio::test]
async fn test_bar_chart_with_custom_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| {
            s.domain_discrete(vec![
                lit("A"),
                lit("B"),
                lit("C"),
                lit("D"),
                lit("E"),
                lit("F"),
                lit("G"),
                lit("H"),
                lit("I"),
            ])
        })
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category"))
        .axis_y(|a| a.title("Value"))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#e74c3c"))
                .stroke(lit("#c0392b"))
                .stroke_width(lit(2.0)),
        );

    assert_visual_match_default(plot, "bar", "bar_chart_custom_colors").await;
}

#[tokio::test]
async fn test_bar_chart_with_narrow_bars() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| {
            s.domain_discrete(vec![
                lit("A"),
                lit("B"),
                lit("C"),
                lit("D"),
                lit("E"),
                lit("F"),
                lit("G"),
                lit("H"),
                lit("I"),
            ])
        })
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(0.7)) // 70% of band width
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#3498db"))
                .stroke(lit("#2980b9"))
                .stroke_width(lit(1.5)),
        );

    assert_visual_match_default(plot, "bar", "bar_chart_narrow_bars").await;
}

#[tokio::test]
async fn test_bar_chart_inferred_domains() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        // No explicit domains - should be inferred from data
        .scale_x(|s| s)
        .scale_y(|s| s.option("zero", lit(true)).option("nice", lit(true)))
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
                .stroke_width(lit(1.0)),
        );

    assert_visual_match_default(plot, "bar", "bar_chart_inferred_domains").await;
}

#[tokio::test]
async fn test_bar_chart_with_gradient_colors() {
    use datafusion::prelude::*;

    let df = datasets::simple_categories();

    // Create a plot where bar colors are calculated based on the value
    // Using a simple formula to interpolate between light grey and blue
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| {
            s.domain_discrete(vec![
                lit("A"),
                lit("B"),
                lit("C"),
                lit("D"),
                lit("E"),
                lit("F"),
                lit("G"),
                lit("H"),
                lit("I"),
            ])
        })
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2("value")
                // Use a conditional expression to vary color based on value
                // This creates a gradient effect from light colors for low values to dark for high values
                .fill(
                    when(col("value").lt(lit(30.0)), lit("#c8d6e5")) // Light blue-grey for low values
                        .when(col("value").lt(lit(50.0)), lit("#8395a7")) // Medium blue-grey
                        .when(col("value").lt(lit(70.0)), lit("#576574")) // Darker blue-grey
                        .when(col("value").lt(lit(85.0)), lit("#2e86ab")) // Blue
                        .otherwise(lit("#0a3d62")) // Dark blue for highest values
                        .unwrap(),
                )
                .stroke(lit("#222222"))
                .stroke_width(lit(1.0))
                .opacity(lit(0.9)),
        );

    assert_visual_match_default(plot, "bar", "bar_chart_gradient_colors").await;
}
