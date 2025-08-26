use super::datasets;
use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::{Cartesian, DefaultCartesianAxis};
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::scales::Band;
use datafusion::logical_expr::{col, lit};
// Visual tests for bar charts

#[tokio::test]
async fn test_simple_bar_chart() {
    let df = datasets::simple_categories();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
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
                .axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#4682b4")
            .stroke("#000000")
            .stroke_width(1.0),
    );

    assert_visual_match_default(plot, "bar", "simple_bar_chart").await;
}

#[tokio::test]
async fn test_bar_chart_with_custom_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
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
                .axis(|a: DefaultCartesianAxis| a.title("Category"))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 120.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value"))
            })
            .y2(col("value"))
            .fill("#e74c3c")
            .stroke("#c0392b")
            .stroke_width(2.0),
    );

    assert_visual_match_default(plot, "bar", "bar_chart_custom_colors").await;
}

#[tokio::test]
async fn test_bar_chart_with_narrow_bars() {
    let df = datasets::simple_categories();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
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
                .axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(0.7)) // 70% of band width
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#3498db")
            .stroke("#2980b9")
            .stroke_width(1.5),
    );

    assert_visual_match_default(plot, "bar", "bar_chart_narrow_bars").await;
}

#[tokio::test]
async fn test_bar_chart_inferred_domains() {
    let df = datasets::simple_categories();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#4682b4")
            .stroke("#000000")
            .stroke_width(1.0),
    );

    assert_visual_match_default(plot, "bar", "bar_chart_inferred_domains").await;
}

#[tokio::test]
async fn test_bar_chart_color_case_expression() {
    use datafusion::prelude::*;

    let df = datasets::simple_categories();

    // Create a bar chart where each bar's color depends on its value
    // This demonstrates data-driven color encoding using conditional expressions
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
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
                .axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            // Use conditional expressions to create a gradient effect
            // Colors range from light blue-grey for low values to dark blue for high values
            .fill_with(
                when(col("value").lt(lit(30.0)), lit("#c8d6e5")) // Light blue-grey
                    .when(col("value").lt(lit(50.0)), lit("#8395a7")) // Medium blue-grey
                    .when(col("value").lt(lit(70.0)), lit("#576574")) // Darker blue-grey
                    .when(col("value").lt(lit(85.0)), lit("#2e86ab")) // Blue
                    .otherwise(lit("#0a3d62")) // Dark blue
                    .unwrap(),
                |c| c.no_scale(),
            )
            .stroke("#222222")
            .stroke_width(1.0)
            .opacity(0.9),
    );

    assert_visual_match_default(plot, "bar", "bar_chart_color_case_expression").await;
}
