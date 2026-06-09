//! Requirement merging, deltas, and the one-round alignment engine.
//!
//! Callers own node identity, traversal, group-key derivation, side-car
//! state, and application. This module groups nodes by caller-supplied keys,
//! merges compatible grid requirements (component-wise max), and reports
//! numeric deltas between local and merged requirements.
//!
//! [`align`] models ONE pure round. Driver loops — re-measure content whose
//! allocation changed, then align again until deltas reach zero or a cap —
//! belong to the caller, because re-measurement is not a layout concern.

use std::collections::HashMap;
use std::hash::Hash;

use crate::grid::GridRequirements;
use crate::region::EdgeDemand;

/// One measured node participating in a single alignment round.
///
/// `id` and `group_key` are caller-owned and opaque: nodes sharing a
/// `group_key` are required to end up with identical track layouts.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentNode<Id = usize, Key = usize> {
    pub id: Id,
    pub group_key: Key,
    pub requirements: GridRequirements,
}

/// Requirement delta from one local node to its group's merged requirements.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDelta<Id = usize> {
    pub id: Id,
    pub content_delta: f32,
    pub edge_delta: f32,
}

impl<Id> NodeDelta<Id> {
    pub fn has_delta(&self) -> bool {
        self.content_delta > 0.0 || self.edge_delta > 0.0
    }
}

/// One group of nodes merged to a shared requirement.
///
/// `deltas` doubles as the group member list (one entry per node, in input
/// order); callers fold side-car state per group through these member IDs.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignedGroup<Id = usize, Key = usize> {
    pub key: Key,
    pub merged: GridRequirements,
    pub deltas: Vec<NodeDelta<Id>>,
}

/// Why a group was not merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedGroupReason {
    /// Only one node carried the key; there is nothing to align.
    Singleton,
    /// Nodes in the group had incompatible (differently shaped) requirements.
    IncompatibleRequirements,
}

/// A group that was intentionally not merged.
#[derive(Clone, Debug, PartialEq)]
pub struct SkippedGroup<Key = usize> {
    pub key: Key,
    pub node_count: usize,
    pub reason: SkippedGroupReason,
}

/// Result of one alignment round.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentPlan<Id = usize, Key = usize> {
    pub node_count: usize,
    pub groups: Vec<AlignedGroup<Id, Key>>,
    pub skipped: Vec<SkippedGroup<Key>>,
}

impl<Id, Key> AlignmentPlan<Id, Key> {
    /// Total number of distinct group keys seen, merged or skipped.
    pub fn group_count(&self) -> usize {
        self.groups.len() + self.skipped.len()
    }

    pub fn changed_node_count(&self) -> usize {
        self.groups
            .iter()
            .flat_map(|group| group.deltas.iter())
            .filter(|delta| delta.has_delta())
            .count()
    }

    pub fn total_content_delta(&self) -> f32 {
        self.groups
            .iter()
            .flat_map(|group| group.deltas.iter())
            .map(|delta| delta.content_delta)
            .sum()
    }

    pub fn total_edge_delta(&self) -> f32 {
        self.groups
            .iter()
            .flat_map(|group| group.deltas.iter())
            .map(|delta| delta.edge_delta)
            .sum()
    }
}

/// Run one alignment round over measured nodes.
///
/// Nodes are grouped by `group_key` in first-seen key order (deterministic
/// for callers that iterate the plan), each multi-node group with compatible
/// shapes is merged by component-wise max, and per-node deltas are reported
/// against the merged requirements.
pub fn align<Id, Key>(nodes: &[AlignmentNode<Id, Key>]) -> AlignmentPlan<Id, Key>
where
    Id: Clone,
    Key: Clone + Eq + Hash,
{
    let mut group_order: Vec<(Key, Vec<usize>)> = Vec::new();
    let mut group_index: HashMap<Key, usize> = HashMap::new();
    for (node_index, node) in nodes.iter().enumerate() {
        match group_index.get(&node.group_key) {
            Some(&index) => group_order[index].1.push(node_index),
            None => {
                group_index.insert(node.group_key.clone(), group_order.len());
                group_order.push((node.group_key.clone(), vec![node_index]));
            }
        }
    }

    let mut groups = Vec::new();
    let mut skipped = Vec::new();
    for (key, member_indices) in group_order {
        if member_indices.len() < 2 {
            skipped.push(SkippedGroup {
                key,
                node_count: member_indices.len(),
                reason: SkippedGroupReason::Singleton,
            });
            continue;
        }

        let Some(merged) = merge_grid_requirements(
            member_indices
                .iter()
                .map(|&index| &nodes[index].requirements),
        ) else {
            skipped.push(SkippedGroup {
                key,
                node_count: member_indices.len(),
                reason: SkippedGroupReason::IncompatibleRequirements,
            });
            continue;
        };

        let deltas = member_indices
            .iter()
            .map(|&index| {
                let node = &nodes[index];
                NodeDelta {
                    id: node.id.clone(),
                    content_delta: grid_content_delta(&node.requirements, &merged),
                    edge_delta: grid_edge_delta(&node.requirements, &merged),
                }
            })
            .collect();

        groups.push(AlignedGroup {
            key,
            merged,
            deltas,
        });
    }

    AlignmentPlan {
        node_count: nodes.len(),
        groups,
        skipped,
    }
}

/// Merge compatible requirements by component-wise maximum.
///
/// Returns `None` when the input is empty or any two requirements have
/// different shapes.
pub fn merge_grid_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a GridRequirements>,
) -> Option<GridRequirements> {
    let mut iter = requirements.into_iter();
    let first = iter.next()?.clone();
    iter.try_fold(first, |mut merged, next| {
        if merged.shape != next.shape {
            return None;
        }

        merged.column_spacing = merged.column_spacing.merge_max(next.column_spacing);
        merged.row_spacing = merged.row_spacing.merge_max(next.row_spacing);
        max_assign_each(&mut merged.column_widths, &next.column_widths);
        max_assign_each(&mut merged.row_heights, &next.row_heights);
        max_assign_edge_each(&mut merged.column_left, &next.column_left);
        max_assign_edge_each(&mut merged.column_right, &next.column_right);
        max_assign_edge_each(&mut merged.row_top, &next.row_top);
        max_assign_edge_each(&mut merged.row_bottom, &next.row_bottom);
        Some(merged)
    })
}

/// Total absolute track-size difference between local and merged requirements.
pub fn grid_content_delta(local: &GridRequirements, merged: &GridRequirements) -> f32 {
    abs_delta_sum(&local.column_widths, &merged.column_widths)
        + abs_delta_sum(&local.row_heights, &merged.row_heights)
}

/// Total absolute edge/spacing difference between local and merged
/// requirements.
pub fn grid_edge_delta(local: &GridRequirements, merged: &GridRequirements) -> f32 {
    abs_edge_delta_sum(&local.column_left, &merged.column_left)
        + abs_edge_delta_sum(&local.column_right, &merged.column_right)
        + abs_edge_delta_sum(&local.row_top, &merged.row_top)
        + abs_edge_delta_sum(&local.row_bottom, &merged.row_bottom)
        + local.column_spacing.abs_delta(merged.column_spacing)
        + local.row_spacing.abs_delta(merged.row_spacing)
}

fn max_assign_each(target: &mut [f32], source: &[f32]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = (*target).max(*source);
    }
}

fn max_assign_edge_each(target: &mut [EdgeDemand], source: &[EdgeDemand]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = target.max_components(*source);
    }
}

fn abs_delta_sum(local: &[f32], merged: &[f32]) -> f32 {
    debug_assert_eq!(local.len(), merged.len());
    local
        .iter()
        .zip(merged.iter())
        .map(|(local, merged)| (merged - local).abs())
        .sum()
}

fn abs_edge_delta_sum(local: &[EdgeDemand], merged: &[EdgeDemand]) -> f32 {
    debug_assert_eq!(local.len(), merged.len());
    local
        .iter()
        .zip(merged.iter())
        .map(|(local, merged)| (merged.total - local.total).abs())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{GridShape, TrackSpacing, total_edge_demands, zero_edge_demands};

    fn requirements(width: f32, left_total: f32) -> GridRequirements {
        GridRequirements {
            shape: GridShape {
                rows: 1,
                columns: 1,
            },
            column_spacing: TrackSpacing::default(),
            row_spacing: TrackSpacing::default(),
            column_widths: vec![width],
            row_heights: vec![40.0],
            column_left: total_edge_demands([left_total]),
            column_right: zero_edge_demands(1),
            row_top: zero_edge_demands(1),
            row_bottom: zero_edge_demands(1),
        }
    }

    #[test]
    fn merge_grid_requirements_keeps_component_maxima() {
        let mut first = requirements(100.0, 4.0);
        first.column_spacing.min_gap = 9.0;
        let mut second = requirements(80.0, 12.0);
        second.row_heights[0] = 55.0;
        second.column_spacing.min_gap = 5.0;
        second.column_spacing.outer_start = 3.0;

        let merged = merge_grid_requirements([&first, &second]).unwrap();

        assert_eq!(merged.column_widths, vec![100.0]);
        assert_eq!(merged.row_heights, vec![55.0]);
        assert_eq!(merged.column_left[0].total, 12.0);
        assert_eq!(
            merged.column_spacing,
            TrackSpacing {
                outer_start: 3.0,
                outer_end: 0.0,
                min_gap: 9.0
            }
        );
    }

    #[test]
    fn merge_grid_requirements_rejects_different_shapes() {
        let first = requirements(100.0, 4.0);
        let mut second = requirements(80.0, 12.0);
        second.shape.columns = 2;

        assert!(merge_grid_requirements([&first, &second]).is_none());
    }

    #[test]
    fn grid_requirement_deltas_are_separated_by_content_and_edge() {
        let local = requirements(100.0, 4.0);
        let mut merged = requirements(120.0, 10.0);
        merged.row_heights[0] = 45.0;
        merged.column_spacing.outer_start = 3.0;
        merged.column_spacing.min_gap = 6.0;

        assert_eq!(grid_content_delta(&local, &merged), 25.0);
        assert_eq!(grid_edge_delta(&local, &merged), 15.0);
    }

    fn node(
        id: usize,
        group_key: &str,
        width: f32,
        left_total: f32,
    ) -> AlignmentNode<usize, String> {
        AlignmentNode {
            id,
            group_key: group_key.to_string(),
            requirements: requirements(width, left_total),
        }
    }

    #[test]
    fn align_merges_groups_in_first_seen_key_order() {
        let nodes = vec![
            node(10, "b", 100.0, 0.0),
            node(11, "a", 80.0, 0.0),
            node(12, "b", 120.0, 0.0),
            node(13, "a", 90.0, 0.0),
        ];

        let plan = align(&nodes);

        assert_eq!(plan.node_count, 4);
        assert_eq!(plan.group_count(), 2);
        assert!(plan.skipped.is_empty());
        assert_eq!(plan.groups[0].key, "b");
        assert_eq!(plan.groups[1].key, "a");
        assert_eq!(plan.groups[0].merged.column_widths, vec![120.0]);
        assert_eq!(plan.groups[1].merged.column_widths, vec![90.0]);
        let member_ids: Vec<usize> = plan.groups[0].deltas.iter().map(|delta| delta.id).collect();
        assert_eq!(member_ids, vec![10, 12]);
    }

    #[test]
    fn align_skips_singletons() {
        let nodes = vec![node(0, "solo", 100.0, 0.0), node(1, "pair", 80.0, 0.0)];

        let plan = align(&nodes[..1]);
        assert_eq!(plan.groups.len(), 0);
        assert_eq!(
            plan.skipped,
            vec![SkippedGroup {
                key: "solo".to_string(),
                node_count: 1,
                reason: SkippedGroupReason::Singleton,
            }]
        );
        assert_eq!(plan.group_count(), 1);
        assert_eq!(plan.changed_node_count(), 0);
    }

    #[test]
    fn align_skips_incompatible_shapes() {
        let mut wide = node(0, "g", 100.0, 0.0);
        wide.requirements.shape.columns = 2;
        wide.requirements.column_widths = vec![100.0, 100.0];
        wide.requirements.column_left = zero_edge_demands(2);
        wide.requirements.column_right = zero_edge_demands(2);
        let nodes = vec![wide, node(1, "g", 80.0, 0.0)];

        let plan = align(&nodes);

        assert!(plan.groups.is_empty());
        assert_eq!(
            plan.skipped[0].reason,
            SkippedGroupReason::IncompatibleRequirements
        );
        assert_eq!(plan.skipped[0].node_count, 2);
    }

    #[test]
    fn align_reports_per_node_deltas_against_merged() {
        let nodes = vec![node(0, "g", 100.0, 4.0), node(1, "g", 80.0, 12.0)];

        let plan = align(&nodes);

        let group = &plan.groups[0];
        assert_eq!(group.merged.column_widths, vec![100.0]);
        assert_eq!(group.merged.column_left[0].total, 12.0);
        assert_eq!(
            group.deltas,
            vec![
                NodeDelta {
                    id: 0,
                    content_delta: 0.0,
                    edge_delta: 8.0,
                },
                NodeDelta {
                    id: 1,
                    content_delta: 20.0,
                    edge_delta: 0.0,
                },
            ]
        );
        assert!(group.deltas[0].has_delta());
        assert_eq!(plan.changed_node_count(), 2);
        assert_eq!(plan.total_content_delta(), 20.0);
        assert_eq!(plan.total_edge_delta(), 8.0);
    }
}
