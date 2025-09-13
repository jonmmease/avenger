//! Legend system for data visualizations
//!
//! This module provides a comprehensive legend system that includes:
//! - Legend specifications and configuration
//! - Legend building utilities
//! - Legend rendering for different mark types
//! - Integration with plot rendering

mod builder;
mod legend_spec;
pub(crate) mod plot_legends;
pub mod renderer;

// Re-export main types
pub use builder::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder,
    OpacityLegendBuilder, ShapeLegendBuilder, SizeLegendBuilder, 
    StrokeDashLegendBuilder, StrokeWidthLegendBuilder
};
pub use legend_spec::{Legend, LegendOrientation, LegendPosition};

// Re-export renderer types
pub use renderer::{
    ChannelInfo, ChannelLegendCapability, ColorbarRenderer, LegendChannel, 
    LegendRenderer, LineLegendRenderer, MergeKey, RectLegendRenderer, 
    SymbolLegendRenderer
};