//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

mod overflow;

pub(crate) use avenger_chart_core::{
    AxisOwnershipMode, ChildFrameGuideSharingView, FacetGuideSharingView,
};
pub use avenger_chart_core::{
    AxisVisibility, CompiledGuide, CoordinateGuide, GuideContext, GuideOverflowPhase,
    GuideSharingContext, GuideUpdate, NoGuide,
};
pub use overflow::{MeasurementResult, OverflowSpaceRequirement, spacing_keys};
