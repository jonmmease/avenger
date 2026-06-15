//! Internal child-frame layout primitives.
//!
//! The layout system is a core Avenger feature. Built-in facet, concat, and
//! coordinate-positioned subplot paths measure child plots as frames, then
//! expose a read-only child-frame view for core layout, guide sharing, debug
//! overlays, and rendering.
//!
//! This module is hidden from public docs on purpose. It is not a general
//! external layout-container API; external coordinate-system crates should use
//! the core `SubplotContainerCoordinateSystem` hook when they need to compile
//! `Subplot` marks.
//!
//! Core invariants:
//!
//! - child indices are unique within one container measurement,
//! - every exposed child frame has exactly one solved region,
//! - region content rectangles are child plot areas in the parent plot-area
//!   coordinate space,
//! - projected child overflow is computed from child frame bounds after each
//!   child-local frame is translated into the parent frame.

#[doc(hidden)]
pub use crate::plot::compiled::ChildFrameContainerView;

pub(crate) use crate::{
    layout::{BoundaryDemand, EdgeTargets},
    plot::compiled::{
        ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel, ChildFrameSharingPath,
        ContainerPathSegment,
    },
};
