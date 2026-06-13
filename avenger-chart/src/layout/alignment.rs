//! Chart-owned requirement alignment: grouping, merging, and deltas for
//! coordinating equivalent layout requirements measured independently.
//!
//! This engine plans the child-frame group solves and reports on them.
//! `align_by` groups container requirements by alignment key (the merge
//! closures carry chart payloads the solver does not model — the
//! guide-gap side-car), the resulting plan selects which cousins enter
//! one concat layout group solve, and the local-vs-merged deltas feed the
//! coordination diagnostics. The merged values applied to containers come
//! from that group solve — `Layout::solve`'s share-key coordination —
//! while this engine's fold serves planning and delta reporting.
//!
//! A round is ONE pure pass: callers own identity, traversal, group keys,
//! and application.

use std::collections::HashMap;
use std::hash::Hash;

use avenger_layout::GridRequirements;

/// One measured node participating in a single alignment round.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AlignmentNode<Id = usize, Key = usize, P = GridRequirements> {
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
pub(crate) struct AlignedGroup<Id = usize, Key = usize, P = GridRequirements> {
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
pub(crate) struct AlignmentPlan<Id = usize, Key = usize, P = GridRequirements> {
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

/// Per-round delta totals recorded by the coordination driver.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RoundDeltas {
    pub content: f32,
    pub edge: f32,
}
