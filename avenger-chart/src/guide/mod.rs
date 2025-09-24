//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

mod coordinate_guide;
mod no_guide;
mod overflow;

pub use no_guide::NoGuide;
pub use overflow::OverflowSpaceRequirement;
pub use coordinate_guide::{CoordinateGuideRender, CoordinateGuideBuilder};

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
