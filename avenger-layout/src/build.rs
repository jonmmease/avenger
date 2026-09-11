//! The unified layout builder: one nested [`Layout`] value describing a
//! whole composition, solved in one step by [`Layout::solve`].
//!
//! # The model: two node kinds, every node is content + overflow
//!
//! A [`Layout`] is either a **leaf** (a measured region: content size plus
//! per-side [`EdgeDemand`] the solver cannot see inside) or a **grid** (an
//! arrangement of children in slots, with spacing, optional uniform-track
//! policy, free-space distribution, and an optional share key for cousin
//! alignment). `row`/`column` are 1×N conveniences.
//!
//! On top of that, every node — leaf or grid — can carry **declared
//! chrome**: named slabs per side (margin, repeatable strips, legend, guide)
//! plus a per-axis [`SolveFor`] sizing mode. Chrome is structured overflow:
//! the solver knows the individual slabs, returns their positioned
//! rectangles in the solution, and repositions them itself when coordination
//! resizes the node. Measured demands stay opaque.
//!
//! Toward the parent, guide slabs extend the `guide` stratum of the node's
//! solved edges ([`crate::region::EdgeGrant`]), legend slabs the `legend`
//! stratum, and strips and margins lift into `total` only (private envelope —
//! never matched against a cousin's strata).
//!
//! That privacy is deliberate, not a missing feature: strips and margins
//! are caller *declarations*, so a caller who wants them equal across
//! cousins can max its own declared sizes before building — no solver
//! involvement required. The `guide`/`legend` strata exist because
//! *measured* demands vary per cousin and only merge inside the solve;
//! a stratum earns a coordination channel exactly when its sizes are
//! measurements rather than declarations.

use crate::geometry::{Edges, Side, Size};
use crate::grid::{GridError, GridShape, GridSlot};
use crate::region::EdgeDemand;

/// Per-axis spacing policy for grid tracks (the gap law's inputs).
///
/// `gap(i, i+1) = max(min_gap, trailing[i].total + leading[i+1].total)`;
/// `outer_start`/`outer_end` reserve space before the first and after the
/// last track.
pub use crate::grid::TrackSpacing as Spacing;

/// Which size layer the solver computes on one axis of a chromed node.
///
/// Every chromed axis has three size layers — envelope, chrome stack,
/// content — and exactly one is computed from the others. The
/// **envelope** is this node's own outer box (content + chrome); it
/// exists at every nesting level, and at the root that outer box is the
/// **canvas** — the whole image ([`crate::LayoutSolution::size`]). So
/// `SolveFor` is recursive: each node picks which of its own layers the
/// solver derives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SolveFor {
    /// The envelope (this node's outer box: the root allocation/canvas,
    /// or the slot when nested) is given; the content gets the remainder,
    /// floored by [`Layout::content_min`]. Chrome is **contained**:
    /// carved inside the envelope, not lifted as overflow.
    Content,
    /// Content is given (the leaf's measured size or the grid's natural
    /// extent); the envelope is the sum of content plus chrome. Chrome is
    /// **overflow**: it lifts into the node's edge demands and shares gap
    /// and container-edge space exactly like measured overflow. This is the
    /// default — it matches the behavior of an un-chromed node. (The solved
    /// envelope is read back as the [`crate::Envelope`].)
    #[default]
    Envelope,
    /// Both envelope and content are given; the two margins absorb the
    /// slack, half each (declared margins are ignored on this axis). Like
    /// `Content`, chrome is contained.
    Margins,
}

/// Sizing policy for one grid track (CSS Grid's value types, minimally).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TrackSize {
    /// Content-sized: the max of the items measured into the track, floored
    /// by the grid's base cell size.
    #[default]
    Auto,
    /// Rigid pixels. Never grows for oversized content (the item overflows
    /// the track); never receives free space or span deficits.
    Fixed(f32),
    /// `fr` weight: a share of the *leftover* free space after fixed,
    /// content, and coordination floors are paid, never below the track's
    /// content (CSS's `minmax(auto, fr)`).
    Flex(f32),
}

/// Per-axis content distribution: where a grid's free space goes when no
/// `Flex` track is present on the axis (CSS `justify-content` /
/// `align-content`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Distribute {
    /// Stretch every track equally (the all-tracks-implicitly-equal-weight
    /// case). The default; matches classic tree allocation behavior.
    #[default]
    StretchTracks,
    /// Pack tracks to the start; free space trails.
    Start,
    /// Center the track run; free space splits around it.
    Center,
    /// Pack tracks to the end; free space leads.
    End,
    /// Distribute free space into the inter-track gaps.
    SpaceBetween,
}

/// Self-alignment of one child within its slot when its extent is smaller
/// (CSS `align-self`/`justify-self`). Deliberately no `Stretch`: stretching
/// is expressed where sizes are owned (`SolveFor::Content` for chromed
/// children, `Distribute::StretchTracks` for grids; a leaf cannot stretch —
/// its content is a measured fact, and growing it is the convergence loop's
/// job).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellAlign {
    #[default]
    Start,
    Center,
    End,
}

impl CellAlign {
    /// Offset of a child of extent `child` within a slot of extent `slot`.
    pub fn offset(self, slot: f32, child: f32) -> f32 {
        match self {
            CellAlign::Start => 0.0,
            CellAlign::Center => ((slot - child) / 2.0).max(0.0),
            CellAlign::End => (slot - child).max(0.0),
        }
    }
}

/// The canvas constraints for one solve: per-axis size of the root
/// envelope (the whole image).
///
/// `Some` fixes the canvas on that axis; `None` derives it from content
/// (natural sizing). All-`None` IS measurement: the canvas is the natural
/// envelope of the content.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SolveOptions {
    pub width: Option<f32>,
    pub height: Option<f32>,
}

/// Error from [`Layout::solve`]. Everything else (shape-mismatched share
/// groups, over-constrained sizing) resolves by documented rule plus
/// diagnostics rather than failing.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    /// A grid child's slot has a zero span or falls outside the grid shape.
    SlotOutOfBounds { slot: GridSlot, shape: GridShape },
    /// The same id appears on two nodes, which would break solution lookup.
    DuplicateId,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SlotOutOfBounds { slot, shape } => write!(
                f,
                "layout child slot {:?} exceeds grid shape {}x{}",
                slot, shape.rows, shape.columns
            ),
            Self::DuplicateId => write!(f, "duplicate node id breaks solution lookup"),
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<GridError> for LayoutError {
    fn from(error: GridError) -> Self {
        match error {
            GridError::SlotOutOfBounds { slot, shape } => Self::SlotOutOfBounds { slot, shape },
        }
    }
}

/// One side's declared chrome slabs, ordered outside-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChromeSide {
    pub(crate) margin: f32,
    pub(crate) strips: Vec<f32>,
    pub(crate) legend: f32,
    pub(crate) guide: f32,
}

/// Declared chrome and sizing for one node.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Chrome {
    pub(crate) sides: Edges<ChromeSide>,
    pub(crate) sizing_x: SolveFor,
    pub(crate) sizing_y: SolveFor,
    pub(crate) content_min: Size,
}

/// One child of a grid node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridChild<Id, Key> {
    pub(crate) slot: GridSlot,
    pub(crate) layout: Layout<Id, Key>,
}

/// The grid node payload.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridSpec<Id, Key> {
    pub(crate) shape: GridShape,
    pub(crate) children: Vec<GridChild<Id, Key>>,
    pub(crate) column_spacing: Spacing,
    pub(crate) row_spacing: Spacing,
    pub(crate) base_cell_size: Size,
    pub(crate) uniform_columns: bool,
    pub(crate) uniform_rows: bool,
    pub(crate) share: Option<Key>,
    pub(crate) column_sizes: Option<Vec<TrackSize>>,
    pub(crate) row_sizes: Option<Vec<TrackSize>>,
    pub(crate) distribute_x: Distribute,
    pub(crate) distribute_y: Distribute,
}

impl<Id, Key> GridSpec<Id, Key> {
    fn new(rows: usize, columns: usize) -> Self {
        Self {
            shape: GridShape { rows, columns },
            children: Vec::new(),
            column_spacing: Spacing::default(),
            row_spacing: Spacing::default(),
            base_cell_size: Size::default(),
            uniform_columns: false,
            uniform_rows: false,
            share: None,
            column_sizes: None,
            row_sizes: None,
            distribute_x: Distribute::default(),
            distribute_y: Distribute::default(),
        }
    }
}

/// The node kind payload.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayoutKind<Id, Key> {
    Leaf {
        content_size: Size,
        demands: Edges<EdgeDemand>,
    },
    Grid(GridSpec<Id, Key>),
}

/// One node of a layout: a measured leaf or a grid of children, optionally
/// chromed. See the [module docs](self) for the model.
///
/// `Id` is caller-owned and opaque (solution lookup); `Key` groups grids for
/// cousin alignment. Both default to `usize`.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout<Id = usize, Key = usize> {
    pub(crate) id: Option<Id>,
    pub(crate) chrome: Chrome,
    pub(crate) align: (CellAlign, CellAlign),
    pub(crate) kind: LayoutKind<Id, Key>,
}

impl<Id, Key> Layout<Id, Key> {
    fn new(kind: LayoutKind<Id, Key>) -> Self {
        Self {
            id: None,
            chrome: Chrome::default(),
            align: (CellAlign::default(), CellAlign::default()),
            kind,
        }
    }

    // --- constructors ---------------------------------------------------

    /// A measured leaf: a content size the solver treats as fact. Attach
    /// per-side overflow with [`Layout::demand`].
    pub fn leaf(content_size: Size) -> Self {
        Self::new(LayoutKind::Leaf {
            content_size,
            demands: Edges::default(),
        })
    }

    /// An empty `rows`×`columns` grid. Fill slots with [`Layout::cell`] /
    /// [`Layout::cell_span`].
    pub fn grid(rows: usize, columns: usize) -> Self {
        Self::new(LayoutKind::Grid(GridSpec::new(rows, columns)))
    }

    /// A 1×N horizontal arrangement.
    pub fn row(children: impl IntoIterator<Item = Layout<Id, Key>>) -> Self {
        let children: Vec<Layout<Id, Key>> = children.into_iter().collect();
        let mut grid = GridSpec::new(1, children.len());
        for (column, child) in children.into_iter().enumerate() {
            grid.children.push(GridChild {
                slot: GridSlot {
                    row: 0,
                    column,
                    row_span: 1,
                    column_span: 1,
                },
                layout: child,
            });
        }
        Self::new(LayoutKind::Grid(grid))
    }

    /// An N×1 vertical arrangement.
    pub fn column(children: impl IntoIterator<Item = Layout<Id, Key>>) -> Self {
        let children: Vec<Layout<Id, Key>> = children.into_iter().collect();
        let mut grid = GridSpec::new(children.len(), 1);
        for (row, child) in children.into_iter().enumerate() {
            grid.children.push(GridChild {
                slot: GridSlot {
                    row,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
                layout: child,
            });
        }
        Self::new(LayoutKind::Grid(grid))
    }

    // --- common builder methods ------------------------------------------

    /// Caller-owned identity for solution lookup. Optional; every solved
    /// region also carries its structural path.
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Self-alignment within a parent slot when this node's extent is
    /// smaller (horizontal, vertical).
    pub fn align_in_cell(mut self, horizontal: CellAlign, vertical: CellAlign) -> Self {
        self.align = (horizontal, vertical);
        self
    }

    // --- chrome ----------------------------------------------------------

    /// Uniform margin on all four sides (outermost chrome layer; lifts into
    /// `total` only).
    pub fn margin(self, size: f32) -> Self {
        self.margin_edges(Edges::new(size, size, size, size))
    }

    /// Per-side margins.
    pub fn margin_edges(mut self, edges: Edges<f32>) -> Self {
        self.chrome.sides.top.margin = edges.top;
        self.chrome.sides.right.margin = edges.right;
        self.chrome.sides.bottom.margin = edges.bottom;
        self.chrome.sides.left.margin = edges.left;
        self
    }

    /// A discrete chrome strip on one side (for charts: title/subtitle rows).
    /// Repeatable; strips stack outside-in in call order. Lifts into `total`
    /// only.
    pub fn strip(mut self, side: Side, size: f32) -> Self {
        self.chrome.sides.side_mut(side).strips.push(size);
        self
    }

    /// The legend chrome slab on one side (content stacking beyond the
    /// guide stratum). Lifts into the `legend` demand stratum.
    pub fn legend(mut self, side: Side, size: f32) -> Self {
        self.chrome.sides.side_mut(side).legend = size;
        self
    }

    /// The guide chrome slab on one side (axis ticks/labels, facet
    /// headers). Lifts into the `guide` demand stratum.
    pub fn guide(mut self, side: Side, size: f32) -> Self {
        self.chrome.sides.side_mut(side).guide = size;
        self
    }

    /// Sizing mode for both axes. See [`SolveFor`].
    pub fn sizing(mut self, sizing: SolveFor) -> Self {
        self.chrome.sizing_x = sizing;
        self.chrome.sizing_y = sizing;
        self
    }

    /// Sizing mode for the horizontal axis only.
    pub fn sizing_x(mut self, sizing: SolveFor) -> Self {
        self.chrome.sizing_x = sizing;
        self
    }

    /// Sizing mode for the vertical axis only.
    pub fn sizing_y(mut self, sizing: SolveFor) -> Self {
        self.chrome.sizing_y = sizing;
        self
    }

    /// Minimum content extent on [`SolveFor::Content`] axes (the content may
    /// then overflow the envelope). Ignored in the other modes.
    pub fn content_min(mut self, size: Size) -> Self {
        self.chrome.content_min = size;
        self
    }

    // --- leaf-only -------------------------------------------------------

    /// Measured overflow on one side of a leaf (opaque to the solver).
    ///
    /// # Panics
    /// If called on a grid node.
    pub fn demand(mut self, side: Side, demand: EdgeDemand) -> Self {
        match &mut self.kind {
            LayoutKind::Leaf { demands, .. } => {
                demands.set_side(side, demand);
                self
            }
            LayoutKind::Grid(_) => panic!("Layout::demand applies to leaf nodes only"),
        }
    }

    // --- grid-only -------------------------------------------------------

    fn grid_mut(&mut self, method: &str) -> &mut GridSpec<Id, Key> {
        match &mut self.kind {
            LayoutKind::Grid(grid) => grid,
            LayoutKind::Leaf { .. } => panic!("Layout::{method} applies to grid nodes only"),
        }
    }

    /// Place a child in one slot.
    pub fn cell(self, row: usize, column: usize, child: impl Into<Layout<Id, Key>>) -> Self {
        self.cell_span(row, column, 1, 1, child)
    }

    /// Place a child spanning `row_span`×`column_span` slots.
    pub fn cell_span(
        mut self,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        child: impl Into<Layout<Id, Key>>,
    ) -> Self {
        self.grid_mut("cell_span").children.push(GridChild {
            slot: GridSlot {
                row,
                column,
                row_span,
                column_span,
            },
            layout: child.into(),
        });
        self
    }

    /// Floor for every inter-track gap on both axes.
    pub fn min_gap(mut self, min_gap: f32) -> Self {
        let grid = self.grid_mut("min_gap");
        grid.column_spacing.min_gap = min_gap;
        grid.row_spacing.min_gap = min_gap;
        self
    }

    /// Full column-axis spacing policy.
    pub fn column_spacing(mut self, spacing: Spacing) -> Self {
        self.grid_mut("column_spacing").column_spacing = spacing;
        self
    }

    /// Full row-axis spacing policy.
    pub fn row_spacing(mut self, spacing: Spacing) -> Self {
        self.grid_mut("row_spacing").row_spacing = spacing;
        self
    }

    /// Per-track minimum cell size floor.
    pub fn base_cell_size(mut self, size: Size) -> Self {
        self.grid_mut("base_cell_size").base_cell_size = size;
        self
    }

    /// Equalize all column tracks to the max (and merge as policy across a
    /// share group, tolerating different track counts).
    pub fn uniform_columns(mut self) -> Self {
        self.grid_mut("uniform_columns").uniform_columns = true;
        self
    }

    /// Equalize all row tracks to the max.
    pub fn uniform_rows(mut self) -> Self {
        self.grid_mut("uniform_rows").uniform_rows = true;
        self
    }

    /// Align this grid's tracks, spacing, and edge chrome with every other
    /// grid carrying the same key, anywhere in the tree (cousin
    /// coordination). The switch between "match your cousins (and leave
    /// slack)" and "fill your container".
    pub fn share(mut self, key: Key) -> Self {
        self.grid_mut("share").share = Some(key);
        self
    }

    /// Declared column track sizes (default all [`TrackSize::Auto`]).
    pub fn columns(mut self, sizes: impl IntoIterator<Item = TrackSize>) -> Self {
        let sizes: Vec<TrackSize> = sizes.into_iter().collect();
        self.grid_mut("columns").column_sizes = Some(sizes);
        self
    }

    /// Declared row track sizes (default all [`TrackSize::Auto`]).
    pub fn rows(mut self, sizes: impl IntoIterator<Item = TrackSize>) -> Self {
        let sizes: Vec<TrackSize> = sizes.into_iter().collect();
        self.grid_mut("rows").row_sizes = Some(sizes);
        self
    }

    /// Free-space distribution on the horizontal axis (only applies when no
    /// `Flex` column is declared).
    pub fn distribute_x(mut self, distribute: Distribute) -> Self {
        self.grid_mut("distribute_x").distribute_x = distribute;
        self
    }

    /// Free-space distribution on the vertical axis.
    pub fn distribute_y(mut self, distribute: Distribute) -> Self {
        self.grid_mut("distribute_y").distribute_y = distribute;
        self
    }
}

impl Edges<ChromeSide> {
    pub(crate) fn side_mut(&mut self, side: Side) -> &mut ChromeSide {
        match side {
            Side::Top => &mut self.top,
            Side::Right => &mut self.right,
            Side::Bottom => &mut self.bottom,
            Side::Left => &mut self.left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_construct_the_documented_shapes() {
        let cell: Layout<&str> = Layout::leaf(Size::new(110.0, 50.0))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: 26.0,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Bottom,
                EdgeDemand {
                    guide: 18.0,
                    legend: 0.0,
                },
            )
            .id("a0");
        match &cell.kind {
            LayoutKind::Leaf {
                content_size,
                demands,
            } => {
                assert_eq!(*content_size, Size::new(110.0, 50.0));
                assert_eq!(
                    demands.left,
                    EdgeDemand {
                        guide: 26.0,
                        legend: 0.0
                    }
                );
                assert_eq!(
                    demands.bottom,
                    EdgeDemand {
                        guide: 18.0,
                        legend: 0.0
                    }
                );
            }
            LayoutKind::Grid(_) => panic!("leaf expected"),
        }
        assert_eq!(cell.id, Some("a0"));

        let chart: Layout = Layout::leaf(Size::default())
            .margin(8.0)
            .strip(Side::Top, 18.0)
            .strip(Side::Top, 12.0)
            .legend(Side::Right, 64.0)
            .guide(Side::Left, 38.0)
            .sizing(SolveFor::Content);
        assert_eq!(chart.chrome.sides.top.strips, vec![18.0, 12.0]);
        assert_eq!(chart.chrome.sides.right.legend, 64.0);
        assert_eq!(chart.chrome.sides.left.guide, 38.0);
        assert_eq!(chart.chrome.sides.left.margin, 8.0);
        assert_eq!(chart.chrome.sizing_x, SolveFor::Content);
        assert_eq!(chart.chrome.sizing_y, SolveFor::Content);

        let group: Layout<usize, &str> = Layout::column(vec![
            Layout::leaf(Size::new(10.0, 10.0)),
            Layout::leaf(Size::new(10.0, 10.0)),
        ])
        .min_gap(14.0)
        .uniform_rows()
        .share("facet-cells")
        .guide(Side::Top, 16.0);
        match &group.kind {
            LayoutKind::Grid(grid) => {
                assert_eq!(
                    grid.shape,
                    GridShape {
                        rows: 2,
                        columns: 1
                    }
                );
                assert_eq!(grid.row_spacing.min_gap, 14.0);
                assert!(grid.uniform_rows);
                assert_eq!(grid.share, Some("facet-cells"));
                assert_eq!(grid.children[1].slot.row, 1);
            }
            LayoutKind::Leaf { .. } => panic!("grid expected"),
        }
        assert_eq!(group.chrome.sides.top.guide, 16.0);

        let grid: Layout = Layout::grid(2, 3)
            .cell(0, 0, Layout::leaf(Size::default()))
            .cell_span(1, 0, 1, 2, Layout::leaf(Size::default()))
            .columns([TrackSize::Flex(2.0), TrackSize::Flex(1.0), TrackSize::Auto])
            .distribute_x(Distribute::SpaceBetween)
            .base_cell_size(Size::new(40.0, 30.0));
        match &grid.kind {
            LayoutKind::Grid(spec) => {
                assert_eq!(spec.children.len(), 2);
                assert_eq!(spec.children[1].slot.column_span, 2);
                assert_eq!(
                    spec.column_sizes,
                    Some(vec![
                        TrackSize::Flex(2.0),
                        TrackSize::Flex(1.0),
                        TrackSize::Auto
                    ])
                );
                assert_eq!(spec.distribute_x, Distribute::SpaceBetween);
                assert_eq!(spec.base_cell_size, Size::new(40.0, 30.0));
            }
            LayoutKind::Leaf { .. } => panic!("grid expected"),
        }
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let leaf: Layout = Layout::leaf(Size::default());
        assert_eq!(leaf.chrome.sizing_x, SolveFor::Envelope);
        assert_eq!(leaf.align, (CellAlign::Start, CellAlign::Start));
        assert_eq!(
            SolveOptions::default(),
            SolveOptions {
                width: None,
                height: None
            }
        );
        assert_eq!(Distribute::default(), Distribute::StretchTracks);
        assert_eq!(TrackSize::default(), TrackSize::Auto);
    }

    #[test]
    #[should_panic(expected = "applies to grid nodes only")]
    fn grid_methods_panic_on_leaves() {
        let _ = Layout::<usize, usize>::leaf(Size::default()).min_gap(4.0);
    }

    #[test]
    #[should_panic(expected = "applies to leaf nodes only")]
    fn leaf_methods_panic_on_grids() {
        let _ = Layout::<usize, usize>::grid(1, 1).demand(
            Side::Top,
            EdgeDemand {
                guide: 1.0,
                legend: 0.0,
            },
        );
    }
}
