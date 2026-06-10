//! Chart-independent layout primitives, solvers, and diagnostics.
//!
//! # The content-plus-overflow model
//!
//! Every laid-out region is a **content rectangle** plus layered **edge
//! demand** on each side:
//!
//! - `inner`: interior chrome between the content rectangle and any outer
//!   content (for charts: axis ticks and labels),
//! - `outer`: content that stacks beyond the inner edge (for charts: legends,
//!   container labels),
//! - `total`: the full rendered envelope on that side
//!   (`total >= inner + outer`).
//!
//! When regions are arranged along an axis, **interior** boundary edges
//! become inter-track gaps, while the **first leading** and **last trailing**
//! edges are excluded from the arrangement's content extent: they overlap the
//! container's own edge overflow instead of consuming interior space. This is
//! what lets sibling content rectangles align exactly while edge chrome of
//! differing sizes stays synchronized.
//!
//! # Modules
//!
//! - [`geometry`]: plain value types (`Size`, `Point`, `Rect`, `Edges`,
//!   `Side`, `Orientation`).
//! - [`region`]: edge demand layering and the placement handoff
//!   (`EdgeDemand`, `EdgeTargets`, `PlacedRegion`, `PlacementSolution`).
//! - [`grid`]: the general solver. Rectangular slots with row/column spans,
//!   per-axis [`grid::TrackSpacing`] (outer offsets plus a `min_gap` floor),
//!   and one gap rule: `gap(i, i+1) = max(min_gap, trailing[i] +
//!   leading[i+1])`.
//! - [`band`]: a one-dimensional orientation adapter over the grid solver
//!   for row/column bands, plus cross-axis alignment.
//! - [`frame`]: the leaf solver. One content rectangle plus per-side chrome
//!   layers (margin, bands, outer, inner), sized from the envelope inward
//!   or from the content outward.
//! - [`alignment`]: requirement merging and deltas for aligning equivalent
//!   grids that are measured independently.
//! - `svg` (behind the `svg` feature): a renderer-independent debug data
//!   model and SVG export for inspecting solved layouts.
//!
//! Item identity is generic (`Id`, defaulting to `usize`) and opaque to this
//! crate: callers own what an ID means and how equivalent regions are
//! grouped.
//!
//! # Choosing a solver
//!
//! - One leaf region's chrome geometry (content plus margin/band/outer/inner
//!   layers per side, sized from either end): [`frame`] ([`Frame`] and its
//!   per-axis solve).
//! - Uniform policy, no per-track content: [`UniformTracks`] (merge and
//!   solve).
//! - Measured per-track content, spans, or holes: [`grid`]
//!   ([`GridRequirements`] and its solve).
//! - One axis with cross-axis alignment: [`band`] (an orientation adapter
//!   over the grid).
//! - Nesting, envelopes, or allocation propagation: [`tree`].
//! - Coordinating equivalent instances measured independently:
//!   [`alignment`] ([`align`] / [`align_by`], with [`ConvergenceTrace`]
//!   for multi-round drivers).

pub mod alignment;
pub mod band;
pub mod frame;
pub mod geometry;
pub mod grid;
pub mod region;
#[cfg(feature = "svg")]
pub mod svg;
pub mod tree;

pub use alignment::{
    AlignedGroup, AlignmentNode, AlignmentPlan, ConvergenceTrace, NodeDelta, RoundDeltas,
    SingletonPolicy, SkippedGroup, SkippedGroupReason, align, align_by,
};
pub use band::{BandItem, BandSolution, BoundaryDemand, CrossAlign, PlacedBandItem};
pub use frame::{
    Frame, FrameAxis, FrameAxisSizing, FrameAxisSolution, FrameSide, FrameSolution,
    SolvedFrameSide, SolvedSlab,
};
pub use geometry::{Edges, Orientation, Rect, Side, Size};
pub use grid::{
    GridError, GridItem, GridRequirements, GridShape, GridSlot, GridSolution, TrackSpacing,
    UniformTrackSolution, UniformTracks,
};
pub use region::{EdgeDemand, EdgeTargets, PlacedRegion, PlacementSolution, project_rect};
#[cfg(feature = "svg")]
pub use svg::{DebugRegion, DebugScene};
pub use tree::{
    LayoutItem, LayoutNode, LayoutSlotContent, SolvedRegion, SolvedTree, TreeEnvelope,
    TreeEnvelopeKind,
};
