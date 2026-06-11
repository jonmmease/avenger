//! One facet coordination round lowered onto a share-keyed
//! `avenger_layout::Layout` and solved in a single pass (seam plan §A).
//!
//! Every requirement node lowers to a uniform grid of its REAL renderable
//! cells — each cell a leaf carrying its (guide, total) overflow envelope
//! as edge demands — with the node's local spacing on the main axis;
//! cousins (nodes with the same coordination scope) share one key, so a
//! single solve coordinates both channels:
//!
//! - **layout**: the solver's share coordination merges spacing
//!   (post-merge values read from `SolvedTracks`). Free-slot nodes share
//!   too: in the legacy merge free nodes always contributed to and received
//!   the group spacing, reverting only their slot count, which stays a
//!   write-back adjustment in `build_round_patches` alongside lane gap
//!   folds and global-edge outer reversion. Slot counts and the
//!   `guide_slot_gap_px` side-car merge chart-side from the group
//!   membership (the solver coordinates geometry, not chart policy
//!   scalars).
//! - **overflow**: per-cell demands fold per track and merge across
//!   cousins through the share patches. Each node's OWN envelope is the
//!   pre-merge view: guide from `requested.inner` (the layered law),
//!   total from `Region::geometric` (the raw law — guide-heavy and
//!   legend-heavy cells on one side do not lift each other within a
//!   band). The group-MERGED total then re-applies the cross-cousin
//!   layered lift (`max(guide) + max(legend)` may exceed `max(total)`:
//!   one cousin's guide layer and another's legend layer must coexist in
//!   the equalized band).
//!
//! The guide-anchor and boundary channels remain chart-side folds over
//! their own scopes (lanes split groups; boundary strips global edges per
//! node), in `coordination_plans`; they consume the tree-derived OWN
//! envelopes.

use std::collections::HashMap;

use avenger_chart_core::{
    CoordinatedLayout, CoordinatedOverflow, FacetAxis, OverflowSpaceRequirement,
};
use avenger_layout::{EdgeDemand, Layout, RegionDetail, Side, Size, SolveOptions, Spacing};

use crate::facet::coordination_plans::{CoordinationNodeKey, RequirementNodeSnapshot};
use crate::plot::compiled::{CoordinationKind, CoordinationScopeKey};

/// Solved values for one round, before the per-node write-back adjustments:
/// the group-merged layout per share key and per node, plus each node's
/// own (pre-merge) and group-equalized whole-band envelopes (only nodes
/// with cells).
pub(crate) struct SolvedRound {
    pub(crate) merged_by_key: HashMap<CoordinationScopeKey, CoordinatedLayout>,
    pub(crate) merged_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) own_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
}

/// Lower every node's uniform-track policy and measured overflow into one
/// share-keyed layout tree, solve it, and read back the group-merged
/// values per node.
pub(crate) fn solve_round(nodes: &[RequirementNodeSnapshot]) -> SolvedRound {
    if nodes.is_empty() {
        return SolvedRound {
            merged_by_key: HashMap::new(),
            merged_by_node: HashMap::new(),
            own_overflow_by_node: HashMap::new(),
            overflow_by_node: HashMap::new(),
        };
    }
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
            // The node's real renderable cells, each carrying its
            // (guide, total) overflow envelope as layered edge demands —
            // the same per-cell construction the band envelope fold used.
            // Spacing coordination is content-free, uniform share groups
            // tolerate ragged counts, and a cell-less node keeps one zero
            // placeholder leaf (the zero envelope).
            let cells = node.overflow_cells.as_deref().unwrap_or(&[]);
            let leaves = if cells.is_empty() {
                vec![Layout::leaf(Size::default())]
            } else {
                cells
                    .iter()
                    .map(|(guide, total)| {
                        let mut leaf: Layout<usize, CoordinationScopeKey> =
                            Layout::leaf(Size::default());
                        for (side, guide_value, total_value) in [
                            (Side::Top, guide.top, total.top),
                            (Side::Right, guide.right, total.right),
                            (Side::Bottom, guide.bottom, total.bottom),
                            (Side::Left, guide.left, total.left),
                        ] {
                            leaf = leaf.demand(
                                side,
                                EdgeDemand::from_inner_and_envelope(guide_value, total_value),
                            );
                        }
                        leaf
                    })
                    .collect::<Vec<_>>()
            };
            match node.axis {
                FacetAxis::Column => Layout::row(leaves)
                    .uniform_columns()
                    .column_spacing(spacing),
                FacetAxis::Row => Layout::column(leaves).uniform_rows().row_spacing(spacing),
            }
            .share(share_key.clone())
        })
        .collect::<Vec<_>>();

    // Members sit on the root grid's diagonal so every member is the only
    // occupant of its row and column: its granted edges from the root are
    // then exactly its own post-patch (group-merged) demands on all four
    // sides.
    let mut root: Layout<usize, CoordinationScopeKey> = Layout::grid(members.len(), members.len());
    for (index, member) in members.into_iter().enumerate() {
        root = root.cell(index, index, member);
    }
    let solved = root
        .solve(&SolveOptions::default())
        .expect("a uniform placeholder round always solves");

    // Per-side envelope reads. OWN: guide from the layered pre-merge
    // demand, total from the raw geometric view (the within-band law).
    let own_envelope = |index: usize| -> CoordinatedOverflow {
        let region = solved.at_path(&[index]).expect("member region");
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: region.requested.top.inner,
                right: region.requested.right.inner,
                bottom: region.requested.bottom.inner,
                left: region.requested.left.inner,
            },
            total: OverflowSpaceRequirement {
                top: region.geometric.top,
                right: region.geometric.right,
                bottom: region.geometric.bottom,
                left: region.geometric.left,
            },
        }
    };
    // MERGED: guide from the post-patch granted demand; total re-applies
    // the cross-cousin layered lift over the group's own envelopes —
    // max(total) raised to max(guide) + max(legend), because one cousin's
    // guide layer and another's legend layer must coexist in the
    // equalized band.
    let merged_total = |group: &[usize], side: fn(&OverflowSpaceRequirement) -> f32| -> f32 {
        let mut max_total = 0.0f32;
        let mut max_guide = 0.0f32;
        let mut max_legend = 0.0f32;
        for &member in group {
            let own = own_envelope(member);
            let guide = side(&own.guide);
            let total = side(&own.total);
            max_total = max_total.max(total);
            max_guide = max_guide.max(guide);
            max_legend = max_legend.max((total - guide).max(0.0));
        }
        max_total.max(max_guide + max_legend)
    };

    let mut merged_by_key = HashMap::new();
    let mut merged_by_node = HashMap::new();
    let mut own_overflow_by_node = HashMap::new();
    let mut overflow_by_node = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let region = solved
            .at_path(&[index])
            .expect("every lowered node has a region");
        let RegionDetail::Grid { tracks } = &region.detail else {
            unreachable!("lowered nodes are grids");
        };
        // Cell-less nodes contribute nothing here; they still receive the
        // group value through the keyed patch lookup downstream, as in the
        // legacy merge.
        if node.overflow_cells.is_some() {
            let group = &group_members[&share_keys[index]];
            let members_with_cells = group
                .iter()
                .copied()
                .filter(|&member| nodes[member].overflow_cells.is_some())
                .collect::<Vec<_>>();
            own_overflow_by_node.insert(node.node_id.clone(), own_envelope(index));
            overflow_by_node.insert(
                node.node_id.clone(),
                CoordinatedOverflow {
                    guide: OverflowSpaceRequirement {
                        top: region.granted.top.inner,
                        right: region.granted.right.inner,
                        bottom: region.granted.bottom.inner,
                        left: region.granted.left.inner,
                    },
                    total: OverflowSpaceRequirement {
                        top: merged_total(&members_with_cells, |sides| sides.top),
                        right: merged_total(&members_with_cells, |sides| sides.right),
                        bottom: merged_total(&members_with_cells, |sides| sides.bottom),
                        left: merged_total(&members_with_cells, |sides| sides.left),
                    },
                },
            );
        }
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

    SolvedRound {
        merged_by_key,
        merged_by_node,
        own_overflow_by_node,
        overflow_by_node,
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
    ) -> RequirementNodeSnapshot {
        RequirementNodeSnapshot {
            node_id: CoordinationNodeKey::new(path),
            key,
            axis: FacetAxis::Column,
            slot_sharing: SharingLevel::default(),
            min_slot_count: 0,
            overflow_cells: None,
            local_layout: layout,
            guide_padding_inner_px: 0.0,
            first_edge_index: 0,
            last_edge_index: 0,
        }
    }

    fn side(value: f32) -> OverflowSpaceRequirement {
        OverflowSpaceRequirement {
            top: value,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    /// The two envelope laws: within a band, a guide-heavy and a
    /// legend-heavy cell on one side do not lift each other (geometric
    /// total); across cousins, one band's guide layer and another's legend
    /// layer must coexist (lifted total).
    #[test]
    fn envelope_laws_within_band_geometric_across_cousins_lifted() {
        let cousins = CoordinationScopeKey::container_group(
            crate::plot::compiled::CoordinationKind::ChildSize,
            1,
            "col:team",
        );
        let lone = CoordinationScopeKey::container_group(
            crate::plot::compiled::CoordinationKind::ChildSize,
            1,
            "col:year",
        );
        // Cousin A is guide-heavy (top guide 5), cousin B legend-heavy
        // (top legend 8); the lone band holds both kinds of cell itself.
        let mut guide_heavy = node(vec![0], cousins.clone(), CoordinatedLayout::default());
        guide_heavy.overflow_cells = Some(vec![(side(5.0), side(5.0))]);
        let mut legend_heavy = node(vec![1], cousins, CoordinatedLayout::default());
        legend_heavy.overflow_cells = Some(vec![(side(0.0), side(8.0))]);
        let mut mixed_cells = node(vec![2], lone, CoordinatedLayout::default());
        mixed_cells.overflow_cells = Some(vec![(side(5.0), side(5.0)), (side(0.0), side(8.0))]);

        let solved = solve_round(&[guide_heavy, legend_heavy, mixed_cells]);
        let own =
            |path: Vec<usize>| solved.own_overflow_by_node[&CoordinationNodeKey::new(path)].clone();
        let merged =
            |path: Vec<usize>| solved.overflow_by_node[&CoordinationNodeKey::new(path)].clone();

        // Within-band law: the lone band's total is the geometric max (8),
        // not the lifted 5 + 8.
        assert_eq!(own(vec![2]).guide.top, 5.0);
        assert_eq!(own(vec![2]).total.top, 8.0);
        assert_eq!(merged(vec![2]).total.top, 8.0, "singleton merge is own");

        // Cross-cousin law: the equalized band must hold A's guide layer
        // AND B's legend layer.
        assert_eq!(own(vec![0]).total.top, 5.0);
        assert_eq!(own(vec![1]).total.top, 8.0);
        assert_eq!(merged(vec![0]).guide.top, 5.0);
        assert_eq!(merged(vec![0]).total.top, 13.0, "cross-cousin lift");
        assert_eq!(merged(vec![1]).total.top, 13.0);
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

        let solved = solve_round(&nodes);
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
