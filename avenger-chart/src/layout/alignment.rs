//! Chart-owned requirement alignment: grouping, merging, and deltas for
//! coordinating equivalent layout requirements measured independently.
//!
//! This engine plans the child-frame group solves and reports on them.
//! `align_by` groups container requirements by alignment key (the merge
//! closures carry chart payloads the solver does not model — the
//! guide-gap side-car), the resulting plan selects which cousins enter
//! one `solve_concat_grid_group`, and the local-vs-merged deltas feed the
//! coordination diagnostics. The merged values applied to containers come
//! from that group solve — `Layout::solve`'s share-key coordination —
//! while this engine's fold serves planning and delta reporting.
//! [`ConvergenceTrace`] records per-round deltas for the driver logs.
//!
//! A round is ONE pure pass: callers own identity, traversal, group keys,
//! and application.

use std::collections::HashMap;
use std::hash::Hash;

use avenger_layout::EdgeGrant;

use super::concat_grid::ChartGridData;

/// One measured node participating in a single alignment round.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AlignmentNode<Id = usize, Key = usize, P = ChartGridData> {
    pub id: Id,
    pub group_key: Key,
    pub requirements: P,
}

/// How a round treats groups with a single member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SingletonPolicy {
    /// Report singleton groups as skipped; there is nothing to align.
    Skip,
}

/// Requirement delta from one local node to its group's merged requirements.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeDelta<Id = usize> {
    pub id: Id,
    pub content_delta: f32,
    pub edge_delta: f32,
}

/// One group of nodes merged to a shared payload. `deltas` doubles as the
/// member list (one entry per node, in input order).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AlignedGroup<Id = usize, Key = usize, P = ChartGridData> {
    pub key: Key,
    pub merged: P,
    pub deltas: Vec<NodeDelta<Id>>,
}

/// Why a group was not merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SkippedGroupReason {
    Singleton,
    IncompatibleRequirements,
}

/// A group that was intentionally not merged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SkippedGroup<Key = usize> {
    pub key: Key,
    pub node_count: usize,
    pub reason: SkippedGroupReason,
}

/// Result of one alignment round.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AlignmentPlan<Id = usize, Key = usize, P = ChartGridData> {
    pub node_count: usize,
    pub groups: Vec<AlignedGroup<Id, Key, P>>,
    pub skipped: Vec<SkippedGroup<Key>>,
}

impl<Id, Key, P> AlignmentPlan<Id, Key, P> {
    /// Total number of distinct group keys seen, merged or skipped.
    pub(crate) fn group_count(&self) -> usize {
        self.groups.len() + self.skipped.len()
    }
}

/// Run one alignment round over measured nodes with a caller-supplied
/// payload. Nodes group by `group_key` in first-seen key order; each group
/// merges via `merge` (`None` marks the group incompatible) and `delta`
/// reports each member's `(content_delta, edge_delta)` against the merge.
pub(crate) fn align_by<Id, Key, P>(
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
pub(crate) struct RoundDeltas {
    pub content: f32,
    pub edge: f32,
}

impl RoundDeltas {
    pub(crate) fn total(self) -> f32 {
        self.content + self.edge
    }
}

/// Cross-round convergence evidence for drivers that alternate alignment
/// rounds with re-measurement.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ConvergenceTrace {
    rounds: Vec<RoundDeltas>,
}

impl ConvergenceTrace {
    pub(crate) fn record_round(&mut self, content: f32, edge: f32) {
        self.rounds.push(RoundDeltas { content, edge });
    }

    pub(crate) fn rounds(&self) -> &[RoundDeltas] {
        &self.rounds
    }

    /// The last recorded round had no deltas beyond `epsilon`.
    pub(crate) fn is_converged(&self, epsilon: f32) -> bool {
        self.rounds
            .last()
            .is_some_and(|round| round.total() <= epsilon)
    }

    /// Two or more consecutive rounds with non-decreasing, non-zero deltas:
    /// evidence the driver is cycling rather than converging.
    pub(crate) fn is_non_converging(&self, epsilon: f32) -> bool {
        self.rounds
            .windows(2)
            .any(|pair| pair[0].total() > epsilon && pair[1].total() >= pair[0].total())
    }
}

/// Merge compatible grid requirements by component-wise maximum. `None`
/// when the input is empty or any two requirements have different shapes.
pub(crate) fn merged_grid_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a ChartGridData>,
) -> Option<ChartGridData> {
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
pub(crate) fn grid_content_delta(local: &ChartGridData, merged: &ChartGridData) -> f32 {
    abs_delta_sum(&local.column_widths, &merged.column_widths)
        + abs_delta_sum(&local.row_heights, &merged.row_heights)
}

/// Total absolute edge/spacing difference against merged requirements.
pub(crate) fn grid_edge_delta(local: &ChartGridData, merged: &ChartGridData) -> f32 {
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

fn max_assign_edge_each(target: &mut [EdgeGrant], source: &[EdgeGrant]) {
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

fn abs_edge_delta_sum(local: &[EdgeGrant], merged: &[EdgeGrant]) -> f32 {
    debug_assert_eq!(local.len(), merged.len());
    local
        .iter()
        .zip(merged.iter())
        .map(|(local, merged)| (merged.total - local.total).abs())
        .sum()
}
