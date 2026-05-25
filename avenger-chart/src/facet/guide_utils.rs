//! Facet-specific guide helpers.
//!
//! Generic band-guide slab measurement and rendering lives in
//! `plot::compiled::container_band_guide`. This module keeps the facet-specific
//! visibility and value-formatting rules that depend on the evaluated facet
//! partition tree.

use datafusion::common::ScalarValue;

#[cfg(test)]
use crate::{
    chart_core::AxisPosition, facet::evaluated_facet_tree::EvaluatedFacetTree,
    plot::compiled::SharingLevel,
};

/// Format a ScalarValue for display as a facet label.
pub(crate) fn format_scalar_value(value: &ScalarValue) -> String {
    crate::partition::format_partition_value(value)
}

/// Determine whether facet guide labels should be visible for a specific facet cell.
///
/// This reuses channel-axis ownership logic so facet guide label ownership
/// follows sharing groups consistently with cartesian axis label ownership.
///
/// Invalid paths fall back to visible to preserve prior permissive behavior.
#[cfg(test)]
pub(crate) fn facet_guide_labels_visible_for_cell(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    axis_position: AxisPosition,
    sharing_level: u8,
) -> bool {
    if facet_path.is_empty() {
        return true;
    }

    facet_tree
        .channel_axis_visibility_for_path_checked(facet_path, axis_position, sharing_level)
        .map(|visibility| visibility.show_labels)
        .unwrap_or(true)
}

/// Determine whether a facet guide title should be visible for a specific cell.
///
/// Facet titles use channel-axis ownership so mixed row/column nesting scopes
/// titles to the strip controlled by the guide axis.
#[cfg(test)]
pub(crate) fn facet_guide_title_visible_for_cell(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    axis_position: AxisPosition,
) -> bool {
    if facet_path.is_empty() {
        return true;
    }

    facet_tree
        .channel_axis_visibility_for_path_checked(
            facet_path,
            axis_position,
            SharingLevel::GLOBAL.raw(),
        )
        .map(|visibility| visibility.show_title)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use indexmap::IndexMap;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn column_then_row_tree() -> EvaluatedFacetTree {
        let make_leaf = || {
            PartitionNode::leaf(
                FacetDirection::Row,
                255,
                "species".to_string(),
                None,
                vec![s("setosa"), s("versicolor"), s("virginica")],
            )
        };

        let mut children = IndexMap::new();
        children.insert(s("short"), Box::new(make_leaf()));
        children.insert(s("medium"), Box::new(make_leaf()));
        children.insert(s("long"), Box::new(make_leaf()));

        EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            255,
            "length_bin".to_string(),
            None,
            children,
        )))
    }

    fn column_row_column_tree() -> EvaluatedFacetTree {
        let make_team_leaf = || {
            PartitionNode::leaf(
                FacetDirection::Column,
                255,
                "team".to_string(),
                None,
                vec![s("Team1"), s("Team2")],
            )
        };

        let make_department_branch = || {
            let mut dept_children = IndexMap::new();
            dept_children.insert(s("Dept1"), Box::new(make_team_leaf()));
            dept_children.insert(s("Dept2"), Box::new(make_team_leaf()));
            PartitionNode::branch(
                FacetDirection::Row,
                255,
                "department".to_string(),
                None,
                dept_children,
            )
        };

        let mut division_children = IndexMap::new();
        division_children.insert(s("DivA"), Box::new(make_department_branch()));
        division_children.insert(s("DivB"), Box::new(make_department_branch()));

        EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            255,
            "division".to_string(),
            None,
            division_children,
        )))
    }

    #[test]
    fn facet_guide_labels_shared_right_only_show_on_far_right_owner() {
        let tree = column_then_row_tree();

        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Right,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Right,
            255
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Right,
            255
        ));
    }

    #[test]
    fn facet_guide_labels_shared_left_only_show_on_far_left_owner() {
        let tree = column_then_row_tree();

        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Left,
            255
        ));
    }

    #[test]
    fn facet_guide_labels_free_sharing_show_on_all_columns() {
        let tree = column_then_row_tree();
        for bucket in ["short", "medium", "long"] {
            assert!(facet_guide_labels_visible_for_cell(
                &tree,
                &[s(bucket)],
                AxisPosition::Right,
                0
            ));
        }
    }

    #[test]
    fn facet_guide_labels_jagged_groups_keep_group_local_owner_hiding() {
        let mut group_a = IndexMap::new();
        group_a.insert(
            s("C1"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_a.insert(
            s("C2"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_a.insert(
            s("C3"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );

        let mut group_b = IndexMap::new();
        group_b.insert(
            s("C1"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );
        group_b.insert(
            s("C2"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "leaf".to_string(),
                None,
                vec![s("L")],
            )),
        );

        let mut outer = IndexMap::new();
        outer.insert(
            s("A"),
            Box::new(PartitionNode::branch(
                FacetDirection::Column,
                255,
                "inner_col".to_string(),
                None,
                group_a,
            )),
        );
        outer.insert(
            s("B"),
            Box::new(PartitionNode::branch(
                FacetDirection::Column,
                255,
                "inner_col".to_string(),
                None,
                group_b,
            )),
        );

        let tree = EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Row,
            255,
            "outer_row".to_string(),
            None,
            outer,
        )));

        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C1")],
            AxisPosition::Right,
            1
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C2")],
            AxisPosition::Right,
            1
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("A"), s("C3")],
            AxisPosition::Right,
            1
        ));

        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("B"), s("C1")],
            AxisPosition::Right,
            1
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("B"), s("C2")],
            AxisPosition::Right,
            1
        ));
    }

    #[test]
    fn facet_guide_title_repeats_for_column_guides_in_each_row_strip() {
        let tree = column_row_column_tree();

        assert!(facet_guide_title_visible_for_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            AxisPosition::Top,
        ));
        assert!(!facet_guide_title_visible_for_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            AxisPosition::Top,
        ));
        assert!(facet_guide_title_visible_for_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            AxisPosition::Top,
        ));
        assert!(!facet_guide_title_visible_for_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            AxisPosition::Top,
        ));
    }
}
