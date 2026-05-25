//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

pub use avenger_chart_core::overflow::spacing_keys;
pub(crate) use avenger_chart_core::{
    AxisOwnershipMode, ChildFrameGuideSharingView, FacetGuideSharingView,
};
pub use avenger_chart_core::{
    AxisVisibility, CompiledGuide, CoordinateGuide, GuideContext, GuideOverflowPhase,
    GuideSharingContext, GuideUpdate, MeasurementResult, NoGuide, OverflowSpaceRequirement,
};
