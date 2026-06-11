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

use std::{collections::HashMap, hash::Hash};

use avenger_chart_core::{
    AxisPosition, CoordinatedLayout, CoordinatedOverflow, CoordinationAxis, FacetAxis,
    FacetEmptyCellPolicy, OverflowSpaceRequirement, SharingLevel,
};

use crate::{
    facet::overflow_projection::{FacetOverflowProjection, project_facet_overflow},
    facet::overflow_projection::{overflow_edge_demands, overflow_from_edge_demands},
    layout::{AlignmentNode, EdgeDemand, Edges, RoundDeltas, SingletonPolicy, align_by},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequirementStage {
    Initial,
    Retargeted,
}

impl RequirementStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Initial => "initial requirements",
            Self::Retargeted => "retargeted requirements",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationScopeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) slot_sharing: SharingLevel,
    pub(crate) min_slot_count: usize,
    pub(crate) measured_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) guide_padding_inner_px: f32,
    pub(crate) first_edge_index: usize,
    pub(crate) last_edge_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequirementSnapshot {
    pub(crate) nodes: Vec<RequirementNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequirementAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    pub(crate) merged_boundary_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationScopeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequirementDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) guide_anchor_overflow_patches_by_node:
        HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) boundary_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone)]
pub(crate) struct RequirementPass {
    pub(crate) stage: RequirementStage,
    pub(crate) snapshot: RequirementSnapshot,
    pub(crate) aggregates: RequirementAggregates,
    pub(crate) distribution: RequirementDistributionPlan,
    /// Total layout deltas of this round (local vs merged), for cross-round
    /// convergence diagnostics.
    pub(crate) layout_round_deltas: RoundDeltas,
    /// Total overflow deltas of this round across the overflow, guide-anchor,
    /// and boundary groupings.
    pub(crate) overflow_round_deltas: RoundDeltas,
}

impl Default for RequirementPass {
    fn default() -> Self {
        Self {
            stage: RequirementStage::Initial,
            snapshot: RequirementSnapshot::default(),
            aggregates: RequirementAggregates::default(),
            distribution: RequirementDistributionPlan::default(),
            layout_round_deltas: RoundDeltas::default(),
            overflow_round_deltas: RoundDeltas::default(),
        }
    }
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

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetPlan {
    pub(crate) node_plans: Vec<RetargetNodePlan>,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodeTrace {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) planned_has_legend_overflow: bool,
    pub(crate) planned_layout_changed: bool,
    pub(crate) planned_axis_owner_ignore_empty_cells: bool,
    pub(crate) planned_band_action: BandRetargetAction,
    pub(crate) planned_child_action_counts: CellRetargetActionCounts,
    pub(crate) planned_child_count: usize,
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
    pub(crate) axis: FacetAxis,
    pub(crate) parent_cross_size_target: Option<f32>,
    pub(crate) child_count: usize,
    pub(crate) child_plans: Vec<FinalPropagationChildPlan>,
    pub(crate) expected_plot_area_adjustments_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPropagationPlan {
    pub(crate) node_plans: Vec<FinalPropagationNodePlan>,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationNodeTrace {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) planned_parent_cross_size_target: Option<f32>,
    pub(crate) planned_child_count: usize,
    pub(crate) planned_child_plan_count: usize,
    pub(crate) planned_plot_area_adjustments_count: usize,
    pub(crate) child_plot_area_adjustments_count: usize,
    pub(crate) scale_range_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPropagationTrace {
    pub(crate) node_results: Vec<FinalPropagationNodeTrace>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordinationRunArtifacts {
    pub(crate) initial_requirement_pass: RequirementPass,
    pub(crate) retarget_trace: RetargetTrace,
    pub(crate) retargeted_requirement_pass: RequirementPass,
    pub(crate) final_propagation_trace: FinalPropagationTrace,
}

#[derive(Debug, Clone)]
pub(crate) struct RoundCollectionInput {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationScopeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) slot_sharing: SharingLevel,
    pub(crate) min_slot_count: usize,
    pub(crate) measured_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) guide_padding_inner_px: f32,
    pub(crate) first_edge_index: usize,
    pub(crate) last_edge_index: usize,
}

#[derive(Debug, Clone)]
struct RequirementScopeMetadata {
    axis_by_path: HashMap<Vec<usize>, FacetAxis>,
    edge_indices_by_path: HashMap<Vec<usize>, (usize, usize)>,
    axis_lane_scope_by_node: HashMap<CoordinationNodeKey, CoordinationScopeKey>,
}

#[derive(Debug, Clone, Default)]
struct RoundCollectionOutput {
    merged_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    merged_boundary_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    merged_layout_by_key: HashMap<CoordinationScopeKey, CoordinatedLayout>,
    overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    guide_anchor_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    boundary_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    layout_round_deltas: RoundDeltas,
    overflow_round_deltas: RoundDeltas,
}

type OverflowEntry = (
    CoordinationNodeKey,
    CoordinationScopeKey,
    CoordinatedOverflow,
);

#[derive(Debug, Clone, Default)]
struct RoundGroupedRequirements {
    overflow_entries: Vec<OverflowEntry>,
    guide_anchor_entries: Vec<OverflowEntry>,
    boundary_entries: Vec<OverflowEntry>,
    max_guide_slot_gap_by_axis_group: HashMap<CoordinationScopeKey, f32>,
}

#[derive(Debug, Clone, Default)]
struct RoundMergedRequirements {
    overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    guide_anchor_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    boundary_overflow_by_key: HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    layout_by_key: HashMap<CoordinationScopeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
struct RoundRequirementPatches {
    overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    guide_anchor_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    boundary_overflow_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    layout_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

impl RequirementScopeMetadata {
    fn collect(nodes: &[RoundCollectionInput]) -> Self {
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
        nodes: &[RoundCollectionInput],
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

    fn guide_anchor_scope_key(&self, node: &RoundCollectionInput) -> CoordinationScopeKey {
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

    fn is_global_edge_side(&self, node: &RoundCollectionInput, side: AxisPosition) -> bool {
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
        node: &RoundCollectionInput,
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
    node: &RoundCollectionInput,
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

fn build_round_collection(nodes: &[RoundCollectionInput]) -> RoundCollectionOutput {
    let scopes = RequirementScopeMetadata::collect(nodes);
    let grouped = collect_round_groups(nodes, &scopes);
    let (mut merged, overflow_round_deltas) = merge_round_groups(&grouped);
    let solved_layout = crate::facet::round_tree::solve_layout_round(nodes);
    let layout_round_deltas = nodes
        .iter()
        .filter_map(|node| {
            solved_layout
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
    merged.layout_by_key = solved_layout.merged_by_key;
    let patches = build_round_patches(nodes, &scopes, &grouped, &merged);

    RoundCollectionOutput {
        merged_overflow_by_key: merged.overflow_by_key,
        merged_boundary_overflow_by_key: merged.boundary_overflow_by_key,
        merged_layout_by_key: merged.layout_by_key,
        overflow_patches_by_node: patches.overflow_by_node,
        guide_anchor_overflow_patches_by_node: patches.guide_anchor_overflow_by_node,
        boundary_overflow_patches_by_node: patches.boundary_overflow_by_node,
        layout_patches_by_node: patches.layout_by_node,
        layout_round_deltas,
        overflow_round_deltas,
    }
}

fn collect_round_groups(
    nodes: &[RoundCollectionInput],
    scopes: &RequirementScopeMetadata,
) -> RoundGroupedRequirements {
    let mut grouped = RoundGroupedRequirements::default();
    for node in nodes {
        if let Some(measured_overflow) = node.measured_overflow.clone() {
            let overflow_key = node.key.with_kind(CoordinationKind::OverflowResidual);
            grouped.overflow_entries.push((
                node.node_id.clone(),
                overflow_key,
                measured_overflow.clone(),
            ));
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

fn merge_round_groups(
    grouped: &RoundGroupedRequirements,
) -> (RoundMergedRequirements, RoundDeltas) {
    let (overflow_by_key, overflow_deltas) = align_overflow_groups(&grouped.overflow_entries);
    let (guide_anchor_overflow_by_key, guide_anchor_deltas) =
        align_overflow_groups(&grouped.guide_anchor_entries);
    let (boundary_overflow_by_key, boundary_deltas) =
        align_overflow_groups(&grouped.boundary_entries);

    let merged = RoundMergedRequirements {
        overflow_by_key,
        guide_anchor_overflow_by_key,
        boundary_overflow_by_key,
        layout_by_key: HashMap::new(),
    };
    let deltas = RoundDeltas {
        content: 0.0,
        edge: overflow_deltas.edge + guide_anchor_deltas.edge + boundary_deltas.edge,
    };
    (merged, deltas)
}

/// Run one overflow grouping as a neutral alignment round.
///
/// Keys and payload pre-projections (guide-anchor lanes, boundary edge
/// stripping) are chart policy and happen before this call; the merge runs
/// through the neutral per-side `Edges<EdgeDemand>` law, which matches
/// `CoordinatedOverflow::merge` exactly: `EdgeDemand::new` lifts totals to
/// `inner + outer`, so the merged total is `max(guide) + max(legend)` on
/// every input.
fn align_overflow_groups(
    entries: &[OverflowEntry],
) -> (
    HashMap<CoordinationScopeKey, CoordinatedOverflow>,
    RoundDeltas,
) {
    let nodes = entries
        .iter()
        .map(|(node_id, key, overflow)| AlignmentNode {
            id: node_id.clone(),
            group_key: key.clone(),
            requirements: overflow.clone(),
        })
        .collect::<Vec<_>>();

    let plan = align_by(
        &nodes,
        SingletonPolicy::Merge,
        |members| {
            let merged = members
                .iter()
                .fold(Edges::<EdgeDemand>::default(), |merged, overflow| {
                    merged.max_components(overflow_edge_demands(overflow))
                });
            Some(overflow_from_edge_demands(merged))
        },
        coordinated_overflow_delta,
    );

    let deltas = RoundDeltas {
        content: plan.total_content_delta(),
        edge: plan.total_edge_delta(),
    };
    let merged_by_key = plan
        .groups
        .into_iter()
        .map(|group| (group.key, group.merged))
        .collect();
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

fn build_round_patches(
    nodes: &[RoundCollectionInput],
    scopes: &RequirementScopeMetadata,
    grouped: &RoundGroupedRequirements,
    merged: &RoundMergedRequirements,
) -> RoundRequirementPatches {
    let mut patches = RoundRequirementPatches::default();
    for node in nodes {
        let overflow_key = node.key.with_kind(CoordinationKind::OverflowResidual);
        if let Some(merged_overflow) = merged.overflow_by_key.get(&overflow_key).cloned() {
            patches
                .overflow_by_node
                .insert(node.node_id.clone(), merged_overflow);
        }
        let guide_anchor_key = scopes.guide_anchor_scope_key(node);
        if let Some(guide_anchor_overflow) = merged
            .guide_anchor_overflow_by_key
            .get(&guide_anchor_key)
            .cloned()
        {
            patches
                .guide_anchor_overflow_by_node
                .insert(node.node_id.clone(), guide_anchor_overflow);
        }
        let boundary_key = node.key.with_kind(CoordinationKind::BoundaryResidual);
        if let Some(boundary_overflow) = merged.boundary_overflow_by_key.get(&boundary_key).cloned()
        {
            patches
                .boundary_overflow_by_node
                .insert(node.node_id.clone(), boundary_overflow);
        }
        let layout_key = node.key.with_kind(CoordinationKind::ChildSize);
        if let Some(merged_layout) = merged.layout_by_key.get(&layout_key).cloned() {
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
            patches.layout_by_node.insert(node.node_id.clone(), layout);
        }
    }

    patches
}

pub(crate) fn build_requirement_pass(
    stage: RequirementStage,
    snapshot: RequirementSnapshot,
) -> RequirementPass {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| RoundCollectionInput {
            node_id: node.node_id.clone(),
            key: node.key.clone(),
            axis: node.axis,
            slot_sharing: node.slot_sharing,
            min_slot_count: node.min_slot_count,
            measured_overflow: node.measured_overflow.clone(),
            local_layout: node.local_layout.clone(),
            guide_padding_inner_px: node.guide_padding_inner_px,
            first_edge_index: node.first_edge_index,
            last_edge_index: node.last_edge_index,
        })
        .collect::<Vec<_>>();
    let round = build_round_collection(&nodes);

    RequirementPass {
        stage,
        snapshot,
        aggregates: RequirementAggregates {
            merged_overflow_by_key: round.merged_overflow_by_key,
            merged_boundary_overflow_by_key: round.merged_boundary_overflow_by_key,
            merged_layout_by_key: round.merged_layout_by_key,
        },
        distribution: RequirementDistributionPlan {
            overflow_patches_by_node: round.overflow_patches_by_node,
            guide_anchor_overflow_patches_by_node: round.guide_anchor_overflow_patches_by_node,
            boundary_overflow_patches_by_node: round.boundary_overflow_patches_by_node,
            layout_patches_by_node: round.layout_patches_by_node,
        },
        layout_round_deltas: round.layout_round_deltas,
        overflow_round_deltas: round.overflow_round_deltas,
    }
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
            measured_overflow,
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
                    measured_overflow: Some(overflow(1.0, 2.0, 3.0, 4.0)),
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
                    measured_overflow: Some(overflow(3.0, 1.0, 5.0, 2.0)),
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

        let pass = build_requirement_pass(RequirementStage::Initial, snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
    }

    #[test]
    fn free_slot_requirement_patches_keep_local_slot_count() {
        let key = scope_key(2, "col:team");
        let left_node = CoordinationNodeKey::new(vec![0, 0]);
        let right_node = CoordinationNodeKey::new(vec![1, 0]);
        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
                nodes: vec![
                    RequirementNodeSnapshot {
                        node_id: left_node.clone(),
                        key: key.clone(),
                        axis: FacetAxis::Column,
                        slot_sharing: SharingLevel::FREE,
                        min_slot_count: 0,
                        measured_overflow: None,
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
                        measured_overflow: None,
                        local_layout: CoordinatedLayout {
                            n: 3,
                            ..Default::default()
                        },
                        guide_padding_inner_px: 0.0,
                        first_edge_index: 0,
                        last_edge_index: 2,
                    },
                ],
            },
        );

        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&left_node)
                .unwrap()
                .n,
            1
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&right_node)
                .unwrap()
                .n,
            3
        );
    }

    #[test]
    fn free_slot_requirement_patches_preserve_minimum_physical_slot_count() {
        let key = scope_key(2, "col:wrap_row");
        let ragged_row = CoordinationNodeKey::new(vec![1]);
        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
                nodes: vec![RequirementNodeSnapshot {
                    node_id: ragged_row.clone(),
                    key,
                    axis: FacetAxis::Column,
                    slot_sharing: SharingLevel::FREE,
                    min_slot_count: 5,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        n: 2,
                        ..Default::default()
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                }],
            },
        );

        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&ragged_row)
                .unwrap()
                .n,
            5
        );
    }

    #[test]
    fn shared_slot_requirement_patches_use_merged_slot_count() {
        let key = scope_key(2, "col:team");
        let left_node = CoordinationNodeKey::new(vec![0, 0]);
        let right_node = CoordinationNodeKey::new(vec![1, 0]);
        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
                nodes: vec![
                    RequirementNodeSnapshot {
                        node_id: left_node.clone(),
                        key: key.clone(),
                        axis: FacetAxis::Column,
                        slot_sharing: SharingLevel::GLOBAL,
                        min_slot_count: 0,
                        measured_overflow: None,
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
                        measured_overflow: None,
                        local_layout: CoordinatedLayout {
                            n: 3,
                            ..Default::default()
                        },
                        guide_padding_inner_px: 0.0,
                        first_edge_index: 0,
                        last_edge_index: 2,
                    },
                ],
            },
        );

        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&left_node)
                .unwrap()
                .n,
            3
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&right_node)
                .unwrap()
                .n,
            3
        );
    }

    #[test]
    fn guide_anchor_pass_shares_column_guides_within_row_lane() {
        let outer_col_key = scope_key(1, "col:division");
        let row_key = scope_key(2, "row:department");
        let team_key = scope_key(3, "col:team");
        let left_top_team = CoordinationNodeKey::new(vec![0, 0]);
        let right_top_team = CoordinationNodeKey::new(vec![1, 0]);
        let left_bottom_team = CoordinationNodeKey::new(vec![0, 1]);

        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
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
            },
        );

        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
                .get(&left_top_team)
                .unwrap()
                .total
                .top,
            29.0
        );
        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
                .get(&right_top_team)
                .unwrap()
                .total
                .top,
            29.0
        );
        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
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

        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
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
            },
        );

        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
                .get(&top_left_team)
                .unwrap()
                .total
                .right,
            31.0
        );
        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
                .get(&bottom_left_team)
                .unwrap()
                .total
                .right,
            31.0
        );
        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
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

        let pass = build_requirement_pass(
            RequirementStage::Initial,
            RequirementSnapshot {
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
            },
        );

        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
                .get(&left_team)
                .unwrap()
                .total
                .top,
            37.0
        );
        assert_eq!(
            pass.distribution
                .guide_anchor_overflow_patches_by_node
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
                    measured_overflow: None,
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
                    measured_overflow: None,
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
                    measured_overflow: None,
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

        let pass = build_requirement_pass(RequirementStage::Initial, snapshot);

        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&outer_col_key)
                .unwrap()
                .guide_slot_gap_px,
            36.0
        );
        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&inner_col_key)
                .unwrap()
                .guide_slot_gap_px,
            27.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&inner_node)
                .unwrap()
                .padding_inner_px,
            27.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&inner_node)
                .unwrap()
                .guide_slot_gap_px,
            36.0
        );
        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&row_key)
                .unwrap()
                .guide_slot_gap_px,
            9.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
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
                    measured_overflow: None,
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
                    measured_overflow: None,
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
                    measured_overflow: None,
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

        let pass = build_requirement_pass(RequirementStage::Initial, snapshot);

        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&outer_node)
                .unwrap()
                .guide_slot_gap_px,
            8.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
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
                    measured_overflow: None,
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
                    measured_overflow: None,
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

        let pass = build_requirement_pass(RequirementStage::Initial, snapshot);

        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&legend_col_key)
                .unwrap()
                .padding_inner_px,
            160.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&CoordinationNodeKey::new(vec![0]))
                .unwrap()
                .guide_slot_gap_px,
            24.0
        );
        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&inner_col_key)
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
                    measured_overflow: None,
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
                    measured_overflow: Some(overflow(0.0, 3.0, 0.0, 39.0)),
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
                    measured_overflow: Some(overflow(0.0, 8.0, 0.0, 8.0)),
                    local_layout: CoordinatedLayout::default(),
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_requirement_pass(RequirementStage::Initial, snapshot);
        let full = pass
            .aggregates
            .merged_overflow_by_key
            .get(&row_key.with_kind(CoordinationKind::OverflowResidual))
            .expect("row group should have full overflow");
        let boundary = pass
            .aggregates
            .merged_boundary_overflow_by_key
            .get(&row_key.with_kind(CoordinationKind::BoundaryResidual))
            .expect("row group should have boundary overflow");

        assert_eq!(full.guide.left, 39.0);
        assert_eq!(boundary.guide.left, 8.0);
        assert_eq!(
            pass.distribution
                .boundary_overflow_patches_by_node
                .get(&left_row_node)
                .unwrap()
                .guide
                .left,
            8.0
        );
        assert_eq!(
            pass.distribution
                .boundary_overflow_patches_by_node
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
                    measured_overflow: Some(overflow(1.0, 1.0, 2.0, 3.0)),
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
                    measured_overflow: Some(overflow(2.0, 4.0, 1.0, 1.0)),
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

        let pass = build_requirement_pass(RequirementStage::Retargeted, snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
    }
}
