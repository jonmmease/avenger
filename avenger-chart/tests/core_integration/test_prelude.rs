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
    let polar_line = Line::<Polar>::new()
        .r(1.0)
        .theta(0.0)
        .geometry_space(GeometrySpace::Display);
    let polar_text = Text::<Polar>::new()
        .r(1.0)
        .theta(0.0)
        .text("label")
        .geometry_space(GeometrySpace::Coordinate);
    let _rect = Rect::<Cartesian>::new();
    assert_eq!(
        polar_line.state().geometry_space,
        Some(GeometrySpace::Display)
    );
    assert_eq!(
        polar_text.state().geometry_space,
        Some(GeometrySpace::Coordinate)
    );

    // Plot
    let _plot = Plot::<Cartesian>::new();
    #[cfg(feature = "pdf")]
    let _pdf_renderer = PdfRenderer::new();
    #[cfg(feature = "svg")]
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
    let _geometry_space = GeometrySpace::Coordinate;

    // Legend builders
    let _legend = ColorLegendBuilder::new()
        .title("My Legend")
        .gradient_thickness(15.0);

    // Test passes if compilation succeeds
}

#[test]
fn polar_text_prelude_imports() {
    use avenger_chart::prelude::*;

    let polar_text = Text::<Polar>::new()
        .r(1.0)
        .theta(0.0)
        .text("label")
        .geometry_space(GeometrySpace::Coordinate);

    assert_eq!(
        polar_text.state().geometry_space,
        Some(GeometrySpace::Coordinate)
    );
}
