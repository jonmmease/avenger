//! Channel system for data visualizations
//!
//! This module provides a comprehensive channel system that includes:
//! - Channel values and expressions
//! - Channel configuration traits and implementations
//! - Channel resolution for references
//! - Position channel specializations
//! - Macros for channel definitions

pub mod config_traits;
pub(crate) mod resolution;
pub mod value;

// Re-export main types
pub use avenger_chart_core::{
    AngleChannelConfig, BaseChannelName, ChannelConfig, ChannelDefault, ChannelDescriptor,
    ChannelValue, ColorChannelConfig, ConditionalValue, GenericPositionConfig,
    OpacityChannelConfig, PatternChannelValue, PositionConfig, ShapeChannelConfig,
    SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
pub use avenger_chart_legend::LegendableChannel;
