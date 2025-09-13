//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

mod coordinate_guide;
mod no_guide;
mod overflow;

pub use coordinate_guide::CoordinateGuide;
pub use no_guide::NoGuide;
pub use overflow::OverflowSpaceRequirement;