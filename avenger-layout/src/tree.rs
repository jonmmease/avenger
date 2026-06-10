//! Nested layout tree: bottom-up requirement collection and top-down solve.
//!
//! A [`LayoutNode`] is a grid whose slots hold either leaf content (a content
//! size plus layered edge demand) or child subtrees. The two sweeps are:
//!
//! - [`tree_envelope`] (bottom-up): each subtree solves locally and exports
//!   what its parent slot sees — the solved content size plus boundary edge
//!   demands. Because a grid excludes the first leading and last trailing
//!   edges from its content extent, the classic facet aggregation rules fall
//!   out structurally: on the main axis only the first/last child's edge
//!   reaches the envelope, while on the cross axis every child's edge merges
//!   by max. A node's own [`LayoutNode::stacked_edges`] (for charts: facet
//!   labels and titles) stack beyond the aggregated child envelope.
//! - [`solve_tree`] (top-down): origins, per-slot content sizes, and edge
//!   targets for every region in root coordinates. When a parent allocates
//!   more space than a subtree's natural extent (for charts: an aligned
//!   sibling grew), the subtree's tracks stretch evenly to fill the
//!   allocation and the stretch propagates to its children.
//!
//! Edge roles are structural here: whether a child sits on the first/last
//! track of an axis is a fact of its slot, not caller-threaded metadata.

use crate::geometry::{Edges, Rect, Size};
use crate::grid::{
    GridError, GridItem, GridShape, GridSlot, GridSolution, TrackSpacing, grid_requirements,
    solve_grid_requirements,
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
    pub child_index: Id,
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
    /// The node's own chrome stacking beyond the aggregated child envelope
    /// (for charts: facet labels and titles). Stacking is additive: it
    /// extends the `outer` and `total` layers of the exported envelope.
    pub stacked_edges: Edges<f32>,
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
    pub child_index: Id,
    pub depth: usize,
    pub content_rect: Rect,
    pub edge_targets: EdgeTargets,
}

/// Result of solving a layout tree top-down.
#[derive(Clone, Debug, PartialEq)]
pub struct SolvedTree<Id = usize> {
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
            child_index: item.child_index.clone(),
            slot: item.slot,
            content_size,
            inner_edges,
            outer_edges,
            total_edges,
        });
        children.push(collected);
    }

    let mut requirements = grid_requirements(node.shape, node.base_cell_size, &grid_items)?;
    requirements.column_spacing = node.column_spacing;
    requirements.row_spacing = node.row_spacing;
    let solution = solve_grid_requirements(&requirements, &grid_items);

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

    let stacked = node.stacked_edges;
    TreeEnvelope {
        content_size: solution.content_size,
        inner_edges: Edges::new(top.inner, right.inner, bottom.inner, left.inner),
        outer_edges: Edges::new(
            top.outer + stacked.top,
            right.outer + stacked.right,
            bottom.outer + stacked.bottom,
            left.outer + stacked.left,
        ),
        total_edges: Edges::new(
            top.total + stacked.top,
            right.total + stacked.right,
            bottom.total + stacked.bottom,
            left.total + stacked.left,
        ),
    }
}

/// Collect a subtree bottom-up and export what its parent slot sees.
pub fn tree_envelope<Id: Clone>(node: &LayoutNode<Id>) -> Result<TreeEnvelope, GridError> {
    let collected = collect_node(node)?;
    Ok(node_envelope(node, &collected))
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
    solve_grid_requirements(&requirements, &collected.grid_items)
}

fn solve_node_into<Id: Clone>(
    node: &LayoutNode<Id>,
    collected: &CollectedNode<Id>,
    allocation: Option<Size>,
    origin: [f32; 2],
    depth: usize,
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
        regions.push(SolvedRegion {
            child_index: item.child_index.clone(),
            depth,
            content_rect: Rect::new(
                region_origin[0],
                region_origin[1],
                slot_size.width,
                slot_size.height,
            ),
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
                regions,
            );
        }
    }

    solution.content_size
}

/// Solve a layout tree top-down into placed regions in root coordinates.
///
/// `allocation` is the content size granted by the caller; when it exceeds
/// the tree's natural extent the root's tracks stretch evenly and the
/// stretch propagates through nested allocations.
pub fn solve_tree<Id: Clone>(
    node: &LayoutNode<Id>,
    allocation: Option<Size>,
) -> Result<SolvedTree<Id>, GridError> {
    let collected = collect_node(node)?;
    let mut regions = Vec::new();
    let content_size = solve_node_into(node, &collected, allocation, [0.0, 0.0], 0, &mut regions);
    Ok(SolvedTree {
        content_size,
        regions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(
        child_index: usize,
        column: usize,
        size: Size,
        total_edges: Edges<f32>,
    ) -> LayoutItem<usize> {
        LayoutItem {
            child_index,
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
            stacked_edges: Edges::default(),
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

        let envelope = tree_envelope(&node).expect("band tree should collect");

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
        inner.stacked_edges = Edges::new(0.0, 35.0, 0.0, 0.0);

        let inner_envelope = tree_envelope(&inner).expect("inner tree should collect");
        assert_eq!(inner_envelope.total_edges.right, 50.0);

        let outer = column_band(
            vec![
                leaf(0, 0, Size::new(100.0, 60.0), Edges::default()),
                LayoutItem {
                    child_index: 1,
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

        let envelope = tree_envelope(&outer).expect("outer tree should collect");
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
                    child_index: 1,
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

        let solved = solve_tree(&root, None).expect("tree should solve");

        // Inner extent: 40 + 6 + 40 = 86; root: 50 + 10 + 86 = 146.
        assert_eq!(solved.content_size, Size::new(146.0, 60.0));
        let rects: Vec<(usize, usize, Rect)> = solved
            .regions
            .iter()
            .map(|region| (region.depth, region.child_index, region.content_rect))
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
                child_index: 0,
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
        let solved = solve_tree(&root, Some(Size::new(120.0, 60.0)))
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
