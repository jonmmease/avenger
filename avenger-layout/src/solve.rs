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

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::build::{
    CellAlign, ChromeSide, Distribute, GridSpec, Layout, LayoutError, LayoutKind, SolveFor,
    SolveOptions, TrackSize,
};
use crate::frame::{FrameAxis, FrameAxisSizing, FrameSide};
use crate::geometry::{Edges, Rect, Size};
use crate::grid::{GridItem, GridRequirements, GridSolution, TrackGrowth};
use crate::region::EdgeGrant;
use crate::solution::{
    Diagnostics, Envelope, LayoutSolution, Region, RegionDetail, SkippedShare, SkippedShareReason,
    SolvedTracks,
};

// --- measurement -----------------------------------------------------------

/// Bottom-up measurement of one node.
pub(crate) struct Measured {
    /// What the parent's grid consumes as this item's content size:
    /// the content extent on `Envelope` axes, the box extent on contained
    /// axes.
    pub(crate) item_size: Size,
    /// Lifted overflow toward the parent (zero on contained axes).
    pub(crate) demands: Edges<EdgeGrant>,
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
    pub(crate) column_growth: Option<Vec<TrackGrowth>>,
    pub(crate) row_growth: Option<Vec<TrackGrowth>>,
}

/// Map declared track sizes onto growth kinds (missing entries are `Auto`).
fn growth_vector(declared: Option<&[TrackSize]>, count: usize) -> Option<Vec<TrackGrowth>> {
    declared.map(|declared| {
        (0..count)
            .map(|index| match declared.get(index) {
                Some(TrackSize::Fixed(_)) => TrackGrowth::Fixed,
                Some(TrackSize::Flex(_)) => TrackGrowth::Flex,
                Some(TrackSize::Auto) | None => TrackGrowth::Auto,
            })
            .collect()
    })
}

/// Pin `Fixed` tracks to their declared pixels (rigid: content neither
/// grows nor shrinks them; an oversized child overflows).
fn pin_fixed_tracks(sizes: &mut [f32], declared: &[TrackSize]) {
    for (index, size) in sizes.iter_mut().enumerate() {
        if let Some(TrackSize::Fixed(pixels)) = declared.get(index) {
            *size = pixels.max(0.0);
        }
    }
}

/// Stack one side's declared chrome onto the content's own demand
/// (the `stacked_guide/legend_edges` law: additive per layer; strips and
/// margins extend `total` only).
fn lift_chrome(demand: EdgeGrant, chrome: &ChromeSide) -> EdgeGrant {
    EdgeGrant::new(
        demand.guide + chrome.guide.max(0.0),
        demand.legend + chrome.legend.max(0.0),
        demand.total + chrome_total(chrome),
    )
}

fn chrome_total(side: &ChromeSide) -> f32 {
    let mut total = side.margin.max(0.0);
    for strip in &side.strips {
        total += strip.max(0.0);
    }
    total + side.legend.max(0.0) + side.guide.max(0.0)
}

/// Fixed (non-margin) chrome on one side: what `Margins` mode keeps rigid.
fn chrome_fixed_total(side: &ChromeSide) -> f32 {
    chrome_total(side) - side.margin.max(0.0)
}

/// Merged floors patched into one shared grid during the pass-2 re-measure.
#[derive(Clone, Debug)]
pub(crate) struct SharePatch {
    pub(crate) column: AxisPatch,
    pub(crate) row: AxisPatch,
}

#[derive(Clone, Debug)]
pub(crate) enum AxisPatch {
    /// Equal-shape merge: per-track floors.
    PerTrack {
        sizes: Vec<f32>,
        leading: Vec<EdgeGrant>,
        trailing: Vec<EdgeGrant>,
        spacing: crate::grid::TrackSpacing,
    },
    /// Uniform policy merge (ragged-tolerant): one track-size floor plus
    /// first/last edge chrome.
    Uniform {
        size: f32,
        first: EdgeGrant,
        last: EdgeGrant,
        spacing: crate::grid::TrackSpacing,
    },
}

fn apply_axis_patch(
    patch: &AxisPatch,
    sizes: &mut [f32],
    leading: &mut [EdgeGrant],
    trailing: &mut [EdgeGrant],
    spacing: &mut crate::grid::TrackSpacing,
) {
    match patch {
        AxisPatch::PerTrack {
            sizes: merged_sizes,
            leading: merged_leading,
            trailing: merged_trailing,
            spacing: merged_spacing,
        } => {
            // Coordination only raises floors: max with the re-measured
            // values so a pass-2 cascade can still grow past the pass-1
            // merge (residual converges through the caller's loop).
            for (size, merged) in sizes.iter_mut().zip(merged_sizes) {
                *size = size.max(*merged);
            }
            for (edge, merged) in leading.iter_mut().zip(merged_leading) {
                *edge = edge.max_components(*merged);
            }
            for (edge, merged) in trailing.iter_mut().zip(merged_trailing) {
                *edge = edge.max_components(*merged);
            }
            *spacing = spacing.merge_max(*merged_spacing);
        }
        AxisPatch::Uniform {
            size: merged_size,
            first,
            last,
            spacing: merged_spacing,
        } => {
            for size in sizes.iter_mut() {
                *size = size.max(*merged_size);
            }
            if let Some(edge) = leading.first_mut() {
                *edge = edge.max_components(*first);
            }
            if let Some(edge) = trailing.last_mut() {
                *edge = edge.max_components(*last);
            }
            *spacing = spacing.merge_max(*merged_spacing);
        }
    }
}

pub(crate) fn measure<Id: Clone, Key>(node: &Layout<Id, Key>) -> Result<Measured, LayoutError> {
    let mut path = Vec::new();
    measure_with(node, &mut path, &HashMap::new())
}

pub(crate) fn measure_with<Id: Clone, Key>(
    node: &Layout<Id, Key>,
    path: &mut Vec<usize>,
    patches: &HashMap<Vec<usize>, SharePatch>,
) -> Result<Measured, LayoutError> {
    let (natural_content, content_demands, mut geometric_total, grid) = match &node.kind {
        LayoutKind::Leaf {
            content_size,
            demands,
        } => {
            // Declarations enter grant space here: everything downstream of
            // measurement works in solved/merged values.
            let grants = Edges::new(
                EdgeGrant::from(demands.top),
                EdgeGrant::from(demands.right),
                EdgeGrant::from(demands.bottom),
                EdgeGrant::from(demands.left),
            );
            let geometric = Edges::new(
                grants.top.total,
                grants.right.total,
                grants.bottom.total,
                grants.left.total,
            );
            (
                Size::new(content_size.width.max(0.0), content_size.height.max(0.0)),
                grants,
                geometric,
                None,
            )
        }
        LayoutKind::Grid(spec) => {
            let mut children = Vec::with_capacity(spec.children.len());
            let mut items = Vec::with_capacity(spec.children.len());
            for (index, child) in spec.children.iter().enumerate() {
                path.push(index);
                let measured = measure_with(&child.layout, path, patches)?;
                path.pop();
                items.push(GridItem {
                    id: index,
                    slot: child.slot,
                    content_size: measured.item_size,
                    guide_edges: Edges::new(
                        measured.demands.top.guide,
                        measured.demands.right.guide,
                        measured.demands.bottom.guide,
                        measured.demands.left.guide,
                    ),
                    legend_edges: Edges::new(
                        measured.demands.top.legend,
                        measured.demands.right.legend,
                        measured.demands.bottom.legend,
                        measured.demands.left.legend,
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
            if spec.uniform_columns && spec.column_sizes.is_none() {
                equalize(&mut requirements.column_widths);
            }
            if spec.uniform_rows && spec.row_sizes.is_none() {
                equalize(&mut requirements.row_heights);
            }
            if let Some(declared) = spec.column_sizes.as_deref() {
                pin_fixed_tracks(&mut requirements.column_widths, declared);
            }
            if let Some(declared) = spec.row_sizes.as_deref() {
                pin_fixed_tracks(&mut requirements.row_heights, declared);
            }
            if let Some(patch) = patches.get(path.as_slice()) {
                apply_axis_patch(
                    &patch.column,
                    &mut requirements.column_widths,
                    &mut requirements.column_left,
                    &mut requirements.column_right,
                    &mut requirements.column_spacing,
                );
                apply_axis_patch(
                    &patch.row,
                    &mut requirements.row_heights,
                    &mut requirements.row_top,
                    &mut requirements.row_bottom,
                    &mut requirements.row_spacing,
                );
                // Fixed tracks stay rigid even against merged floors
                // (group members declare identical TrackSize vectors).
                if let Some(declared) = spec.column_sizes.as_deref() {
                    pin_fixed_tracks(&mut requirements.column_widths, declared);
                }
                if let Some(declared) = spec.row_sizes.as_deref() {
                    pin_fixed_tracks(&mut requirements.row_heights, declared);
                }
            }
            let column_growth = growth_vector(spec.column_sizes.as_deref(), spec.shape.columns);
            let row_growth = growth_vector(spec.row_sizes.as_deref(), spec.shape.rows);
            let natural = requirements.solve_with_growth(
                &items,
                column_growth.as_deref(),
                row_growth.as_deref(),
            );

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
                    column_growth,
                    row_growth,
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
            demands.left = EdgeGrant::default();
            demands.right = EdgeGrant::default();
            geometric_total.left = 0.0;
            geometric_total.right = 0.0;
        }
        SolveFor::Margins => {
            item_size.width = chrome_fixed_total(&chrome.sides.left)
                + natural_content.width
                + chrome_fixed_total(&chrome.sides.right);
            demands.left = EdgeGrant::default();
            demands.right = EdgeGrant::default();
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
            demands.top = EdgeGrant::default();
            demands.bottom = EdgeGrant::default();
            geometric_total.top = 0.0;
            geometric_total.bottom = 0.0;
        }
        SolveFor::Margins => {
            item_size.height = chrome_fixed_total(&chrome.sides.top)
                + natural_content.height
                + chrome_fixed_total(&chrome.sides.bottom);
            demands.top = EdgeGrant::default();
            demands.bottom = EdgeGrant::default();
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

#[allow(clippy::too_many_arguments)]
fn place_axis(
    sizing: SolveFor,
    leading: &ChromeSide,
    trailing: &ChromeSide,
    content_min: f32,
    natural_content: f32,
    content_target: f32,
    align: CellAlign,
    slot_start: f32,
    slot_extent: f32,
) -> AxisPlacement {
    match sizing {
        SolveFor::Envelope => {
            let content_extent = content_target;
            let offset = align.offset(slot_extent, content_extent);
            let content_start = slot_start + offset;
            let axis = FrameAxis {
                sizing: FrameAxisSizing::Content {
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
                    SolveFor::Content => FrameAxisSizing::Envelope {
                        extent: slot_extent,
                    },
                    _ => FrameAxisSizing::EnvelopeAndContent {
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
/// Layers carve from the outside in (margin → strips → legend → guide); within
/// one layer, vertical sides (top/bottom) carve before horizontal
/// (left/right), so a top strip runs wider than a left strip of the same layer
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

    let strip_steps = top
        .strips
        .len()
        .max(bottom.strips.len())
        .max(left.strips.len())
        .max(right.strips.len());

    // One carving step: sizes per side for this layer, in vertical-first
    // order. Emits non-empty slabs and shrinks the remaining rect.
    let mut step = |layer: ChromeLayer,
                    strip_index: usize,
                    top_size: f32,
                    right_size: f32,
                    bottom_size: f32,
                    left_size: f32,
                    slabs: &mut Vec<ChromeSlab>| {
        if top_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Top,
                strip_index,
                rect: Rect::new(x0, y0, x1 - x0, top_size),
            });
        }
        y0 += top_size;
        if bottom_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Bottom,
                strip_index,
                rect: Rect::new(x0, y1 - bottom_size, x1 - x0, bottom_size),
            });
        }
        y1 -= bottom_size;
        if left_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Left,
                strip_index,
                rect: Rect::new(x0, y0, left_size, y1 - y0),
            });
        }
        x0 += left_size;
        if right_size > 0.0 {
            slabs.push(ChromeSlab {
                layer,
                side: Side::Right,
                strip_index,
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
    for index in 0..strip_steps {
        let strip = |side: &crate::frame::SolvedFrameSide| {
            side.strips.get(index).map(|slab| slab.size).unwrap_or(0.0)
        };
        step(
            ChromeLayer::Strip,
            index,
            strip(top),
            strip(right),
            strip(bottom),
            strip(left),
            slabs,
        );
    }
    step(
        ChromeLayer::Legend,
        0,
        top.legend.size,
        right.legend.size,
        bottom.legend.size,
        left.legend.size,
        slabs,
    );
    step(
        ChromeLayer::Guide,
        0,
        top.guide.size,
        right.guide.size,
        bottom.guide.size,
        left.guide.size,
        slabs,
    );
}

fn frame_side(side: &ChromeSide) -> FrameSide {
    FrameSide {
        margin: side.margin,
        strips: side.strips.clone(),
        legend: side.legend,
        guide: side.guide,
    }
}

/// Runtime state for share-key coordination during placement.
///
/// The dry pass records each shared grid's free-space offer (its slot minus
/// its coordinated natural extent); the real pass stretches every group
/// member by the group's **minimum** offer (the min-slack rule: free space
/// is distributed in policy space, never per instance, so cousins stay
/// congruent and none overflows).
struct ShareState<'a> {
    by_path: &'a HashMap<Vec<usize>, usize>,
    /// Per group: `[x, y]` slack. Dry pass: collecting minima
    /// (starts at infinity). Real pass: the granted slack.
    slack: Vec<[f32; 2]>,
    dry: bool,
}

impl ShareState<'_> {
    fn group(&self, path: &[usize]) -> Option<usize> {
        if self.by_path.is_empty() {
            return None;
        }
        self.by_path.get(path).copied()
    }
}

#[allow(clippy::too_many_arguments)]
fn place<Id: Clone, Key>(
    node: &Layout<Id, Key>,
    measured: &Measured,
    requested: &Measured,
    slot: Rect,
    granted: Edges<EdgeGrant>,
    depth: usize,
    path: &[usize],
    regions: &mut Vec<Region<Id>>,
    shares: &mut ShareState<'_>,
) {
    // Content target on Envelope axes: leaves keep their measured size;
    // unshared grids fill their slot; shared grids stretch only by the
    // group's min-slack (policy space).
    let group = if measured.grid.is_some() {
        shares.group(path)
    } else {
        None
    };
    let (target_w, target_h) = match (&measured.grid, group) {
        (None, _) => (
            measured.natural_content.width,
            measured.natural_content.height,
        ),
        (Some(_), None) => (slot.width, slot.height),
        (Some(_), Some(group)) => {
            if shares.dry {
                (
                    measured.natural_content.width,
                    measured.natural_content.height,
                )
            } else {
                let slack = shares.slack[group];
                (
                    (measured.natural_content.width + slack[0]).min(slot.width),
                    (measured.natural_content.height + slack[1]).min(slot.height),
                )
            }
        }
    };

    let horizontal = place_axis(
        node.chrome.sizing_x,
        &node.chrome.sides.left,
        &node.chrome.sides.right,
        node.chrome.content_min.width,
        measured.natural_content.width,
        target_w,
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
        target_h,
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

    // Dry pass: record this shared grid's free-space offer.
    if let Some(group) = group
        && shares.dry
    {
        let offer_x = (content_avail(&horizontal, node.chrome.sizing_x, slot.width)
            - measured.natural_content.width)
            .max(0.0);
        let offer_y = (content_avail(&vertical, node.chrome.sizing_y, slot.height)
            - measured.natural_content.height)
            .max(0.0);
        let slack = &mut shares.slack[group];
        slack[0] = slack[0].min(offer_x);
        slack[1] = slack[1].min(offer_y);
    }

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
        requested: requested.demands,
        coordinated: measured.demands,
        granted,
        geometric_total: measured.geometric_total,
        detail: RegionDetail::Leaf, // patched below for grids
    });

    let Some(grid) = &measured.grid else {
        return;
    };
    let LayoutKind::Grid(spec) = &node.kind else {
        unreachable!("measured grid implies grid kind");
    };

    // Track distribution target: shared grids distribute only up to their
    // policy-space extent even when a contained-mode box gave them more.
    let track_target = match group {
        Some(group) if !shares.dry => {
            let slack = shares.slack[group];
            Size::new(
                (measured.natural_content.width + slack[0]).min(content.width),
                (measured.natural_content.height + slack[1]).min(content.height),
            )
        }
        Some(_) => measured.natural_content,
        None => Size::new(content.width, content.height),
    };
    let solution = distribute(grid, spec, track_target);

    regions[region_index].detail = RegionDetail::Grid {
        tracks: SolvedTracks {
            column_starts: solution.column_starts.clone(),
            column_sizes: solution.column_widths.clone(),
            row_starts: solution.row_starts.clone(),
            row_sizes: solution.row_heights.clone(),
            column_spacing: solution.column_spacing,
            row_spacing: solution.row_spacing,
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
        let child_requested = requested
            .grid
            .as_ref()
            .map(|requested_grid| &requested_grid.children[index])
            .unwrap_or(&grid.children[index]);
        place(
            &child.layout,
            &grid.children[index],
            child_requested,
            slot_rect,
            child_granted,
            depth + 1,
            &child_path,
            regions,
            shares,
        );
    }
}

/// The content extent available to a node on one axis: its slot on
/// `Envelope` axes, the frame-solved content on contained axes.
fn content_avail(placement: &AxisPlacement, sizing: SolveFor, slot_extent: f32) -> f32 {
    match sizing {
        SolveFor::Envelope => slot_extent,
        SolveFor::Content | SolveFor::Margins => placement.chrome.content.size,
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

/// Grow one axis's tracks for `free` extra space. `Flex` tracks (if any)
/// take the leftover by weight; otherwise `StretchTracks` spreads it evenly
/// over non-`Fixed` tracks. Returns whether sizes changed (requiring a
/// re-solve); `false` leaves the free space to `offset_starts`.
fn expand_tracks(
    sizes: &mut [f32],
    free: f32,
    distribute: Distribute,
    declared: Option<&[TrackSize]>,
) -> bool {
    if free <= 0.0 || sizes.is_empty() {
        return false;
    }
    if let Some(declared) = declared {
        let weights: Vec<(usize, f32)> = declared
            .iter()
            .enumerate()
            .filter_map(|(index, track)| match track {
                TrackSize::Flex(weight) if *weight > 0.0 && index < sizes.len() => {
                    Some((index, *weight))
                }
                _ => None,
            })
            .collect();
        let total: f32 = weights.iter().map(|(_, weight)| weight).sum();
        if total > 0.0 {
            for (index, weight) in weights {
                sizes[index] += free * weight / total;
            }
            return true;
        }
    }
    if distribute != Distribute::StretchTracks {
        return false;
    }
    let stretchable: Vec<usize> = (0..sizes.len())
        .filter(|&index| {
            !matches!(
                declared.and_then(|declared| declared.get(index)),
                Some(TrackSize::Fixed(_))
            )
        })
        .collect();
    if stretchable.is_empty() {
        return false; // all Fixed: free space trails.
    }
    let extra = free / stretchable.len() as f32;
    for index in stretchable {
        sizes[index] += extra;
    }
    true
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

    let mut requirements = grid.requirements.clone();
    let absorbed_x = expand_tracks(
        &mut requirements.column_widths,
        free_x,
        spec.distribute_x,
        spec.column_sizes.as_deref(),
    );
    let absorbed_y = expand_tracks(
        &mut requirements.row_heights,
        free_y,
        spec.distribute_y,
        spec.row_sizes.as_deref(),
    );
    let mut solution = if absorbed_x || absorbed_y {
        requirements.solve_with_growth(
            &grid.items,
            grid.column_growth.as_deref(),
            grid.row_growth.as_deref(),
        )
    } else {
        natural.clone()
    };

    if !absorbed_x && free_x > 0.0 {
        offset_starts(&mut solution.column_starts, spec.distribute_x, free_x);
    }
    if !absorbed_y && free_y > 0.0 {
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

// --- share-key coordination --------------------------------------------------

/// One shared grid collected from the pass-1 measurement.
struct ShareEntry<'a, Key> {
    key: &'a Key,
    path: Vec<usize>,
    uniform_columns: bool,
    uniform_rows: bool,
    column_sizes: Option<&'a [crate::build::TrackSize]>,
    row_sizes: Option<&'a [crate::build::TrackSize]>,
    requirements: &'a GridRequirements,
}

fn collect_shares<'a, Id, Key>(
    node: &'a Layout<Id, Key>,
    measured: &'a Measured,
    path: &mut Vec<usize>,
    entries: &mut Vec<ShareEntry<'a, Key>>,
    diagnostics: &mut Diagnostics,
) {
    let (LayoutKind::Grid(spec), Some(grid)) = (&node.kind, &measured.grid) else {
        return;
    };
    if (spec.uniform_columns && spec.column_sizes.is_some())
        || (spec.uniform_rows && spec.row_sizes.is_some())
    {
        // Contradictory: declared track sizes win, uniform is ignored.
        diagnostics.uniform_conflicts += 1;
    }
    if let Some(key) = &spec.share {
        entries.push(ShareEntry {
            key,
            path: path.clone(),
            uniform_columns: spec.uniform_columns && spec.column_sizes.is_none(),
            uniform_rows: spec.uniform_rows && spec.row_sizes.is_none(),
            column_sizes: spec.column_sizes.as_deref(),
            row_sizes: spec.row_sizes.as_deref(),
            requirements: &grid.requirements,
        });
    }
    for (index, child) in spec.children.iter().enumerate() {
        path.push(index);
        collect_shares(
            &child.layout,
            &grid.children[index],
            path,
            entries,
            diagnostics,
        );
        path.pop();
    }
}

enum ShareAxis {
    Column,
    Row,
}

/// Merge one axis of a share group: uniform policy merge when every member
/// is uniform on the axis (ragged-tolerant), per-track merge when shapes
/// agree, `None` otherwise.
fn merge_share_axis<Key>(
    members: &[usize],
    entries: &[ShareEntry<'_, Key>],
    axis: ShareAxis,
) -> Option<AxisPatch> {
    let sizes = |m: usize| -> &[f32] {
        match axis {
            ShareAxis::Column => &entries[m].requirements.column_widths,
            ShareAxis::Row => &entries[m].requirements.row_heights,
        }
    };
    let leading = |m: usize| -> &[EdgeGrant] {
        match axis {
            ShareAxis::Column => &entries[m].requirements.column_left,
            ShareAxis::Row => &entries[m].requirements.row_top,
        }
    };
    let trailing = |m: usize| -> &[EdgeGrant] {
        match axis {
            ShareAxis::Column => &entries[m].requirements.column_right,
            ShareAxis::Row => &entries[m].requirements.row_bottom,
        }
    };
    let spacing = |m: usize| match axis {
        ShareAxis::Column => entries[m].requirements.column_spacing,
        ShareAxis::Row => entries[m].requirements.row_spacing,
    };
    let uniform = |m: usize| match axis {
        ShareAxis::Column => entries[m].uniform_columns,
        ShareAxis::Row => entries[m].uniform_rows,
    };

    let merged_spacing = members
        .iter()
        .map(|&m| spacing(m))
        .fold(crate::grid::TrackSpacing::default(), |a, b| a.merge_max(b));

    if members.iter().all(|&m| uniform(m)) {
        let size = members
            .iter()
            .flat_map(|&m| sizes(m).iter().copied())
            .fold(0.0f32, f32::max);
        let first = members
            .iter()
            .filter_map(|&m| leading(m).first().copied())
            .fold(EdgeGrant::default(), |a, b| a.max_components(b));
        let last = members
            .iter()
            .filter_map(|&m| trailing(m).last().copied())
            .fold(EdgeGrant::default(), |a, b| a.max_components(b));
        return Some(AxisPatch::Uniform {
            size,
            first,
            last,
            spacing: merged_spacing,
        });
    }

    let count = sizes(members[0]).len();
    if members.iter().any(|&m| sizes(m).len() != count) {
        return None;
    }
    let mut merged_sizes = sizes(members[0]).to_vec();
    let mut merged_leading = leading(members[0]).to_vec();
    let mut merged_trailing = trailing(members[0]).to_vec();
    for &m in &members[1..] {
        for (target, source) in merged_sizes.iter_mut().zip(sizes(m)) {
            *target = target.max(*source);
        }
        for (target, source) in merged_leading.iter_mut().zip(leading(m)) {
            *target = target.max_components(*source);
        }
        for (target, source) in merged_trailing.iter_mut().zip(trailing(m)) {
            *target = target.max_components(*source);
        }
    }
    Some(AxisPatch::PerTrack {
        sizes: merged_sizes,
        leading: merged_leading,
        trailing: merged_trailing,
        spacing: merged_spacing,
    })
}

type SharePlan = (
    HashMap<Vec<usize>, SharePatch>,
    HashMap<Vec<usize>, usize>,
    usize,
);

fn plan_shares<Key: Eq + Hash>(
    entries: &[ShareEntry<'_, Key>],
    diagnostics: &mut Diagnostics,
) -> SharePlan {
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut index_of: HashMap<&Key, usize> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        match index_of.get(entry.key) {
            Some(&group) => order[group].push(index),
            None => {
                index_of.insert(entry.key, order.len());
                order.push(vec![index]);
            }
        }
    }

    let mut patches = HashMap::new();
    let mut by_path = HashMap::new();
    let mut group_count = 0;
    for members in order {
        let skip = |reason: SkippedShareReason, diagnostics: &mut Diagnostics| {
            diagnostics.skipped_groups.push(SkippedShare {
                member_paths: members.iter().map(|&m| entries[m].path.clone()).collect(),
                reason,
            });
        };
        let first = &entries[members[0]];
        if members.iter().any(|&m| {
            entries[m].column_sizes != first.column_sizes || entries[m].row_sizes != first.row_sizes
        }) {
            skip(SkippedShareReason::TrackSizeMismatch, diagnostics);
            continue;
        }
        let column = merge_share_axis(&members, entries, ShareAxis::Column);
        let row = merge_share_axis(&members, entries, ShareAxis::Row);
        let (Some(column), Some(row)) = (column, row) else {
            skip(SkippedShareReason::ShapeMismatch, diagnostics);
            continue;
        };
        let patch = SharePatch { column, row };
        for &member in &members {
            patches.insert(entries[member].path.clone(), patch.clone());
            by_path.insert(entries[member].path.clone(), group_count);
        }
        group_count += 1;
    }
    (patches, by_path, group_count)
}

// --- entry point -------------------------------------------------------------

impl<Id: Clone + Eq + Hash, Key: Eq + Hash> Layout<Id, Key> {
    /// Solve this layout in one step: measure up, coordinate share groups
    /// (one pure round), allocate down. See the [crate docs](crate) and
    /// [`SolveOptions`].
    ///
    /// # Errors
    /// Returns an error for invalid slots, duplicate IDs, non-finite input
    /// values, or coordinates that overflow the finite `f32` range.
    pub fn solve(&self, options: &SolveOptions) -> Result<LayoutSolution<Id>, LayoutError> {
        finite_values(
            "canvas size",
            options.width.into_iter().chain(options.height),
        )?;
        validate_inputs(self)?;
        let mut seen = HashSet::new();
        check_duplicate_ids(self, &mut seen)?;

        // Pass 1: natural measurement; collect shared grids.
        let pass1 = measure(self)?;
        let mut diagnostics = Diagnostics::default();
        let mut entries = Vec::new();
        let mut walk_path = Vec::new();
        collect_shares(self, &pass1, &mut walk_path, &mut entries, &mut diagnostics);
        let (patches, by_path, group_count) = plan_shares(&entries, &mut diagnostics);
        drop(entries);

        // Pass 2: re-measure with the merged floors patched in; the patched
        // envelopes cascade through ancestors.
        let coordinated;
        let measured = if patches.is_empty() {
            &pass1
        } else {
            coordinated = measure_with(self, &mut Vec::new(), &patches)?;
            &coordinated
        };

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
        let root_slot = Rect::new(slot_x, slot_y, slot_w, slot_h);

        // Min-slack: a dry placement records every shared grid's free-space
        // offer; the real pass stretches each group by the minimum.
        let mut state = ShareState {
            by_path: &by_path,
            slack: vec![[f32::INFINITY; 2]; group_count],
            dry: true,
        };
        if group_count > 0 {
            let mut scratch = Vec::new();
            place(
                self,
                measured,
                &pass1,
                root_slot,
                measured.demands,
                0,
                &[],
                &mut scratch,
                &mut state,
            );
            for slack in &mut state.slack {
                for value in slack.iter_mut() {
                    if !value.is_finite() {
                        *value = 0.0;
                    }
                }
            }
        }
        state.dry = false;

        let mut regions = Vec::new();
        place(
            self,
            measured,
            &pass1,
            root_slot,
            measured.demands,
            0,
            &[],
            &mut regions,
            &mut state,
        );

        // The root box can legitimately outgrow a contained-axis allocation
        // (the content_min floor wins over the envelope); the solved size
        // covers the actual root geometry.
        let root = &regions[0];
        let mut max_x = root.content.x + root.content.width;
        let mut max_y = root.content.y + root.content.height;
        for slab in &root.slabs {
            max_x = max_x.max(slab.rect.x + slab.rect.width);
            max_y = max_y.max(slab.rect.y + slab.rect.height);
        }
        let size = Size::new(envelope_w.max(max_x), envelope_h.max(max_y));

        if !size.width.is_finite()
            || !size.height.is_finite()
            || regions.iter().any(|region| !region_is_finite(region))
        {
            return Err(LayoutError::CoordinateOverflow);
        }
        let root_content = regions[0].content;
        Ok(LayoutSolution {
            size,
            envelope: Envelope {
                content_size: Size::new(root_content.width, root_content.height),
                coordinated: measured.demands,
                geometric_total: measured.geometric_total,
            },
            regions,
            diagnostics,
        })
    }
}

fn finite_values(
    field: &'static str,
    values: impl IntoIterator<Item = f32>,
) -> Result<(), LayoutError> {
    if values.into_iter().all(f32::is_finite) {
        Ok(())
    } else {
        Err(LayoutError::NonFiniteInput { field })
    }
}

fn validate_inputs<Id, Key>(node: &Layout<Id, Key>) -> Result<(), LayoutError> {
    use crate::Side;
    finite_values(
        "content minimum",
        [
            node.chrome.content_min.width,
            node.chrome.content_min.height,
        ],
    )?;
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        let chrome = node.chrome.sides.side(side);
        finite_values(
            "edge reservation",
            [chrome.margin, chrome.legend, chrome.guide]
                .into_iter()
                .chain(chrome.strips.iter().copied()),
        )?;
    }
    match &node.kind {
        LayoutKind::Leaf {
            content_size,
            demands,
        } => {
            finite_values("content size", [content_size.width, content_size.height])?;
            for edge in [demands.top, demands.right, demands.bottom, demands.left] {
                finite_values("edge demand", [edge.guide, edge.legend])?;
            }
        }
        LayoutKind::Grid(spec) => {
            finite_values(
                "base cell size",
                [spec.base_cell_size.width, spec.base_cell_size.height],
            )?;
            for spacing in [spec.column_spacing, spec.row_spacing] {
                finite_values(
                    "track spacing",
                    [spacing.outer_start, spacing.outer_end, spacing.min_gap],
                )?;
            }
            for track in spec
                .column_sizes
                .iter()
                .chain(spec.row_sizes.iter())
                .flatten()
            {
                if let TrackSize::Fixed(value) | TrackSize::Flex(value) = track {
                    finite_values("track size or weight", [*value])?;
                }
            }
            for child in &spec.children {
                if !child.slot.fits(spec.shape) {
                    return Err(LayoutError::SlotOutOfBounds {
                        slot: child.slot,
                        shape: spec.shape,
                    });
                }
                validate_inputs(&child.layout)?;
            }
        }
    }
    Ok(())
}

fn region_is_finite<Id>(region: &Region<Id>) -> bool {
    let rect_is_finite = |rect: Rect| {
        [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.x + rect.width,
            rect.y + rect.height,
        ]
        .into_iter()
        .all(f32::is_finite)
    };
    let edge_is_finite = |edge: EdgeGrant| {
        [edge.guide, edge.legend, edge.total]
            .into_iter()
            .all(f32::is_finite)
    };
    rect_is_finite(region.slot)
        && rect_is_finite(region.content)
        && region.slabs.iter().all(|slab| rect_is_finite(slab.rect))
        && [region.requested, region.coordinated, region.granted]
            .into_iter()
            .all(|edges| {
                [edges.top, edges.right, edges.bottom, edges.left]
                    .into_iter()
                    .all(edge_is_finite)
            })
        && match &region.detail {
            RegionDetail::Leaf => true,
            RegionDetail::Grid { tracks } => tracks
                .column_starts
                .iter()
                .chain(&tracks.column_sizes)
                .chain(&tracks.row_starts)
                .chain(&tracks.row_sizes)
                .all(|v| v.is_finite()),
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
    if let Some(id) = &node.id
        && !seen.insert(id)
    {
        return Err(LayoutError::DuplicateId);
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
    use crate::frame::{FrameAxis, FrameAxisSizing, FrameSide};
    use crate::geometry::{Edges, Rect, Side, Size};
    use crate::region::EdgeDemand;

    fn chart_leaf() -> Layout<&'static str> {
        Layout::leaf(Size::default())
            .margin(8.0)
            .strip(Side::Top, 18.0)
            .legend(Side::Right, 64.0)
            .guide(Side::Left, 38.0)
            .guide(Side::Bottom, 22.0)
            .content_min(Size::new(50.0, 40.0))
            .id("chart")
    }

    /// Solve the chart-leaf chrome per axis through the internal frame
    /// solver (the parity oracle for the three sizing modes).
    fn chart_axes(
        sizing_x: FrameAxisSizing,
        sizing_y: FrameAxisSizing,
    ) -> (
        crate::frame::FrameAxisSolution,
        crate::frame::FrameAxisSolution,
    ) {
        let side = |margin: f32, strips: &[f32], legend: f32, guide: f32| FrameSide {
            margin,
            strips: strips.to_vec(),
            legend,
            guide,
        };
        let horizontal = FrameAxis {
            sizing: sizing_x,
            leading: side(8.0, &[], 0.0, 38.0),
            trailing: side(8.0, &[], 64.0, 0.0),
            content_min: 50.0,
        }
        .solve();
        let vertical = FrameAxis {
            sizing: sizing_y,
            leading: side(8.0, &[18.0], 0.0, 0.0),
            trailing: side(8.0, &[], 0.0, 22.0),
            content_min: 40.0,
        }
        .solve();
        (horizontal, vertical)
    }

    fn frame_content_rect(
        horizontal: &crate::frame::FrameAxisSolution,
        vertical: &crate::frame::FrameAxisSolution,
    ) -> Rect {
        Rect::new(
            horizontal.content.start,
            vertical.content.start,
            horizontal.content.size,
            vertical.content.size,
        )
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
        let (horizontal, vertical) = chart_axes(
            FrameAxisSizing::Envelope { extent: 400.0 },
            FrameAxisSizing::Envelope { extent: 300.0 },
        );

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame_content_rect(&horizontal, &vertical));
        assert_eq!(solved.size, Size::new(horizontal.extent, vertical.extent));
        assert_eq!(region.slot, Rect::new(0.0, 0.0, 400.0, 300.0));
    }

    #[test]
    fn envelope_mode_matches_frame_content_fixed() {
        // Natural sizing: content given (the floor is the content here),
        // envelope derived. Chrome lifts as overflow, so the content rect
        // starts after the leading chrome totals.
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin(8.0)
            .strip(Side::Top, 18.0)
            .legend(Side::Right, 64.0)
            .guide(Side::Left, 38.0)
            .guide(Side::Bottom, 22.0)
            .id("chart")
            .solve(&SolveOptions::default())
            .expect("solve");
        let (horizontal, vertical) = chart_axes(
            FrameAxisSizing::Content { content: 200.0 },
            FrameAxisSizing::Content { content: 150.0 },
        );

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame_content_rect(&horizontal, &vertical));
        assert_eq!(solved.size, Size::new(horizontal.extent, vertical.extent));
    }

    #[test]
    fn margins_mode_matches_frame_both_fixed() {
        let solved = Layout::<&str>::leaf(Size::new(200.0, 150.0))
            .margin(8.0)
            .strip(Side::Top, 18.0)
            .legend(Side::Right, 64.0)
            .guide(Side::Left, 38.0)
            .guide(Side::Bottom, 22.0)
            .id("chart")
            .sizing(SolveFor::Margins)
            .solve(&SolveOptions {
                width: Some(400.0),
                height: Some(300.0),
            })
            .expect("solve");
        let (horizontal, vertical) = chart_axes(
            FrameAxisSizing::EnvelopeAndContent {
                extent: 400.0,
                content: 200.0,
            },
            FrameAxisSizing::EnvelopeAndContent {
                extent: 300.0,
                content: 150.0,
            },
        );

        let region = solved.region(&"chart").expect("chart region");
        assert_eq!(region.content, frame_content_rect(&horizontal, &vertical));
        assert_eq!(solved.size, Size::new(horizontal.extent, vertical.extent));
    }

    #[test]
    fn mixed_axis_allocation_solves_each_axis_independently() {
        // Width figure-constrained (solve for content), height
        // plot-area-sized (content given, envelope derived).
        let solved = Layout::<&str>::leaf(Size::new(0.0, 150.0))
            .margin(8.0)
            .guide(Side::Left, 38.0)
            .guide(Side::Bottom, 22.0)
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

    #[test]
    fn nested_grid_reproduces_the_tree_solver_values() {
        // Nested-grid composition; expected values are exact solver
        // outputs, pinned as literals (a grid nesting a grid).
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

        assert_eq!(new.envelope().content_size, Size::new(146.0, 60.0));
        for (path, expected) in [
            (vec![0usize], Rect::new(0.0, 0.0, 50.0, 60.0)),
            (vec![1], Rect::new(60.0, 0.0, 86.0, 60.0)),
            (vec![1, 0], Rect::new(60.0, 0.0, 40.0, 60.0)),
            (vec![1, 1], Rect::new(106.0, 0.0, 40.0, 60.0)),
        ] {
            let new_region = new.at_path(&path).expect("matching path");
            assert_eq!(new_region.content, expected, "content rect at {path:?}");
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
    fn grid_chrome_reproduces_the_stacked_edges_law() {
        // Grid chrome stacks onto child demands: a 15px child right demand
        // (legend: content stacking, distinct from the header's stratum)
        // plus a 35px guide header = a 50px envelope with the header on
        // the guide (coordinated) stratum.
        let new: Layout = Layout::row([Layout::leaf(Size::new(100.0, 60.0)).demand(
            Side::Right,
            EdgeDemand {
                guide: 0.0,
                legend: 15.0,
            },
        )])
        .guide(Side::Right, 35.0);
        let solved = new.solve(&SolveOptions::default()).expect("solve");

        assert_eq!(solved.envelope().coordinated.right.total, 50.0);
        assert_eq!(solved.envelope().coordinated.right.guide, 35.0);
    }

    #[test]
    fn envelope_carries_layered_and_geometric_views() {
        // Mixed dominance: all-guide top 10 vs all-legend top 8. Layered
        // lifts to 18; geometric reports the raw max 10.
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(100.0, 60.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 10.0,
                    legend: 0.0,
                },
            ),
            Layout::leaf(Size::new(100.0, 60.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 0.0,
                    legend: 8.0,
                },
            ),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        assert_eq!(solved.envelope().coordinated.top.total, 18.0);
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
        // Two plots in a row with different top guide demands: the shared
        // row reserves the max; both contents align by sharing the track;
        // granted reports the track-level demand.
        let root: Layout = Layout::row([
            Layout::leaf(Size::new(100.0, 60.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 20.0,
                    legend: 0.0,
                },
            ),
            Layout::leaf(Size::new(100.0, 60.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 8.0,
                    legend: 0.0,
                },
            ),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        let first = solved.at_path(&[0]).unwrap();
        let second = solved.at_path(&[1]).unwrap();
        assert_eq!(first.content.y, second.content.y);
        assert_eq!(second.requested.top.guide, 8.0);
        assert_eq!(second.granted.top.guide, 20.0, "track-level grant");
        assert_eq!(solved.envelope().coordinated.top.guide, 20.0);
    }

    #[test]
    fn strips_on_all_four_sides_carve_with_corner_rule() {
        use crate::solution::{ChromeLayer, ChromeSlab};
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(10.0)
            .strip(Side::Top, 20.0)
            .strip(Side::Left, 30.0)
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
        // Strip layer: the top strip runs wider than the left strip of
        // the same layer; the left strip starts below the top strip.
        assert_eq!(
            slab(ChromeLayer::Strip, Side::Top).rect,
            Rect::new(10.0, 10.0, 380.0, 20.0)
        );
        assert_eq!(
            slab(ChromeLayer::Strip, Side::Left).rect,
            Rect::new(10.0, 30.0, 30.0, 260.0)
        );
        assert_eq!(region.content, Rect::new(40.0, 30.0, 350.0, 260.0));
    }

    #[test]
    fn repeated_strips_stack_outside_in() {
        use crate::solution::{ChromeLayer, ChromeSlab};
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(8.0)
            .strip(Side::Top, 18.0) // title (outermost)
            .strip(Side::Top, 12.0) // subtitle (inside the title)
            .sizing(SolveFor::Content)
            .id("chart")
            .solve(&SolveOptions {
                width: Some(200.0),
                height: Some(100.0),
            })
            .expect("solve");

        let region = solved.region(&"chart").expect("region");
        let strips: Vec<&ChromeSlab> = region
            .slabs
            .iter()
            .filter(|slab| slab.layer == ChromeLayer::Strip)
            .collect();
        assert_eq!(strips.len(), 2);
        assert_eq!(strips[0].strip_index, 0);
        assert_eq!(strips[0].rect, Rect::new(8.0, 8.0, 184.0, 18.0));
        assert_eq!(strips[1].strip_index, 1);
        assert_eq!(strips[1].rect, Rect::new(8.0, 26.0, 184.0, 12.0));
    }

    #[test]
    fn horizontal_inner_slabs_are_content_sized_on_cross_axis() {
        use crate::solution::ChromeLayer;
        let solved = Layout::<&str>::leaf(Size::default())
            .margin(8.0)
            .guide(Side::Left, 38.0)
            .guide(Side::Bottom, 22.0)
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
            .find(|slab| slab.layer == ChromeLayer::Guide && slab.side == Side::Left)
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
            .guide(Side::Left, 38.0)
            .id("chart")
            .solve(&SolveOptions::default())
            .expect("solve");

        let region = solved.region(&"chart").expect("region");
        assert_eq!(region.content, Rect::new(46.0, 8.0, 200.0, 150.0));
        let left_inner = region
            .slabs
            .iter()
            .find(|slab| slab.layer == ChromeLayer::Guide && slab.side == Side::Left)
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

    fn facet_cell(
        width: f32,
        height: f32,
        left: f32,
        bottom: f32,
    ) -> Layout<&'static str, &'static str> {
        Layout::leaf(Size::new(width, height))
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: left,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Bottom,
                EdgeDemand {
                    guide: bottom,
                    legend: 0.0,
                },
            )
    }

    #[test]
    fn shared_columns_coordinate_tracks_and_chrome() {
        // The nested-facet-columns gallery scenario: two 2x1 columns with
        // different cell sizes and chrome, shared as cousins.
        let column_a = Layout::column(vec![
            facet_cell(110.0, 50.0, 26.0, 18.0).id("a0"),
            facet_cell(110.0, 70.0, 26.0, 0.0).id("a1"),
        ])
        .min_gap(14.0)
        .share("cols")
        .id("col_a");
        let column_b = Layout::column(vec![
            facet_cell(150.0, 64.0, 9.0, 5.0).id("b0"),
            facet_cell(150.0, 40.0, 9.0, 0.0).id("b1"),
        ])
        .min_gap(14.0)
        .share("cols")
        .id("col_b");
        let fig = Layout::row(vec![column_a, column_b]).min_gap(14.0);

        let solved = fig.solve(&SolveOptions::default()).expect("solve");
        assert!(solved.diagnostics().skipped_groups.is_empty());

        // Merged tracks on both members.
        for id in ["col_a", "col_b"] {
            let region = solved.region(&id).expect("column region");
            let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
                panic!("grid expected");
            };
            assert_eq!(tracks.column_sizes, vec![150.0], "{id} column width");
            assert_eq!(tracks.row_sizes, vec![64.0, 70.0], "{id} row heights");
        }

        // Rows align across cousins.
        let a0 = solved.region(&"a0").unwrap();
        let b0 = solved.region(&"b0").unwrap();
        let a1 = solved.region(&"a1").unwrap();
        let b1 = solved.region(&"b1").unwrap();
        assert_eq!(a0.slot.y, b0.slot.y);
        assert_eq!(a1.slot.y, b1.slot.y);

        // Granted chrome exceeds b's own request (the hatched story).
        assert_eq!(b0.requested.left.total, 9.0);
        assert_eq!(b0.granted.left.total, 26.0);
        assert_eq!(b0.requested.bottom.total, 5.0);
        assert_eq!(b0.granted.bottom.total, 18.0);

        // Content never lies: a's cells keep their measured width inside
        // the grown slot.
        assert_eq!(a0.slot.width, 150.0);
        assert_eq!(a0.content.width, 110.0);

        // The inter-column gap absorbs b's granted left chrome:
        // max(min_gap 14, a.right 0 + b.left 26) = 26.
        let col_a = solved.region(&"col_a").unwrap();
        let col_b = solved.region(&"col_b").unwrap();
        assert_eq!(col_b.slot.x - (col_a.slot.x + col_a.slot.width), 26.0);
    }

    #[test]
    fn region_geometric_reports_raw_totals_without_lift() {
        // One side with a guide-heavy cell (guide 5) and a legend-heavy
        // cell (legend 8): the layered demand lifts total to guide + legend,
        // while the geometric view keeps the raw per-side maximum.
        let band: Layout<&str> = Layout::row(vec![
            Layout::leaf(Size::new(40.0, 30.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 5.0,
                    legend: 0.0,
                },
            ),
            Layout::leaf(Size::new(40.0, 30.0)).demand(
                Side::Top,
                EdgeDemand {
                    guide: 0.0,
                    legend: 8.0,
                },
            ),
        ])
        .id("band");
        let solved = band.solve(&SolveOptions::default()).expect("solve");
        let region = solved.region(&"band").expect("band region");
        assert_eq!(region.requested.top.guide, 5.0);
        assert_eq!(region.requested.top.legend, 8.0);
        assert_eq!(region.requested.top.total, 13.0, "layered law lifts");
        assert_eq!(
            region.geometric_total.top, 8.0,
            "geometric law keeps raw max"
        );
    }

    #[test]
    fn solved_tracks_report_merged_spacing() {
        use crate::build::Spacing;
        let g1 = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 50.0)),
            Layout::leaf(Size::new(80.0, 50.0)),
        ])
        .column_spacing(Spacing {
            outer_start: 4.0,
            outer_end: 0.0,
            min_gap: 14.0,
        })
        .share("bands")
        .id("g1");
        let g2 = Layout::row(vec![
            Layout::leaf(Size::new(90.0, 50.0)),
            Layout::leaf(Size::new(70.0, 50.0)),
        ])
        .column_spacing(Spacing {
            outer_start: 0.0,
            outer_end: 9.0,
            min_gap: 6.0,
        })
        .share("bands")
        .id("g2");
        let lone = Layout::row(vec![
            Layout::leaf(Size::new(50.0, 30.0)),
            Layout::leaf(Size::new(50.0, 30.0)),
        ])
        .column_spacing(Spacing {
            outer_start: 1.0,
            outer_end: 2.0,
            min_gap: 3.0,
        })
        .id("lone");
        let root: Layout<&str, &str> = Layout::column(vec![g1, g2, lone]);

        let solved = root.solve(&SolveOptions::default()).expect("solve");
        assert!(solved.diagnostics().skipped_groups.is_empty());

        let tracks = |id: &'static str| {
            let region = solved.region(&id).expect("grid region");
            let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
                panic!("grid expected for {id}");
            };
            tracks.clone()
        };

        // Shared members both report the group-merged spacing.
        let merged = Spacing {
            outer_start: 4.0,
            outer_end: 9.0,
            min_gap: 14.0,
        };
        assert_eq!(tracks("g1").column_spacing, merged);
        assert_eq!(tracks("g2").column_spacing, merged);

        // An unshared grid reports its own declared spacing.
        assert_eq!(
            tracks("lone").column_spacing,
            Spacing {
                outer_start: 1.0,
                outer_end: 2.0,
                min_gap: 3.0,
            }
        );
    }

    #[test]
    fn ragged_uniform_group_merges_policy() {
        let g1 = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 50.0)),
            Layout::leaf(Size::new(80.0, 50.0)),
            Layout::leaf(Size::new(90.0, 50.0)),
        ])
        .uniform_columns()
        .share("bands")
        .id("g1");
        let g2 = Layout::row(vec![
            Layout::leaf(Size::new(60.0, 50.0)),
            Layout::leaf(Size::new(120.0, 50.0)),
        ])
        .uniform_columns()
        .share("bands")
        .id("g2");
        let root: Layout<&str, &str> = Layout::column(vec![g1, g2]);

        let solved = root.solve(&SolveOptions::default()).expect("solve");
        assert!(solved.diagnostics().skipped_groups.is_empty());
        for id in ["g1", "g2"] {
            let region = solved.region(&id).unwrap();
            let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
                panic!("grid expected");
            };
            assert!(
                tracks.column_sizes.iter().all(|&width| width == 120.0),
                "{id} tracks {:?}",
                tracks.column_sizes
            );
        }
    }

    #[test]
    fn shape_mismatch_skips_non_uniform_group() {
        let g1 = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 50.0)),
            Layout::leaf(Size::new(100.0, 50.0)),
        ])
        .share("g")
        .id("g1");
        let g2 = Layout::row(vec![Layout::leaf(Size::new(60.0, 50.0))])
            .share("g")
            .id("g2");
        let root: Layout<&str, &str> = Layout::column(vec![g1, g2]);

        let solved = root.solve(&SolveOptions::default()).expect("solve");
        assert_eq!(solved.diagnostics().skipped_groups.len(), 1);
        assert_eq!(
            solved.diagnostics().skipped_groups[0].reason,
            crate::solution::SkippedShareReason::ShapeMismatch
        );
        // Unpatched: each keeps its own track sizes.
        let g1 = solved.region(&"g1").unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &g1.detail else {
            panic!("grid expected");
        };
        assert_eq!(tracks.column_sizes, vec![100.0, 100.0]);
    }

    #[test]
    fn min_slack_keeps_asymmetric_cousins_congruent() {
        let g1 = Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0))])
            .share("g")
            .id("g1");
        let g2 = Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0))])
            .share("g")
            .id("g2");
        // g1 sits in a column whose track is widened to 200 by a sibling;
        // g2's slot is exactly its natural width. Offers: 100 vs 0.
        let root: Layout<&str, &str> = Layout::row(vec![
            Layout::column(vec![g1, Layout::leaf(Size::new(200.0, 50.0))]),
            Layout::column(vec![g2]),
        ]);

        let solved = root.solve(&SolveOptions::default()).expect("solve");
        let g1 = solved.region(&"g1").unwrap();
        let g2 = solved.region(&"g2").unwrap();
        // Min-slack is zero: neither stretches; cousins stay congruent.
        assert_eq!(g1.content.width, 100.0);
        assert_eq!(g2.content.width, 100.0);
        assert_eq!(g1.slot.width, 200.0, "the wide slot is honest");
    }

    #[test]
    fn singleton_share_stretches_by_its_own_offer() {
        let root: Layout<&str, &str> = Layout::row(vec![
            Layout::row(vec![Layout::leaf(Size::new(40.0, 20.0))])
                .share("solo")
                .id("g"),
        ]);
        let solved = root
            .solve(&SolveOptions {
                width: Some(120.0),
                height: None,
            })
            .expect("solve");
        let region = solved.region(&"g").unwrap();
        // Policy-space stretch on a group of one is just that instance
        // stretching: it fills like an unshared grid.
        assert_eq!(region.content.width, 120.0);
    }

    #[test]
    fn uniform_conflict_reports_diagnostic_and_declared_sizes_win() {
        use crate::build::TrackSize;
        let root: Layout = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 50.0)),
            Layout::leaf(Size::new(80.0, 50.0)),
        ])
        .uniform_columns()
        .columns([TrackSize::Auto, TrackSize::Auto])
        .id(1);
        let solved = root.solve(&SolveOptions::default()).expect("solve");
        assert_eq!(solved.diagnostics().uniform_conflicts, 1);
        let region = solved.region(&1).unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
            panic!("grid expected");
        };
        assert_eq!(tracks.column_sizes, vec![100.0, 80.0], "uniform ignored");
    }

    #[test]
    fn flex_tracks_split_leftover_by_weight() {
        use crate::build::TrackSize::{Auto, Flex};
        let root: Layout = Layout::row(vec![
            Layout::leaf(Size::new(40.0, 20.0)),
            Layout::leaf(Size::new(40.0, 20.0)),
            Layout::leaf(Size::new(40.0, 20.0)),
        ])
        .columns([Flex(2.0), Flex(1.0), Auto])
        .id(0);
        let solved = root
            .solve(&SolveOptions {
                width: Some(240.0),
                height: None,
            })
            .expect("solve");
        let region = solved.region(&0).unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
            panic!("grid expected");
        };
        // 120 free over weights 2:1; the Auto track gets none.
        assert_eq!(tracks.column_sizes, vec![120.0, 80.0, 40.0]);
        assert_eq!(solved.size.width, 240.0);
    }

    #[test]
    fn fixed_track_is_rigid_and_oversized_content_overflows() {
        use crate::build::TrackSize::{Auto, Fixed};
        let root: Layout = Layout::row(vec![
            Layout::leaf(Size::new(100.0, 50.0)).id(1),
            Layout::leaf(Size::new(50.0, 50.0)).id(2),
        ])
        .columns([Fixed(60.0), Auto])
        .id(0);
        let solved = root
            .solve(&SolveOptions {
                width: Some(200.0),
                height: None,
            })
            .expect("solve");
        let region = solved.region(&0).unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
            panic!("grid expected");
        };
        // Fixed stays 60 (not grown for the 100-wide child, not stretched);
        // all 90 of free space goes to the Auto track.
        assert_eq!(tracks.column_sizes, vec![60.0, 140.0]);
        let first = solved.region(&1).unwrap();
        assert_eq!(first.slot.width, 60.0);
        assert_eq!(first.content.width, 100.0, "honest overflow");
    }

    #[test]
    fn flex_tracks_floor_at_content() {
        use crate::build::TrackSize::Flex;
        let root: Layout = Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0))])
            .columns([Flex(1.0)])
            .id(0);
        let solved = root.solve(&SolveOptions::default()).expect("solve");
        let region = solved.region(&0).unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
            panic!("grid expected");
        };
        assert_eq!(tracks.column_sizes, vec![100.0], "minmax(auto, fr)");
    }

    #[test]
    fn span_deficits_avoid_fixed_tracks() {
        use crate::build::TrackSize::{Auto, Fixed};
        let root: Layout = Layout::grid(1, 2)
            .cell(0, 0, Layout::leaf(Size::new(40.0, 50.0)))
            .cell(0, 1, Layout::leaf(Size::new(50.0, 50.0)))
            .cell_span(0, 0, 1, 2, Layout::leaf(Size::new(200.0, 50.0)))
            .columns([Fixed(50.0), Auto])
            .id(0);
        let solved = root.solve(&SolveOptions::default()).expect("solve");
        let region = solved.region(&0).unwrap();
        let crate::solution::RegionDetail::Grid { tracks } = &region.detail else {
            panic!("grid expected");
        };
        // The span's 100px deficit lands entirely on the Auto track.
        assert_eq!(tracks.column_sizes, vec![50.0, 150.0]);
    }

    #[test]
    fn content_delta_drives_the_convergence_loop() {
        let build = |width: f32| -> Layout {
            Layout::row(vec![
                Layout::leaf(Size::new(width, 50.0)),
                Layout::leaf(Size::new(width, 50.0)),
            ])
        };
        let first = build(100.0).solve(&SolveOptions::default()).expect("solve");
        let same = build(100.0).solve(&SolveOptions::default()).expect("solve");
        assert_eq!(first.content_delta(&same), 0.0);

        let wider = build(130.0).solve(&SolveOptions::default()).expect("solve");
        assert_eq!(first.content_delta(&wider), 30.0);

        let reshaped: Layout = Layout::row(vec![Layout::leaf(Size::new(100.0, 50.0))]);
        let reshaped = reshaped.solve(&SolveOptions::default()).expect("solve");
        assert_eq!(first.content_delta(&reshaped), f32::INFINITY);
    }

    /// Pin the pass provenance of the three `Region` edge fields:
    /// `requested` is pass-1 (pre-merge), `coordinated` is pass-2 (the
    /// node's own ask raised to share-group floors), `granted` is the
    /// parent's track allocation (which also folds in unshared siblings).
    ///
    /// The numbers mirror the chart's envelope-laws fixture: a guide-heavy
    /// member (guide 5 within envelope 5) shares a key with a legend-heavy
    /// member (guide 0 within envelope 8); the coordinated side must hold
    /// both layers at once (total 13 = 5 + 8 — the cross-cousin lift).
    #[test]
    fn region_edges_pin_pass_provenance() {
        use crate::region::EdgeGrant;

        let member = |guide: f32, envelope: f32| -> Layout<&'static str, &'static str> {
            Layout::row(vec![Layout::leaf(Size::new(40.0, 30.0)).demand(
                Side::Top,
                EdgeDemand::from_guide_and_envelope(guide, envelope),
            )])
            .share("k")
        };
        let root: Layout<&str, &str> = Layout::row(vec![
            member(5.0, 5.0).id("guide-heavy"),
            member(0.0, 8.0).id("legend-heavy"),
            Layout::leaf(Size::new(40.0, 30.0))
                .demand(
                    Side::Top,
                    EdgeDemand {
                        guide: 20.0,
                        legend: 0.0,
                    },
                )
                .id("fat"),
        ]);
        let solved = root.solve(&SolveOptions::default()).expect("solve");

        let a = solved.region(&"guide-heavy").expect("member region");
        // Pass 1: the member's own natural ask.
        assert_eq!(a.requested.top, EdgeGrant::new(5.0, 0.0, 5.0));
        // Pass 2: raised to the share group's merged floors — one member's
        // guide layer and the other's legend layer coexist.
        assert_eq!(a.coordinated.top, EdgeGrant::new(5.0, 8.0, 13.0));
        // Parent allocation: the row's top track edge also folds in the
        // unshared sibling's layers per stratum (its 20px guide layer
        // joins the merged guide; the lift law raises the total to the
        // stratum sum).
        assert_eq!(a.granted.top, EdgeGrant::new(20.0, 8.0, 28.0));

        let b = solved.region(&"legend-heavy").expect("member region");
        assert_eq!(b.requested.top, EdgeGrant::new(0.0, 8.0, 8.0));
        assert_eq!(b.coordinated.top, EdgeGrant::new(5.0, 8.0, 13.0));
        assert_eq!(b.granted.top, EdgeGrant::new(20.0, 8.0, 28.0));

        // Nodes no share patch touches keep coordinated == requested.
        let fat = solved.region(&"fat").expect("leaf region");
        assert_eq!(fat.coordinated, fat.requested);
    }
}
