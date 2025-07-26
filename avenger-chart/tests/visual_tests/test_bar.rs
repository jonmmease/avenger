//! Visual tests for bar charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::lit;

use super::datasets;
use super::helpers::{assert_visual_match, assert_visual_match_default};

#[tokio::test]
async fn test_simple_bar_chart() {
    let df = datasets::simple_categories();

    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|scale| {
            scale.domain_discrete(vec![
                lit("A"), lit("B"), lit("C"), lit("D"), lit("E"),
                lit("F"), lit("G"), lit("H"), lit("I"),
            ])
        })
        .scale_y(|scale| scale.domain((0.0, 100.0)))
        .axis_x(|axis| axis.title("Category").grid(false))
        .axis_y(|axis| axis.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .x2("category")
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
        .scale_x(|s| s.domain_discrete(vec![
            lit("A"), lit("B"), lit("C"), lit("D"), lit("E"),
            lit("F"), lit("G"), lit("H"), lit("I")
        ]))
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category"))
        .axis_y(|a| a.title("Value"))
        .mark(
            Rect::new()
                .x("category")
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#e74c3c"))  // Different color
                .stroke(lit("#c0392b"))
                .stroke_width(lit(2.0)),
        );
    
    assert_visual_match_default(plot, "bar", "bar_chart_custom_colors").await;
}

#[tokio::test]
async fn test_bar_chart_with_lower_tolerance() {
    let df = datasets::simple_categories();
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        .scale_x(|s| s.domain_discrete(vec![
            lit("A"), lit("B"), lit("C"), lit("D"), lit("E"),
            lit("F"), lit("G"), lit("H"), lit("I")
        ]))
        .scale_y(|s| s.domain((0.0, 100.0)))
        .axis_x(|a| a.title("Category").grid(false))
        .axis_y(|a| a.title("Value").grid(true))
        .mark(
            Rect::new()
                .x("category")
                .y(lit(0.0))
                .y2("value")
                .fill(lit("#3498db"))
                .stroke(lit("#2980b9"))
                .stroke_width(lit(1.5)),
        );
    
    // Use 90% tolerance for this test (more lenient)
    assert_visual_match(plot, "bar", "bar_chart_lower_tolerance", 0.90).await;
}