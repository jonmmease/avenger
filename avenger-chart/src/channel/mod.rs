//! Channel system for data visualizations
//!
//! This module provides a comprehensive channel system that includes:
//! - Channel values and expressions
//! - Channel configuration traits and implementations
//! - Channel resolution for references
//! - Position channel specializations
//! - Macros for channel definitions

pub mod config_traits;
mod configs;
mod descriptor;
#[macro_use]
mod macros;
mod position;
#[macro_use]
mod position_macros;
pub(crate) mod resolution;
pub mod value;

// Re-export main types
pub use self::configs::{
    AngleChannelConfig, ColorChannelConfig, OpacityChannelConfig, ShapeChannelConfig,
    SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
pub use self::descriptor::{ChannelDefault, ChannelDescriptor};
pub use self::position::GenericPositionConfig;
pub use self::value::{BaseChannelName, ChannelValue, ConditionalValue};
pub use crate::chart_core::{ChannelConfig, PositionConfig};
pub use crate::legend::LegendableChannel;
