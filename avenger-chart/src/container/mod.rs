//! Child-frame container extension primitives.
//!
//! Container coordinate systems are coordinate systems whose marks are child
//! plots. Facets, horizontal/vertical concat, and coordinate-positioned
//! subplots all measure those child plots as frames, then expose a read-only
//! child-frame container view for layout, guide sharing, debug overlays, and
//! rendering.
//!
//! This module is the intended home for that vocabulary. Most items are still
//! crate-private while the extension boundary settles; external containers
//! should eventually depend on this module rather than on `plot::compiled`.
//!
//! Core invariants:
//!
//! - child indices are unique within one container measurement,
//! - every exposed child frame has exactly one render placement,
//! - render-placement origins are child frame origins, not child plot-area
//!   origins,
//! - placement content size is expressed in the parent plot-area coordinate
//!   space,
//! - projected child overflow is computed from child frame bounds after each
//!   child-local frame is translated into the parent frame.

#[doc(hidden)]
pub use crate::plot::compiled::ChildFrameContainerView;

pub(crate) use crate::{
    layout::{
        BandChildFrameInput, BandChildFramePlacement, BandSpacing, BoundaryDemand1D,
        ChildFramePlacementResult, ChildFrameRenderPlacement, project_child_frame_bounds,
    },
    plot::compiled::{
        ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel, ChildFrameSharingPath,
        ContainerPathSegment,
    },
};
