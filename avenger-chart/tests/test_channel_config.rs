// Test that channel-level scale and legend configuration works

use avenger_chart::prelude::*;
use datafusion::prelude::SessionContext;
use palette::Srgba;

// Helper macro to compile plot and extract configs
macro_rules! compile_and_check {
    ($plot:expr) => {{
        let ctx = SessionContext::new();
        $plot.compile(&ctx).await.unwrap()
    }};
}

#[tokio::test]
async fn test_channel_scale_config() {
    // Test that we can configure scales directly on channel values
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.scale(|s| {
                    s.range_colors(vec![
                        Srgba::new(1.0, 0.498, 0.0, 1.0),     // Orange
                        Srgba::new(0.122, 0.467, 0.706, 1.0), // Blue
                        Srgba::new(0.173, 0.627, 0.173, 1.0), // Green
                    ])
                })
            })
            .size_with(col("value"), |c| {
                c.scale(|s| s.range_interval(lit(10.0), lit(100.0)))
            }),
    );

    let compiled = compile_and_check!(plot);

    // The scale configs should be extracted and stored in the compiled plot
    assert!(compiled.scale_specs().contains_key("fill"));
    assert!(compiled.scale_specs().contains_key("size"));
}

#[tokio::test]
async fn test_channel_legend_config() {
    // Test that we can configure legends directly on channel values
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.legend(|l| l.title("Category").visible(true))
            })
            .size_with(col("value"), |c| c.legend(|l| l.visible(false))),
    );

    let compiled = compile_and_check!(plot);

    // The legend configs should be extracted and stored in the compiled plot
    assert!(compiled.legends().contains_key("fill"));
    // Check that fields are set (values are now LogicalExprNode)
    assert!(compiled.legends()["fill"].title.is_set());
    assert!(compiled.legends()["fill"].visible.is_set());

    assert!(compiled.legends().contains_key("size"));
    assert!(compiled.legends()["size"].visible.is_set());
}

#[tokio::test]
async fn test_channel_scale_with_typed() {
    // Test that we can use typed scales on channel values
    let plot = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")).fill_with(
        col("category"),
        |c| {
            c.scale_with::<Ordinal>(|s| {
                s.range_colors(vec![
                    Srgba::new(0.984, 0.706, 0.682, 1.0), // Light pink
                    Srgba::new(0.702, 0.804, 0.890, 1.0), // Light blue
                    Srgba::new(0.800, 0.922, 0.773, 1.0), // Light green
                ])
            })
        },
    ));

    let compiled = compile_and_check!(plot);

    // The scale config should be extracted
    assert!(compiled.scale_specs().contains_key("fill"));
}

#[tokio::test]
async fn test_channel_config() {
    // Test that channel-level configs work properly
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 100.0)))) // Channel-level config
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 50.0)))) // Channel-level config
            // Note: Position channels don't have legends, they have axes
            .fill_with(col("category"), |c| {
                c.scale(|s| s) // Configure scale to ensure it gets created
                    .legend(|l| l.visible(false))
            }),
    );

    let compiled = compile_and_check!(plot);

    // Channel-level configs should exist
    assert!(compiled.scale_specs().contains_key("x")); // From channel
    assert!(compiled.scale_specs().contains_key("y")); // From channel
    assert!(compiled.scale_specs().contains_key("fill")); // From channel with scale config

    assert!(compiled.legends().contains_key("fill")); // From channel
    assert!(compiled.legends()["fill"].visible.is_set());

    // Position channels don't have legends, they have axes
    assert!(!compiled.legends().contains_key("x"));
    assert!(!compiled.legends().contains_key("y"));
}

#[tokio::test]
async fn test_no_legend_helper() {
    // Test the no_legend helper method
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| c.no_legend()),
    );

    let compiled = compile_and_check!(plot);

    // The legend should be created but marked as not visible
    assert!(compiled.legends().contains_key("fill"));
    assert!(compiled.legends()["fill"].visible.is_set());
}

#[tokio::test]
async fn test_direct_expr_scale_config() {
    // Test that we can call scale() directly on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 100.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 50.0))))
            .fill_with(col("category"), |c| {
                c.scale(|s| {
                    s.range_colors(vec![
                        Srgba::new(1.0, 0.0, 0.0, 1.0), // Red
                        Srgba::new(0.0, 1.0, 0.0, 1.0), // Green
                        Srgba::new(0.0, 0.0, 1.0, 1.0), // Blue
                    ])
                })
            }),
    );

    let compiled = compile_and_check!(plot);

    // The scale configs should be extracted and stored
    assert!(compiled.scale_specs().contains_key("x"));
    assert!(compiled.scale_specs().contains_key("y"));
    assert!(compiled.scale_specs().contains_key("fill"));
}

#[tokio::test]
async fn test_direct_expr_legend_config() {
    // Test that we can call legend() directly on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.legend(|l| l.title("Category").visible(true))
            })
            .size_with(col("value"), |c| c.no_legend()),
    );

    let compiled = compile_and_check!(plot);

    // The legend configs should be extracted
    assert!(compiled.legends().contains_key("fill"));
    assert!(compiled.legends()["fill"].title.is_set());
    assert!(compiled.legends()["fill"].visible.is_set());

    assert!(compiled.legends().contains_key("size"));
    assert!(compiled.legends()["size"].visible.is_set());
}

#[tokio::test]
async fn test_direct_expr_combined_config() {
    // Test combining scale and legend config on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 100.0))))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| {
                    s.range_colors(vec![
                        Srgba::new(1.0, 0.5, 0.0, 1.0), // Orange
                    ])
                })
                .legend(|l| l.visible(false))
            }),
    );

    let compiled = compile_and_check!(plot);

    // Check that scale config is present for y
    assert!(compiled.scale_specs().contains_key("y"));
    // Position channels don't have legends
    assert!(!compiled.legends().contains_key("y"));

    assert!(compiled.scale_specs().contains_key("fill"));
    assert!(compiled.legends().contains_key("fill"));
    assert!(compiled.legends()["fill"].visible.is_set());
}

#[tokio::test]
async fn test_band_with_scale_config() {
    // Test that band() and scale() can be chained on expressions
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("category"), |c| {
                c.band(0.5)
                    .scale(|s| s.domain_discrete(vec![lit("A"), lit("B"), lit("C")]))
            })
            .y_with(col("value"), |c| c.scale(|s| s.domain((0.0, 100.0)))),
    );

    let compiled = compile_and_check!(plot);

    // The scale configs should be extracted
    assert!(compiled.scale_specs().contains_key("x"));
    assert!(compiled.scale_specs().contains_key("y"));
}

#[tokio::test]
async fn test_path_with_no_scale_and_scaled_config() {
    let raw_path_plot = Plot::<Cartesian>::new().mark(
        PathMark::new()
            .x(col("x"))
            .y(col("y"))
            .path_with(col("svg_path"), |c| c.no_scale()),
    );

    let raw_compiled = compile_and_check!(raw_path_plot);
    assert!(!raw_compiled.scale_specs().contains_key("path"));

    let scaled_path_plot =
        Plot::<Cartesian>::new().mark(PathMark::new().x(col("x")).y(col("y")).path_with(
            col("path_kind"),
            |c| {
                c.scale_with::<Ordinal>(|s| {
                    s.domain_discrete(vec![lit("triangle"), lit("diamond")])
                        .range_discrete(vec![
                            "M -8 -8 L 8 -8 L 0 8 Z",
                            "M 0 -10 L 10 0 L 0 10 L -10 0 Z",
                        ])
                })
            },
        ));

    let scaled_compiled = compile_and_check!(scaled_path_plot);
    assert!(scaled_compiled.scale_specs().contains_key("path"));
}

#[tokio::test]
async fn test_identity_unscaled() {
    // Test that identity() creates unscaled values
    // Note: we need to add scale config to make it show up in scale_specs
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s)) // Scaled by default, with config
            .y(lit(50.0)) // Explicitly unscaled
            .fill("red"),
    ); // String literal unscaled

    let compiled = compile_and_check!(plot);

    // Only x should have a scale config
    assert!(compiled.scale_specs().contains_key("x"));
    assert!(!compiled.scale_specs().contains_key("y"));
    assert!(!compiled.scale_specs().contains_key("fill"));
}
