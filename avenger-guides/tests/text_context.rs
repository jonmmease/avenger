use avenger_format::NumberFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::marks::TextGeometryUtils;
use avenger_guides::{
    axis::{
        continuous::make_continuous_axis_marks,
        opts::{AxisConfig, AxisOrientation},
    },
    legend::symbol::{make_symbol_legend, SymbolLegendConfig},
};
use avenger_scales::scales::linear::LinearScale;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine};

#[test]
fn guide_bounds_use_the_supplied_font_context() {
    let context = |family: &str| {
        LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                default_sans_serif_family: Some(family.to_string()),
                ..avenger_typst_label::bundled_font_options()
            },
        })
    };
    let proportional = context("Lato");
    let monospace = context("DejaVu Sans Mono");
    let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    let config = AxisConfig {
        orientation: AxisOrientation::Bottom,
        dimensions: [100.0, 100.0],
        grid: false,
        format: D3NumberFormatProvider::new().prepare(",").unwrap().into(),
        style: Default::default(),
    };
    let title = "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii";
    let axis = |engine: &LabelEngine| {
        make_continuous_axis_marks(&scale, title, [0.0, 0.0], &config, engine)
            .unwrap()
            .bounding_box(engine)
    };
    assert_ne!(axis(&proportional), axis(&monospace));
    let config = SymbolLegendConfig {
        text: title.to_string().into(),
        ..Default::default()
    };
    let legend = |engine: &LabelEngine| {
        make_symbol_legend(&config, engine)
            .unwrap()
            .bounding_box(engine)
    };
    assert_ne!(legend(&proportional), legend(&monospace));
}
