//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

mod coordinate_guide;
mod no_guide;
mod overflow;

pub use avenger_chart_core::{GuideContext, GuideOverflowPhase, GuideUpdate};
pub use coordinate_guide::{CompiledGuide, CoordinateGuide, GuideSharingContext};
pub use no_guide::NoGuide;
pub use overflow::{MeasurementResult, OverflowSpaceRequirement, spacing_keys};
