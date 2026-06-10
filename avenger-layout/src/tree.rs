//! Nested layout tree: bottom-up requirement collection and top-down solve.
//!
//! A [`LayoutNode`] is a grid whose slots hold either leaf content (a content
//! size plus layered edge demand) or child subtrees. The two sweeps are:
//!
//! - [`LayoutNode::envelope`] (bottom-up): each subtree solves locally and exports
//!   what its parent slot sees — the solved content size plus boundary edge
//!   demands. Because a grid excludes the first leading and last trailing
//!   edges from its content extent, the classic facet aggregation rules fall
//!   out structurally: on the main axis only the first/last child's edge
//!   reaches the envelope, while on the cross axis every child's edge merges
//!   by max. A node's own stacked chrome (for charts: facet
//!   labels/titles on the inner layer, band legends on the outer layer)
//!   stacks beyond the aggregated child envelope.
//! - [`LayoutNode::solve`] (top-down): origins, per-slot content sizes, and edge
//!   targets for every region in root coordinates. When a parent allocates
//!   more space than a subtree's natural extent (for charts: an aligned
//!   sibling grew), the subtree's tracks stretch evenly to fill the
//!   allocation and the stretch propagates to its children.
//!
//! Edge roles are structural here: whether a child sits on the first/last
//! track of an axis is a fact of its slot, not caller-threaded metadata.

use crate::geometry::{Edges, Rect, Size};
use crate::grid::{
    GridError, GridItem, GridRequirements, GridShape, GridSlot, GridSolution, TrackSpacing,
};
use crate::region::{EdgeDemand, EdgeTargets};

/// Content of one slot in a layout tree.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutSlotContent<Id = usize> {
    Leaf {
        content_size: Size,
        inner_edges: Edges<f32>,
        outer_edges: Edges<f32>,
        total_edges: Edges<f32>,
    },
    Node(LayoutNode<Id>),
}

/// One slotted child of a layout node.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutItem<Id = usize> {
    pub id: Id,
    pub slot: GridSlot,
    pub content: LayoutSlotContent<Id>,
}

/// A grid of slots holding leaves or child subtrees.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode<Id = usize> {
    pub shape: GridShape,
    pub column_spacing: TrackSpacing,
    pub row_spacing: TrackSpacing,
    pub base_cell_size: Size,
    /// The node's own inner-layer chrome stacking beyond the aggregated
    /// child envelope (for charts: facet labels and titles, which extend the
    /// guide layer). Additive: extends `inner` and `total`.
    pub stacked_inner_edges: Edges<f32>,
    /// The node's own outer-layer chrome stacking beyond the aggregated
    /// child envelope (for charts: band-level legends). Additive: extends
    /// `outer` and `total`.
    pub stacked_outer_edges: Edges<f32>,
    pub items: Vec<LayoutItem<Id>>,
}

/// What a parent slot sees of a solved subtree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeEnvelope {
    pub content_size: Size,
    pub inner_edges: Edges<f32>,
    pub outer_edges: Edges<f32>,
    pub total_edges: Edges<f32>,
}

/// One placed region of a solved tree, in root content coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct SolvedRegion<Id = usize> {
    pub id: Id,
    pub depth: usize,
    /// Index route from the root: the position of each ancestor item (and
    /// finally this item) within its parent's `items`. Length is
    /// `depth + 1`; structural, independent of caller ids.
    pub path: Vec<usize>,
    pub content_rect: Rect,
    /// The overflow this region asked for: a leaf's own (lifted) edge
    /// demand, or a subtree's envelope — exactly what the parent solve
    /// consumed.
    pub requested: EdgeTargets,
    /// The coordinated overflow the solve produced for this region's slot
    /// (per-track merged edge demand).
    pub edge_targets: EdgeTargets,
}

/// Result of solving a layout tree top-down.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeSolution<Id = usize> {
    pub content_size: Size,
    pub regions: Vec<SolvedRegion<Id>>,
}

struct CollectedNode<Id> {
    grid_items: Vec<GridItem<Id>>,
    solution: GridSolution,
    /// Parallel to `grid_items`: collected subtree for `Node` items.
    children: Vec<Option<CollectedNode<Id>>>,
}

fn collect_node<Id: Clone>(node: &LayoutNode<Id>) -> Result<CollectedNode<Id>, GridError> {
    let mut grid_items = Vec::with_capacity(node.items.len());
    let mut children = Vec::with_capacity(node.items.len());
    for item in &node.items {
        let (content_size, inner_edges, outer_edges, total_edges, collected) = match &item.content {
            LayoutSlotContent::Leaf {
                content_size,
                inner_edges,
                outer_edges,
                total_edges,
            } => (
                *content_size,
                *inner_edges,
                *outer_edges,
                *total_edges,
                None,
            ),
            LayoutSlotContent::Node(child) => {
                let collected = collect_node(child)?;
                let envelope = node_envelope(child, &collected);
                (
                    envelope.content_size,
                    envelope.inner_edges,
                    envelope.outer_edges,
                    envelope.total_edges,
                    Some(collected),
                )
            }
        };
        grid_items.push(GridItem {
            id: item.id.clone(),
            slot: item.slot,
            content_size,
            inner_edges,
            outer_edges,
            total_edges,
        });
        children.push(collected);
    }

    let mut requirements =
        GridRequirements::from_items(node.shape, node.base_cell_size, &grid_items)?;
    requirements.column_spacing = node.column_spacing;
    requirements.row_spacing = node.row_spacing;
    let solution = requirements.solve(&grid_items);

    Ok(CollectedNode {
        grid_items,
        solution,
        children,
    })
}

fn boundary_demand(demands: &[EdgeDemand], index: usize) -> EdgeDemand {
    demands.get(index).copied().unwrap_or_default()
}

fn node_envelope<Id>(node: &LayoutNode<Id>, collected: &CollectedNode<Id>) -> TreeEnvelope {
    let solution = &collected.solution;
    let top = boundary_demand(&solution.row_top, 0);
    let bottom = boundary_demand(&solution.row_bottom, node.shape.rows.saturating_sub(1));
    let left = boundary_demand(&solution.column_left, 0);
    let right = boundary_demand(&solution.column_right, node.shape.columns.saturating_sub(1));

    let stacked_inner = node.stacked_inner_edges;
    let stacked_outer = node.stacked_outer_edges;
    TreeEnvelope {
        content_size: solution.content_size,
        inner_edges: Edges::new(
            top.inner + stacked_inner.top,
            right.inner + stacked_inner.right,
            bottom.inner + stacked_inner.bottom,
            left.inner + stacked_inner.left,
        ),
        outer_edges: Edges::new(
            top.outer + stacked_outer.top,
            right.outer + stacked_outer.right,
            bottom.outer + stacked_outer.bottom,
            left.outer + stacked_outer.left,
        ),
        total_edges: Edges::new(
            top.total + stacked_inner.top + stacked_outer.top,
            right.total + stacked_inner.right + stacked_outer.right,
            bottom.total + stacked_inner.bottom + stacked_outer.bottom,
            left.total + stacked_inner.left + stacked_outer.left,
        ),
    }
}

/// Which law the envelope's `total` layer reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeEnvelopeKind {
    /// Coordination view: per-side totals are lifted to `inner + outer`, so
    /// a side's total is `max(inner) + max(outer)` across contributors —
    /// the space regions occupy after coordinated patches apply.
    Layered,
    /// Geometric view: per-side totals merge by raw maximum across
    /// contributors, without the lift — the raw rendered envelope.
    /// The `inner` layer is identical in both kinds; `outer` is derived as
    /// `(total - inner).max(0)`.
    Geometric,
}

impl<Id: Clone> LayoutNode<Id> {
    /// Collect this subtree bottom-up and export what its parent slot sees
    /// under the given envelope law.
    pub fn envelope(&self, kind: TreeEnvelopeKind) -> Result<TreeEnvelope, GridError> {
        let collected = collect_node(self)?;
        match kind {
            TreeEnvelopeKind::Layered => Ok(node_envelope(self, &collected)),
            TreeEnvelopeKind::Geometric => Ok(geometric_envelope(self, &collected, kind)),
        }
    }

    /// Solve this tree top-down into placed regions in root coordinates.
    ///
    /// `allocation` is the content size granted by the caller; when it
    /// exceeds the tree's natural extent the root's tracks stretch evenly
    /// and the stretch propagates through nested allocations.
    pub fn solve(&self, allocation: Option<Size>) -> Result<TreeSolution<Id>, GridError> {
        let collected = collect_node(self)?;
        let mut regions = Vec::new();
        let content_size = solve_node_into(
            self,
            &collected,
            allocation,
            [0.0, 0.0],
            0,
            &[],
            &mut regions,
        );
        Ok(TreeSolution {
            content_size,
            regions,
        })
    }
}

/// Whether an item's slot touches each side of the node.
fn slot_edge_roles<Id>(node: &LayoutNode<Id>, slot: GridSlot) -> Edges<bool> {
    Edges::new(
        slot.row == 0,
        slot.column_end() == node.shape.columns,
        slot.row_end() == node.shape.rows,
        slot.column == 0,
    )
}

fn geometric_envelope<Id: Clone>(
    node: &LayoutNode<Id>,
    collected: &CollectedNode<Id>,
    kind: TreeEnvelopeKind,
) -> TreeEnvelope {
    // Content size and the inner layer are kind-independent: gaps and
    // content extents always use the lifted grid law, and `inner` carries
    // raw values through `EdgeDemand` unchanged.
    let layered = node_envelope(node, &collected);

    let mut total = Edges::new(0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (index, item) in node.items.iter().enumerate() {
        let raw_total = match (&item.content, collected.children[index].as_ref()) {
            (LayoutSlotContent::Leaf { total_edges, .. }, _) => *total_edges,
            (LayoutSlotContent::Node(child), Some(child_collected)) => {
                geometric_envelope(child, child_collected, kind).total_edges
            }
            (LayoutSlotContent::Node(_), None) => Edges::default(),
        };
        let roles = slot_edge_roles(node, item.slot);
        if roles.top {
            total.top = total.top.max(raw_total.top);
        }
        if roles.right {
            total.right = total.right.max(raw_total.right);
        }
        if roles.bottom {
            total.bottom = total.bottom.max(raw_total.bottom);
        }
        if roles.left {
            total.left = total.left.max(raw_total.left);
        }
    }

    let stacked_inner = node.stacked_inner_edges;
    let stacked_outer = node.stacked_outer_edges;
    let total_edges = Edges::new(
        total.top + stacked_inner.top + stacked_outer.top,
        total.right + stacked_inner.right + stacked_outer.right,
        total.bottom + stacked_inner.bottom + stacked_outer.bottom,
        total.left + stacked_inner.left + stacked_outer.left,
    );
    let inner_edges = layered.inner_edges;
    TreeEnvelope {
        content_size: layered.content_size,
        outer_edges: Edges::new(
            (total_edges.top - inner_edges.top).max(0.0),
            (total_edges.right - inner_edges.right).max(0.0),
            (total_edges.bottom - inner_edges.bottom).max(0.0),
            (total_edges.left - inner_edges.left).max(0.0),
        ),
        inner_edges,
        total_edges,
    }
}

/// Stretch solved tracks evenly so the content extent meets `target`.
fn stretch_solution<Id>(
    collected: &CollectedNode<Id>,
    spacing: (TrackSpacing, TrackSpacing),
    target: Size,
) -> GridSolution {
    let solution = &collected.solution;
    let mut column_widths = solution.column_widths.clone();
    let mut row_heights = solution.row_heights.clone();

    let width_deficit = target.width - solution.content_size.width;
    if width_deficit > 0.0 && !column_widths.is_empty() {
        let extra = width_deficit / column_widths.len() as f32;
        for width in &mut column_widths {
            *width += extra;
        }
    }
    let height_deficit = target.height - solution.content_size.height;
    if height_deficit > 0.0 && !row_heights.is_empty() {
        let extra = height_deficit / row_heights.len() as f32;
        for height in &mut row_heights {
            *height += extra;
        }
    }

    // Track sizes already satisfy every span constraint after stretching;
    // re-solving recomputes starts and the content extent.
    let requirements = crate::grid::GridRequirements {
        shape: GridShape {
            rows: row_heights.len(),
            columns: column_widths.len(),
        },
        column_spacing: spacing.0,
        row_spacing: spacing.1,
        column_widths,
        row_heights,
        column_left: solution.column_left.clone(),
        column_right: solution.column_right.clone(),
        row_top: solution.row_top.clone(),
        row_bottom: solution.row_bottom.clone(),
    };
    requirements.solve(&collected.grid_items)
}

fn solve_node_into<Id: Clone>(
    node: &LayoutNode<Id>,
    collected: &CollectedNode<Id>,
    allocation: Option<Size>,
    origin: [f32; 2],
    depth: usize,
    path: &[usize],
    regions: &mut Vec<SolvedRegion<Id>>,
) -> Size {
    let solution = match allocation {
        Some(target)
            if target.width > collected.solution.content_size.width
                || target.height > collected.solution.content_size.height =>
        {
            stretch_solution(collected, (node.column_spacing, node.row_spacing), target)
        }
        _ => collected.solution.clone(),
    };

    for (index, item) in node.items.iter().enumerate() {
        let slot_origin = solution.content_origin_for_slot(item.slot);
        let slot_size = solution.content_size_for_slot(item.slot);
        let region_origin = [origin[0] + slot_origin[0], origin[1] + slot_origin[1]];
        let mut item_path = path.to_vec();
        item_path.push(index);
        regions.push(SolvedRegion {
            id: item.id.clone(),
            depth,
            path: item_path.clone(),
            content_rect: Rect::new(
                region_origin[0],
                region_origin[1],
                slot_size.width,
                slot_size.height,
            ),
            requested: collected.grid_items[index].requested_targets(),
            edge_targets: solution.edge_targets_for_slot(item.slot),
        });

        if let (LayoutSlotContent::Node(child), Some(child_collected)) =
            (&item.content, collected.children[index].as_ref())
        {
            solve_node_into(
                child,
                child_collected,
                Some(slot_size),
                region_origin,
                depth + 1,
                &item_path,
                regions,
            );
        }
    }

    solution.content_size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: usize, column: usize, size: Size, total_edges: Edges<f32>) -> LayoutItem<usize> {
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
                total_edges,
            },
        }
    }

    fn column_band(items: Vec<LayoutItem<usize>>, min_gap: f32) -> LayoutNode<usize> {
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
    fn column_band_envelope_uses_first_last_main_and_max_cross() {
        // The facet aggregation rules: on the main axis only the first/last
        // child's edge reaches the envelope; on the cross axis every child's
        // edge merges by max.
        let node = column_band(
            vec![
                leaf(
                    0,
                    0,
                    Size::new(100.0, 60.0),
                    Edges::new(4.0, 9.0, 1.0, 15.0),
                ),
                leaf(1, 1, Size::new(100.0, 60.0), Edges::new(7.0, 3.0, 8.0, 2.0)),
                leaf(
                    2,
                    2,
                    Size::new(100.0, 60.0),
                    Edges::new(2.0, 11.0, 5.0, 6.0),
                ),
            ],
            0.0,
        );

        let envelope = node
            .envelope(TreeEnvelopeKind::Layered)
            .expect("band tree should collect");

        assert_eq!(envelope.total_edges.left, 15.0, "first child's left");
        assert_eq!(envelope.total_edges.right, 11.0, "last child's right");
        assert_eq!(envelope.total_edges.top, 7.0, "max top across children");
        assert_eq!(
            envelope.total_edges.bottom, 8.0,
            "max bottom across children"
        );
        // Interior boundary edges become gaps: (9 + 2) and (3 + 6).
        assert_eq!(envelope.content_size, Size::new(320.0, 60.0));
    }

    #[test]
    fn geometric_envelope_reports_raw_max_totals() {
        // Mixed dominance on the cross axis: cell 0 is all guide (top
        // total 10), cell 1 is all legend (top total 8). The layered law
        // lifts to max(guide) + max(legend) = 18; the measured law reports
        // the raw maximum rendered envelope, 10.
        let node = column_band(
            vec![
                LayoutItem {
                    id: 0,
                    slot: GridSlot {
                        row: 0,
                        column: 0,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Leaf {
                        content_size: Size::new(100.0, 60.0),
                        inner_edges: Edges::new(10.0, 0.0, 0.0, 0.0),
                        outer_edges: Edges::default(),
                        total_edges: Edges::new(10.0, 0.0, 0.0, 0.0),
                    },
                },
                LayoutItem {
                    id: 1,
                    slot: GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Leaf {
                        content_size: Size::new(100.0, 60.0),
                        inner_edges: Edges::default(),
                        outer_edges: Edges::new(8.0, 0.0, 0.0, 0.0),
                        total_edges: Edges::new(8.0, 0.0, 0.0, 0.0),
                    },
                },
            ],
            0.0,
        );

        let layered = node
            .envelope(TreeEnvelopeKind::Layered)
            .expect("band tree should collect");
        let measured = node
            .envelope(TreeEnvelopeKind::Geometric)
            .expect("band tree should collect");

        assert_eq!(layered.total_edges.top, 18.0);
        assert_eq!(measured.total_edges.top, 10.0);
        assert_eq!(measured.inner_edges.top, 10.0, "inner layer is shared");
        assert_eq!(measured.outer_edges.top, 0.0);
        assert_eq!(measured.content_size, layered.content_size);
    }

    #[test]
    fn stacked_node_chrome_extends_child_envelope() {
        // The documented nested-facet stacking chain: a subplot's 15px right
        // overflow plus 20px labels plus 15px title = a 50px envelope.
        let mut inner = column_band(
            vec![leaf(
                0,
                0,
                Size::new(100.0, 60.0),
                Edges::new(0.0, 15.0, 0.0, 0.0),
            )],
            0.0,
        );
        inner.stacked_inner_edges = Edges::new(0.0, 35.0, 0.0, 0.0);

        let inner_envelope = inner
            .envelope(TreeEnvelopeKind::Layered)
            .expect("inner tree should collect");
        assert_eq!(inner_envelope.total_edges.right, 50.0);

        let outer = column_band(
            vec![
                leaf(0, 0, Size::new(100.0, 60.0), Edges::default()),
                LayoutItem {
                    id: 1,
                    slot: GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Node(inner),
                },
            ],
            0.0,
        );

        let envelope = outer
            .envelope(TreeEnvelopeKind::Layered)
            .expect("outer tree should collect");
        assert_eq!(
            envelope.total_edges.right, 50.0,
            "the rightmost subtree's stacked envelope reaches the outer edge"
        );
    }

    #[test]
    fn solve_tree_places_nested_regions_in_root_coordinates() {
        let inner = column_band(
            vec![
                leaf(10, 0, Size::new(40.0, 60.0), Edges::default()),
                leaf(11, 1, Size::new(40.0, 60.0), Edges::default()),
            ],
            6.0,
        );
        let root = column_band(
            vec![
                leaf(0, 0, Size::new(50.0, 60.0), Edges::default()),
                LayoutItem {
                    id: 1,
                    slot: GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                    content: LayoutSlotContent::Node(inner),
                },
            ],
            10.0,
        );

        let solved = root.solve(None).expect("tree should solve");

        // Inner extent: 40 + 6 + 40 = 86; root: 50 + 10 + 86 = 146.
        assert_eq!(solved.content_size, Size::new(146.0, 60.0));
        let rects: Vec<(usize, usize, Rect)> = solved
            .regions
            .iter()
            .map(|region| (region.depth, region.id, region.content_rect))
            .collect();
        assert_eq!(
            rects,
            vec![
                (0, 0, Rect::new(0.0, 0.0, 50.0, 60.0)),
                (0, 1, Rect::new(60.0, 0.0, 86.0, 60.0)),
                (1, 10, Rect::new(60.0, 0.0, 40.0, 60.0)),
                (1, 11, Rect::new(106.0, 0.0, 40.0, 60.0)),
            ]
        );
    }

    #[test]
    fn allocation_stretch_propagates_to_nested_children() {
        let inner = column_band(
            vec![
                leaf(10, 0, Size::new(40.0, 60.0), Edges::default()),
                leaf(11, 1, Size::new(40.0, 60.0), Edges::default()),
            ],
            0.0,
        );
        let root = column_band(
            vec![LayoutItem {
                id: 0,
                slot: GridSlot {
                    row: 0,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
                content: LayoutSlotContent::Node(inner),
            }],
            0.0,
        );

        // Natural extent is 80x60; the caller grants 120x60 (for charts: an
        // aligned sibling grew). The root track stretches, and the nested
        // band's tracks absorb the surplus evenly: 40 -> 60 each.
        let solved = root
            .solve(Some(Size::new(120.0, 60.0)))
            .expect("tree should solve with allocation");

        assert_eq!(solved.content_size, Size::new(120.0, 60.0));
        let leaf_rects: Vec<Rect> = solved
            .regions
            .iter()
            .filter(|region| region.depth == 1)
            .map(|region| region.content_rect)
            .collect();
        assert_eq!(
            leaf_rects,
            vec![
                Rect::new(0.0, 0.0, 60.0, 60.0),
                Rect::new(60.0, 0.0, 60.0, 60.0),
            ]
        );
    }
}
