//! Immutable plans for facet requirement coordination and realization.
//!
//! The subtle part of facet layout is scope selection. Requirement snapshots use
//! semantic `CoordinationScopeKey` values for the behavior being coordinated,
//! while `CoordinationNodeKey` remains only a plan-local traversal address.
//! Full overflows, boundary overflows, and child-size/layout requirements derive
//! kinded keys from each node's semantic scope. Sibling-boundary overflows strip
//! globally outer edges before coordinating spacing between siblings, and guide
//! anchors coordinate within branch-local lanes so jagged facet trees can align
//! visible guides without borrowing space from unrelated branches.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::Arc,
};

use avenger_chart_core::{
    AvengerChartError, AxisPosition, CoordinatedLayout, CoordinatedOverflow, CoordinationAxis,
    FacetAxis, FacetEmptyCellPolicy, OverflowSpaceRequirement, SharingLevel,
};

use crate::{
    facet::coordination_solution::CoordinationSolution,
    facet::overflow_projection::{FacetOverflowProjection, project_facet_overflow},
    facet::overflow_projection::{overflow_edge_demands, overflow_from_edge_demands},
    layout::{EdgeGrant, Edges, RoundDeltas},
    plot::compiled::{CoordinationKind, CoordinationScopeKey},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoordinationNodeKey {
    pub(crate) path: Vec<usize>,
}

impl CoordinationNodeKey {
    pub(crate) fn new(path: Vec<usize>) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationScopeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) slot_sharing: SharingLevel,
    pub(crate) min_slot_count: usize,
    /// The renderable cells' (guide, total) overflow envelopes, lowered
    /// directly into the round solve. `None` for cell-less bands.
    pub(crate) overflow_cells: Option<Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) guide_padding_inner_px: f32,
    pub(crate) first_edge_index: usize,
    pub(crate) last_edge_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequirementSnapshot {
    pub(crate) nodes: Vec<RequirementNodeSnapshot>,
}

/// Per-channel group counts of one round, for driver logs and tests (the
/// per-key merged maps themselves are internal to solution construction).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RoundDiagnostics {
    pub(crate) overflow_groups: usize,
    pub(crate) boundary_overflow_groups: usize,
    pub(crate) layout_groups: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct RequirementPass {
    pub(crate) snapshot: RequirementSnapshot,
    /// The round's product: post-adjustment per-node channel values. The
    /// apply walk installs this Arc on every band.
    pub(crate) solution: Arc<CoordinationSolution>,
    /// Total layout deltas of this round (local vs merged), for cross-round
    /// convergence diagnostics.
    pub(crate) layout_round_deltas: RoundDeltas,
    /// Total overflow deltas of this round across the overflow, guide-anchor,
    /// and boundary groupings.
    pub(crate) overflow_round_deltas: RoundDeltas,
    pub(crate) diagnostics: RoundDiagnostics,
}

#[derive(Debug, Clone)]
pub(crate) struct AxisSlab {
    pub(crate) start: f32,
    pub(crate) end: f32,
}

impl AxisSlab {
    pub(crate) fn total(&self) -> f32 {
        self.start + self.end
    }

    pub(crate) fn has_slab(&self) -> bool {
        self.start > 0.0 || self.end > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlotAreaSize {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl PlotAreaSize {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FacetOwnershipRequirement {
    pub(crate) has_holes: bool,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) axis_owner_ignore_empty_cells: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodeRequirements {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) child_count: usize,
    pub(crate) coordinated_overflow: CoordinatedOverflow,
    pub(crate) coordinated_layout: Option<CoordinatedLayout>,
    pub(crate) layout_changed: bool,
    pub(crate) legend_main_axis_slab: AxisSlab,
    pub(crate) has_legend_overflow: bool,
    pub(crate) ownership: FacetOwnershipRequirement,
    pub(crate) child_plot_areas: Vec<PlotAreaSize>,
    pub(crate) target_subplot_cross_size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BandRetargetAction {
    Preserve,
    ApplyCoordinatedLayout,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PlotAreaTarget {
    pub(crate) width: Option<f32>,
    pub(crate) height: Option<f32>,
}

impl PlotAreaTarget {
    pub(crate) fn has_any_target(&self) -> bool {
        self.width.is_some() || self.height.is_some()
    }

    pub(crate) fn resolve(self, current: PlotAreaSize) -> PlotAreaSize {
        PlotAreaSize::new(
            self.width.unwrap_or(current.width).max(1.0),
            self.height.unwrap_or(current.height).max(1.0),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CellRetargetAction {
    pub(crate) plot_area_target: Option<PlotAreaTarget>,
}

impl CellRetargetAction {
    pub(crate) fn preserve() -> Self {
        Self {
            plot_area_target: None,
        }
    }

    pub(crate) fn retarget_plot_area(plot_area_target: PlotAreaTarget) -> Self {
        debug_assert!(plot_area_target.has_any_target());
        Self {
            plot_area_target: Some(plot_area_target),
        }
    }

    pub(crate) fn retargets_plot_area(&self) -> bool {
        self.plot_area_target.is_some()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CellRetargetActionCounts {
    pub(crate) preserve: usize,
    pub(crate) retarget_plot_area: usize,
    pub(crate) width_targets: usize,
    pub(crate) height_targets: usize,
}

impl CellRetargetActionCounts {
    pub(crate) fn from_actions(actions: &[CellRetargetAction]) -> Self {
        let mut counts = Self::default();
        for action in actions {
            match action.plot_area_target {
                None => counts.preserve += 1,
                Some(target) => {
                    counts.retarget_plot_area += 1;
                    counts.width_targets += usize::from(target.width.is_some());
                    counts.height_targets += usize::from(target.height.is_some());
                }
            }
        }
        counts
    }

    #[cfg(test)]
    pub(crate) fn plot_area_retarget_count(&self) -> usize {
        self.retarget_plot_area
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodeActions {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) band_action: BandRetargetAction,
    pub(crate) child_actions: Vec<CellRetargetAction>,
}

impl RetargetNodeActions {
    pub(crate) fn child_action_counts(&self) -> CellRetargetActionCounts {
        CellRetargetActionCounts::from_actions(&self.child_actions)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodePlan {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) requirements: RetargetNodeRequirements,
    pub(crate) actions: RetargetNodeActions,
}

/// Per-node summary of one applied retarget decision. `node_id`, `axis`,
/// and the decision counts exist for test assertions; the rest feeds the
/// driver's run-level logging.
#[derive(Debug, Clone)]
pub(crate) struct RetargetNodeTrace {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) node_id: CoordinationNodeKey,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) axis: FacetAxis,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) planned_child_action_counts: CellRetargetActionCounts,
    pub(crate) parent_cross_size_propagated: bool,
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) band_layout_applied: bool,
    pub(crate) plot_area_retarget_count: usize,
    pub(crate) width_retarget_count: usize,
    pub(crate) height_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetNodeOutcome {
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) band_layout_applied: bool,
    pub(crate) plot_area_retarget_count: usize,
    pub(crate) width_retarget_count: usize,
    pub(crate) height_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetTrace {
    pub(crate) node_results: Vec<RetargetNodeTrace>,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationChildPlan {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) child_index: usize,
    pub(crate) target_plot_area_width: Option<f32>,
    pub(crate) target_plot_area_height: Option<f32>,
    pub(crate) target_band_range_end: Option<f32>,
    pub(crate) adjust_plot_area: bool,
    pub(crate) update_band_range: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationNodePlan {
    pub(crate) node_id: CoordinationNodeKey,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) axis: FacetAxis,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) parent_cross_size_target: Option<f32>,
    pub(crate) child_count: usize,
    pub(crate) child_plans: Vec<FinalPropagationChildPlan>,
    pub(crate) expected_plot_area_adjustments_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationNodeTrace {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) node_id: CoordinationNodeKey,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) planned_child_count: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) planned_child_plan_count: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) planned_plot_area_adjustments_count: usize,
    pub(crate) child_plot_area_adjustments_count: usize,
    pub(crate) scale_range_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPropagationTrace {
    pub(crate) node_results: Vec<FinalPropagationNodeTrace>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordinationRunArtifacts {
    pub(crate) initial_requirement_pass: RequirementPass,
    pub(crate) retarget_trace: RetargetTrace,
    pub(crate) retargeted_requirement_pass: RequirementPass,
    pub(crate) final_propagation_trace: FinalPropagationTrace,
}

#[derive(Debug, Clone)]
struct RequirementScopeMetadata {
    axis_by_path: HashMap<Vec<usize>, FacetAxis>,
    edge_indices_by_path: HashMap<Vec<usize>, (usize, usize)>,
    axis_lane_scope_by_node: HashMap<CoordinationNodeKey, CoordinationScopeKey>,
}

type OverflowEntry = (
    CoordinationNodeKey,
    CoordinationScopeKey,
    CoordinatedOverflow,
);

#[derive(Debug, Clone, Default)]
struct RoundGroupedRequirements {
    guide_anchor_entries: Vec<OverflowEntry>,
    boundary_entries: Vec<OverflowEntry>,
    max_guide_slot_gap_by_axis_group: HashMap<CoordinationScopeKey, f32>,
}

impl RequirementScopeMetadata {
    fn collect(nodes: &[RequirementNodeSnapshot]) -> Self {
        let axis_by_path = nodes
            .iter()
            .map(|node| (node.node_id.path.clone(), node.axis))
            .collect::<HashMap<_, _>>();
        let edge_indices_by_path = nodes
            .iter()
            .map(|node| {
                (
                    node.node_id.path.clone(),
                    (node.first_edge_index, node.last_edge_index),
                )
            })
            .collect::<HashMap<_, _>>();
        let axis_lane_scope_by_node = Self::axis_lane_scope_keys(nodes, &axis_by_path);

        Self {
            axis_by_path,
            edge_indices_by_path,
            axis_lane_scope_by_node,
        }
    }

    fn axis_lane_scope_keys(
        nodes: &[RequirementNodeSnapshot],
        axis_by_path: &HashMap<Vec<usize>, FacetAxis>,
    ) -> HashMap<CoordinationNodeKey, CoordinationScopeKey> {
        // Share guide-only gutters through uninterrupted same-axis facet chains
        // (`column -> column`, `row -> row`). Orthogonal facets start a new lane.
        let mut node_refs = nodes.iter().collect::<Vec<_>>();
        node_refs.sort_by_key(|node| node.node_id.path.len());

        let mut group_by_path: HashMap<Vec<usize>, CoordinationScopeKey> = HashMap::new();
        let mut group_by_node: HashMap<CoordinationNodeKey, CoordinationScopeKey> = HashMap::new();

        for node in node_refs {
            let parent_group = node.node_id.path.split_last().and_then(|(_, parent_path)| {
                let parent_path = parent_path.to_vec();
                let parent_axis = axis_by_path.get(&parent_path).copied()?;
                if parent_axis == node.axis {
                    group_by_path.get(&parent_path).cloned()
                } else {
                    None
                }
            });
            let group = parent_group.unwrap_or_else(|| {
                CoordinationScopeKey::container_lane(
                    CoordinationKind::GuideLane,
                    coordination_axis_from_facet_axis(node.axis),
                    node.node_id.path.clone(),
                )
            });

            group_by_path.insert(node.node_id.path.clone(), group.clone());
            group_by_node.insert(node.node_id.clone(), group);
        }

        group_by_node
    }

    fn guide_anchor_scope_key(&self, node: &RequirementNodeSnapshot) -> CoordinationScopeKey {
        let mut lane_path = Vec::new();
        let mut parent_path = Vec::new();

        for idx in &node.node_id.path {
            if let Some(parent_axis) = self.axis_by_path.get(&parent_path)
                && *parent_axis != node.axis
            {
                lane_path.push(*idx);
            }
            parent_path.push(*idx);
        }

        CoordinationScopeKey::lane_from_scope(CoordinationKind::GuideAnchor, &node.key, lane_path)
    }

    fn is_global_edge_side(&self, node: &RequirementNodeSnapshot, side: AxisPosition) -> bool {
        let axis = side_axis(side);
        let is_start = side_is_start(side);
        let mut governed_by_axis = node.axis == axis;
        let mut parent_path = Vec::new();

        for idx in &node.node_id.path {
            let Some(parent_axis) = self.axis_by_path.get(&parent_path) else {
                return false;
            };

            if *parent_axis == axis {
                governed_by_axis = true;
                let Some((first_edge_index, last_edge_index)) =
                    self.edge_indices_by_path.get(&parent_path).copied()
                else {
                    return false;
                };
                let expected_edge = if is_start {
                    first_edge_index
                } else {
                    last_edge_index
                };
                if *idx != expected_edge {
                    return false;
                }
            }

            parent_path.push(*idx);
        }

        governed_by_axis
    }

    fn sibling_boundary_overflow(
        &self,
        node: &RequirementNodeSnapshot,
        overflow: &CoordinatedOverflow,
    ) -> CoordinatedOverflow {
        let mut boundary = overflow.clone();
        for side in [
            AxisPosition::Top,
            AxisPosition::Right,
            AxisPosition::Bottom,
            AxisPosition::Left,
        ] {
            if self.is_global_edge_side(node, side) {
                set_overflow_side(&mut boundary.guide, side, 0.0);
                set_overflow_side(&mut boundary.total, side, 0.0);
            }
        }
        boundary
    }
}

fn guide_anchor_overflow_for_node(
    node: &RequirementNodeSnapshot,
    overflow: &CoordinatedOverflow,
) -> CoordinatedOverflow {
    project_facet_overflow(
        overflow,
        FacetOverflowProjection::GuideAnchor { axis: node.axis },
    )
}

fn side_axis(side: AxisPosition) -> FacetAxis {
    match side {
        AxisPosition::Left | AxisPosition::Right => FacetAxis::Column,
        AxisPosition::Top | AxisPosition::Bottom => FacetAxis::Row,
    }
}

fn side_is_start(side: AxisPosition) -> bool {
    matches!(side, AxisPosition::Left | AxisPosition::Top)
}

fn set_overflow_side(overflow: &mut OverflowSpaceRequirement, side: AxisPosition, value: f32) {
    match side {
        AxisPosition::Top => overflow.top = value,
        AxisPosition::Right => overflow.right = value,
        AxisPosition::Bottom => overflow.bottom = value,
        AxisPosition::Left => overflow.left = value,
    }
}

/// Test-fixture entry: build a pass from a snapshot alone, deriving the
/// round with [`test_solved_round`]'s plain group-max folds. Production
/// rounds come from the real-tree solve
/// (`tree_solve::tree_solved_round`) via
/// [`build_requirement_pass_with_round`].
#[cfg(test)]
pub(crate) fn build_requirement_pass(
    snapshot: RequirementSnapshot,
) -> Result<RequirementPass, AvengerChartError> {
    let solved = test_solved_round(&snapshot.nodes);
    build_requirement_pass_with_round(snapshot, solved)
}

/// FIXTURE FOLD (tests only): a `SolvedRound` from plain group-max merges
/// over snapshot nodes — the merge laws the production solve realizes
/// through `avenger_layout` share coordination, computed directly. Layout
/// scalars merge by max over the share group; each node's own envelope
/// follows the within-band law (main-axis edges from the first/last
/// renderable cell, cross edges by max, totals geometric); the merged
/// overflow applies the cross-cousin lift (`max(total)` raised to
/// `max(guide) + max(legend)`).
#[cfg(test)]
pub(crate) fn test_solved_round(
    nodes: &[RequirementNodeSnapshot],
) -> crate::facet::tree_solve::SolvedRound {
    use crate::plot::compiled::CoordinationKind;

    let mut groups: HashMap<CoordinationScopeKey, Vec<usize>> = HashMap::new();
    let mut share_keys = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let key = node.key.with_kind(CoordinationKind::ChildSize);
        groups.entry(key.clone()).or_default().push(index);
        share_keys.push(key);
    }

    let own_envelope = |node: &RequirementNodeSnapshot| -> Option<CoordinatedOverflow> {
        let cells = node.overflow_cells.as_deref()?;
        if cells.is_empty() {
            return Some(CoordinatedOverflow::default());
        }
        let first = node.first_edge_index.min(cells.len() - 1);
        let last = node.last_edge_index.min(cells.len() - 1);
        let max_side = |pick: fn(&OverflowSpaceRequirement) -> f32, guide: bool| -> f32 {
            cells
                .iter()
                .map(|(g, t)| pick(if guide { g } else { t }))
                .fold(0.0f32, f32::max)
        };
        let (guide, total) = match node.axis {
            FacetAxis::Column => (
                OverflowSpaceRequirement {
                    top: max_side(|s| s.top, true),
                    bottom: max_side(|s| s.bottom, true),
                    left: cells[first].0.left,
                    right: cells[last].0.right,
                },
                OverflowSpaceRequirement {
                    top: max_side(|s| s.top, false),
                    bottom: max_side(|s| s.bottom, false),
                    left: cells[first].1.left,
                    right: cells[last].1.right,
                },
            ),
            FacetAxis::Row => (
                OverflowSpaceRequirement {
                    left: max_side(|s| s.left, true),
                    right: max_side(|s| s.right, true),
                    top: cells[first].0.top,
                    bottom: cells[last].0.bottom,
                },
                OverflowSpaceRequirement {
                    left: max_side(|s| s.left, false),
                    right: max_side(|s| s.right, false),
                    top: cells[first].1.top,
                    bottom: cells[last].1.bottom,
                },
            ),
        };
        Some(CoordinatedOverflow { guide, total })
    };

    let mut merged_by_key = HashMap::new();
    let mut merged_by_node = HashMap::new();
    let mut own_overflow_by_node = HashMap::new();
    let mut overflow_by_node = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let group = &groups[&share_keys[index]];
        let mut merged = CoordinatedLayout::default();
        for &member in group {
            let local = &nodes[member].local_layout;
            merged.padding_inner_px = merged.padding_inner_px.max(local.padding_inner_px);
            merged.guide_slot_gap_px = merged.guide_slot_gap_px.max(local.guide_slot_gap_px);
            merged.outer_start = merged.outer_start.max(local.outer_start);
            merged.outer_end = merged.outer_end.max(local.outer_end);
            merged.n = merged.n.max(local.n);
        }
        merged_by_key.insert(share_keys[index].clone(), merged.clone());
        merged_by_node.insert(node.node_id.clone(), merged);

        let Some(own) = own_envelope(node) else {
            continue;
        };
        let members_with_cells = group
            .iter()
            .copied()
            .filter(|&member| nodes[member].overflow_cells.is_some())
            .collect::<Vec<_>>();
        let merged_side = |side: fn(&OverflowSpaceRequirement) -> f32| -> (f32, f32) {
            let mut max_total = 0.0f32;
            let mut max_guide = 0.0f32;
            let mut max_legend = 0.0f32;
            for &member in &members_with_cells {
                let member_own = own_envelope(&nodes[member]).unwrap_or_default();
                let guide = side(&member_own.guide);
                let total = side(&member_own.total);
                max_total = max_total.max(total);
                max_guide = max_guide.max(guide);
                max_legend = max_legend.max((total - guide).max(0.0));
            }
            (max_guide, max_total.max(max_guide + max_legend))
        };
        let (guide_top, total_top) = merged_side(|s| s.top);
        let (guide_right, total_right) = merged_side(|s| s.right);
        let (guide_bottom, total_bottom) = merged_side(|s| s.bottom);
        let (guide_left, total_left) = merged_side(|s| s.left);
        overflow_by_node.insert(
            node.node_id.clone(),
            CoordinatedOverflow {
                guide: OverflowSpaceRequirement {
                    top: guide_top,
                    right: guide_right,
                    bottom: guide_bottom,
                    left: guide_left,
                },
                total: OverflowSpaceRequirement {
                    top: total_top,
                    right: total_right,
                    bottom: total_bottom,
                    left: total_left,
                },
            },
        );
        own_overflow_by_node.insert(node.node_id.clone(), own);
    }

    crate::facet::tree_solve::SolvedRound {
        merged_by_key,
        merged_by_node,
        own_overflow_by_node,
        overflow_by_node,
    }
}

/// Build one requirement pass from a solved round (production rounds come
/// from the real-tree solve, `tree_solve::tree_solved_round`); chart-side
/// folds, write-back adjustments, and solution construction live here,
/// with solution coverage validated at construction.
pub(crate) fn build_requirement_pass_with_round(
    snapshot: RequirementSnapshot,
    solved: crate::facet::tree_solve::SolvedRound,
) -> Result<RequirementPass, AvengerChartError> {
    let nodes = &snapshot.nodes;
    let scopes = RequirementScopeMetadata::collect(nodes);

    let grouped = collect_round_groups(nodes, &scopes, &solved.own_overflow_by_node);

    // Layout channel: solver-merged spacing plus chart-side scalars.
    let layout_round_deltas = nodes
        .iter()
        .filter_map(|node| {
            solved
                .merged_by_node
                .get(&node.node_id)
                .map(|merged| coordinated_layout_delta(&node.local_layout, merged))
        })
        .fold(RoundDeltas::default(), |deltas, (content, edge)| {
            RoundDeltas {
                content: deltas.content + content,
                edge: deltas.edge + edge,
            }
        });

    // Full-overflow channel: each measured node's group-equalized envelope
    // from the solved round (the keyed map hands the value to overflow-less
    // cousins through the solution lookup).
    let mut overflow_by_key = HashMap::new();
    let mut full_overflow_deltas = RoundDeltas::default();
    for node in nodes {
        let Some(own) = solved.own_overflow_by_node.get(&node.node_id) else {
            continue;
        };
        let merged_overflow = solved
            .overflow_by_node
            .get(&node.node_id)
            .expect("every measured node has a solved overflow");
        let (content, edge) = coordinated_overflow_delta(own, merged_overflow);
        full_overflow_deltas.content += content;
        full_overflow_deltas.edge += edge;
        overflow_by_key.insert(
            node.key.with_kind(CoordinationKind::OverflowResidual),
            merged_overflow.clone(),
        );
    }

    // Guide-anchor and boundary channels: chart-side folds over their own
    // scopes (lanes split groups; boundary strips global edges per node).
    let (guide_anchor_overflow_by_key, guide_anchor_deltas) =
        fold_overflow_entries(&grouped.guide_anchor_entries);
    let (boundary_overflow_by_key, boundary_deltas) =
        fold_overflow_entries(&grouped.boundary_entries);

    let overflow_round_deltas = RoundDeltas {
        content: full_overflow_deltas.content
            + guide_anchor_deltas.content
            + boundary_deltas.content,
        edge: full_overflow_deltas.edge + guide_anchor_deltas.edge + boundary_deltas.edge,
    };
    let diagnostics = RoundDiagnostics {
        overflow_groups: overflow_by_key.len(),
        boundary_overflow_groups: boundary_overflow_by_key.len(),
        layout_groups: solved.merged_by_key.len(),
    };

    let solution = build_round_solution(
        nodes,
        &scopes,
        &grouped,
        &overflow_by_key,
        &guide_anchor_overflow_by_key,
        &boundary_overflow_by_key,
        &solved.merged_by_key,
    );
    validate_round_solution_coverage(nodes, &solution)?;

    Ok(RequirementPass {
        snapshot,
        solution: Arc::new(solution),
        layout_round_deltas,
        overflow_round_deltas,
        diagnostics,
    })
}

/// Construction-time coverage law: the layout channel covers every snapshot
/// node, and the three overflow channels cover exactly the overflow-bearing
/// nodes.
fn validate_round_solution_coverage(
    nodes: &[RequirementNodeSnapshot],
    solution: &CoordinationSolution,
) -> Result<(), AvengerChartError> {
    let validate = |label: &str,
                    actual: HashSet<&CoordinationNodeKey>,
                    expected: HashSet<&CoordinationNodeKey>|
     -> Result<(), AvengerChartError> {
        if actual == expected {
            return Ok(());
        }
        let missing = expected
            .difference(&actual)
            .map(|node| node.path.clone())
            .collect::<Vec<_>>();
        let unexpected = actual
            .difference(&expected)
            .map(|node| node.path.clone())
            .collect::<Vec<_>>();
        Err(AvengerChartError::InternalError(format!(
            "requirement {label} coverage mismatch: missing={missing:?}, unexpected={unexpected:?}"
        )))
    };

    let snapshot_nodes: HashSet<&CoordinationNodeKey> =
        nodes.iter().map(|node| &node.node_id).collect();
    validate(
        "layout patch",
        solution.layout_by_node.keys().collect(),
        snapshot_nodes,
    )?;

    let overflow_required_nodes: HashSet<&CoordinationNodeKey> = nodes
        .iter()
        .filter(|node| node.overflow_cells.is_some())
        .map(|node| &node.node_id)
        .collect();
    validate(
        "overflow patch",
        solution.overflow_by_node.keys().collect(),
        overflow_required_nodes.clone(),
    )?;
    validate(
        "boundary-overflow patch",
        solution.boundary_overflow_by_node.keys().collect(),
        overflow_required_nodes.clone(),
    )?;
    validate(
        "guide-anchor-overflow patch",
        solution.guide_anchor_overflow_by_node.keys().collect(),
        overflow_required_nodes,
    )?;
    Ok(())
}

fn collect_round_groups(
    nodes: &[RequirementNodeSnapshot],
    scopes: &RequirementScopeMetadata,
    own_overflow_by_node: &HashMap<CoordinationNodeKey, CoordinatedOverflow>,
) -> RoundGroupedRequirements {
    let mut grouped = RoundGroupedRequirements::default();
    for node in nodes {
        if let Some(measured_overflow) = own_overflow_by_node.get(&node.node_id).cloned() {
            let guide_anchor_key = scopes.guide_anchor_scope_key(node);
            let guide_anchor_overflow = guide_anchor_overflow_for_node(node, &measured_overflow);
            grouped.guide_anchor_entries.push((
                node.node_id.clone(),
                guide_anchor_key,
                guide_anchor_overflow,
            ));
            let boundary_overflow = scopes.sibling_boundary_overflow(node, &measured_overflow);
            let boundary_key = node.key.with_kind(CoordinationKind::BoundaryResidual);
            grouped
                .boundary_entries
                .push((node.node_id.clone(), boundary_key, boundary_overflow));
        }
        if let Some(group) = scopes.axis_lane_scope_by_node.get(&node.node_id) {
            let max_gap = grouped
                .max_guide_slot_gap_by_axis_group
                .entry(group.clone())
                .or_default();
            *max_gap = max_gap.max(node.guide_padding_inner_px);
        }
    }

    grouped
}

fn coordination_axis_from_facet_axis(axis: FacetAxis) -> CoordinationAxis {
    match axis {
        FacetAxis::Column => CoordinationAxis::Horizontal,
        FacetAxis::Row => CoordinationAxis::Vertical,
    }
}

/// Fold one overflow grouping: per-key `Edges<EdgeGrant>` component max
/// (the same law the solver's share patches apply), plus per-entry deltas
/// against the merged value.
///
/// Keys and payload pre-projections (guide-anchor lanes, boundary edge
/// stripping) are chart policy and happen before this call; the merge runs
/// through the neutral per-side `Edges<EdgeGrant>` law, which matches
/// `CoordinatedOverflow::merge` exactly: `EdgeGrant::new` lifts totals to
/// `inner + outer`, so the merged total is `max(guide) + max(legend)` on
/// every input.
fn fold_overflow_entries(
    entries: &[OverflowEntry],
) -> (
    HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    RoundDeltas,
) {
    let mut merged_demands: HashMap<CoordinationScopeKey, Edges<EdgeGrant>> = HashMap::new();
    for (_, key, overflow) in entries {
        let entry = merged_demands.entry(key.clone()).or_default();
        *entry = entry.max_components(overflow_edge_demands(overflow));
    }
    let merged_by_key = merged_demands
        .into_iter()
        .map(|(key, demands)| (key, overflow_from_edge_demands(demands)))
        .collect::<HashMap<_, _>>();

    let mut deltas = RoundDeltas::default();
    for (_, key, overflow) in entries {
        let (content, edge) = coordinated_overflow_delta(overflow, &merged_by_key[key]);
        deltas.content += content;
        deltas.edge += edge;
    }
    (merged_by_key, deltas)
}

/// Overflow is all chrome: deltas land on the edge channel.
fn coordinated_overflow_delta(
    local: &CoordinatedOverflow,
    merged: &CoordinatedOverflow,
) -> (f32, f32) {
    let edge = (merged.guide.top - local.guide.top).abs()
        + (merged.guide.right - local.guide.right).abs()
        + (merged.guide.bottom - local.guide.bottom).abs()
        + (merged.guide.left - local.guide.left).abs()
        + (merged.total.top - local.total.top).abs()
        + (merged.total.right - local.total.right).abs()
        + (merged.total.bottom - local.total.bottom).abs()
        + (merged.total.left - local.total.left).abs();
    (0.0, edge)
}

/// Two-channel delta for coordinated layout policy: slot-count difference is
/// content, chrome/spacing differences are edge.
fn coordinated_layout_delta(local: &CoordinatedLayout, merged: &CoordinatedLayout) -> (f32, f32) {
    let content = merged.n.abs_diff(local.n) as f32;
    let edge = (merged.padding_inner_px - local.padding_inner_px).abs()
        + (merged.guide_slot_gap_px - local.guide_slot_gap_px).abs()
        + (merged.outer_start - local.outer_start).abs()
        + (merged.outer_end - local.outer_end).abs();
    (content, edge)
}

/// Apply the per-node write-back adjustments (free-n reversion, lane-gap
/// fold, global-edge outer reversion) and assemble the round's solution
/// maps from the merged per-key channel values.
fn build_round_solution(
    nodes: &[RequirementNodeSnapshot],
    scopes: &RequirementScopeMetadata,
    grouped: &RoundGroupedRequirements,
    merged_overflow_by_key: &HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    merged_guide_anchor_overflow_by_key: &HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    merged_boundary_overflow_by_key: &HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    merged_layout_by_key: &HashMap<CoordinationScopeKey, CoordinatedLayout>,
) -> CoordinationSolution {
    let mut solution = CoordinationSolution::default();
    for node in nodes {
        let overflow_key = node.key.with_kind(CoordinationKind::OverflowResidual);
        if let Some(merged_overflow) = merged_overflow_by_key.get(&overflow_key).cloned() {
            solution
                .overflow_by_node
                .insert(node.node_id.clone(), merged_overflow);
        }
        let guide_anchor_key = scopes.guide_anchor_scope_key(node);
        if let Some(guide_anchor_overflow) = merged_guide_anchor_overflow_by_key
            .get(&guide_anchor_key)
            .cloned()
        {
            solution
                .guide_anchor_overflow_by_node
                .insert(node.node_id.clone(), guide_anchor_overflow);
        }
        let boundary_key = node.key.with_kind(CoordinationKind::BoundaryResidual);
        if let Some(boundary_overflow) = merged_boundary_overflow_by_key.get(&boundary_key).cloned()
        {
            solution
                .boundary_overflow_by_node
                .insert(node.node_id.clone(), boundary_overflow);
        }
        let layout_key = node.key.with_kind(CoordinationKind::ChildSize);
        if let Some(merged_layout) = merged_layout_by_key.get(&layout_key).cloned() {
            let mut layout = merged_layout;
            if node.slot_sharing.is_free() {
                layout.n = node.local_layout.n.max(node.min_slot_count);
            }
            if let Some(axis_group) = scopes.axis_lane_scope_by_node.get(&node.node_id)
                && let Some(group_gap) = grouped.max_guide_slot_gap_by_axis_group.get(axis_group)
            {
                layout.guide_slot_gap_px = layout.guide_slot_gap_px.max(*group_gap);
            }
            let (start_side, end_side) = match node.axis {
                FacetAxis::Column => (AxisPosition::Left, AxisPosition::Right),
                FacetAxis::Row => (AxisPosition::Top, AxisPosition::Bottom),
            };
            if scopes.is_global_edge_side(node, start_side) {
                layout.outer_start = node.local_layout.outer_start;
            }
            if scopes.is_global_edge_side(node, end_side) {
                layout.outer_end = node.local_layout.outer_end;
            }
            solution.layout_by_node.insert(node.node_id.clone(), layout);
        }
    }

    solution
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overflow(top: f32, right: f32, bottom: f32, left: f32) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top,
                right,
                bottom,
                left,
            },
            total: OverflowSpaceRequirement {
                top,
                right,
                bottom,
                left,
            },
        }
    }

    fn snapshot_node(
        path: &[usize],
        key: &CoordinationScopeKey,
        axis: FacetAxis,
        measured_overflow: Option<CoordinatedOverflow>,
    ) -> RequirementNodeSnapshot {
        RequirementNodeSnapshot {
            node_id: CoordinationNodeKey::new(path.to_vec()),
            key: key.clone(),
            axis,
            slot_sharing: SharingLevel::GLOBAL,
            min_slot_count: 0,
            overflow_cells: measured_overflow
                .as_ref()
                .map(|overflow| vec![(overflow.guide.clone(), overflow.total.clone())]),
            local_layout: CoordinatedLayout {
                padding_inner_px: 0.0,
                guide_slot_gap_px: 0.0,
                outer_start: 0.0,
                outer_end: 0.0,
                n: 2,
            },
            guide_padding_inner_px: 0.0,
            first_edge_index: 0,
            last_edge_index: 1,
        }
    }

    fn scope_key(depth: usize, identity: &str) -> CoordinationScopeKey {
        CoordinationScopeKey::container_group(CoordinationKind::ChildSize, depth, identity)
    }

    fn build_pass(snapshot: RequirementSnapshot) -> RequirementPass {
        build_requirement_pass(snapshot).expect("requirement pass builds")
    }

    #[test]
    fn initial_requirement_pass_groups_and_distributes_overflow_layout() {
        let key = scope_key(1, "col:group");
        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(1.0, 2.0, 3.0, 4.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 2.0,
                        guide_slot_gap_px: 2.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 2.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(3.0, 1.0, 5.0, 2.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 4.0,
                        guide_slot_gap_px: 4.0,
                        outer_start: 2.0,
                        outer_end: 1.0,
                        n: 4,
                    },
                    guide_padding_inner_px: 4.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.diagnostics.overflow_groups, 1);
        assert_eq!(pass.diagnostics.layout_groups, 1);
        assert_eq!(pass.solution.overflow_by_node.len(), 2);
        assert_eq!(pass.solution.layout_by_node.len(), 2);
    }

    #[test]
    fn free_slot_requirement_patches_keep_local_slot_count() {
        let key = scope_key(2, "col:team");
        let left_node = CoordinationNodeKey::new(vec![0, 0]);
        let right_node = CoordinationNodeKey::new(vec![1, 0]);
        let pass = build_pass(RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: left_node.clone(),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::FREE,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        n: 1,
                        ..Default::default()
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 0,
                },
                RequirementNodeSnapshot {
                    node_id: right_node.clone(),
                    key,
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::FREE,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        n: 3,
                        ..Default::default()
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 2,
                },
            ],
        });

        assert_eq!(pass.solution.layout_by_node.get(&left_node).unwrap().n, 1);
        assert_eq!(pass.solution.layout_by_node.get(&right_node).unwrap().n, 3);
    }

    #[test]
    fn free_slot_requirement_patches_preserve_minimum_physical_slot_count() {
        let key = scope_key(2, "col:wrap_row");
        let ragged_row = CoordinationNodeKey::new(vec![1]);
        let pass = build_pass(RequirementSnapshot {
            nodes: vec![RequirementNodeSnapshot {
                node_id: ragged_row.clone(),
                key,
                axis: FacetAxis::Column,
                slot_sharing: SharingLevel::FREE,
                min_slot_count: 5,
                overflow_cells: None,
                local_layout: CoordinatedLayout {
                    n: 2,
                    ..Default::default()
                },
                guide_padding_inner_px: 0.0,
                first_edge_index: 0,
                last_edge_index: 1,
            }],
        });

        assert_eq!(pass.solution.layout_by_node.get(&ragged_row).unwrap().n, 5);
    }

    #[test]
    fn shared_slot_requirement_patches_use_merged_slot_count() {
        let key = scope_key(2, "col:team");
        let left_node = CoordinationNodeKey::new(vec![0, 0]);
        let right_node = CoordinationNodeKey::new(vec![1, 0]);
        let pass = build_pass(RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: left_node.clone(),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        n: 1,
                        ..Default::default()
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 0,
                },
                RequirementNodeSnapshot {
                    node_id: right_node.clone(),
                    key,
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        n: 3,
                        ..Default::default()
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 2,
                },
            ],
        });

        assert_eq!(pass.solution.layout_by_node.get(&left_node).unwrap().n, 3);
        assert_eq!(pass.solution.layout_by_node.get(&right_node).unwrap().n, 3);
    }

    #[test]
    fn guide_anchor_pass_shares_column_guides_within_row_lane() {
        let outer_col_key = scope_key(1, "col:division");
        let row_key = scope_key(2, "row:department");
        let team_key = scope_key(3, "col:team");
        let left_top_team = CoordinationNodeKey::new(vec![0, 0]);
        let right_top_team = CoordinationNodeKey::new(vec![1, 0]);
        let left_bottom_team = CoordinationNodeKey::new(vec![0, 1]);

        let pass = build_pass(RequirementSnapshot {
            nodes: vec![
                snapshot_node(&[], &outer_col_key, FacetAxis::Column, None),
                snapshot_node(&[0], &row_key, FacetAxis::Row, None),
                snapshot_node(&[1], &row_key, FacetAxis::Row, None),
                snapshot_node(
                    &[0, 0],
                    &team_key,
                    FacetAxis::Column,
                    Some(overflow(11.0, 0.0, 0.0, 0.0)),
                ),
                snapshot_node(
                    &[1, 0],
                    &team_key,
                    FacetAxis::Column,
                    Some(overflow(29.0, 0.0, 0.0, 0.0)),
                ),
                snapshot_node(
                    &[0, 1],
                    &team_key,
                    FacetAxis::Column,
                    Some(overflow(43.0, 0.0, 0.0, 0.0)),
                ),
            ],
        });

        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&left_top_team)
                .unwrap()
                .total
                .top,
            29.0
        );
        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&right_top_team)
                .unwrap()
                .total
                .top,
            29.0
        );
        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&left_bottom_team)
                .unwrap()
                .total
                .top,
            43.0
        );
    }

    #[test]
    fn guide_anchor_pass_shares_row_guides_within_column_lane() {
        let outer_row_key = scope_key(1, "row:division");
        let col_key = scope_key(2, "col:department");
        let team_key = scope_key(3, "row:team");
        let top_left_team = CoordinationNodeKey::new(vec![0, 0]);
        let bottom_left_team = CoordinationNodeKey::new(vec![1, 0]);
        let top_right_team = CoordinationNodeKey::new(vec![0, 1]);

        let pass = build_pass(RequirementSnapshot {
            nodes: vec![
                snapshot_node(&[], &outer_row_key, FacetAxis::Row, None),
                snapshot_node(&[0], &col_key, FacetAxis::Column, None),
                snapshot_node(&[1], &col_key, FacetAxis::Column, None),
                snapshot_node(
                    &[0, 0],
                    &team_key,
                    FacetAxis::Row,
                    Some(overflow(0.0, 17.0, 0.0, 0.0)),
                ),
                snapshot_node(
                    &[1, 0],
                    &team_key,
                    FacetAxis::Row,
                    Some(overflow(0.0, 31.0, 0.0, 0.0)),
                ),
                snapshot_node(
                    &[0, 1],
                    &team_key,
                    FacetAxis::Row,
                    Some(overflow(0.0, 47.0, 0.0, 0.0)),
                ),
            ],
        });

        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&top_left_team)
                .unwrap()
                .total
                .right,
            31.0
        );
        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&bottom_left_team)
                .unwrap()
                .total
                .right,
            31.0
        );
        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&top_right_team)
                .unwrap()
                .total
                .right,
            47.0
        );
    }

    #[test]
    fn guide_anchor_pass_does_not_split_lanes_for_same_axis_ancestors() {
        let division_key = scope_key(1, "col:division");
        let team_key = scope_key(2, "col:team");
        let left_team = CoordinationNodeKey::new(vec![0]);
        let right_team = CoordinationNodeKey::new(vec![1]);

        let pass = build_pass(RequirementSnapshot {
            nodes: vec![
                snapshot_node(&[], &division_key, FacetAxis::Column, None),
                snapshot_node(
                    &[0],
                    &team_key,
                    FacetAxis::Column,
                    Some(overflow(13.0, 0.0, 0.0, 0.0)),
                ),
                snapshot_node(
                    &[1],
                    &team_key,
                    FacetAxis::Column,
                    Some(overflow(37.0, 0.0, 0.0, 0.0)),
                ),
            ],
        });

        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&left_team)
                .unwrap()
                .total
                .top,
            37.0
        );
        assert_eq!(
            pass.solution
                .guide_anchor_overflow_by_node
                .get(&right_team)
                .unwrap()
                .total
                .top,
            37.0
        );
    }

    #[test]
    fn requirement_pass_shares_guide_slot_gap_across_same_axis_groups() {
        let outer_col_key = scope_key(1, "col:division");
        let inner_col_key = scope_key(2, "col:dept");
        let row_key = scope_key(3, "row:team");
        let outer_node = CoordinationNodeKey::new(vec![0]);
        let inner_node = CoordinationNodeKey::new(vec![0, 0]);
        let row_node = CoordinationNodeKey::new(vec![0, 0, 0]);

        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: outer_node.clone(),
                    key: outer_col_key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 36.0,
                        guide_slot_gap_px: 36.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 36.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: inner_node.clone(),
                    key: inner_col_key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 27.0,
                        guide_slot_gap_px: 27.0,
                        outer_start: 3.0,
                        outer_end: 4.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 27.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 9.0,
                        guide_slot_gap_px: 9.0,
                        outer_start: 5.0,
                        outer_end: 6.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 9.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);

        // The lane fold raises the inner column's guide gap to the lane max
        // (36 from the outer column) while padding keeps the inner group's
        // own merge (27) — the per-key intermediate is internal now, so the
        // law is asserted through the per-node solution values.
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&outer_node)
                .unwrap()
                .guide_slot_gap_px,
            36.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&inner_node)
                .unwrap()
                .padding_inner_px,
            27.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&inner_node)
                .unwrap()
                .guide_slot_gap_px,
            36.0
        );
        // Rows are an orthogonal lane: the row band keeps its own gap.
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&row_node)
                .unwrap()
                .guide_slot_gap_px,
            9.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&inner_node)
                .unwrap()
                .outer_start,
            3.0
        );
    }

    #[test]
    fn requirement_pass_does_not_share_inner_padding_across_orthogonal_breaks() {
        let outer_row_key = scope_key(1, "row:division");
        let inner_row_key = scope_key(3, "row:team");
        let outer_node = CoordinationNodeKey::new(vec![0]);
        let inner_node = CoordinationNodeKey::new(vec![0, 0, 0]);

        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: outer_node.clone(),
                    key: outer_row_key,
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 8.0,
                        guide_slot_gap_px: 8.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 8.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0, 0]),
                    key: scope_key(2, "col:department"),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 12.0,
                        guide_slot_gap_px: 12.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 12.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: inner_node.clone(),
                    key: inner_row_key,
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 40.0,
                        guide_slot_gap_px: 40.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 40.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);

        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&outer_node)
                .unwrap()
                .guide_slot_gap_px,
            8.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&inner_node)
                .unwrap()
                .guide_slot_gap_px,
            40.0
        );
    }

    #[test]
    fn requirement_pass_keeps_legend_only_padding_local_to_its_group() {
        let legend_col_key = scope_key(1, "col:legend-owner");
        let inner_col_key = scope_key(2, "col:inner");

        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: legend_col_key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 160.0,
                        guide_slot_gap_px: 24.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 24.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0, 0]),
                    key: inner_col_key.clone(),
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 24.0,
                        guide_slot_gap_px: 24.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 24.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);

        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&CoordinationNodeKey::new(vec![0]))
                .unwrap()
                .padding_inner_px,
            160.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&CoordinationNodeKey::new(vec![0]))
                .unwrap()
                .guide_slot_gap_px,
            24.0
        );
        assert_eq!(
            pass.solution
                .layout_by_node
                .get(&CoordinationNodeKey::new(vec![0, 0]))
                .unwrap()
                .padding_inner_px,
            24.0
        );
    }

    #[test]
    fn requirement_pass_strips_global_edges_from_boundary_overflow_only() {
        let root_key = scope_key(1, "col:division");
        let row_key = scope_key(2, "row:team");
        let left_row_node = CoordinationNodeKey::new(vec![0]);
        let right_row_node = CoordinationNodeKey::new(vec![1]);

        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![]),
                    key: root_key,
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 0.0,
                        guide_slot_gap_px: 0.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: left_row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(0.0, 3.0, 0.0, 39.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout::default(),
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: right_row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(0.0, 8.0, 0.0, 8.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout::default(),
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);
        let full = pass
            .solution
            .overflow_by_node
            .get(&left_row_node)
            .expect("row group should have full overflow");
        let boundary = pass
            .solution
            .boundary_overflow_by_node
            .get(&left_row_node)
            .expect("row group should have boundary overflow");

        assert_eq!(full.guide.left, 39.0);
        assert_eq!(boundary.guide.left, 8.0);
        assert_eq!(
            pass.solution
                .boundary_overflow_by_node
                .get(&left_row_node)
                .unwrap()
                .guide
                .left,
            8.0
        );
        assert_eq!(
            pass.solution
                .boundary_overflow_by_node
                .get(&right_row_node)
                .unwrap()
                .guide
                .left,
            8.0
        );
    }

    #[test]
    fn retargeted_requirement_pass_reconciles_overflow_layout() {
        let key = scope_key(1, "row:group");
        let snapshot = RequirementSnapshot {
            nodes: vec![
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(1.0, 1.0, 2.0, 3.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 3.0,
                        guide_slot_gap_px: 3.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 3.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key,
                    axis: FacetAxis::Row,
                    slot_sharing: SharingLevel::GLOBAL,
                    min_slot_count: 0,
                    overflow_cells: Some(overflow(2.0, 4.0, 1.0, 1.0))
                        .map(|o| vec![(o.guide, o.total)]),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 5.0,
                        guide_slot_gap_px: 5.0,
                        outer_start: 2.0,
                        outer_end: 1.0,
                        n: 5,
                    },
                    guide_padding_inner_px: 5.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.diagnostics.overflow_groups, 1);
        assert_eq!(pass.diagnostics.layout_groups, 1);
        assert_eq!(pass.solution.overflow_by_node.len(), 2);
        assert_eq!(pass.solution.layout_by_node.len(), 2);
    }
}
