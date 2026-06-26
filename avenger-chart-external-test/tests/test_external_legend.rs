//! Integration tests for core-only custom legend renderer imports.

use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{
    ChannelInfo, Legend, LegendChannel, LegendRenderer, LegendRendererSelection, Mark, Maybe,
    Size2D, Theme,
};
use avenger_chart_external_test::external_mark::{HexBin, HexBinLegendRenderer};
use avenger_scales::scales::linear::LinearScale;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

#[test]
fn core_legend_spec_can_be_authored_without_legend_crate() {
    let legend = Legend::new()
        .title("Species")
        .position("right")
        .columns(2)
        .gradient_thickness(12);

    assert!(matches!(legend.title, Maybe::Set(Some(_))));
    assert!(matches!(legend.position, Maybe::Set(Some(_))));
    assert!(matches!(legend.columns, Maybe::Set(Some(_))));
    assert!(matches!(legend.gradient_thickness, Maybe::Set(Some(_))));
}

#[tokio::test]
async fn custom_legend_renderer_uses_core_contracts() {
    let renderer = HexBinLegendRenderer;
    let channel = LegendChannel {
        name: "fill".to_string(),
        expression: None,
        scale: LinearScale::configured((0.0, 1.0), (0.0, 1.0)),
        channel_type: "fill".to_string(),
        sharing_level: None,
        mark_type: "hexbin".to_string(),
        mark_index: 0,
        related_channels: std::collections::HashMap::<String, ChannelInfo>::new(),
    };

    assert!(renderer.can_evaluate(std::slice::from_ref(&channel)));

    let text_measurer = avenger_text::measurement::default_text_measurer();
    let size = renderer
        .measure(
            &[channel],
            &Legend::new(),
            Size2D {
                width: 100.0,
                height: 100.0,
            },
            &Theme::light(),
            &IndexMap::new(),
            &SessionContext::new(),
            &text_measurer,
        )
        .await
        .expect("custom renderer measures");

    assert_eq!(size.width, 24.0);
    assert_eq!(size.height, 18.0);
}

#[tokio::test]
async fn external_mark_can_return_custom_legend_renderer_selection() {
    let compiled = HexBin::<Cartesian>::new()
        .compile_untransformed(&SessionContext::new())
        .await
        .expect("compile external mark");
    let scale = LinearScale::configured((0.0, 1.0), (0.0, 1.0));

    let selection = compiled
        .preferred_legend_renderer("fill", &scale)
        .expect("hexbin fill legend renderer");

    assert!(matches!(selection, LegendRendererSelection::Custom(_)));
}
