//! Integration tests for direct legend authoring imports.
//!
//! These tests intentionally depend on `avenger-chart-core` and
//! `avenger-chart-legend` directly, rather than using the top-level
//! `avenger-chart` facade for legend builder APIs.

use avenger_chart_core::{ChannelConfig, ChannelValue, ColorChannelConfig, Maybe};
use avenger_chart_legend::{
    renderer_for_kind, ColorLegendBuilder, CompiledColorbar, LegendBuilder, LegendRenderer,
    LegendRendererKind, LegendableChannel, LegendableChannelValue,
};
use datafusion::prelude::col;

#[test]
fn direct_legend_builder_configures_core_legend_spec() {
    let legend = ColorLegendBuilder::new()
        .title("Species")
        .position("right")
        .columns(2)
        .gradient_thickness(12)
        .build();

    assert!(matches!(legend.title, Maybe::Set(Some(_))));
    assert!(matches!(legend.position, Maybe::Set(Some(_))));
    assert!(matches!(legend.columns, Maybe::Set(Some(_))));
    assert!(matches!(legend.gradient_thickness, Maybe::Set(Some(_))));
}

#[test]
fn direct_legend_trait_configures_core_channel_config() {
    let channel = ColorChannelConfig::new(col("species").into())
        .legend(|legend| legend.title("Species").position("right"))
        .into_inner();

    let legend = channel
        .get_legend_config()
        .expect("legend config should be attached to the scaled channel");

    assert!(matches!(legend.title, Maybe::Set(Some(_))));
    assert!(matches!(legend.position, Maybe::Set(Some(_))));
}

#[test]
fn direct_legend_value_extension_configures_core_channel_value() {
    let legend = ColorLegendBuilder::new().visible(false).build();
    let channel = ChannelValue::from(col("species")).legend(legend);

    assert!(channel.has_legend_config());
}

#[test]
fn direct_legend_renderer_imports_resolve_from_legend_crate() {
    let renderer = renderer_for_kind(LegendRendererKind::Symbol);
    assert_eq!(renderer.name(), "CompiledSymbolLegend");

    let colorbar = CompiledColorbar::new();
    assert!(colorbar.prefers_flexible_layout());
}
