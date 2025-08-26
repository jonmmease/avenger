use super::datasets;
use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::{Cartesian, DefaultCartesianAxis};
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use avenger_chart::scales::{Band, Linear, Log, Pow, Threshold};
use datafusion::logical_expr::{col, lit};
use palette::rgb::Srgba;
// Visual tests for various scales with color ranges

#[tokio::test]
async fn test_bar_chart_linear_color_interpolation() {
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
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("value"), |c| {
                c.scale_with::<Linear>(|s| {
                    s
                        // Domain will be inferred from data automatically
                        .range_colors(vec![
                            Srgba::new(0.97, 0.96, 0.89, 1.0), // Light cream (#f8f5e4)
                            Srgba::new(0.96, 0.64, 0.38, 1.0), // Light orange (#f5a462)
                            Srgba::new(0.84, 0.19, 0.11, 1.0), // Dark red (#d6301d)
                        ])
                })
            }) // Map value through the linear color scale
            .stroke("#333333")
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

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("value"), |c| {
                c.scale_with::<Log>(|s| {
                    s.base(10.0)
                        // Domain will be inferred from data automatically
                        .range_colors(vec![
                            Srgba::new(0.99, 0.99, 0.87, 1.0), // Light yellow (#fffde4)
                            Srgba::new(0.42, 0.69, 0.45, 1.0), // Medium green (#6bb074)
                            Srgba::new(0.00, 0.27, 0.21, 1.0), // Dark green (#004534)
                        ])
                })
            }) // Map value through the log color scale
            .stroke("#222222")
            .stroke_width(0.5)
            .opacity(0.95),
    );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_log_color_interpolation").await;
}

#[tokio::test]
async fn test_bar_chart_pow_color_interpolation() {
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
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("value"), |c| {
                c.scale_with::<Pow>(|s| {
                    s.exponent(2.0)
                        // Domain will be inferred from data automatically
                        .range_colors(vec![
                            Srgba::new(0.94, 0.91, 0.96, 1.0), // Light purple (#f0e8f5)
                            Srgba::new(0.61, 0.31, 0.64, 1.0), // Medium purple (#9b4fa3)
                            Srgba::new(0.25, 0.00, 0.29, 1.0), // Dark purple (#3f004a)
                        ])
                })
            }) // Map value through the power color scale
            .stroke("#333333")
            .stroke_width(0.5)
            .opacity(0.95),
    );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_pow_color_interpolation").await;
}

#[tokio::test]
async fn test_bar_chart_sqrt_color_interpolation() {
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
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("value"), |c| {
                c.scale_with::<Pow>(|s| {
                    s.exponent(0.5)
                        // Domain will be inferred from data automatically
                        .range_colors(vec![
                            Srgba::new(0.97, 0.91, 0.81, 1.0), // Light tan (#f8e8cf)
                            Srgba::new(0.94, 0.60, 0.15, 1.0), // Orange (#f09a27)
                            Srgba::new(0.58, 0.21, 0.05, 1.0), // Dark brown (#943508)
                        ])
                })
            }) // Map value through the sqrt color scale
            .stroke("#222222")
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
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("value"), |c| {
                c.scale_with::<Threshold>(|s| {
                    s.domain_discrete(vec![lit(30.0f32), lit(50.0f32), lit(70.0f32), lit(85.0f32)])
                        .range_discrete(vec![
                            "#c8d6e5", // Light blue-grey (< 30)
                            "#8395a7", // Medium blue-grey (30-50)
                            "#576574", // Darker blue-grey (50-70)
                            "#2e86ab", // Blue (70-85)
                            "#0a3d62", // Dark blue (> 85)
                        ])
                })
            }) // Map value through the threshold scale
            .stroke("#222222")
            .stroke_width(1.0)
            .opacity(0.9),
    );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_threshold_scale_colors").await;
}

#[tokio::test]
async fn test_bar_chart_linear_color_default_colors() {
    let df = datasets::simple_categories();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
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

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s)
                    .axis(|a: DefaultCartesianAxis| a.title("Category").grid(false))
            }) // Domain will be inferred from data
            .x2_with(col("category"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale(|s| s.domain((0.0, 100.0)))
                    .axis(|a: DefaultCartesianAxis| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill(col("category")) // Map fill to category column
            .stroke("#222222")
            .stroke_width(1.0),
    );

    assert_visual_match_default(plot, "bar_scale_color", "bar_chart_ordinal_scale_colors").await;
}
