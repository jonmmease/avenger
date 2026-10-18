use std::sync::Arc;

use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geometry::marks::TextGeometryUtils;
use avenger_guides::{
    axis::{Axis, AxisConfig},
    legend::{Legend, LegendConfig},
    GuideSide,
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
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    };
    let proportional = context("Lato");
    let monospace = context("DejaVu Sans Mono");
    let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
    let config = AxisConfig {
        side: GuideSide::Bottom,
        tick_format: ",".into(),
        tick_label: None,
        style: Default::default(),
    };
    let title = "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii";
    let axis = |engine: &LabelEngine| {
        Axis::continuous(&scale, title, &config, engine)
            .unwrap()
            .marks([100.0, 100.0])
            .bounding_box(engine)
    };
    assert_ne!(axis(&proportional), axis(&monospace));
    let config = LegendConfig {
        text: title.to_string().into(),
        ..Default::default()
    };
    let legend = |engine: &LabelEngine| {
        Legend::new(&config, engine)
            .unwrap()
            .marks(500.0)
            .group
            .bounding_box(engine)
    };
    assert_ne!(legend(&proportional), legend(&monospace));
}
