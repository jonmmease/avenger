//! Visual tests for bar charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::logical_expr::lit;

use super::{compare_images, get_baseline_path, VisualTestConfig};
use super::helpers::PlotTestExt;
use super::test_data;

/// Create a simple bar chart plot
fn simple_bar_chart() -> Plot<Cartesian> {
    let df = test_data::simple_categories().expect("Failed to create test data");

    Plot::new(Cartesian)
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
        )
}

#[tokio::test]
async fn test_simple_bar_chart() {
    let baseline_path = get_baseline_path("simple_bar_chart");
    let rendered = simple_bar_chart().to_image().await;

    compare_images(&baseline_path, rendered, &VisualTestConfig::default())
        .expect("Visual test 'simple_bar_chart' failed");
}

/// Test to update the baseline - run this manually when visuals change intentionally
#[tokio::test]
#[ignore] // Only run when explicitly requested
async fn update_simple_bar_chart_baseline() {
    let rendered = simple_bar_chart().to_image().await;
    let baseline_path = get_baseline_path("simple_bar_chart");
    
    rendered
        .save(&baseline_path)
        .expect("Failed to update baseline");
    
    println!("Updated baseline: {}", baseline_path);
}

/// Example of how concise a new test can be with our helpers!
/// This creates a more complex chart with just a few lines
#[tokio::test]
async fn test_bar_chart_with_custom_colors() {
    let df = test_data::simple_categories().expect("Failed to create test data");
    
    // Look how concise this is now!
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
    
    let baseline_path = get_baseline_path("bar_chart_custom_colors");
    let rendered = plot.to_image().await;
    
    compare_images(&baseline_path, rendered, &VisualTestConfig::default())
        .expect("Visual test 'bar_chart_custom_colors' failed");
}

#[tokio::test]
#[ignore]
async fn update_bar_chart_custom_colors_baseline() {
    let df = test_data::simple_categories().expect("Failed to create test data");
    
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
                .fill(lit("#e74c3c"))
                .stroke(lit("#c0392b"))
                .stroke_width(lit(2.0)),
        );
    
    let rendered = plot.to_image().await;
    let baseline_path = get_baseline_path("bar_chart_custom_colors");
    
    rendered
        .save(&baseline_path)
        .expect("Failed to update baseline");
    
    println!("Updated baseline: {}", baseline_path);
}