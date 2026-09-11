//! Chart-independent layout: build one nested [`Layout`], solve it in one
//! step, read the solution, render it to SVG.
//!
//! # The model: two node kinds, every node is content + overflow
//!
//! A [`Layout`] is either a **leaf** (a measured region: content size plus
//! per-side [`EdgeDemand`] the solver cannot see inside) or a **grid** (an
//! arrangement of children in slots with spans and holes, per-axis
//! [`Spacing`], declared [`TrackSize`]s, free-space [`Distribute`] policy,
//! optional uniform tracks, and an optional **share key** for cousin
//! alignment). [`Layout::row`]/[`Layout::column`] are 1×N conveniences.
//!
//! Every node — leaf or grid — can additionally carry **declared chrome**:
//! named slabs per side (margin, repeatable strips, legend, guide) plus a
//! per-axis [`SolveFor`] sizing mode. Chrome is structured overflow: the
//! solver knows the individual slabs and returns their positioned
//! rectangles ([`ChromeSlab`]); measured demands stay opaque. Guide slabs
//! extend the `guide` stratum of the node's solved edges ([`EdgeGrant`])
//! toward the parent, legend slabs the `legend` stratum; strips and margins
//! lift into `total` only (private envelope, never matched against a
//! cousin's strata).
//!
//! # The laws
//!
//! - **Gap law**: `gap(i, i+1) = max(min_gap, trailing[i] + leading[i+1])`;
//!   the first leading and last trailing edges are excluded from a grid's
//!   content extent (they overlap the container's own edge overflow).
//! - **Lift law**: a solved side's ([`EdgeGrant`]) `total >= guide +
//!   legend`; declarations ([`EdgeDemand`]) carry no total of their own.
//! - **Free-space precedence**: fixed → content → coordination → fr →
//!   stretch/justify. Coordination ([`Layout::share`]) only raises floors;
//!   on shared axes free space distributes in policy space under the
//!   min-slack rule so cousins stay congruent.
//! - **Content never lies**: a region's [`content`](Region::content)
//!   rectangle is its measured/solved extent positioned by [`CellAlign`]
//!   within its [`slot`](Region::slot) (the allotment); the solver never
//!   falsifies a measurement to fill space.
//!
//! # One solve, no loop
//!
//! [`Layout::solve`] runs measure-up → coordinate (one pure round over all
//! share groups) → allocate-down, once. Demands are constant inputs; if a
//! caller's measurements depend on allocated sizes (chart tick labels), the
//! caller re-measures at the granted allotments and solves again —
//! [`LayoutSolution::content_delta`] drives that loop.
//!
//! # Reading a solution
//!
//! [`LayoutSolution`] holds every node as a [`Region`] (queryable by caller
//! id or structural path): the slot allotment, the honest content rect,
//! positioned chrome slabs, requested-vs-granted demands, and solved grid
//! tracks. [`LayoutSolution::to_svg`] renders the debug gallery view;
//! [`svg_panels`] stacks several solutions with captions. The committed
//! gallery in `tests/baselines/` doubles as the visual reference for every
//! feature.

mod build;
mod frame;
mod geometry;
mod grid;
mod region;
mod solution;
mod solve;
mod svg;

pub use build::{
    CellAlign, Distribute, Layout, LayoutError, SolveFor, SolveOptions, Spacing, TrackSize,
};
pub use geometry::{Edges, Rect, Side, Size};
pub use grid::{GridError, GridItem, GridRequirements, GridShape, GridSlot, GridSolution};
pub use region::{EdgeDemand, EdgeGrant};
pub use solution::{
    ChromeLayer, ChromeSlab, Diagnostics, Envelope, LayoutSolution, Region, RegionDetail,
    SkippedShare, SkippedShareReason, SolvedTracks,
};
pub use svg::{SvgOptions, svg_panels};
