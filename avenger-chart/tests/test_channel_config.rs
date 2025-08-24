// Test that channel-level scale and legend configuration works

use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::scales::Ordinal;
use datafusion::logical_expr::{col, lit};
use palette::Srgba;

#[test]
fn test_channel_scale_config() {
    // Test that we can configure scales directly on channel values
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("category").scale(|s| {
                s.range_colors(vec![
                    Srgba::new(1.0, 0.498, 0.0, 1.0),     // Orange
                    Srgba::new(0.122, 0.467, 0.706, 1.0), // Blue
                    Srgba::new(0.173, 0.627, 0.173, 1.0), // Green
                ])
            }))
            .size(col("value").scale(|s| s.range_interval(lit(10.0), lit(100.0)))),
    );

    // The scale configs should be extracted and stored in the plot
    assert!(plot.scale_specs().contains_key("fill"));
    assert!(plot.scale_specs().contains_key("size"));
}

#[test]
fn test_channel_legend_config() {
    // Test that we can configure legends directly on channel values
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("category").legend(|l| l.title("Category").visible(true)))
            .size(col("value").legend(|l| l.visible(false))),
    );

    // The legend configs should be extracted and stored in the plot
    assert!(plot.legends().contains_key("fill"));
    assert_eq!(plot.legends()["fill"].title, Some("Category".to_string()));
    assert!(plot.legends()["fill"].visible);

    assert!(plot.legends().contains_key("size"));
    assert!(!plot.legends()["size"].visible);
}

#[test]
fn test_channel_scale_with_typed() {
    // Test that we can use typed scales on channel values
    let plot = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")).fill(
        col("category").scale_with::<Ordinal, _>(|s| {
            s.range_colors(vec![
                Srgba::new(0.984, 0.706, 0.682, 1.0), // Light pink
                Srgba::new(0.702, 0.804, 0.890, 1.0), // Light blue
                Srgba::new(0.800, 0.922, 0.773, 1.0), // Light green
            ])
        }),
    ));

    // The scale config should be extracted
    assert!(plot.scale_specs().contains_key("fill"));
}

#[test]
fn test_channel_and_plot_config_coexist() {
    // Test that plot-level and channel-level configs can coexist
    let plot = Plot::<Cartesian>::new()
        .scale("x", |s| s.domain((0.0, 100.0))) // Plot-level config
        .legend("x", |l| l.visible(false)) // Plot-level config
        .mark(
            Symbol::new()
                .x(col("x")) // No channel config, uses plot config
                .y(col("y")
                    .scale(|s| s.domain((0.0, 50.0))) // Channel-level config
                    .legend(|l| l.title("Y Values"))) // Channel-level config
                .fill(col("category")),
        );

    // Both plot-level and channel-level configs should exist
    assert!(plot.scale_specs().contains_key("x")); // From plot
    assert!(plot.scale_specs().contains_key("y")); // From channel

    assert!(plot.legends().contains_key("x")); // From plot
    assert!(!plot.legends()["x"].visible);

    assert!(plot.legends().contains_key("y")); // From channel
    assert_eq!(plot.legends()["y"].title, Some("Y Values".to_string()));
}

#[test]
fn test_no_legend_helper() {
    // Test the no_legend helper method
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("category").no_legend()),
    );

    // The legend should be created but marked as not visible
    assert!(plot.legends().contains_key("fill"));
    assert!(!plot.legends()["fill"].visible);
}

#[test]
fn test_direct_expr_scale_config() {
    // Test that we can call scale() directly on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x").scale(|s| s.domain((0.0, 100.0))))
            .y(col("y").scale(|s| s.domain((0.0, 50.0))))
            .fill(col("category").scale(|s| {
                s.range_colors(vec![
                    Srgba::new(1.0, 0.0, 0.0, 1.0), // Red
                    Srgba::new(0.0, 1.0, 0.0, 1.0), // Green
                    Srgba::new(0.0, 0.0, 1.0, 1.0), // Blue
                ])
            })),
    );

    // The scale configs should be extracted and stored
    assert!(plot.scale_specs().contains_key("x"));
    assert!(plot.scale_specs().contains_key("y"));
    assert!(plot.scale_specs().contains_key("fill"));
}

#[test]
fn test_direct_expr_legend_config() {
    // Test that we can call legend() directly on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("category").legend(|l| l.title("Category").visible(true)))
            .size(col("value").no_legend()),
    );

    // The legend configs should be extracted
    assert!(plot.legends().contains_key("fill"));
    assert_eq!(plot.legends()["fill"].title, Some("Category".to_string()));
    assert!(plot.legends()["fill"].visible);

    assert!(plot.legends().contains_key("size"));
    assert!(!plot.legends()["size"].visible);
}

#[test]
fn test_direct_expr_combined_config() {
    // Test combining scale and legend config on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y")
                .scale(|s| s.domain((0.0, 100.0)))
                .legend(|l| l.title("Y Values")))
            .fill(
                col("category")
                    .scale_with::<Ordinal, _>(|s| {
                        s.range_colors(vec![
                            Srgba::new(1.0, 0.5, 0.0, 1.0), // Orange
                        ])
                    })
                    .legend(|l| l.visible(false)),
            ),
    );

    // Check that both scale and legend configs are present
    assert!(plot.scale_specs().contains_key("y"));
    assert!(plot.legends().contains_key("y"));
    assert_eq!(plot.legends()["y"].title, Some("Y Values".to_string()));

    assert!(plot.scale_specs().contains_key("fill"));
    assert!(plot.legends().contains_key("fill"));
    assert!(!plot.legends()["fill"].visible);
}

#[test]
fn test_band_with_scale_config() {
    // Test that band() and scale() can be chained on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("category")
                .band(0.5)
                .scale(|s| s.domain_discrete(vec![lit("A"), lit("B"), lit("C")])))
            .y(col("value").scale(|s| s.domain((0.0, 100.0)))),
    );

    // The scale configs should be extracted
    assert!(plot.scale_specs().contains_key("x"));
    assert!(plot.scale_specs().contains_key("y"));
}

#[test]
fn test_identity_unscaled() {
    // Test that identity() creates unscaled values
    // Note: we need to add scale config to make it show up in scale_specs
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x").scale(|s| s)) // Scaled by default, with config
            .y(lit(50.0).identity()) // Explicitly unscaled
            .fill("red".identity()),
    ); // String literal unscaled

    // Only x should have a scale config
    assert!(plot.scale_specs().contains_key("x"));
    assert!(!plot.scale_specs().contains_key("y"));
    assert!(!plot.scale_specs().contains_key("fill"));
}
