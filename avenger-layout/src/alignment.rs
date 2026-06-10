//! Requirement merging, deltas, and the one-round alignment engine.
//!
//! Callers own node identity, traversal, group-key derivation, side-car
//! state, and application. This module groups nodes by caller-supplied keys,
//! merges each group's payloads, and reports numeric deltas between local
//! and merged payloads.
//!
//! [`align_by`] is the payload-generic round: grouping, skip reasons, member
//! IDs, and delta bookkeeping are engine mechanics, while merge and delta
//! laws are caller-supplied. [`align`] is the [`GridRequirements`]
//! specialization. Deltas use two conventional channels — `content` (sizes)
//! and `edge` (chrome/spacing) — that callers map their payload onto.
//!
//! A round is ONE pure pass. Driver loops — re-measure content whose
//! allocation changed, then align again until deltas reach zero or a cap —
//! belong to the caller, because re-measurement is not a layout concern.
//! [`ConvergenceTrace`] records per-round deltas for such drivers.

use std::collections::HashMap;
use std::hash::Hash;

use crate::grid::GridRequirements;
use crate::region::EdgeDemand;

/// One measured node participating in a single alignment round.
///
/// `id` and `group_key` are caller-owned and opaque: nodes sharing a
/// `group_key` are required to end up with identical coordinated payloads.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentNode<Id = usize, Key = usize, P = GridRequirements> {
    pub id: Id,
    pub group_key: Key,
    pub requirements: P,
}

/// How a round treats groups with a single member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingletonPolicy {
    /// Report singleton groups as skipped; there is nothing to align.
    /// The grid alignment pass uses this.
    Skip,
    /// Merge singleton groups (merged == the lone payload) so every node
    /// receives a patch. Coordination passes that must install coordinated
    /// state on every node use this.
    Merge,
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

/// One group of nodes merged to a shared payload.
///
/// `deltas` doubles as the group member list (one entry per node, in input
/// order); callers fold side-car state per group through these member IDs.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignedGroup<Id = usize, Key = usize, P = GridRequirements> {
    pub key: Key,
    pub merged: P,
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
pub struct AlignmentPlan<Id = usize, Key = usize, P = GridRequirements> {
    pub node_count: usize,
    pub groups: Vec<AlignedGroup<Id, Key, P>>,
    pub skipped: Vec<SkippedGroup<Key>>,
}

impl<Id, Key, P> AlignmentPlan<Id, Key, P> {
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

/// Run one alignment round over measured grid nodes.
///
/// The [`GridRequirements`] specialization of [`align_by`]: singleton groups
/// are skipped, compatible multi-node groups merge by component-wise max,
/// and deltas report track sizes (content) vs edges/spacing (edge).
pub fn align<Id, Key>(nodes: &[AlignmentNode<Id, Key>]) -> AlignmentPlan<Id, Key>
where
    Id: Clone,
    Key: Clone + Eq + Hash,
{
    align_by(
        nodes,
        SingletonPolicy::Skip,
        |members| GridRequirements::merged(members.iter().copied()),
        |local, merged| (local.content_delta(merged), local.edge_delta(merged)),
    )
}

/// Run one alignment round over measured nodes with a caller-supplied
/// payload.
///
/// Nodes are grouped by `group_key` in first-seen key order (deterministic
/// for callers that iterate the plan). Each group is merged by `merge`
/// (returning `None` marks the group incompatible), and `delta` reports each
/// member's `(content_delta, edge_delta)` against the merged payload.
pub fn align_by<Id, Key, P>(
    nodes: &[AlignmentNode<Id, Key, P>],
    singleton_policy: SingletonPolicy,
    mut merge: impl FnMut(&[&P]) -> Option<P>,
    mut delta: impl FnMut(&P, &P) -> (f32, f32),
) -> AlignmentPlan<Id, Key, P>
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
        if member_indices.len() < 2 && singleton_policy == SingletonPolicy::Skip {
            skipped.push(SkippedGroup {
                key,
                node_count: member_indices.len(),
                reason: SkippedGroupReason::Singleton,
            });
            continue;
        }

        let members = member_indices
            .iter()
            .map(|&index| &nodes[index].requirements)
            .collect::<Vec<_>>();
        let Some(merged) = merge(&members) else {
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
                let (content_delta, edge_delta) = delta(&node.requirements, &merged);
                NodeDelta {
                    id: node.id.clone(),
                    content_delta,
                    edge_delta,
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

/// Per-round delta totals recorded by a multi-round driver.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoundDeltas {
    pub content: f32,
    pub edge: f32,
}

impl RoundDeltas {
    pub fn total(self) -> f32 {
        self.content + self.edge
    }
}

/// Cross-round convergence evidence for drivers that alternate alignment
/// rounds with re-measurement.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConvergenceTrace {
    rounds: Vec<RoundDeltas>,
}

impl ConvergenceTrace {
    pub fn record_round(&mut self, content: f32, edge: f32) {
        self.rounds.push(RoundDeltas { content, edge });
    }

    pub fn record_plan<Id, Key, P>(&mut self, plan: &AlignmentPlan<Id, Key, P>) {
        self.record_round(plan.total_content_delta(), plan.total_edge_delta());
    }

    pub fn rounds(&self) -> &[RoundDeltas] {
        &self.rounds
    }

    /// The last recorded round had no deltas beyond `epsilon`.
    pub fn is_converged(&self, epsilon: f32) -> bool {
        self.rounds
            .last()
            .is_some_and(|round| round.total() <= epsilon)
    }

    /// Two or more consecutive rounds with non-decreasing, non-zero deltas:
    /// evidence the driver is cycling rather than converging.
    pub fn is_non_converging(&self, epsilon: f32) -> bool {
        self.rounds
            .windows(2)
            .any(|pair| pair[0].total() > epsilon && pair[1].total() >= pair[0].total())
    }
}

impl GridRequirements {
    /// Merge compatible requirements by component-wise maximum.
    ///
    /// Returns `None` when the input is empty or any two requirements have
    /// different shapes.
    pub fn merged<'a>(
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

    /// Total absolute track-size difference against merged requirements.
    pub fn content_delta(&self, merged: &GridRequirements) -> f32 {
        let local = self;
        abs_delta_sum(&local.column_widths, &merged.column_widths)
            + abs_delta_sum(&local.row_heights, &merged.row_heights)
    }

    /// Total absolute edge/spacing difference against merged requirements.
    pub fn edge_delta(&self, merged: &GridRequirements) -> f32 {
        let local = self;
        abs_edge_delta_sum(&local.column_left, &merged.column_left)
            + abs_edge_delta_sum(&local.column_right, &merged.column_right)
            + abs_edge_delta_sum(&local.row_top, &merged.row_top)
            + abs_edge_delta_sum(&local.row_bottom, &merged.row_bottom)
            + local.column_spacing.abs_delta(merged.column_spacing)
            + local.row_spacing.abs_delta(merged.row_spacing)
    }
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

        let merged = GridRequirements::merged([&first, &second]).unwrap();

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

        assert!(GridRequirements::merged([&first, &second]).is_none());
    }

    #[test]
    fn grid_requirement_deltas_are_separated_by_content_and_edge() {
        let local = requirements(100.0, 4.0);
        let mut merged = requirements(120.0, 10.0);
        merged.row_heights[0] = 45.0;
        merged.column_spacing.outer_start = 3.0;
        merged.column_spacing.min_gap = 6.0;

        assert_eq!(local.content_delta(&merged), 25.0);
        assert_eq!(local.edge_delta(&merged), 15.0);
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

    fn payload_node(id: usize, group_key: &str, value: f32) -> AlignmentNode<usize, String, f32> {
        AlignmentNode {
            id,
            group_key: group_key.to_string(),
            requirements: value,
        }
    }

    fn max_merge(members: &[&f32]) -> Option<f32> {
        members.iter().copied().copied().reduce(f32::max)
    }

    fn abs_delta(local: &f32, merged: &f32) -> (f32, f32) {
        (0.0, (merged - local).abs())
    }

    #[test]
    fn align_by_merges_custom_payloads_with_caller_laws() {
        let nodes = vec![
            payload_node(0, "g", 10.0),
            payload_node(1, "g", 25.0),
            payload_node(2, "h", 5.0),
        ];

        let plan = align_by(&nodes, SingletonPolicy::Skip, max_merge, abs_delta);

        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].merged, 25.0);
        assert_eq!(plan.groups[0].deltas[0].edge_delta, 15.0);
        assert_eq!(plan.groups[0].deltas[1].edge_delta, 0.0);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].reason, SkippedGroupReason::Singleton);
    }

    #[test]
    fn align_by_merge_policy_patches_singletons() {
        let nodes = vec![payload_node(0, "solo", 10.0), payload_node(1, "g", 5.0)];

        let plan = align_by(&nodes, SingletonPolicy::Merge, max_merge, abs_delta);

        assert!(plan.skipped.is_empty());
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].key, "solo");
        assert_eq!(plan.groups[0].merged, 10.0);
        assert_eq!(plan.groups[0].deltas.len(), 1);
        assert!(!plan.groups[0].deltas[0].has_delta());
    }

    #[test]
    fn edge_demand_edges_merge_component_wise() {
        use crate::geometry::Edges;

        let left = Edges::new(
            EdgeDemand::new(10.0, 0.0, 10.0),
            EdgeDemand::default(),
            EdgeDemand::default(),
            EdgeDemand::new(1.0, 2.0, 3.0),
        );
        let right = Edges::new(
            EdgeDemand::new(0.0, 10.0, 10.0),
            EdgeDemand::default(),
            EdgeDemand::default(),
            EdgeDemand::new(4.0, 0.0, 4.0),
        );

        let merged = left.max_components(right);
        assert_eq!(merged.top, EdgeDemand::new(10.0, 10.0, 20.0));
        assert_eq!(merged.left, EdgeDemand::new(4.0, 2.0, 6.0));
    }

    #[test]
    fn convergence_trace_detects_settling_and_cycling() {
        let mut settling = ConvergenceTrace::default();
        settling.record_round(30.0, 10.0);
        settling.record_round(5.0, 1.0);
        settling.record_round(0.0, 0.0);
        assert!(settling.is_converged(0.01));
        assert!(!settling.is_non_converging(0.01));
        assert_eq!(settling.rounds().len(), 3);

        let mut cycling = ConvergenceTrace::default();
        cycling.record_round(12.0, 0.0);
        cycling.record_round(12.0, 0.0);
        assert!(!cycling.is_converged(0.01));
        assert!(cycling.is_non_converging(0.01));
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
