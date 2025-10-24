//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

mod coordinate_guide;
mod no_guide;
mod overflow;

pub use coordinate_guide::{CompiledGuide, CoordinateGuide, FacetDirection, UnifiableChannelInfo};
pub use no_guide::NoGuide;
pub use overflow::OverflowSpaceRequirement;

use std::collections::HashSet;

/// Context passed to guides during rendering and measurement
///
/// This separates internal guide state from user-provided parameters,
/// avoiding pollution of the params namespace.
#[derive(Debug, Clone, Default)]
pub struct GuideContext {
    /// Channels whose axes should be suppressed (hidden) during rendering.
    /// Used by faceting to suppress inner subplot axes that are unified at the outer level.
    pub suppressed_axes: HashSet<String>,
}

impl GuideContext {
    /// Create an empty context
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context that suppresses specific axis channels
    pub fn with_suppressed_axes(channels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            suppressed_axes: channels.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Check if an axis channel should be suppressed
    pub fn is_axis_suppressed(&self, channel: &str) -> bool {
        self.suppressed_axes.contains(channel)
    }
}

/// Trait for composable guide updates
///
/// This trait allows guides to be composed by updating one with another,
/// similar to how axes can be composed using the AxisUpdate trait.
pub trait GuideUpdate: Clone + Default {
    /// Update this guide with values from another guide
    ///
    /// Values from `other` take precedence over values in `self`.
    fn update(self, other: Self) -> Self;
}
