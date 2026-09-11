use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::{
    axis::{numeric::make_numeric_axis_marks_with_text_engine, opts::AxisConfig},
    legend::symbol::{make_symbol_legend_with_text_engine, SymbolLegendConfig},
};
use avenger_scales::scales::linear::LinearScale;
use avenger_text::{FontResolutionOptions, TextEngine};

#[test]
fn guide_bounds_use_the_supplied_font_context() {
    let context = |family: &str| {
        TextEngine::with_font_resolution(&FontResolutionOptions {
            load_system_fonts: false,
            default_sans_serif_family: Some(family.to_string()),
            ..avenger_text::default_font_resolution()
        })
        .unwrap()
    };
    let proportional = context("Lato");
    let monospace = context("DejaVu Sans Mono");
    let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    let title = "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii";
    let axis = |engine: &TextEngine| {
        make_numeric_axis_marks_with_text_engine(
            &scale,
            title,
            [0.0, 0.0],
            &AxisConfig::default(),
            engine,
        )
        .unwrap()
        .bounding_box_with_text_engine(engine)
    };
    assert_ne!(axis(&proportional), axis(&monospace));
    let config = SymbolLegendConfig {
        text: title.to_string().into(),
        ..Default::default()
    };
    let legend = |engine: &TextEngine| {
        make_symbol_legend_with_text_engine(&config, engine)
            .unwrap()
            .bounding_box_with_text_engine(engine)
    };
    assert_ne!(legend(&proportional), legend(&monospace));
}
