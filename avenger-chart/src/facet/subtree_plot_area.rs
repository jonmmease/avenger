//! Estimated plot-area sizes for plot-area-sized facet subtrees.
//!
//! These helpers synthesize an initial subtree size from the evaluated facet
//! tree and a requested leaf plot size. They intentionally do not include
//! coordinated overflows, guide padding, legends, or explicit placement gaps.
//! After coordination, plot-area-sized facets use realized explicit placement as
//! the source of truth for final subtree extents.

use datafusion::common::ScalarValue;

use crate::{
    coords::FacetAxis,
    facet::FacetDirection,
    facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionContent, PartitionNode},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlotAreaSize {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl PlotAreaSize {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub(crate) fn from_leaf(leaf: LeafPlotAreaSize) -> Self {
        Self::new(leaf.width, leaf.height)
    }

    pub(crate) fn clamped(self) -> Self {
        Self::new(self.width.max(1.0), self.height.max(1.0))
    }

    pub(crate) fn main_size(self, axis: FacetAxis) -> f32 {
        match axis {
            FacetAxis::Column => self.width,
            FacetAxis::Row => self.height,
        }
    }

    pub(crate) fn dimensions(self) -> (f32, f32) {
        (self.width, self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LeafPlotAreaSize {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl LeafPlotAreaSize {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

pub(crate) fn estimate_subtree_plot_area_from_leaf_size(
    node: &PartitionNode,
    leaf: LeafPlotAreaSize,
) -> PlotAreaSize {
    match &node.content {
        PartitionContent::Leaf { values } => {
            let count = values.len().max(1) as f32;
            match node.direction {
                FacetDirection::Column => PlotAreaSize::new(leaf.width * count, leaf.height),
                FacetDirection::Row => PlotAreaSize::new(leaf.width, leaf.height * count),
            }
        }
        PartitionContent::Branch { children } => {
            let mut child_sizes = children
                .values()
                .map(|child| estimate_subtree_plot_area_from_leaf_size(child.as_ref(), leaf));

            let Some(first) = child_sizes.next() else {
                return PlotAreaSize::from_leaf(leaf);
            };

            match node.direction {
                FacetDirection::Column => {
                    let mut total_width = first.width;
                    let mut max_height = first.height;
                    for size in child_sizes {
                        total_width += size.width;
                        max_height = max_height.max(size.height);
                    }
                    PlotAreaSize::new(total_width, max_height)
                }
                FacetDirection::Row => {
                    let mut max_width = first.width;
                    let mut total_height = first.height;
                    for size in child_sizes {
                        max_width = max_width.max(size.width);
                        total_height += size.height;
                    }
                    PlotAreaSize::new(max_width, total_height)
                }
            }
        }
    }
}

pub(crate) fn estimate_root_plot_area_from_leaf_size(
    facet_tree: &EvaluatedFacetTree,
    leaf: LeafPlotAreaSize,
) -> PlotAreaSize {
    facet_tree
        .root()
        .map(|root| estimate_subtree_plot_area_from_leaf_size(root, leaf))
        .unwrap_or_else(|| PlotAreaSize::from_leaf(leaf).clamped())
}

pub(crate) fn estimate_path_plot_area_from_leaf_size(
    facet_tree: &EvaluatedFacetTree,
    path: &[ScalarValue],
    leaf: LeafPlotAreaSize,
) -> PlotAreaSize {
    facet_tree
        .node_at_path(path)
        .map(|node| estimate_subtree_plot_area_from_leaf_size(node, leaf))
        .unwrap_or_else(|| PlotAreaSize::from_leaf(leaf))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    fn value(label: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(label.to_string()))
    }

    fn leaf_node(direction: FacetDirection, count: usize) -> PartitionNode {
        let values: Vec<ScalarValue> = (0..count).map(|idx| value(&idx.to_string())).collect();
        PartitionNode::leaf(direction, 0, "field".to_string(), None, values)
    }

    fn empty_branch_node(direction: FacetDirection) -> PartitionNode {
        PartitionNode::branch(direction, 0, "field".to_string(), None, IndexMap::new())
    }

    fn branch_node(
        direction: FacetDirection,
        children: Vec<(&str, PartitionNode)>,
    ) -> PartitionNode {
        let children = children
            .into_iter()
            .map(|(key, child)| (value(key), Box::new(child)))
            .collect::<IndexMap<_, _>>();
        PartitionNode::branch(direction, 0, "field".to_string(), None, children)
    }

    #[test]
    fn column_leaf_estimate_multiplies_width_by_value_count() {
        let leaf = LeafPlotAreaSize::new(100.0, 50.0);
        let size =
            estimate_subtree_plot_area_from_leaf_size(&leaf_node(FacetDirection::Column, 3), leaf);

        assert_eq!(size, PlotAreaSize::new(300.0, 50.0));
    }

    #[test]
    fn row_leaf_estimate_multiplies_height_by_value_count() {
        let leaf = LeafPlotAreaSize::new(100.0, 50.0);
        let size =
            estimate_subtree_plot_area_from_leaf_size(&leaf_node(FacetDirection::Row, 3), leaf);

        assert_eq!(size, PlotAreaSize::new(100.0, 150.0));
    }

    #[test]
    fn nested_column_branch_sums_width_and_takes_max_height() {
        let leaf = LeafPlotAreaSize::new(100.0, 50.0);
        let node = branch_node(
            FacetDirection::Column,
            vec![
                ("a", leaf_node(FacetDirection::Row, 2)),
                ("b", leaf_node(FacetDirection::Row, 3)),
            ],
        );
        let size = estimate_subtree_plot_area_from_leaf_size(&node, leaf);

        assert_eq!(size, PlotAreaSize::new(200.0, 150.0));
    }

    #[test]
    fn ragged_row_branch_takes_max_width_and_sums_height() {
        let leaf = LeafPlotAreaSize::new(100.0, 50.0);
        let node = branch_node(
            FacetDirection::Row,
            vec![
                ("a", leaf_node(FacetDirection::Column, 2)),
                ("b", leaf_node(FacetDirection::Column, 3)),
            ],
        );
        let size = estimate_subtree_plot_area_from_leaf_size(&node, leaf);

        assert_eq!(size, PlotAreaSize::new(300.0, 100.0));
    }

    #[test]
    fn empty_leaf_and_empty_branch_fall_back_to_one_leaf_plot_area() {
        let leaf = LeafPlotAreaSize::new(100.0, 50.0);
        let empty_leaf =
            estimate_subtree_plot_area_from_leaf_size(&leaf_node(FacetDirection::Column, 0), leaf);
        let empty_branch = estimate_subtree_plot_area_from_leaf_size(
            &empty_branch_node(FacetDirection::Row),
            leaf,
        );

        assert_eq!(empty_leaf, PlotAreaSize::new(100.0, 50.0));
        assert_eq!(empty_branch, PlotAreaSize::new(100.0, 50.0));
    }

    #[test]
    fn empty_root_clamps_but_missing_path_preserves_leaf_size() {
        let leaf = LeafPlotAreaSize::new(0.25, 0.5);
        let facet_tree = EvaluatedFacetTree::empty();

        assert_eq!(
            estimate_root_plot_area_from_leaf_size(&facet_tree, leaf),
            PlotAreaSize::new(1.0, 1.0)
        );
        assert_eq!(
            estimate_path_plot_area_from_leaf_size(&facet_tree, &[value("missing")], leaf),
            PlotAreaSize::new(0.25, 0.5)
        );
    }
}
