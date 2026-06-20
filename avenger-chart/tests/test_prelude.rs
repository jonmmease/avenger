#[test]
fn test_prelude_imports() {
    // This test verifies that all expected items are available from the prelude
    use avenger_chart::prelude::*;

    // Coordinate systems
    let _cartesian: Cartesian;
    let _polar: Polar;
    let _zerod: ZeroDCoord;

    // Marks
    let _symbol = Symbol::<Cartesian>::new();
    let _line = Line::<Cartesian>::new();
    let _rect = Rect::<Cartesian>::new();

    // Plot
    let _plot = Plot::<Cartesian>::new();
    let _svg_renderer = SvgRenderer::new();

    // Channel value expressions
    let _col = col("x");
    let _lit = lit(42);

    // Channel configs with trait methods
    let config = ColorChannelConfig::new(col("color").into());
    let _configured = config
        .scale(|s| s.domain((0.0, 100.0)))
        .legend(|l| l.title("Color Legend"));

    // Scales
    let _linear = Scale::<Linear>::new();
    let _band = Scale::<Band>::new();
    let _nested_band = Scale::<NestedBand>::new();
    let _nested_spec = NestedBandSpec::default();
    let _nested_level = NestedBandLevelSpec::default();
    let _nest_scope = NestScope::Shared;
    let _position_boundary = PositionBoundary::level_band(0, 0.5);
    let _nested_expr = nested(["quarter", "team"]);

    // Legend builders
    let _legend = ColorLegendBuilder::new()
        .title("My Legend")
        .gradient_thickness(15.0);

    // Test passes if compilation succeeds
}
