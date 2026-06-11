//! The one-step solver behind [`Layout::solve`]: measure up, coordinate
//! (share keys), allocate down.
//!
//! # Laws
//!
//! - **Measure up**: leaves report their measured facts; grids solve tracks
//!   to natural envelopes (gap law, span constraints, base cell sizes);
//!   each node's declared chrome applies per its [`SolveFor`] mode —
//!   `Envelope` axes lift chrome into the node's edge demands (overflow),
//!   `Content`/`Margins` axes contain it (the node becomes an opaque box).
//! - **Allocate down**: the root receives the per-axis allocation (or its
//!   natural envelope); each node carves chrome and hands content inward;
//!   grids distribute free space per [`Distribute`] and place children.
//! - **Content never lies**: a region's `content` rectangle is its
//!   measured/solved extent positioned by `CellAlign` within its `slot`
//!   (the allotment). Leaves never stretch; grids fill their slot
//!   (distributing free space per their own policy); contained chrome
//!   boxes fill their slot and solve content inside.

use std::collections::HashSet;
use std::hash::Hash;

use crate::build::{
    CellAlign, ChromeSide, Distribute, GridSpec, Layout, LayoutError, LayoutKind, SolveFor,
    SolveOptions,
};
use crate::frame::{FrameAxis, FrameAxisSizing, FrameSide};
use crate::geometry::{Edges, Rect, Size};
use crate::grid::{GridItem, GridRequirements, GridSolution};
use crate::region::EdgeDemand;
use crate::solution::{Diagnostics, Envelope, LayoutSolution, Region, RegionDetail, SolvedTracks};

// --- measurement -----------------------------------------------------------

/// Bottom-up measurement of one node.
pub(crate) struct Measured {
    /// What the parent's grid consumes as this item's content size:
    /// the content extent on `Envelope` axes, the box extent on contained
    /// axes.
    pub(crate) item_size: Size,
    /// Lifted overflow toward the parent (zero on contained axes).
    pub(crate) demands: Edges<EdgeDemand>,
    /// Raw per-side totals (no lift) for the geometric envelope view.
    pub(crate) geometric_total: Edges<f32>,
    /// The node's own content extent before chrome.
    pub(crate) natural_content: Size,
    pub(crate) grid: Option<MeasuredGrid>,
}

pub(crate) struct MeasuredGrid {
    pub(crate) items: Vec<GridItem<usize>>,
    pub(crate) requirements: GridRequirements,
    pub(crate) natural: GridSolution,
    pub(crate) children: Vec<Measured>,
}

/// Stack one side's declared chrome onto the content's own demand
/// (the `stacked_inner/outer_edges` law: additive per layer; bands and
/// margins extend `total` only).
fn lift_chrome(demand: EdgeDemand, chrome: &ChromeSide) -> EdgeDemand {
    EdgeDemand::new(
        demand.inner + chrome.inner.max(0.0),
        demand.outer + chrome.outer.max(0.0),
        demand.total + chrome_total(chrome),
    )
}

fn chrome_total(side: &ChromeSide) -> f32 {
    let mut total = side.margin.max(0.0);
    for band in &side.bands {
        total += band.max(0.0);
    }
    total + side.outer.max(0.0) + side.inner.max(0.0)
}

/// Fixed (non-margin) chrome on one side: what `Margins` mode keeps rigid.
fn chrome_fixed_total(side: &ChromeSide) -> f32 {
    chrome_total(side) - side.margin.max(0.0)
}

pub(crate) fn measure<Id: Clone, Key>(node: &Layout<Id, Key>) -> Result<Measured, LayoutError> {
    let (natural_content, content_demands, mut geometric_total, grid) = match &node.kind {
        LayoutKind::Leaf {
            content_size,
            demands,
        } => {
            let geometric = Edges::new(
                demands.top.total,
                demands.right.total,
                demands.bottom.total,
                demands.left.total,
            );
            (
                Size::new(content_size.width.max(0.0), content_size.height.max(0.0)),
                *demands,
                geometric,
                None,
            )
        }
        LayoutKind::Grid(spec) => {
            let mut children = Vec::with_capacity(spec.children.len());
            let mut items = Vec::with_capacity(spec.children.len());
            for (index, child) in spec.children.iter().enumerate() {
                let measured = measure(&child.layout)?;
                items.push(GridItem {
                    id: index,
                    slot: child.slot,
                    content_size: measured.item_size,
                    inner_edges: Edges::new(
                        measured.demands.top.inner,
                        measured.demands.right.inner,
                        measured.demands.bottom.inner,
                        measured.demands.left.inner,
                    ),
                    outer_edges: Edges::new(
                        measured.demands.top.outer,
                        measured.demands.right.outer,
                        measured.demands.bottom.outer,
                        measured.demands.left.outer,
                    ),
                    total_edges: Edges::new(
                        measured.demands.top.total,
                        measured.demands.right.total,
                        measured.demands.bottom.total,
                        measured.demands.left.total,
                    ),
                });
                children.push(measured);
            }

            let mut requirements =
                GridRequirements::from_items(spec.shape, spec.base_cell_size, &items)?;
            requirements.column_spacing = spec.column_spacing;
            requirements.row_spacing = spec.row_spacing;
            if spec.uniform_columns {
                equalize(&mut requirements.column_widths);
            }
            if spec.uniform_rows {
                equalize(&mut requirements.row_heights);
            }
            let natural = requirements.solve(&items);

            // Boundary demands: first/last track edges reach the envelope;
            // interior edges became gaps.
            let demands = Edges::new(
                natural.row_top.first().copied().unwrap_or_default(),
                natural.column_right.last().copied().unwrap_or_default(),
                natural.row_bottom.last().copied().unwrap_or_default(),
                natural.column_left.first().copied().unwrap_or_default(),
            );

            // Geometric view: raw max of edge children's raw totals.
            let mut geometric = Edges::new(0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (child, measured) in spec.children.iter().zip(children.iter()) {
                let slot = child.slot;
                if slot.row == 0 {
                    geometric.top = geometric.top.max(measured.geometric_total.top);
                }
                if slot.row_end() == spec.shape.rows {
                    geometric.bottom = geometric.bottom.max(measured.geometric_total.bottom);
                }
                if slot.column == 0 {
                    geometric.left = geometric.left.max(measured.geometric_total.left);
                }
                if slot.column_end() == spec.shape.columns {
                    geometric.right = geometric.right.max(measured.geometric_total.right);
                }
            }

            (
                natural.content_size,
                demands,
                geometric,
                Some(MeasuredGrid {
                    items,
                    requirements,
                    natural,
                    children,
                }),
            )
        }
    };

    // Apply declared chrome per axis mode.
    let chrome = &node.chrome;
    let mut item_size = natural_content;
    let mut demands = content_demands;

    match chrome.sizing_x {
        SolveFor::Envelope => {
            demands.left = lift_chrome(demands.left, &chrome.sides.left);
            demands.right = lift_chrome(demands.right, &chrome.sides.right);
            geometric_total.left += chrome_total(&chrome.sides.left);
            geometric_total.right += chrome_total(&chrome.sides.right);
        }
        SolveFor::Content => {
            let content = natural_content.width.max(chrome.content_min.width.max(0.0));
            item_size.width =
                chrome_total(&chrome.sides.left) + content + chrome_total(&chrome.sides.right);
            demands.left = EdgeDemand::default();
            demands.right = EdgeDemand::default();
            geometric_total.left = 0.0;
            geometric_total.right = 0.0;
        }
        SolveFor::Margins => {
            item_size.width = chrome_fixed_total(&chrome.sides.left)
                + natural_content.width
                + chrome_fixed_total(&chrome.sides.right);
            demands.left = EdgeDemand::default();
            demands.right = EdgeDemand::default();
            geometric_total.left = 0.0;
            geometric_total.right = 0.0;
        }
    }
    match chrome.sizing_y {
        SolveFor::Envelope => {
            demands.top = lift_chrome(demands.top, &chrome.sides.top);
            demands.bottom = lift_chrome(demands.bottom, &chrome.sides.bottom);
            geometric_total.top += chrome_total(&chrome.sides.top);
            geometric_total.bottom += chrome_total(&chrome.sides.bottom);
        }
        SolveFor::Content => {
            let content = natural_content
                .height
                .max(chrome.content_min.height.max(0.0));
            item_size.height =
                chrome_total(&chrome.sides.top) + content + chrome_total(&chrome.sides.bottom);
            demands.top = EdgeDemand::default();
            demands.bottom = EdgeDemand::default();
            geometric_total.top = 0.0;
            geometric_total.bottom = 0.0;
        }
        SolveFor::Margins => {
            item_size.height = chrome_fixed_total(&chrome.sides.top)
                + natural_content.height
                + chrome_fixed_total(&chrome.sides.bottom);
            demands.top = EdgeDemand::default();
            demands.bottom = EdgeDemand::default();
            geometric_total.top = 0.0;
            geometric_total.bottom = 0.0;
        }
    }

    Ok(Measured {
        item_size,
        demands,
        geometric_total,
        natural_content,
        grid,
    })
}

fn equalize(sizes: &mut [f32]) {
    let max = sizes.iter().copied().fold(0.0f32, f32::max);
    for size in sizes {
        *size = max;
    }
}

// --- allocation ------------------------------------------------------------

/// Per-axis placement of one node within its slot: where its content lands,
/// plus the solved chrome stack (for slab carving) and the absolute origin
/// of the chromed box on this axis.
struct AxisPlacement {
    content_start: f32,
    content_extent: f32,
    /// The full per-axis chrome solution (slab sizes; flexible margins
    /// already resolved).
    chrome: crate::frame::FrameAxisSolution,
    /// Absolute start of the chromed box: the slot start on contained axes,
    /// `content_start - leading chrome` on `Envelope` axes (chrome hangs
    /// outside the content into gap/edge space).
    box_start: f32,
}

fn place_axis(
    sizing: SolveFor,
    leading: &ChromeSide,
    trailing: &ChromeSide,
    content_min: f32,
    natural_content: f32,
    fills_slot: bool,
    align: CellAlign,
    slot_start: f32,
    slot_extent: f32,
) -> AxisPlacement {
    match sizing {
        SolveFor::Envelope => {
            let content_extent = if fills_slot {
                slot_extent
            } else {
                natural_content
            };
            let offset = align.offset(slot_extent, content_extent);
            let content_start = slot_start + offset;
            let axis = FrameAxis {
                sizing: FrameAxisSizing::ContentFixed {
                    content: content_extent,
                },
                leading: frame_side(leading),
                trailing: frame_side(trailing),
                content_min,
            };
            let solved = axis.solve();
            let box_start = content_start - solved.content.start;
            AxisPlacement {
                content_start,
                content_extent,
                chrome: solved,
                box_start,
            }
        }
        SolveFor::Content | SolveFor::Margins => {
            let axis = FrameAxis {
                sizing: match sizing {
                    SolveFor::Content => FrameAxisSizing::EnvelopeFixed {
                        extent: slot_extent,
                    },
                    _ => FrameAxisSizing::EnvelopeAndContentFixed {
                        extent: slot_extent,
                        content: natural_content,
                    },
                },
                leading: frame_side(leading),
                trailing: frame_side(trailing),
                content_min,
            };
            let solved = axis.solve();
            AxisPlacement {
                content_start: slot_start + solved.content.start,
                content_extent: solved.content.size,
                chrome: solved,
                box_start: slot_start,
            }
        }
    }
}

/// Carve 2D slab rectangles from the per-axis chrome solutions.
///
/// Layers carve from the outside in (margin → bands → outer → inner); within
/// one layer, vertical sides (top/bottom) carve before horizontal
/// (left/right), so a top band runs wider than a left band of the same layer
/// and corners belong to the outer-more / vertical-first slab. The innermost
/// slabs end up exactly content-sized on their cross axis.
fn carve_slabs(
    horizontal: &AxisPlacement,
    vertical: &AxisPlacement,
    slabs: &mut Vec<crate::solution::ChromeSlab>,
) {
    use crate::geometry::Side;
    use crate::solution::{ChromeLayer, ChromeSlab};

    // Remaining rect starts as the full chromed box.
    let mut x0 = horizontal.box_start;
    let mut x1 = horizontal.box_start + horizontal.chrome.extent;
    let mut y0 = vertical.box_start;
    let mut y1 = vertical.box_start + vertical.chrome.extent;

    let top = &vertical.chrome.leading;
    let bottom = &vertical.chrome.trailing;
    let left = &horizontal.chrome.leading;
    let right = &horizontal.chrome.trailing;

    let band_steps = top
        .bands
        .len()
        .max(bottom.bands.len())
        .max(left.bands.len())
        .max(right.bands.len());

    // One carving step: sizes per side for this layer, in vertical-first
    // order. Emits non-empty slabs and shrinks the remaining rect.
    let mut step = |layer: ChromeLayer,
                    band_index: usize,
                    top_size: f32,
                    right_size: f32,
                    bottom_size: f32,
                    left_size: f32,
                    slabs: &mut Vec<ChromeSlab>| {
        if top_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Top,
                band_index,
                rect: Rect::new(x0, y0, x1 - x0, top_size),
            });
        }
        y0 += top_size;
        if bottom_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Bottom,
                band_index,
                rect: Rect::new(x0, y1 - bottom_size, x1 - x0, bottom_size),
            });
        }
        y1 -= bottom_size;
        if left_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Left,
                band_index,
                rect: Rect::new(x0, y0, left_size, y1 - y0),
            });
        }
        x0 += left_size;
        if right_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Right,
                band_index,
                rect: Rect::new(x1 - right_size, y0, right_size, y1 - y0),
            });
        }
        x1 -= right_size;
    };

    step(
        ChromeLayer::Margin,
        0,
        top.margin.size,
        right.margin.size,
        bottom.margin.size,
        left.margin.size,
        slabs,
    );
    for index in 0..band_steps {
        let band = |side: &crate::frame::SolvedFrameSide| {
            side.bands.get(index).map(|slab| slab.size).unwrap_or(0.0)
        };
        step(
            ChromeLayer::Band,
            index,
            band(top),
            band(right),
            band(bottom),
            band(left),
            slabs,
        );
    }
    step(
        ChromeLayer::Outer,
        0,
        top.outer.size,
        right.outer.size,
        bottom.outer.size,
        left.outer.size,
        slabs,
    );
    step(
        ChromeLayer::Inner,
        0,
        top.inner.size,
        right.inner.size,
        bottom.inner.size,
        left.inner.size,
        slabs,
    );
}

fn frame_side(side: &ChromeSide) -> FrameSide {
    FrameSide {
        margin: side.margin,
        bands: side.bands.clone(),
        outer: side.outer,
        inner: side.inner,
    }
}

fn place<Id: Clone, Key>(
    node: &Layout<Id, Key>,
    measured: &Measured,
    slot: Rect,
    granted: Edges<EdgeDemand>,
    depth: usize,
    path: &[usize],
    regions: &mut Vec<Region<Id>>,
) {
    let fills_slot = measured.grid.is_some();
    let horizontal = place_axis(
        node.chrome.sizing_x,
        &node.chrome.sides.left,
        &node.chrome.sides.right,
        node.chrome.content_min.width,
        measured.natural_content.width,
        fills_slot,
        node.align.0,
        slot.x,
        slot.width,
    );
    let vertical = place_axis(
        node.chrome.sizing_y,
        &node.chrome.sides.top,
        &node.chrome.sides.bottom,
        node.chrome.content_min.height,
        measured.natural_content.height,
        fills_slot,
        node.align.1,
        slot.y,
        slot.height,
    );
    let content = Rect::new(
        horizontal.content_start,
        vertical.content_start,
        horizontal.content_extent,
        vertical.content_extent,
    );

    let mut slabs = Vec::new();
    carve_slabs(&horizontal, &vertical, &mut slabs);

    let region_index = regions.len();
    regions.push(Region {
        id: node.id.clone(),
        path: path.to_vec(),
        depth,
        slot,
        content,
        slabs,
        requested: measured.demands,
        granted,
        detail: RegionDetail::Leaf, // patched below for grids
    });

    let Some(grid) = &measured.grid else {
        return;
    };
    let LayoutKind::Grid(spec) = &node.kind else {
        unreachable!("measured grid implies grid kind");
    };

    let solution = distribute(grid, spec, Size::new(content.width, content.height));

    regions[region_index].detail = RegionDetail::Grid {
        tracks: SolvedTracks {
            column_starts: solution
                .column_starts
                .iter()
                .map(|start| content.x + start)
                .collect(),
            column_sizes: solution.column_widths.clone(),
            row_starts: solution
                .row_starts
                .iter()
                .map(|start| content.y + start)
                .collect(),
            row_sizes: solution.row_heights.clone(),
        },
    };

    for (index, child) in spec.children.iter().enumerate() {
        let slot_rect = slot_rect_positional(&solution, child.slot, content);
        let child_granted = Edges::new(
            solution
                .row_top
                .get(child.slot.row)
                .copied()
                .unwrap_or_default(),
            solution
                .column_right
                .get(child.slot.column_end() - 1)
                .copied()
                .unwrap_or_default(),
            solution
                .row_bottom
                .get(child.slot.row_end() - 1)
                .copied()
                .unwrap_or_default(),
            solution
                .column_left
                .get(child.slot.column)
                .copied()
                .unwrap_or_default(),
        );
        let mut child_path = path.to_vec();
        child_path.push(index);
        place(
            &child.layout,
            &grid.children[index],
            slot_rect,
            child_granted,
            depth + 1,
            &child_path,
            regions,
        );
    }
}

/// A child's slot rectangle computed positionally (robust against
/// distribution modes that move starts without resizing tracks).
fn slot_rect_positional(
    solution: &GridSolution,
    slot: crate::grid::GridSlot,
    content: Rect,
) -> Rect {
    let x = solution.column_starts[slot.column];
    let y = solution.row_starts[slot.row];
    let last_column = slot.column_end() - 1;
    let last_row = slot.row_end() - 1;
    let width = solution.column_starts[last_column] + solution.column_widths[last_column] - x;
    let height = solution.row_starts[last_row] + solution.row_heights[last_row] - y;
    Rect::new(content.x + x, content.y + y, width, height)
}

/// Apply the grid's free-space policy for a target content size, returning
/// the solution to place children with.
fn distribute<Id, Key>(
    grid: &MeasuredGrid,
    spec: &GridSpec<Id, Key>,
    target: Size,
) -> GridSolution {
    let natural = &grid.natural;
    let free_x = (target.width - natural.content_size.width).max(0.0);
    let free_y = (target.height - natural.content_size.height).max(0.0);
    if free_x <= 0.0 && free_y <= 0.0 {
        return natural.clone();
    }

    let stretch_x = spec.distribute_x == Distribute::StretchTracks && free_x > 0.0;
    let stretch_y = spec.distribute_y == Distribute::StretchTracks && free_y > 0.0;
    let mut solution = if stretch_x || stretch_y {
        let mut requirements = grid.requirements.clone();
        if stretch_x && !requirements.column_widths.is_empty() {
            let extra = free_x / requirements.column_widths.len() as f32;
            for width in &mut requirements.column_widths {
                *width += extra;
            }
        }
        if stretch_y && !requirements.row_heights.is_empty() {
            let extra = free_y / requirements.row_heights.len() as f32;
            for height in &mut requirements.row_heights {
                *height += extra;
            }
        }
        requirements.solve(&grid.items)
    } else {
        natural.clone()
    };

    if !stretch_x && free_x > 0.0 {
        offset_starts(&mut solution.column_starts, spec.distribute_x, free_x);
    }
    if !stretch_y && free_y > 0.0 {
        offset_starts(&mut solution.row_starts, spec.distribute_y, free_y);
    }
    solution
}

fn offset_starts(starts: &mut [f32], distribute: Distribute, free: f32) {
    match distribute {
        Distribute::StretchTracks | Distribute::Start => {}
        Distribute::Center => {
            for start in starts.iter_mut() {
                *start += free / 2.0;
            }
        }
        Distribute::End => {
            for start in starts.iter_mut() {
                *start += free;
            }
        }
        Distribute::SpaceBetween => {
            let gaps = starts.len().saturating_sub(1);
            if gaps == 0 {
                return;
            }
            let extra = free / gaps as f32;
            for (index, start) in starts.iter_mut().enumerate() {
                *start += extra * index as f32;
            }
        }
    }
}

// --- entry point -------------------------------------------------------------

impl<Id: Clone + Eq + Hash, Key> Layout<Id, Key> {
    /// Solve this layout in one step: measure up, coordinate share groups,
    /// allocate down. See the [crate docs](crate) and [`SolveOptions`].
    pub fn solve(&self, options: &SolveOptions) -> Result<LayoutSolution<Id>, LayoutError> {
        let mut seen = HashSet::new();
        check_duplicate_ids(self, &mut seen)?;

        let measured = measure(self)?;

        // Root slot per axis: contained axes treat the allocation as the
        // box; `Envelope` axes treat it as the envelope around content plus
        // overflow (content gets the remainder).
        let (slot_x, slot_w, envelope_w) = root_axis(
            self.chrome.sizing_x,
            options.width,
            measured.item_size.width,
            measured.demands.left.total,
            measured.demands.right.total,
        );
        let (slot_y, slot_h, envelope_h) = root_axis(
            self.chrome.sizing_y,
            options.height,
            measured.item_size.height,
            measured.demands.top.total,
            measured.demands.bottom.total,
        );

        let mut regions = Vec::new();
        place(
            self,
            &measured,
            Rect::new(slot_x, slot_y, slot_w, slot_h),
            measured.demands,
            0,
            &[],
            &mut regions,
        );

        let root_content = regions[0].content;
        Ok(LayoutSolution {
            size: Size::new(envelope_w, envelope_h),
            envelope: Envelope {
                content_size: Size::new(root_content.width, root_content.height),
                layered: measured.demands,
                geometric_total: measured.geometric_total,
            },
            regions,
            diagnostics: Diagnostics::default(),
        })
    }
}

/// Root slot start/extent and envelope extent for one axis.
fn root_axis(
    sizing: SolveFor,
    allocation: Option<f32>,
    item_extent: f32,
    leading_total: f32,
    trailing_total: f32,
) -> (f32, f32, f32) {
    match sizing {
        SolveFor::Content | SolveFor::Margins => {
            // The item extent IS the box extent; the allocation overrides it.
            let extent = allocation.unwrap_or(item_extent).max(0.0);
            (0.0, extent, extent)
        }
        SolveFor::Envelope => {
            let content = match allocation {
                Some(extent) => (extent - leading_total - trailing_total).max(0.0),
                None => item_extent,
            };
            let envelope = content + leading_total + trailing_total;
            (leading_total, content, envelope)
        }
    }
}

fn check_duplicate_ids<'a, Id: Eq + Hash, Key>(
    node: &'a Layout<Id, Key>,
    seen: &mut HashSet<&'a Id>,
) -> Result<(), LayoutError> {
    if let Some(id) = &node.id {
        if !seen.insert(id) {
            return Err(LayoutError::DuplicateId);
        }
    }
    if let LayoutKind::Grid(spec) = &node.kind {
        for child in &spec.children {
            check_duplicate_ids(&child.layout, seen)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::build::{CellAlign, Distribute, Layout, LayoutError, SolveFor, SolveOptions};
    use crate::frame::{Frame, FrameAxis, FrameAxisSizing, FrameSide};
    use crate::geometry::{Edges, Rect, Side, Size};
    use crate::grid::{GridShape, GridSlot, TrackSpacing};
    use crate::region::EdgeDemand;
    use crate::tree::{LayoutItem, LayoutNode, LayoutSlotContent};

    fn chart_leaf() -> Layout<&'static str> {
        Layout::leaf(Size::default())
            .margin(8.0)
            .band(Side::Top, 18.0)
            .outer(Side::Right, 64.0)
            .inner(Side::Left, 38.0)
            .inner(Side::Bottom, 22.0)
            .content_min(Size::new(50.0, 40.0))
            .id("chart")
    }

    fn chart_frame(sizing_x: FrameAxisSizing, sizing_y: FrameAxisSizing) -> Frame {
        let side = |margin: f32, bands: &[f32], outer: f32, inner: f32| FrameSide {
            margin,
            bands: bands.to_vec(),
            outer,
            inner,
        };
        Frame {
            horizontal: FrameAxis {
                sizing: sizing_x,
                leading: side(8.0, &[], 0.0, 38.0),
                trailing: side(8.0, &[], 64.0, 0.0),
                content_min: 50.0,
            },
            vertical: FrameAxis {
                sizing: sizing_y,
                leading: side(8.0, &[18.0], 0.0, 0.0),
                trailing: side(8.0, &[], 0.0, 22.0),
                content_min: 40.0,
            },
        }
    }

    #[test]
    fn content_mode_matches_frame_envelope_fixed() {
        let solved = chart_leaf()
            .sizing(SolveFor::Content)
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(300.0),
            })
            .expect("solve");
        let frame = chart_frame(
            FrameAxisSizing::EnvelopeFixed { extent: 400.0 },
            FrameAxisSizing::EnvelopeFixed { extent: 300.0 },
        )
        .solve();

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame.content_rect());
        assert_eq!(solved.size, frame.extent());
        assert_eq!(region.slot, Rect::new(0.0, 0.0, 400.0, 300.0));
    }

    #[test]
    fn envelope_mode_matches_frame_content_fixed() {
        // Natural sizing: content given (the floor is the content here),
        // envelope derived. Chrome lifts as overflow, so the content rect
        // starts after the leading chrome totals.
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin(8.0)
            .band(Side::Top, 18.0)
            .outer(Side::Right, 64.0)
            .inner(Side::Left, 38.0)
            .inner(Side::Bottom, 22.0)
            .id("chart")
            .solve(&SolveOptions::default())
            .expect("solve");
        let frame = chart_frame(
            FrameAxisSizing::ContentFixed { content: 200.0 },
            FrameAxisSizing::ContentFixed { content: 150.0 },
        )
        .solve();

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame.content_rect());
        assert_eq!(solved.size, frame.extent());
    }

    #[test]
    fn margins_mode_matches_frame_both_fixed() {
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin(8.0)
            .band(Side::Top, 18.0)
            .outer(Side::Right, 64.0)
            .inner(Side::Left, 38.0)
            .inner(Side::Bottom, 22.0)
            .id("chart")
            .sizing(SolveFor::Margins)
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(300.0),
            })
            .expect("solve");
        let frame = chart_frame(
            FrameAxisSizing::EnvelopeAndContentFixed {
                extent: 400.0,
                content: 200.0,
            },
            FrameAxisSizing::EnvelopeAndContentFixed {
                extent: 300.0,
                content: 150.0,
            },
        )
        .solve();

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame.content_rect());
        assert_eq!(solved.size, frame.extent());
    }

    #[test]
    fn mixed_axis_allocation_solves_each_axis_independently() {
        // Width figure-constrained (solve for content), height
        // plot-area-sized (content given, envelope derived).
        let solved = Layout::<&str>::leaf(Size::new(0.0, 150.0))
            .margin(8.0)
            .inner(Side::Left, 38.0)
            .inner(Side::Bottom, 22.0)
            .sizing_x(SolveFor::Content)
            .id("chart")
            .solve(&SolveOptions {
                width: Some(400.0),
                height: None,
            })
            .expect("solve");

        let region = solved.region(&"chart").expect("chart region");
        // Horizontal: 400 - (8 + 38) - 8 = 346 content.
        assert_eq!(region.content.width, 346.0);
        assert_eq!(region.content.x, 46.0);
        assert_eq!(solved.size.width, 400.0);
        // Vertical: envelope derived = 8 + 150 + 22 + 8.
        assert_eq!(region.content.height, 150.0);
        assert_eq!(region.content.y, 8.0);
        assert_eq!(solved.size.height, 188.0);
    }

    fn tree_leaf(id: usize, column: usize, size: Size) -> LayoutItem<usize> {
        LayoutItem {
            id,
            slot: GridSlot {
                row: 0,
                column,
                row_span: 1,
                column_span: 1,
            },
            content: LayoutSlotContent::Leaf {
                content_size: size,
                inner_edges: Edges::default(),
                outer_edges: Edges::default(),
                total_edges: Edges::default(),
            },
        }
    }

    fn tree_band(items: Vec<LayoutItem<usize>>, min_gap: f32) -> LayoutNode<usize> {
        LayoutNode {
            shape: GridShape {
                rows: 1,
                columns: items.len(),
            },
            column_spacing: TrackSpacing {
                outer_start: 0.0,
                outer_end: 0.0,
                min_gap,
            },
            row_spacing: TrackSpacing::default(),
            base_cell_size: Size::default(),
            stacked_inner_edges: Edges::default(),
            stacked_outer_edges: Edges::default(),
            items,
        }
    }

    #[test]
    fn nested_grid_matches_tree_solver() {
        // The tree.rs nested-band case, expressed on both APIs.
        let old_inner = tree_band(
            vec![
                tree_leaf(10, 0, Size::new(40.0, 60.0)),
                tree_leaf(11, 1, Size::new(40.0, 60.0)),
            ],
            6.0,
        );
        let old_root = tree_band(
            vec![
                tree_leaf(0, 0, Size::new(50.0, 60.0)),
                LayoutItem {
                    id: 1,
                    slot: GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Node(old_inner),
                },
            ],
            10.0,
        );
        let old = old_root.solve(None).expect("tree solve");

        let new_root: Layout = Layout::row([
            Layout::leaf(Size::new(50.0, 60.0)),
            Layout::row([
                Layout::leaf(Size::new(40.0, 60.0)),
                Layout::leaf(Size::new(40.0, 60.0)),
            ])
            .min_gap(6.0),
        ])
        .min_gap(10.0);
        let new = new_root.solve(&SolveOptions::default()).expect("solve");

        assert_eq!(new.envelope().content_size, old.content_size);
        for old_region in &old.regions {
            let new_region = new.at_path(&old_region.path).expect("matching path");
            assert_eq!(
                new_region.content, old_region.content_rect,
                "content rect at path {:?}",
                old_region.path
            );
        }
    }

    #[test]
    fn stretch_grows_slots_but_not_leaf_content() {
        // Old tree law stretched leaf content rects to fill; the new law
        // reports honest content and a stretched slot (the allotment the
        // loop adopts).
        let root: Layout = Layout::row([Layout::row([
            Layout::leaf(Size::new(40.0, 60.0)),
            Layout::leaf(Size::new(40.0, 60.0)),
        ])]);
        let solved = root
            .solve(&SolveOptions {
                width: Some(120.0),
                height: None,
            })
            .expect("solve");

        let inner = solved.at_path(&[0]).expect("inner grid");
        assert_eq!(inner.content, Rect::new(0.0, 0.0, 120.0, 60.0));
        let first = solved.at_path(&[0, 0]).expect("first leaf");
        assert_eq!(first.slot, Rect::new(0.0, 0.0, 60.0, 60.0));
        assert_eq!(first.content, Rect::new(0.0, 0.0, 40.0, 60.0));
        let second = solved.at_path(&[0, 1]).expect("second leaf");
        assert_eq!(second.slot, Rect::new(60.0, 0.0, 60.0, 60.0));
        assert_eq!(second.content, Rect::new(60.0, 0.0, 40.0, 60.0));
    }

    #[test]
    fn cell_align_positions_ragged_children() {
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(40.0, 120.0)),
            Layout::leaf(Size::new(40.0, 70.0)).align_in_cell(CellAlign::Start, CellAlign::Center),
            Layout::leaf(Size::new(40.0, 95.0)).align_in_cell(CellAlign::Start, CellAlign::End),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        let centered = solved.at_path(&[1]).expect("centered");
        assert_eq!(centered.slot.height, 120.0);
        assert_eq!(centered.content.y, 25.0);
        assert_eq!(centered.content.height, 70.0);
        let end = solved.at_path(&[2]).expect("end");
        assert_eq!(end.content.y, 25.0);
        assert_eq!(end.content.height, 95.0);
    }

    #[test]
    fn grid_chrome_matches_stacked_edges_law() {
        // Old: stacked_inner_edges on the node. New: .inner on the grid.
        let mut old = tree_band(vec![tree_leaf(0, 0, Size::new(100.0, 60.0))], 0.0);
        old.items[0] = LayoutItem {
            id: 0,
            slot: old.items[0].slot,
            content: LayoutSlotContent::Leaf {
                content_size: Size::new(100.0, 60.0),
                inner_edges: Edges::default(),
                outer_edges: Edges::default(),
                total_edges: Edges::new(0.0, 15.0, 0.0, 0.0),
            },
        };
        old.stacked_inner_edges = Edges::new(0.0, 35.0, 0.0, 0.0);
        let old_envelope = old
            .envelope(crate::tree::TreeEnvelopeKind::Layered)
            .expect("envelope");

        let new: Layout = Layout::row([
            Layout::leaf(Size::new(100.0, 60.0)).demand(Side::Right, EdgeDemand::total(15.0))
        ])
        .inner(Side::Right, 35.0);
        let solved = new.solve(&SolveOptions::default()).expect("solve");

        assert_eq!(old_envelope.total_edges.right, 50.0);
        assert_eq!(solved.envelope().layered.right.total, 50.0);
        assert_eq!(
            solved.envelope().layered.right.inner,
            old_envelope.inner_edges.right
        );
    }

    #[test]
    fn envelope_carries_layered_and_geometric_views() {
        // Mixed dominance: all-guide top 10 vs all-legend top 8. Layered
        // lifts to 18; geometric reports the raw max 10.
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(100.0, 60.0))
                .demand(Side::Top, EdgeDemand::new(10.0, 0.0, 10.0)),
            Layout::leaf(Size::new(100.0, 60.0)).demand(Side::Top, EdgeDemand::new(0.0, 8.0, 8.0)),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        assert_eq!(solved.envelope().layered.top.total, 18.0);
        assert_eq!(solved.envelope().geometric_total.top, 10.0);
    }

    #[test]
    fn space_between_distributes_free_space_into_gaps() {
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(40.0, 20.0)),
            Layout::leaf(Size::new(40.0, 20.0)),
            Layout::leaf(Size::new(40.0, 20.0)),
        ])
        .distribute_x(Distribute::SpaceBetween);
        let solved = root
            .solve(&SolveOptions {
                width: Some(160.0),
                height: None,
            })
            .expect("solve");

        // 40 free over 2 gaps: starts 0, 60, 120.
        assert_eq!(solved.at_path(&[0]).unwrap().slot.x, 0.0);
        assert_eq!(solved.at_path(&[1]).unwrap().slot.x, 60.0);
        assert_eq!(solved.at_path(&[2]).unwrap().slot.x, 120.0);
        assert_eq!(solved.size.width, 160.0);
    }

    #[test]
    fn duplicate_ids_error() {
        let root: Layout<i32> = Layout::row([
            Layout::leaf(Size::default()).id(7),
            Layout::leaf(Size::default()).id(7),
        ]);
        let result = root.solve(&SolveOptions::default());
        assert_eq!(result.unwrap_err(), LayoutError::DuplicateId);
    }

    #[test]
    fn within_grid_top_demands_aggregate_per_track() {
        // Two plots in a row with different top inner demands: the shared
        // row reserves the max; both contents align by sharing the track;
        // granted reports the track-level demand.
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(100.0, 60.0))
                .demand(Side::Top, EdgeDemand::new(20.0, 0.0, 20.0)),
            Layout::leaf(Size::new(100.0, 60.0)).demand(Side::Top, EdgeDemand::new(8.0, 0.0, 8.0)),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        let first = solved.at_path(&[0]).unwrap();
        let second = solved.at_path(&[1]).unwrap();
        assert_eq!(first.content.y, second.content.y);
        assert_eq!(second.requested.top.inner, 8.0);
        assert_eq!(second.granted.top.inner, 20.0, "track-level grant");
        assert_eq!(solved.envelope().layered.top.inner, 20.0);
    }

    #[test]
    fn bands_on_all_four_sides_carve_with_corner_rule() {
        use crate::solution::{ChromeLayer, ChromeSlab};
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(10.0)
            .band(Side::Top, 20.0)
            .band(Side::Left, 30.0)
            .sizing(SolveFor::Content)
            .id("box")
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(300.0),
            })
            .expect("solve");

        let region = solved.region(&"box").expect("region");
        let slab = |layer: ChromeLayer, side: Side| -> &ChromeSlab {
            region
                .slabs
                .iter()
                .find(|slab| slab.layer == layer && slab.side == side)
                .expect("slab present")
        };

        // Margins carve first, vertical before horizontal: top/bottom span
        // the full box, left/right sit between them (corners belong to the
        // vertical slabs).
        assert_eq!(
            slab(ChromeLayer::Margin, Side::Top).rect,
            Rect::new(0.0, 0.0, 400.0, 10.0)
        );
        assert_eq!(
            slab(ChromeLayer::Margin, Side::Left).rect,
            Rect::new(0.0, 10.0, 10.0, 280.0)
        );
        // Band layer: the top band runs wider than the left band of the
        // same layer; the left band starts below the top band.
        assert_eq!(
            slab(ChromeLayer::Band, Side::Top).rect,
            Rect::new(10.0, 10.0, 380.0, 20.0)
        );
        assert_eq!(
            slab(ChromeLayer::Band, Side::Left).rect,
            Rect::new(10.0, 30.0, 30.0, 260.0)
        );
        assert_eq!(region.content, Rect::new(40.0, 30.0, 350.0, 260.0));
    }

    #[test]
    fn repeated_bands_stack_outside_in() {
        use crate::solution::{ChromeLayer, ChromeSlab};
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(8.0)
            .band(Side::Top, 18.0) // title (outermost)
            .band(Side::Top, 12.0) // subtitle (inside the title)
            .sizing(SolveFor::Content)
            .id("chart")
            .solve(&SolveOptions {
                width: Some(200.0),
                height: Some(100.0),
            })
            .expect("solve");

        let region = solved.region(&"chart").expect("region");
        let bands: Vec<&ChromeSlab> = region
            .slabs
            .iter()
            .filter(|slab| slab.layer == ChromeLayer::Band)
            .collect();
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[0].band_index, 0);
        assert_eq!(bands[0].rect, Rect::new(8.0, 8.0, 184.0, 18.0));
        assert_eq!(bands[1].band_index, 1);
        assert_eq!(bands[1].rect, Rect::new(8.0, 26.0, 184.0, 12.0));
    }

    #[test]
    fn horizontal_inner_slabs_are_content_sized_on_cross_axis() {
        use crate::solution::ChromeLayer;
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(8.0)
            .inner(Side::Left, 38.0)
            .inner(Side::Bottom, 22.0)
            .sizing(SolveFor::Content)
            .id("chart")
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(300.0),
            })
            .expect("solve");

        let region = solved.region(&"chart").expect("region");
        let left_inner = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Inner && slab.side == Side::Left)
            .expect("left inner");
        assert_eq!(left_inner.rect.y, region.content.y);
        assert_eq!(left_inner.rect.height, region.content.height);
        assert_eq!(left_inner.rect.x + left_inner.rect.width, region.content.x);
    }

    #[test]
    fn envelope_mode_chrome_hangs_outside_the_content_rect() {
        use crate::solution::ChromeLayer;
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin(8.0)
            .inner(Side::Left, 38.0)
            .id("chart")
            .solve(&SolveOptions::default())
            .expect("solve");

        let region = solved.region(&"chart").expect("region");
        assert_eq!(region.content, Rect::new(46.0, 8.0, 200.0, 150.0));
        let left_inner = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Inner && slab.side == Side::Left)
            .expect("left inner");
        assert_eq!(left_inner.rect, Rect::new(8.0, 8.0, 38.0, 150.0));
        let left_margin = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Margin && slab.side == Side::Left)
            .expect("left margin");
        assert_eq!(left_margin.rect.x, 0.0);
    }

    #[test]
    fn margins_mode_carves_solved_flexible_margins() {
        use crate::solution::ChromeLayer;
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin_edges(Edges::new(0.0, 13.0, 0.0, 7.0))
            .sizing_x(SolveFor::Margins)
            .sizing_y(SolveFor::Envelope)
            .id("box")
            .solve(&SolveOptions {
                width: Some(400.0),
                height: None,
            })
            .expect("solve");

        let region = solved.region(&"box").expect("region");
        // Declared margins are ignored on the Margins axis; each side gets
        // half the 200 slack.
        let left = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Margin && slab.side == Side::Left)
            .expect("left margin");
        let right = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Margin && slab.side == Side::Right)
            .expect("right margin");
        assert_eq!(left.rect.width, 100.0);
        assert_eq!(right.rect.width, 100.0);
        assert_eq!(region.content.width, 200.0);
    }
}
