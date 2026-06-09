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
//! - [`alignment`]: requirement merging and deltas for aligning equivalent
//!   grids that are measured independently.
//!
//! Item identity is generic (`Id`, defaulting to `usize`) and opaque to this
//! crate: callers own what an ID means and how equivalent regions are
//! grouped.

pub mod alignment;
pub mod band;
pub mod geometry;
pub mod grid;
pub mod region;

pub use alignment::{grid_content_delta, grid_edge_delta, merge_grid_requirements};
pub use band::{BandItem, BandSolution, BandSpacing, BoundaryDemand, CrossAlign, PlacedBandItem};
pub use geometry::{Edges, Orientation, Point, Rect, Side, Size};
pub use grid::{
    GridError, GridItem, GridRequirements, GridShape, GridSlot, GridSolution, TrackSpacing,
    edge_demand_totals, grid_requirements, solve_grid_requirements, span_axis_extent,
    total_edge_demands, zero_edge_demands,
};
pub use region::{EdgeDemand, EdgeTargets, PlacedRegion, PlacementSolution, project_rect};
