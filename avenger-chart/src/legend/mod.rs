//! Legend system for data visualizations
//!
//! This module provides a comprehensive legend system that includes:
//! - Legend specifications and configuration
//! - Legend building utilities
//! - Legend rendering for different mark types
//! - Integration with plot rendering

pub mod colorbar_overlay;
pub(crate) mod plot_legends;
pub mod renderer;

// Re-export main types
pub use avenger_chart_core::LegendRendererKind;
pub use avenger_chart_legend::{
    AngleLegendBuilder, ColorLegendBuilder, Legend, LegendBuilder, LegendOrientation,
    LegendPosition, LegendRendererSelection, LegendableChannel, LegendableChannelValue,
    OpacityLegendBuilder, ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder,
    StrokeWidthLegendBuilder,
};
pub use colorbar_overlay::ColorbarOverlay;

// Re-export renderer types
pub use renderer::{
    ChannelInfo, ChannelLegendCapability, CompiledColorbar, CompiledLineLegend, CompiledRectLegend,
    CompiledSymbolLegend, LegendChannel, LegendRenderer, MergeKey, renderer_for_kind,
};
