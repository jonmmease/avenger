//! Grid layout for charts.
//!
//! A [`Layout`] is a tree of leaves and containers. Leaves are the parts that the layout does
//! not look inside, such as plots and legends. A [`Row`] or [`Column`] places its children in
//! order, and nested rows and columns share one table of tracks. A [`Grid`] places its children
//! in slots on declared column and row tracks. A [`Wrap`] places them in as many columns as fit,
//! and a [`Flow`] breaks them into lines by their widths. Any node can carry [`Slab`]s on its
//! sides, such as axes and headers, and [`Overlay`]s over it. Solving the tree gives a
//! [`Solution`] with one positioned [`Region`] per node, in which plot edges line up across
//! rows, columns and nested tables.
//!
//! Containers declare rules for tagged slabs. [`collapse_tag`](Grid::collapse_tag) keeps one
//! axis for each run of panels in a lane that show the same [`ContentKey`], and
//! [`merge_tag`](Grid::merge_tag) puts one title or legend on the container when every panel
//! shows the same one. [`Solution::representative`] finds the region that draws each slab.
//!
//! ```
//! use avenger_layout::{Layout, Leaf, Rect, Row, Side, Size, Slab, SolveOptions};
//!
//! let plot = |id| {
//!     Leaf::new()
//!         .min_size(Size::new(200.0, 150.0))
//!         .id(id)
//!         .slab(Side::Left, Slab::new().min_width(40.0))
//!         .slab(Side::Bottom, Slab::new().min_height(24.0))
//! };
//! let page: Layout<&str> = Row::new([plot("a"), plot("b")]).into();
//! let solution = page.solve(&SolveOptions::sized(600.0, 300.0)).unwrap();
//! // The first axis sits in the root's left margin, and the second in the gap between the plots.
//! assert_eq!(solution.region(&"a").unwrap().content, Rect::new(40.0, 0.0, 260.0, 276.0));
//! assert_eq!(solution.region(&"b").unwrap().content, Rect::new(340.0, 0.0, 260.0, 276.0));
//! ```
//!
//! Leaves whose size depends on the space they get, such as axes and wrapped text, are measured
//! through [`Measure`] with [`Layout::measure`], which returns a [`Measured`] that can solve
//! again at other sizes.

#[cfg(feature = "diagnostics")]
pub mod diagnostics;
mod explain;
mod geometry;
mod lower;
mod measure;
mod measured;
mod node;
mod report;
mod share;
mod snap;
mod solution;
mod solve;
mod tracks;
mod validate;

pub use explain::{
    AspectFit, ExplainedSizes, Explanation, GapExplanation, GapTerm, MatchGroup, MatchedParts,
    RaisedOverhang, RaisedWidth, Representation, RepresentationKind, TrackExplanation,
    TrackOwnerExplanation,
};
pub use geometry::{Direction, Edges, Rect, Side, Size};
pub use measure::{Holds, Measure, Measurement};
pub use measured::Measured;
pub use node::{
    Column, ContentKey, Distribute, Extent, Flow, Gap, Grid, GridSlot, Layout, Leaf, Overflow,
    Overlay, Placement, Position, Row, Sizing, Slab, SolveOptions, Tag, Track, TrackSize, Wrap,
};
pub use report::{GapAt, PointExplanation, TrackAt, Warning};
pub use solution::{Cell, GridTracks, PathStep, Region, RegionKind, Solution, TrackSpan};
pub use validate::LayoutError;
