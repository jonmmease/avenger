pub mod builder;
pub mod channel_config;
pub mod layout;
pub mod renderer;
mod theme;

pub use avenger_chart_core::{Legend, LegendOrientation, LegendPosition, LegendRendererKind};
pub use builder::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder, OpacityLegendBuilder,
    ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};
pub use channel_config::{LegendableChannel, LegendableChannelValue};
pub use layout::measure_legend_size_with_channels;
pub use renderer::{
    ChannelInfo, ChannelLegendCapability, CompiledColorbar, CompiledLineLegend, CompiledRectLegend,
    CompiledSymbolLegend, LegendChannel, LegendRenderer, MergeKey, renderer_for_kind,
};
pub use theme::{apply_legend_theme_defaults, themed_default_legend};
