//! Read-only layout coordination nodes for measured child-frame containers.
//!
//! These nodes describe measured container requirements without mutating the
//! measurement tree. Later coordination phases can group compatible nodes,
//! merge their requirements, and apply aligned solutions through concrete
//! container adapters.

use indexmap::IndexMap;
use tracing::debug;

use crate::{
    concat::{ConcatCoordMeasurement, GridShape, GridSlotRect, GridTrackRequirements},
    plot::compiled::{
        ChildFrameKey, ComponentsMeasurement, ContainerPathSegment,
        container_path_without_facet_segments,
    },
};

use avenger_chart_core::AvengerChartError;

/// One physical measured occurrence of a child-frame container.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameContainerInstanceKey {
    pub(crate) container_path: Vec<ContainerPathSegment>,
}

impl ChildFrameContainerInstanceKey {
    pub(crate) fn new(container_path: Vec<ContainerPathSegment>) -> Self {
        Self { container_path }
    }
}

/// Semantic identity used to align equivalent container instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameContainerTemplateKey {
    pub(crate) container_path_template: Vec<ContainerPathSegment>,
}

impl ChildFrameContainerTemplateKey {
    fn from_instance(
        instance_key: &ChildFrameContainerInstanceKey,
        scope: LayoutCoordinationScope,
    ) -> Self {
        let container_path_template = match scope {
            LayoutCoordinationScope::TemplatePathWithoutFacetSegments => {
                container_path_without_facet_segments(&instance_key.container_path)
            }
        };
        Self {
            container_path_template,
        }
    }
}

/// Kind of child-frame container described by a layout coordination node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameContainerKind {
    GridConcat,
}

/// Rule used to project a physical instance into a reusable template key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LayoutCoordinationScope {
    TemplatePathWithoutFacetSegments,
}

/// One child slot inside a layout coordination node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChildFrameLayoutSlot {
    pub(crate) child_index: usize,
    pub(crate) child_key: ChildFrameKey,
    pub(crate) slot: GridSlotRect,
}

/// Grouping key for compatible alignment nodes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayoutAlignmentKey {
    pub(crate) template_key: ChildFrameContainerTemplateKey,
    pub(crate) kind: ChildFrameContainerKind,
    pub(crate) topology: ChildFrameLayoutTopology,
}

/// Physical topology that must match for nodes to share one aligned solution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameLayoutTopology {
    Grid {
        shape: GridShape,
        slots: Vec<ChildFrameLayoutSlotTopology>,
    },
}

/// Slot topology omits measured sizes but preserves child identity and slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildFrameLayoutSlotTopology {
    pub(crate) child_key: ChildFrameKey,
    pub(crate) slot: GridSlotRect,
}

/// Read-only measured layout coordination node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChildFrameLayoutCoordinationNode {
    pub(crate) instance_key: ChildFrameContainerInstanceKey,
    pub(crate) template_key: ChildFrameContainerTemplateKey,
    pub(crate) kind: ChildFrameContainerKind,
    pub(crate) alignment_scope: LayoutCoordinationScope,
    pub(crate) topology: ChildFrameLayoutTopology,
    pub(crate) slots: Vec<ChildFrameLayoutSlot>,
    pub(crate) requirements: ChildFrameLayoutRequirements,
}

impl ChildFrameLayoutCoordinationNode {
    pub(crate) fn alignment_key(&self) -> LayoutAlignmentKey {
        LayoutAlignmentKey {
            template_key: self.template_key.clone(),
            kind: self.kind,
            topology: self.topology.clone(),
        }
    }
}

/// Local requirements exported by a measured child-frame container.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChildFrameLayoutRequirements {
    Grid(GridTrackRequirements),
}

pub(crate) fn grid_layout_coordination_node(
    concat: &ConcatCoordMeasurement,
) -> Result<Option<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let Some(shape) = concat.grid_shape() else {
        return Ok(None);
    };

    let instance_key = ChildFrameContainerInstanceKey::new(concat.container_path()?);
    let alignment_scope = LayoutCoordinationScope::TemplatePathWithoutFacetSegments;
    let template_key =
        ChildFrameContainerTemplateKey::from_instance(&instance_key, alignment_scope);
    let slots = concat.grid_layout_slots()?;
    let topology = ChildFrameLayoutTopology::Grid {
        shape,
        slots: slots
            .iter()
            .map(|slot| ChildFrameLayoutSlotTopology {
                child_key: slot.child_key.clone(),
                slot: slot.slot,
            })
            .collect(),
    };
    let requirements = ChildFrameLayoutRequirements::Grid(concat.grid_track_requirements()?);

    Ok(Some(ChildFrameLayoutCoordinationNode {
        instance_key,
        template_key,
        kind: ChildFrameContainerKind::GridConcat,
        alignment_scope,
        topology,
        slots,
        requirements,
    }))
}

pub(crate) fn collect_child_frame_layout_coordination_nodes(
    measurement: &ComponentsMeasurement,
) -> Result<Vec<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let mut nodes = Vec::new();
    collect_child_frame_layout_coordination_nodes_into(measurement, &mut nodes)?;
    Ok(nodes)
}

/// Measurement-only diagnostics for child-frame layout alignment.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChildFrameLayoutAlignmentDiagnostics {
    pub(crate) exported_node_count: usize,
    pub(crate) alignment_group_count: usize,
    pub(crate) merged_groups: Vec<LayoutAlignmentGroupDiagnostics>,
    pub(crate) skipped_groups: Vec<LayoutAlignmentSkippedGroup>,
}

impl ChildFrameLayoutAlignmentDiagnostics {
    pub(crate) fn total_track_delta(&self) -> f32 {
        self.merged_groups
            .iter()
            .flat_map(|group| group.node_deltas.iter())
            .map(|delta| delta.track_delta)
            .sum()
    }

    pub(crate) fn total_slab_delta(&self) -> f32 {
        self.merged_groups
            .iter()
            .flat_map(|group| group.node_deltas.iter())
            .map(|delta| delta.slab_delta)
            .sum()
    }
}

/// Diagnostics for one multi-instance alignment group.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutAlignmentGroupDiagnostics {
    pub(crate) key: LayoutAlignmentKey,
    pub(crate) node_count: usize,
    pub(crate) merged_requirements: ChildFrameLayoutRequirements,
    pub(crate) node_deltas: Vec<ChildFrameLayoutRequirementDelta>,
}

/// Requirement delta from one local instance to a merged requirement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChildFrameLayoutRequirementDelta {
    pub(crate) instance_key: ChildFrameContainerInstanceKey,
    pub(crate) track_delta: f32,
    pub(crate) slab_delta: f32,
}

impl ChildFrameLayoutRequirementDelta {
    pub(crate) fn has_delta(&self) -> bool {
        self.track_delta > 0.0 || self.slab_delta > 0.0
    }
}

/// Alignment groups that were intentionally not merged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutAlignmentSkippedGroup {
    pub(crate) key: LayoutAlignmentKey,
    pub(crate) node_count: usize,
    pub(crate) reason: LayoutAlignmentSkippedReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayoutAlignmentSkippedReason {
    Singleton,
    IncompatibleRequirements,
}

pub(crate) fn diagnose_child_frame_layout_alignment(
    measurement: &ComponentsMeasurement,
) -> Result<ChildFrameLayoutAlignmentDiagnostics, AvengerChartError> {
    let nodes = collect_child_frame_layout_coordination_nodes(measurement)?;
    let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
    if diagnostics.exported_node_count > 0 {
        let changed_node_count = diagnostics
            .merged_groups
            .iter()
            .flat_map(|group| group.node_deltas.iter())
            .filter(|delta| delta.has_delta())
            .count();
        debug!(
            target: "avenger_chart::layout_coordination",
            exported_node_count = diagnostics.exported_node_count,
            alignment_group_count = diagnostics.alignment_group_count,
            merged_group_count = diagnostics.merged_groups.len(),
            skipped_group_count = diagnostics.skipped_groups.len(),
            changed_node_count,
            total_track_delta = diagnostics.total_track_delta(),
            total_slab_delta = diagnostics.total_slab_delta(),
            "child-frame layout alignment diagnostics"
        );
    }
    Ok(diagnostics)
}

pub(crate) fn build_child_frame_layout_alignment_diagnostics(
    nodes: &[ChildFrameLayoutCoordinationNode],
) -> ChildFrameLayoutAlignmentDiagnostics {
    let mut grouped: IndexMap<LayoutAlignmentKey, Vec<&ChildFrameLayoutCoordinationNode>> =
        IndexMap::new();
    for node in nodes {
        grouped.entry(node.alignment_key()).or_default().push(node);
    }

    let mut merged_groups = Vec::new();
    let mut skipped_groups = Vec::new();
    for (key, group_nodes) in grouped.iter() {
        if group_nodes.len() < 2 {
            skipped_groups.push(LayoutAlignmentSkippedGroup {
                key: key.clone(),
                node_count: group_nodes.len(),
                reason: LayoutAlignmentSkippedReason::Singleton,
            });
            continue;
        }

        let Some(merged_requirements) = merge_child_frame_layout_requirements(
            group_nodes.iter().map(|node| &node.requirements),
        ) else {
            skipped_groups.push(LayoutAlignmentSkippedGroup {
                key: key.clone(),
                node_count: group_nodes.len(),
                reason: LayoutAlignmentSkippedReason::IncompatibleRequirements,
            });
            continue;
        };

        let node_deltas = group_nodes
            .iter()
            .map(|node| ChildFrameLayoutRequirementDelta {
                instance_key: node.instance_key.clone(),
                track_delta: requirement_track_delta(&node.requirements, &merged_requirements),
                slab_delta: requirement_slab_delta(&node.requirements, &merged_requirements),
            })
            .collect();

        merged_groups.push(LayoutAlignmentGroupDiagnostics {
            key: key.clone(),
            node_count: group_nodes.len(),
            merged_requirements,
            node_deltas,
        });
    }

    ChildFrameLayoutAlignmentDiagnostics {
        exported_node_count: nodes.len(),
        alignment_group_count: grouped.len(),
        merged_groups,
        skipped_groups,
    }
}

fn merge_child_frame_layout_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a ChildFrameLayoutRequirements>,
) -> Option<ChildFrameLayoutRequirements> {
    let mut iter = requirements.into_iter();
    let first = iter.next()?.clone();
    iter.try_fold(first, |merged, next| match (merged, next) {
        (
            ChildFrameLayoutRequirements::Grid(mut merged),
            ChildFrameLayoutRequirements::Grid(next),
        ) if merged.shape == next.shape => {
            max_assign_each(&mut merged.column_widths, &next.column_widths);
            max_assign_each(&mut merged.row_heights, &next.row_heights);
            max_assign_each(&mut merged.column_left, &next.column_left);
            max_assign_each(&mut merged.column_right, &next.column_right);
            max_assign_each(&mut merged.row_top, &next.row_top);
            max_assign_each(&mut merged.row_bottom, &next.row_bottom);
            Some(ChildFrameLayoutRequirements::Grid(merged))
        }
        _ => None,
    })
}

fn max_assign_each(target: &mut [f32], source: &[f32]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = (*target).max(*source);
    }
}

fn requirement_track_delta(
    local: &ChildFrameLayoutRequirements,
    merged: &ChildFrameLayoutRequirements,
) -> f32 {
    match (local, merged) {
        (ChildFrameLayoutRequirements::Grid(local), ChildFrameLayoutRequirements::Grid(merged)) => {
            abs_delta_sum(&local.column_widths, &merged.column_widths)
                + abs_delta_sum(&local.row_heights, &merged.row_heights)
        }
    }
}

fn requirement_slab_delta(
    local: &ChildFrameLayoutRequirements,
    merged: &ChildFrameLayoutRequirements,
) -> f32 {
    match (local, merged) {
        (ChildFrameLayoutRequirements::Grid(local), ChildFrameLayoutRequirements::Grid(merged)) => {
            abs_delta_sum(&local.column_left, &merged.column_left)
                + abs_delta_sum(&local.column_right, &merged.column_right)
                + abs_delta_sum(&local.row_top, &merged.row_top)
                + abs_delta_sum(&local.row_bottom, &merged.row_bottom)
        }
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

fn collect_child_frame_layout_coordination_nodes_into(
    measurement: &ComponentsMeasurement,
    nodes: &mut Vec<ChildFrameLayoutCoordinationNode>,
) -> Result<(), AvengerChartError> {
    if let Some(concat) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
    {
        if let Some(node) = grid_layout_coordination_node(concat)? {
            nodes.push(node);
        }
    }

    if let Some(container) = measurement.child_frame_container_view()? {
        for placement in container.placement().render_placements() {
            let child = container.child_measurement(placement.child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame measurement for child index {} while collecting layout coordination nodes",
                    placement.child_index
                ))
            })?;
            collect_child_frame_layout_coordination_nodes_into(child, nodes)?;
        }
    }

    Ok(())
}
