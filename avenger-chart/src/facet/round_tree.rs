//! One facet coordination round lowered onto a share-keyed
//! `avenger_layout::Layout` and solved in a single pass (seam plan §A).
//!
//! Phase scope (P2): the facet-band LAYOUT channel. Every requirement node
//! lowers to a uniform grid of placeholder tracks carrying its local
//! spacing, and cousins — nodes with the same coordination scope — share
//! one key, so the solver's share coordination merges their spacing.
//! Free-slot nodes share too: in the legacy merge free nodes always
//! contributed to and received the group spacing, reverting only their slot
//! count, which stays a write-back adjustment in `build_round_patches`
//! alongside lane gap folds and global-edge outer reversion. Slot counts
//! and the `guide_slot_gap_px` side-car merge chart-side from the group
//! membership (the solver coordinates geometry, not chart policy scalars).
//!
//! Overflow channels (full, guide-anchor, boundary) stay on the legacy
//! merge until P3.

use std::collections::HashMap;

use avenger_chart_core::{CoordinatedLayout, FacetAxis};
use avenger_layout::{Layout, RegionDetail, Size, SolveOptions, Spacing};

use crate::facet::coordination_plans::{CoordinationNodeKey, RoundCollectionInput};
use crate::plot::compiled::{CoordinationKind, CoordinationScopeKey};

/// Solved layout values for one round: the group-merged view per share key
/// and per node, before the per-node write-back adjustments.
pub(crate) struct SolvedLayoutRound {
    pub(crate) merged_by_key: HashMap<CoordinationScopeKey, CoordinatedLayout>,
    pub(crate) merged_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

/// Lower every node's uniform-track policy into one share-keyed layout
/// tree, solve it, and read back the group-merged layout per node.
pub(crate) fn solve_layout_round(nodes: &[RoundCollectionInput]) -> SolvedLayoutRound {
    let mut group_members: HashMap<CoordinationScopeKey, Vec<usize>> = HashMap::new();
    let mut share_keys = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let share_key = node.key.with_kind(CoordinationKind::ChildSize);
        group_members
            .entry(share_key.clone())
            .or_default()
            .push(index);
        share_keys.push(share_key);
    }

    let members = nodes
        .iter()
        .zip(&share_keys)
        .map(|(node, share_key)| {
            let spacing = Spacing {
                outer_start: node.local_layout.outer_start,
                outer_end: node.local_layout.outer_end,
                min_gap: node.local_layout.padding_inner_px,
            };
            // Placeholder tracks: spacing coordination is content-free, and
            // uniform share groups tolerate ragged counts.
            let leaves = (0..node.local_layout.n.max(1))
                .map(|_| Layout::leaf(Size::default()))
                .collect::<Vec<_>>();
            match node.axis {
                FacetAxis::Column => Layout::row(leaves)
                    .uniform_columns()
                    .column_spacing(spacing),
                FacetAxis::Row => Layout::column(leaves).uniform_rows().row_spacing(spacing),
            }
            .share(share_key.clone())
        })
        .collect::<Vec<_>>();

    let root: Layout<usize, CoordinationScopeKey> = Layout::row(members);
    let solved = root
        .solve(&SolveOptions::default())
        .expect("a uniform placeholder round always solves");

    let mut merged_by_key = HashMap::new();
    let mut merged_by_node = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let region = solved
            .at_path(&[index])
            .expect("every lowered node has a region");
        let RegionDetail::Grid { tracks } = &region.detail else {
            unreachable!("lowered nodes are grids");
        };
        let solved_spacing = match node.axis {
            FacetAxis::Column => tracks.column_spacing,
            FacetAxis::Row => tracks.row_spacing,
        };
        // Chart policy scalars merge over the share group's membership.
        let group = &group_members[&share_keys[index]];
        let n = group
            .iter()
            .map(|&member| nodes[member].local_layout.n)
            .max()
            .unwrap_or(node.local_layout.n);
        let guide_slot_gap_px = group
            .iter()
            .map(|&member| nodes[member].local_layout.guide_slot_gap_px)
            .fold(0.0f32, f32::max);

        let merged = CoordinatedLayout {
            padding_inner_px: solved_spacing.min_gap,
            guide_slot_gap_px,
            outer_start: solved_spacing.outer_start,
            outer_end: solved_spacing.outer_end,
            n,
        };
        merged_by_key.insert(share_keys[index].clone(), merged.clone());
        merged_by_node.insert(node.node_id.clone(), merged);
    }

    SolvedLayoutRound {
        merged_by_key,
        merged_by_node,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::SharingLevel;

    fn node(
        path: Vec<usize>,
        key: CoordinationScopeKey,
        layout: CoordinatedLayout,
    ) -> RoundCollectionInput {
        RoundCollectionInput {
            node_id: CoordinationNodeKey::new(path),
            key,
            axis: FacetAxis::Column,
            slot_sharing: SharingLevel::default(),
            min_slot_count: 0,
            measured_overflow: None,
            local_layout: layout,
            guide_padding_inner_px: 0.0,
            first_edge_index: 0,
            last_edge_index: 0,
        }
    }

    /// Cousins merge spacing through the solver's share coordination;
    /// slot counts and the guide-gap side-car fold chart-side; nodes in a
    /// different scope keep their own values.
    #[test]
    fn round_solve_merges_cousins_and_keeps_scopes_separate() {
        let cousins = CoordinationScopeKey::container_group(
            crate::plot::compiled::CoordinationKind::ChildSize,
            1,
            "col:team",
        );
        let other = CoordinationScopeKey::container_group(
            crate::plot::compiled::CoordinationKind::ChildSize,
            1,
            "col:year",
        );
        let nodes = vec![
            node(
                vec![0],
                cousins.clone(),
                CoordinatedLayout {
                    n: 2,
                    padding_inner_px: 14.0,
                    guide_slot_gap_px: 6.0,
                    outer_start: 4.0,
                    outer_end: 0.0,
                },
            ),
            node(
                vec![1],
                cousins,
                CoordinatedLayout {
                    n: 3,
                    padding_inner_px: 8.0,
                    guide_slot_gap_px: 11.0,
                    outer_start: 0.0,
                    outer_end: 9.0,
                },
            ),
            node(
                vec![2],
                other,
                CoordinatedLayout {
                    n: 1,
                    padding_inner_px: 5.0,
                    guide_slot_gap_px: 2.0,
                    outer_start: 1.0,
                    outer_end: 1.0,
                },
            ),
        ];

        let solved = solve_layout_round(&nodes);
        let merged = |path: Vec<usize>| {
            solved
                .merged_by_node
                .get(&CoordinationNodeKey::new(path))
                .cloned()
                .expect("merged layout for node")
        };

        let assert_layout = |actual: CoordinatedLayout, expected: CoordinatedLayout| {
            assert_eq!(actual.n, expected.n);
            assert_eq!(actual.padding_inner_px, expected.padding_inner_px);
            assert_eq!(actual.guide_slot_gap_px, expected.guide_slot_gap_px);
            assert_eq!(actual.outer_start, expected.outer_start);
            assert_eq!(actual.outer_end, expected.outer_end);
        };

        let expected = CoordinatedLayout {
            n: 3,
            padding_inner_px: 14.0,
            guide_slot_gap_px: 11.0,
            outer_start: 4.0,
            outer_end: 9.0,
        };
        assert_layout(merged(vec![0]), expected.clone());
        assert_layout(merged(vec![1]), expected);
        assert_layout(
            merged(vec![2]),
            CoordinatedLayout {
                n: 1,
                padding_inner_px: 5.0,
                guide_slot_gap_px: 2.0,
                outer_start: 1.0,
                outer_end: 1.0,
            },
        );
    }
}
