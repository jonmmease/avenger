//! Visual tests for various scales with color ranges

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::ChannelValue;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::{col, lit};
use palette::rgb::Srgba;

use super::datasets;
use super::helpers::assert_visual_match_default;

#[tokio::test]
async fn test_bar_chart_linear_color_interpolation() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| {
            s.scale_type("band").domain_discrete(vec![
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
        // Linear scale with color range
        .scale_fill(|s| {
            s.scale_type("linear")
                // Domain will be inferred from data automatically
                .range_color(vec![
                    Srgba::new(0.97, 0.96, 0.89, 1.0), // Light cream (#f8f5e4)
                    Srgba::new(0.96, 0.64, 0.38, 1.0), // Light orange (#f5a462)
                    Srgba::new(0.84, 0.19, 0.11, 1.0), // Dark red (#d6301d)
                ])
        })
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("value")) // Map value through the linear color scale
                .stroke("#333333".identity())
                .stroke_width(0.5)
                .opacity(0.95),
        );

    assert_visual_match_default(
        plot,
        "bar_scale_color",
        "bar_chart_linear_color_interpolation",
    )
    .await;
}

#[tokio::test]
async fn test_bar_chart_log_color_interpolation() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| {
            s.scale_type("band").domain_discrete(vec![
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
        // Log scale with color range
        .scale_fill(|s| {
            s.scale_type("log")
                .option("base", lit(10.0))
                // Domain will be inferred from data automatically
                .range_color(vec![
                    Srgba::new(0.99, 0.99, 0.87, 1.0), // Light yellow (#fffde4)
                    Srgba::new(0.42, 0.69, 0.45, 1.0), // Medium green (#6bb074)
                    Srgba::new(0.00, 0.27, 0.21, 1.0), // Dark green (#004534)
                ])
        })
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("value")) // Map value through the log color scale
                .stroke("#222222".identity())
                .stroke_width(0.5)
                .opacity(0.95),
        );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_log_color_interpolation").await;
}

#[tokio::test]
async fn test_bar_chart_pow_color_interpolation() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| {
            s.scale_type("band").domain_discrete(vec![
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
        // Power scale with color range (exponent = 2)
        .scale_fill(|s| {
            s.scale_type("pow")
                .option("exponent", lit(2.0))
                // Domain will be inferred from data automatically
                .range_color(vec![
                    Srgba::new(0.94, 0.91, 0.96, 1.0), // Light purple (#f0e8f5)
                    Srgba::new(0.61, 0.31, 0.64, 1.0), // Medium purple (#9b4fa3)
                    Srgba::new(0.25, 0.00, 0.29, 1.0), // Dark purple (#3f004a)
                ])
        })
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("value")) // Map value through the power color scale
                .stroke("#333333".identity())
                .stroke_width(0.5)
                .opacity(0.95),
        );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_pow_color_interpolation").await;
}

#[tokio::test]
async fn test_bar_chart_sqrt_color_interpolation() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| {
            s.scale_type("band").domain_discrete(vec![
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
        // Square root scale with color range
        .scale_fill(|s| {
            s.scale_type("sqrt")
                // Domain will be inferred from data automatically
                .range_color(vec![
                    Srgba::new(0.97, 0.91, 0.81, 1.0), // Light tan (#f8e8cf)
                    Srgba::new(0.94, 0.60, 0.15, 1.0), // Orange (#f09a27)
                    Srgba::new(0.58, 0.21, 0.05, 1.0), // Dark brown (#943508)
                ])
        })
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("value")) // Map value through the sqrt color scale
                .stroke("#222222".identity())
                .stroke_width(0.75),
        );

    assert_visual_match_default(
        plot,
        "bar_scale_color",
        "bar_chart_sqrt_color_interpolation",
    )
    .await;
}

#[tokio::test]
async fn test_bar_chart_threshold_scale_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| {
            s.scale_type("band").domain_discrete(vec![
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
        // Add threshold scale for colors
        .scale_fill(|s| {
            s.scale_type("threshold")
                .domain_discrete(vec![lit(30.0f32), lit(50.0f32), lit(70.0f32), lit(85.0f32)])
                .range_discrete(vec![
                    lit("#c8d6e5"), // Light blue-grey (< 30)
                    lit("#8395a7"), // Medium blue-grey (30-50)
                    lit("#576574"), // Darker blue-grey (50-70)
                    lit("#2e86ab"), // Blue (70-85)
                    lit("#0a3d62"), // Dark blue (> 85)
                ])
        })
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("value")) // Map value through the threshold scale
                .stroke("#222222".identity())
                .stroke_width(1.0)
                .opacity(0.9),
        );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_threshold_scale_colors").await;
}

#[tokio::test]
async fn test_bar_chart_linear_color_default_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill(col("value"))
                .stroke("#222222")
                .stroke_width(1.0),
        );

    assert_visual_match_default(
        plot,
        "bar_scale_color",
        "bar_chart_linear_color_default_colors",
    )
    .await;
}

#[tokio::test]
async fn test_bar_chart_ordinal_scale_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|s| s.scale_type("band")) // Domain will be inferred from data
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x(col("category"))
                .x2(ChannelValue::column("category").with_band(1.0))
                .y(lit(0.0).scaled())
                .y2(col("value"))
                .fill(col("category")) // Map fill to category column
                .stroke("#222222".identity())
                .stroke_width(1.0),
        );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_ordinal_scale_colors").await;
}
