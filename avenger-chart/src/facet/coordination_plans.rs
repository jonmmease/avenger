use std::collections::HashMap;

use crate::{
    cartesian::axis::AxisPosition,
    coords::{CoordinatedLayout, CoordinatedOverflow, FacetAxis},
    facet::coordination::CoordinationGroupKey,
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
pub(crate) struct InitialRequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) axis: FacetAxis,
    pub(crate) measured_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) guide_padding_inner_px: f32,
    pub(crate) first_edge_index: usize,
    pub(crate) last_edge_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementSnapshot {
    pub(crate) nodes: Vec<InitialRequirementNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_boundary_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) boundary_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementPass {
    pub(crate) snapshot: InitialRequirementSnapshot,
    pub(crate) aggregates: InitialRequirementAggregates,
    pub(crate) distribution: InitialRequirementDistributionPlan,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetedRequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) axis: FacetAxis,
    pub(crate) measured_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) guide_padding_inner_px: f32,
    pub(crate) first_edge_index: usize,
    pub(crate) last_edge_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementSnapshot {
    pub(crate) nodes: Vec<RetargetedRequirementNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_boundary_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) boundary_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementPass {
    pub(crate) snapshot: RetargetedRequirementSnapshot,
    pub(crate) aggregates: RetargetedRequirementAggregates,
    pub(crate) distribution: RetargetedRequirementDistributionPlan,
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
    pub(crate) initial_requirement_pass: InitialRequirementPass,
    pub(crate) retarget_trace: RetargetTrace,
    pub(crate) retargeted_requirement_pass: RetargetedRequirementPass,
    pub(crate) final_propagation_trace: FinalPropagationTrace,
}

fn merge_overflow_groups(
    overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>>,
) -> HashMap<CoordinationGroupKey, CoordinatedOverflow> {
    overflow_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedOverflow::default();
            for v in values {
                merged.merge(&v);
            }
            (key, merged)
        })
        .collect()
}

fn merge_layout_groups(
    layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>>,
) -> HashMap<CoordinationGroupKey, CoordinatedLayout> {
    layout_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedLayout::default();
            for v in values {
                merged.merge(&v);
            }
            (key, merged)
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RoundCollectionInput {
    node_id: CoordinationNodeKey,
    key: CoordinationGroupKey,
    axis: FacetAxis,
    measured_overflow: Option<CoordinatedOverflow>,
    local_layout: CoordinatedLayout,
    guide_padding_inner_px: f32,
    first_edge_index: usize,
    last_edge_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AxisPaddingGroupKey {
    axis: FacetAxis,
    root_path: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct RoundCollectionOutput {
    merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    merged_boundary_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
    overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    boundary_overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

fn axis_padding_group_keys(
    nodes: &[RoundCollectionInput],
) -> HashMap<CoordinationNodeKey, AxisPaddingGroupKey> {
    // Share guide-only gutters through uninterrupted same-axis facet chains
    // (`column -> column`, `row -> row`). Orthogonal facets start a new chain.
    let axis_by_path = nodes
        .iter()
        .map(|node| (node.node_id.path.clone(), node.axis))
        .collect::<HashMap<_, _>>();
    let mut node_refs = nodes.iter().collect::<Vec<_>>();
    node_refs.sort_by_key(|node| node.node_id.path.len());

    let mut group_by_path: HashMap<Vec<usize>, AxisPaddingGroupKey> = HashMap::new();
    let mut group_by_node: HashMap<CoordinationNodeKey, AxisPaddingGroupKey> = HashMap::new();

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
        let group = parent_group.unwrap_or_else(|| AxisPaddingGroupKey {
            axis: node.axis,
            root_path: node.node_id.path.clone(),
        });

        group_by_path.insert(node.node_id.path.clone(), group.clone());
        group_by_node.insert(node.node_id.clone(), group);
    }

    group_by_node
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

fn set_overflow_side(
    overflow: &mut crate::coords::OverflowSpaceRequirement,
    side: AxisPosition,
    value: f32,
) {
    match side {
        AxisPosition::Top => overflow.top = value,
        AxisPosition::Right => overflow.right = value,
        AxisPosition::Bottom => overflow.bottom = value,
        AxisPosition::Left => overflow.left = value,
    }
}

fn node_global_edge_for_side(
    node: &RoundCollectionInput,
    side: AxisPosition,
    axis_by_path: &HashMap<Vec<usize>, FacetAxis>,
    edge_indices_by_path: &HashMap<Vec<usize>, (usize, usize)>,
) -> bool {
    let axis = side_axis(side);
    let is_start = side_is_start(side);
    let mut governed_by_axis = node.axis == axis;
    let mut parent_path = Vec::new();

    for idx in &node.node_id.path {
        let Some(parent_axis) = axis_by_path.get(&parent_path) else {
            return false;
        };

        if *parent_axis == axis {
            governed_by_axis = true;
            let Some((first_edge_index, last_edge_index)) =
                edge_indices_by_path.get(&parent_path).copied()
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

fn strip_global_edge_overflow_for_boundary_coordination(
    node: &RoundCollectionInput,
    overflow: &CoordinatedOverflow,
    axis_by_path: &HashMap<Vec<usize>, FacetAxis>,
    edge_indices_by_path: &HashMap<Vec<usize>, (usize, usize)>,
) -> CoordinatedOverflow {
    let mut boundary = overflow.clone();
    for side in [
        AxisPosition::Top,
        AxisPosition::Right,
        AxisPosition::Bottom,
        AxisPosition::Left,
    ] {
        if node_global_edge_for_side(node, side, axis_by_path, edge_indices_by_path) {
            set_overflow_side(&mut boundary.guide, side, 0.0);
            set_overflow_side(&mut boundary.total, side, 0.0);
        }
    }
    boundary
}

fn build_round_collection(nodes: &[RoundCollectionInput]) -> RoundCollectionOutput {
    let mut overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    let mut boundary_overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();
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
    let axis_padding_group_by_node = axis_padding_group_keys(nodes);
    let mut max_guide_padding_by_axis_group: HashMap<AxisPaddingGroupKey, f32> = HashMap::new();

    for node in nodes {
        if let Some(measured_overflow) = node.measured_overflow.clone() {
            overflow_by_key
                .entry(node.key.clone())
                .or_default()
                .push(measured_overflow.clone());
            let boundary_overflow = strip_global_edge_overflow_for_boundary_coordination(
                node,
                &measured_overflow,
                &axis_by_path,
                &edge_indices_by_path,
            );
            boundary_overflow_by_key
                .entry(node.key.clone())
                .or_default()
                .push(boundary_overflow);
        }
        layout_by_key
            .entry(node.key.clone())
            .or_default()
            .push(node.local_layout.clone());
        if let Some(group) = axis_padding_group_by_node.get(&node.node_id) {
            let max_padding = max_guide_padding_by_axis_group
                .entry(group.clone())
                .or_default();
            *max_padding = max_padding.max(node.guide_padding_inner_px);
        }
    }

    let merged_overflow_by_key = merge_overflow_groups(overflow_by_key);
    let merged_boundary_overflow_by_key = merge_overflow_groups(boundary_overflow_by_key);
    let merged_layout_by_key = merge_layout_groups(layout_by_key);
    let mut overflow_patches_by_node = HashMap::new();
    let mut boundary_overflow_patches_by_node = HashMap::new();
    let mut layout_patches_by_node = HashMap::new();

    for node in nodes {
        if let Some(merged) = merged_overflow_by_key.get(&node.key).cloned() {
            overflow_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if let Some(merged) = merged_boundary_overflow_by_key.get(&node.key).cloned() {
            boundary_overflow_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if let Some(merged) = merged_layout_by_key.get(&node.key).cloned() {
            let mut layout = merged;
            if let Some(axis_group) = axis_padding_group_by_node.get(&node.node_id)
                && let Some(group_padding) = max_guide_padding_by_axis_group.get(axis_group)
            {
                layout.padding_inner_px = layout.padding_inner_px.max(*group_padding);
            }
            let (start_side, end_side) = match node.axis {
                FacetAxis::Column => (AxisPosition::Left, AxisPosition::Right),
                FacetAxis::Row => (AxisPosition::Top, AxisPosition::Bottom),
            };
            if node_global_edge_for_side(node, start_side, &axis_by_path, &edge_indices_by_path) {
                layout.outer_start = node.local_layout.outer_start;
            }
            if node_global_edge_for_side(node, end_side, &axis_by_path, &edge_indices_by_path) {
                layout.outer_end = node.local_layout.outer_end;
            }
            layout_patches_by_node.insert(node.node_id.clone(), layout);
        }
    }

    RoundCollectionOutput {
        merged_overflow_by_key,
        merged_boundary_overflow_by_key,
        merged_layout_by_key,
        overflow_patches_by_node,
        boundary_overflow_patches_by_node,
        layout_patches_by_node,
    }
}

pub(crate) fn build_initial_requirement_pass(
    snapshot: InitialRequirementSnapshot,
) -> InitialRequirementPass {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| RoundCollectionInput {
            node_id: node.node_id.clone(),
            key: node.key.clone(),
            axis: node.axis,
            measured_overflow: node.measured_overflow.clone(),
            local_layout: node.local_layout.clone(),
            guide_padding_inner_px: node.guide_padding_inner_px,
            first_edge_index: node.first_edge_index,
            last_edge_index: node.last_edge_index,
        })
        .collect::<Vec<_>>();
    let round = build_round_collection(&nodes);

    InitialRequirementPass {
        snapshot,
        aggregates: InitialRequirementAggregates {
            merged_overflow_by_key: round.merged_overflow_by_key,
            merged_boundary_overflow_by_key: round.merged_boundary_overflow_by_key,
            merged_layout_by_key: round.merged_layout_by_key,
        },
        distribution: InitialRequirementDistributionPlan {
            overflow_patches_by_node: round.overflow_patches_by_node,
            boundary_overflow_patches_by_node: round.boundary_overflow_patches_by_node,
            layout_patches_by_node: round.layout_patches_by_node,
        },
    }
}

pub(crate) fn build_retargeted_requirement_pass(
    snapshot: RetargetedRequirementSnapshot,
) -> RetargetedRequirementPass {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| RoundCollectionInput {
            node_id: node.node_id.clone(),
            key: node.key.clone(),
            axis: node.axis,
            measured_overflow: node.measured_overflow.clone(),
            local_layout: node.local_layout.clone(),
            guide_padding_inner_px: node.guide_padding_inner_px,
            first_edge_index: node.first_edge_index,
            last_edge_index: node.last_edge_index,
        })
        .collect::<Vec<_>>();
    let round = build_round_collection(&nodes);

    RetargetedRequirementPass {
        snapshot,
        aggregates: RetargetedRequirementAggregates {
            merged_overflow_by_key: round.merged_overflow_by_key,
            merged_boundary_overflow_by_key: round.merged_boundary_overflow_by_key,
            merged_layout_by_key: round.merged_layout_by_key,
        },
        distribution: RetargetedRequirementDistributionPlan {
            overflow_patches_by_node: round.overflow_patches_by_node,
            boundary_overflow_patches_by_node: round.boundary_overflow_patches_by_node,
            layout_patches_by_node: round.layout_patches_by_node,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::OverflowSpaceRequirement;

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

    #[test]
    fn initial_requirement_pass_groups_and_distributes_overflow_layout() {
        let key = CoordinationGroupKey::new(1, "col:group");
        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: Some(overflow(1.0, 2.0, 3.0, 4.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 2.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 2.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key: key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: Some(overflow(3.0, 1.0, 5.0, 2.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 4.0,
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

        let pass = build_initial_requirement_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
    }

    #[test]
    fn requirement_pass_shares_inner_padding_across_same_axis_groups() {
        let outer_col_key = CoordinationGroupKey::new(1, "col:division");
        let inner_col_key = CoordinationGroupKey::new(2, "col:dept");
        let row_key = CoordinationGroupKey::new(3, "row:team");
        let outer_node = CoordinationNodeKey::new(vec![0]);
        let inner_node = CoordinationNodeKey::new(vec![0, 0]);
        let row_node = CoordinationNodeKey::new(vec![0, 0, 0]);

        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: outer_node.clone(),
                    key: outer_col_key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 36.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 36.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: inner_node.clone(),
                    key: inner_col_key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 27.0,
                        outer_start: 3.0,
                        outer_end: 4.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 27.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 9.0,
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

        let pass = build_initial_requirement_pass(snapshot);

        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&outer_col_key)
                .unwrap()
                .padding_inner_px,
            36.0
        );
        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&inner_col_key)
                .unwrap()
                .padding_inner_px,
            27.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&inner_node)
                .unwrap()
                .padding_inner_px,
            36.0
        );
        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&row_key)
                .unwrap()
                .padding_inner_px,
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
        let outer_row_key = CoordinationGroupKey::new(1, "row:division");
        let inner_row_key = CoordinationGroupKey::new(3, "row:team");
        let outer_node = CoordinationNodeKey::new(vec![0]);
        let inner_node = CoordinationNodeKey::new(vec![0, 0, 0]);

        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: outer_node.clone(),
                    key: outer_row_key,
                    axis: FacetAxis::Row,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 8.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 8.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0, 0]),
                    key: CoordinationGroupKey::new(2, "col:department"),
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 12.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 12.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: inner_node.clone(),
                    key: inner_row_key,
                    axis: FacetAxis::Row,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 40.0,
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

        let pass = build_initial_requirement_pass(snapshot);

        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&outer_node)
                .unwrap()
                .padding_inner_px,
            8.0
        );
        assert_eq!(
            pass.distribution
                .layout_patches_by_node
                .get(&inner_node)
                .unwrap()
                .padding_inner_px,
            40.0
        );
    }

    #[test]
    fn requirement_pass_keeps_legend_only_padding_local_to_its_group() {
        let legend_col_key = CoordinationGroupKey::new(1, "col:legend-owner");
        let inner_col_key = CoordinationGroupKey::new(2, "col:inner");

        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: legend_col_key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 160.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 24.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0, 0]),
                    key: inner_col_key.clone(),
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 24.0,
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

        let pass = build_initial_requirement_pass(snapshot);

        assert_eq!(
            pass.aggregates
                .merged_layout_by_key
                .get(&legend_col_key)
                .unwrap()
                .padding_inner_px,
            160.0
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
        let root_key = CoordinationGroupKey::new(1, "col:division");
        let row_key = CoordinationGroupKey::new(2, "row:team");
        let left_row_node = CoordinationNodeKey::new(vec![0]);
        let right_row_node = CoordinationNodeKey::new(vec![1]);

        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![]),
                    key: root_key,
                    axis: FacetAxis::Column,
                    measured_overflow: None,
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 0.0,
                        outer_start: 0.0,
                        outer_end: 0.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: left_row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    measured_overflow: Some(overflow(0.0, 3.0, 0.0, 39.0)),
                    local_layout: CoordinatedLayout::default(),
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                InitialRequirementNodeSnapshot {
                    node_id: right_row_node.clone(),
                    key: row_key.clone(),
                    axis: FacetAxis::Row,
                    measured_overflow: Some(overflow(0.0, 8.0, 0.0, 8.0)),
                    local_layout: CoordinatedLayout::default(),
                    guide_padding_inner_px: 0.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
            ],
        };

        let pass = build_initial_requirement_pass(snapshot);
        let full = pass
            .aggregates
            .merged_overflow_by_key
            .get(&row_key)
            .expect("row group should have full overflow");
        let boundary = pass
            .aggregates
            .merged_boundary_overflow_by_key
            .get(&row_key)
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
        let key = CoordinationGroupKey::new(1, "row:group");
        let snapshot = RetargetedRequirementSnapshot {
            nodes: vec![
                RetargetedRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    axis: FacetAxis::Row,
                    measured_overflow: Some(overflow(1.0, 1.0, 2.0, 3.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 3.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    guide_padding_inner_px: 3.0,
                    first_edge_index: 0,
                    last_edge_index: 1,
                },
                RetargetedRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key,
                    axis: FacetAxis::Row,
                    measured_overflow: Some(overflow(2.0, 4.0, 1.0, 1.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 5.0,
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

        let pass = build_retargeted_requirement_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
    }
}
