//! Layout coordination nodes for measured child-frame containers.
//!
//! These nodes describe measured container requirements. The alignment pass can
//! group compatible nodes, merge their requirements, and apply aligned
//! solutions through concrete container adapters.

use indexmap::IndexMap;
use tracing::debug;

use crate::{
    concat::{ConcatCoordMeasurement, GridShape, GridSlotRect, GridTrackRequirements},
    coords::FacetAxis,
    facet::{
        coord::FacetBandCoordMeasurement,
        placement::{FacetBandPlacement, resolve_facet_band_placement},
    },
    plot::compiled::{ChildFrameKey, ComponentsMeasurement, ContainerPathSegment},
    positioned_subplot::PositionedCoordMeasurement,
};

use avenger_chart_core::{AvengerChartError, CoordinatedLayout};

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
    pub(crate) semantic_tag: Option<String>,
}

impl ChildFrameContainerTemplateKey {
    fn from_instance(
        instance_key: &ChildFrameContainerInstanceKey,
        scope: LayoutCoordinationScope,
    ) -> Self {
        let container_path_template = match scope {
            LayoutCoordinationScope::TemplatePathWithoutFacetSegments => {
                layout_alignment_template_path(&instance_key.container_path)
            }
        };
        Self {
            container_path_template,
            semantic_tag: None,
        }
    }

    fn from_instance_with_semantic_tag(
        instance_key: &ChildFrameContainerInstanceKey,
        scope: LayoutCoordinationScope,
        semantic_tag: String,
    ) -> Self {
        Self {
            semantic_tag: Some(semantic_tag),
            ..Self::from_instance(instance_key, scope)
        }
    }
}

fn layout_alignment_template_path(path: &[ContainerPathSegment]) -> Vec<ContainerPathSegment> {
    path.iter()
        .filter_map(|segment| match segment {
            ContainerPathSegment::FacetValue { .. } => None,
            ContainerPathSegment::ConcatChild { key: Some(key), .. } => repeat_template_key(key)
                .map_or_else(
                    || Some(segment.clone()),
                    |template_key| Some(ContainerPathSegment::concat_child(0, Some(template_key))),
                ),
            _ => Some(segment.clone()),
        })
        .collect()
}

fn repeat_template_key(key: &str) -> Option<&'static str> {
    if key.starts_with("repeat_cell:") {
        Some("repeat_cell:*")
    } else if key.starts_with("repeat_col:") {
        Some("repeat_col:*")
    } else if key.starts_with("repeat_row:") {
        Some("repeat_row:*")
    } else if key.starts_with("repeat_item:") {
        Some("repeat_item:*")
    } else {
        None
    }
}

/// Kind of child-frame container described by a layout coordination node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChildFrameContainerKind {
    FacetColumn,
    FacetRow,
    HConcat,
    VConcat,
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
    let Some(shape) = concat.layout_coordination_shape() else {
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
    let kind = match concat.band_direction() {
        Some(crate::layout::BandDirection::Horizontal) => ChildFrameContainerKind::HConcat,
        Some(crate::layout::BandDirection::Vertical) => ChildFrameContainerKind::VConcat,
        None => ChildFrameContainerKind::GridConcat,
    };

    Ok(Some(ChildFrameLayoutCoordinationNode {
        instance_key,
        template_key,
        kind,
        alignment_scope,
        topology,
        slots,
        requirements,
    }))
}

pub(crate) fn facet_band_layout_coordination_node(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
) -> Result<Option<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let shape = facet_layout_coordination_shape(facet_band);
    let instance_key = ChildFrameContainerInstanceKey::new(facet_band.scope_path_prefix.clone());
    let alignment_scope = LayoutCoordinationScope::TemplatePathWithoutFacetSegments;
    let template_key = ChildFrameContainerTemplateKey::from_instance_with_semantic_tag(
        &instance_key,
        alignment_scope,
        facet_layout_semantic_tag(facet_band),
    );
    let slots = facet_layout_slots(facet_band)?;
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
    let requirements = ChildFrameLayoutRequirements::Grid(facet_grid_track_requirements(
        measurement,
        facet_band,
        shape,
    )?);
    let kind = match facet_band.axis {
        FacetAxis::Column => ChildFrameContainerKind::FacetColumn,
        FacetAxis::Row => ChildFrameContainerKind::FacetRow,
    };

    Ok(Some(ChildFrameLayoutCoordinationNode {
        instance_key,
        template_key,
        kind,
        alignment_scope,
        topology,
        slots,
        requirements,
    }))
}

fn facet_layout_semantic_tag(facet_band: &FacetBandCoordMeasurement) -> String {
    format!(
        "facet:{}:{}:{}",
        facet_band.axis.coordination_key_prefix(),
        facet_band.facet_depth,
        facet_band.coordination_field_identity
    )
}

fn facet_layout_coordination_shape(facet_band: &FacetBandCoordMeasurement) -> GridShape {
    match facet_band.axis {
        FacetAxis::Column => GridShape {
            rows: 1,
            columns: facet_band.cells.len().max(1),
        },
        FacetAxis::Row => GridShape {
            rows: facet_band.cells.len().max(1),
            columns: 1,
        },
    }
}

fn facet_layout_slot(axis: FacetAxis, child_index: usize) -> GridSlotRect {
    match axis {
        FacetAxis::Column => GridSlotRect {
            row: 0,
            column: child_index,
            row_span: 1,
            column_span: 1,
        },
        FacetAxis::Row => GridSlotRect {
            row: child_index,
            column: 0,
            row_span: 1,
            column_span: 1,
        },
    }
}

fn facet_layout_slots(
    facet_band: &FacetBandCoordMeasurement,
) -> Result<Vec<ChildFrameLayoutSlot>, AvengerChartError> {
    facet_band
        .cells
        .iter()
        .enumerate()
        .map(|(child_index, cell)| {
            Ok(ChildFrameLayoutSlot {
                child_index,
                child_key: facet_band.child_scope_key_for_cell(cell).child_key,
                slot: facet_layout_slot(facet_band.axis, child_index),
            })
        })
        .collect()
}

fn facet_grid_track_requirements(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
    shape: GridShape,
) -> Result<GridTrackRequirements, AvengerChartError> {
    let placement = resolve_facet_band_placement(measurement)?.ok_or_else(|| {
        AvengerChartError::InternalError(
            "Facet grid track requirements requested for non-facet measurement".to_string(),
        )
    })?;
    let mut requirements = facet_grid_track_requirements_from_placement(shape, &placement)?;
    let active_layout = facet_band
        .coordinated_layout
        .as_ref()
        .unwrap_or(&facet_band.local_layout);
    requirements.guide_slot_gap_px = active_layout
        .guide_slot_gap_px
        .max(facet_band.guide_padding_inner_px);
    Ok(requirements)
}

fn facet_grid_track_requirements_from_placement(
    shape: GridShape,
    placement: &FacetBandPlacement,
) -> Result<GridTrackRequirements, AvengerChartError> {
    let mut requirements = GridTrackRequirements {
        shape,
        guide_slot_gap_px: 0.0,
        column_outer_start: 0.0,
        column_outer_end: 0.0,
        row_outer_start: 0.0,
        row_outer_end: 0.0,
        column_widths: vec![0.0; shape.columns],
        row_heights: vec![0.0; shape.rows],
        column_left: vec![0.0; shape.columns],
        column_right: vec![0.0; shape.columns],
        row_top: vec![0.0; shape.rows],
        row_bottom: vec![0.0; shape.rows],
    };

    match placement.axis {
        FacetAxis::Column => {
            if let Some(first) = placement.cells.first() {
                requirements.column_outer_start = first.main_axis_start.max(0.0);
            }
            if let Some(last) = placement.cells.last() {
                requirements.column_outer_end =
                    (placement.main_axis_extent - last.main_axis_start - last.main_axis_size)
                        .max(0.0);
            }
            if let Some(height) = placement.cross_axis_extent {
                requirements.row_heights[0] = height;
            }
            for cell in &placement.cells {
                if cell.cell_index >= shape.columns {
                    return Err(AvengerChartError::InternalError(format!(
                        "Facet column placement cell index {} exceeded coordination columns {}",
                        cell.cell_index, shape.columns
                    )));
                }
                requirements.column_widths[cell.cell_index] = cell.main_axis_size;
            }
            for pair in placement.cells.windows(2) {
                let left = &pair[0];
                let right = &pair[1];
                let gap = right.main_axis_start - left.main_axis_start - left.main_axis_size;
                if left.cell_index + 1 < shape.columns {
                    requirements.column_right[left.cell_index] = gap.max(0.0);
                }
            }
        }
        FacetAxis::Row => {
            if let Some(first) = placement.cells.first() {
                requirements.row_outer_start = first.main_axis_start.max(0.0);
            }
            if let Some(last) = placement.cells.last() {
                requirements.row_outer_end =
                    (placement.main_axis_extent - last.main_axis_start - last.main_axis_size)
                        .max(0.0);
            }
            if let Some(width) = placement.cross_axis_extent {
                requirements.column_widths[0] = width;
            }
            for cell in &placement.cells {
                if cell.cell_index >= shape.rows {
                    return Err(AvengerChartError::InternalError(format!(
                        "Facet row placement cell index {} exceeded coordination rows {}",
                        cell.cell_index, shape.rows
                    )));
                }
                requirements.row_heights[cell.cell_index] = cell.main_axis_size;
            }
            for pair in placement.cells.windows(2) {
                let top = &pair[0];
                let bottom = &pair[1];
                let gap = bottom.main_axis_start - top.main_axis_start - top.main_axis_size;
                if top.cell_index + 1 < shape.rows {
                    requirements.row_bottom[top.cell_index] = gap.max(0.0);
                }
            }
        }
    }

    Ok(requirements)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FacetBandGridApplyUnsupported {
    EmptyBand,
    NestedFacetChild,
    ScaleBackedPlacement,
    TopologyMismatch,
}

pub(crate) fn facet_band_grid_apply_layout(
    facet_band: &FacetBandCoordMeasurement,
    requirements: &GridTrackRequirements,
) -> Result<CoordinatedLayout, FacetBandGridApplyUnsupported> {
    if facet_band.cells.is_empty() {
        return Err(FacetBandGridApplyUnsupported::EmptyBand);
    }

    if facet_band.cells.iter().any(|cell| {
        cell.measurement
            .coord_measurement
            .as_any()
            .is::<FacetBandCoordMeasurement>()
    }) {
        return Err(FacetBandGridApplyUnsupported::NestedFacetChild);
    }

    if !facet_band.uses_explicit_placement() {
        return Err(FacetBandGridApplyUnsupported::ScaleBackedPlacement);
    }

    let expected_shape = facet_layout_coordination_shape(facet_band);
    if requirements.shape != expected_shape {
        return Err(FacetBandGridApplyUnsupported::TopologyMismatch);
    }

    facet_grid_requirements_to_coordinated_layout(facet_band.axis, requirements)
}

fn facet_grid_requirements_to_coordinated_layout(
    axis: FacetAxis,
    requirements: &GridTrackRequirements,
) -> Result<CoordinatedLayout, FacetBandGridApplyUnsupported> {
    let (n, outer_start, outer_end, padding_inner_px) = match axis {
        FacetAxis::Column if requirements.shape.rows == 1 => (
            requirements.shape.columns,
            requirements.column_outer_start,
            requirements.column_outer_end,
            max_adjacent_gap(&requirements.column_right, &requirements.column_left),
        ),
        FacetAxis::Row if requirements.shape.columns == 1 => (
            requirements.shape.rows,
            requirements.row_outer_start,
            requirements.row_outer_end,
            max_adjacent_gap(&requirements.row_bottom, &requirements.row_top),
        ),
        _ => return Err(FacetBandGridApplyUnsupported::TopologyMismatch),
    };

    if n == 0 {
        return Err(FacetBandGridApplyUnsupported::EmptyBand);
    }

    Ok(CoordinatedLayout {
        padding_inner_px,
        guide_slot_gap_px: requirements.guide_slot_gap_px,
        outer_start,
        outer_end,
        n,
    })
}

fn max_adjacent_gap(trailing: &[f32], leading: &[f32]) -> f32 {
    if trailing.is_empty() || leading.is_empty() {
        return 0.0;
    }
    (0..trailing.len().saturating_sub(1))
        .map(|index| trailing[index].max(0.0) + leading[index + 1].max(0.0))
        .fold(0.0, f32::max)
}

pub(crate) fn apply_facet_band_grid_track_requirements(
    measurement: &mut ComponentsMeasurement,
    requirements: &GridTrackRequirements,
) -> Result<bool, AvengerChartError> {
    let Some(facet_band) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
    else {
        return Ok(false);
    };

    let layout = match facet_band_grid_apply_layout(facet_band, requirements) {
        Ok(layout) => layout,
        Err(reason) => {
            debug!(
                target: "avenger_chart::layout_coordination",
                ?reason,
                axis = ?facet_band.axis,
                cell_count = facet_band.cells.len(),
                "skipping facet-band layout alignment apply"
            );
            return Ok(false);
        }
    };

    let active_layout = facet_band
        .coordinated_layout
        .as_ref()
        .unwrap_or(&facet_band.local_layout);
    if coordinated_layout_values_equal(active_layout, &layout) {
        return Ok(false);
    }

    facet_band.set_coordinated_layout_value(layout);
    facet_band.recompute_explicit_placement_if_needed();
    Ok(true)
}

fn coordinated_layout_values_equal(left: &CoordinatedLayout, right: &CoordinatedLayout) -> bool {
    left.padding_inner_px == right.padding_inner_px
        && left.guide_slot_gap_px == right.guide_slot_gap_px
        && left.outer_start == right.outer_start
        && left.outer_end == right.outer_end
        && left.n == right.n
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

/// Summary of an alignment pass that mutates measured child-frame containers.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ChildFrameLayoutAlignmentApplyTrace {
    pub(crate) exported_node_count: usize,
    pub(crate) alignment_group_count: usize,
    pub(crate) planned_group_count: usize,
    pub(crate) applied_container_count: usize,
}

impl ChildFrameLayoutAlignmentApplyTrace {
    pub(crate) fn changed(&self) -> bool {
        self.applied_container_count > 0
    }
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

pub(crate) fn apply_child_frame_layout_alignment(
    measurement: &mut ComponentsMeasurement,
) -> Result<ChildFrameLayoutAlignmentApplyTrace, AvengerChartError> {
    let nodes = collect_child_frame_layout_coordination_nodes(measurement)?;
    let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
    let plans = alignment_solution_plans(&diagnostics);

    let mut trace = ChildFrameLayoutAlignmentApplyTrace {
        exported_node_count: diagnostics.exported_node_count,
        alignment_group_count: diagnostics.alignment_group_count,
        planned_group_count: plans.len(),
        applied_container_count: 0,
    };

    if !plans.is_empty() {
        apply_child_frame_layout_alignment_recursive(measurement, &plans, &mut trace)?;
    }

    if trace.exported_node_count > 0 {
        debug!(
            target: "avenger_chart::layout_coordination",
            exported_node_count = trace.exported_node_count,
            alignment_group_count = trace.alignment_group_count,
            planned_group_count = trace.planned_group_count,
            applied_container_count = trace.applied_container_count,
            "child-frame layout alignment apply"
        );
    }

    Ok(trace)
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

fn alignment_solution_plans(
    diagnostics: &ChildFrameLayoutAlignmentDiagnostics,
) -> IndexMap<LayoutAlignmentKey, ChildFrameLayoutRequirements> {
    diagnostics
        .merged_groups
        .iter()
        .filter(|group| layout_alignment_key_has_apply_adapter(&group.key))
        .filter(|group| group.node_deltas.iter().any(|delta| delta.has_delta()))
        .map(|group| (group.key.clone(), group.merged_requirements.clone()))
        .collect()
}

fn layout_alignment_key_has_apply_adapter(key: &LayoutAlignmentKey) -> bool {
    matches!(
        key.kind,
        ChildFrameContainerKind::FacetColumn
            | ChildFrameContainerKind::FacetRow
            | ChildFrameContainerKind::HConcat
            | ChildFrameContainerKind::VConcat
            | ChildFrameContainerKind::GridConcat
    )
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
            merged.guide_slot_gap_px = merged.guide_slot_gap_px.max(next.guide_slot_gap_px);
            merged.column_outer_start = merged.column_outer_start.max(next.column_outer_start);
            merged.column_outer_end = merged.column_outer_end.max(next.column_outer_end);
            merged.row_outer_start = merged.row_outer_start.max(next.row_outer_start);
            merged.row_outer_end = merged.row_outer_end.max(next.row_outer_end);
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
                + (merged.column_outer_start - local.column_outer_start).abs()
                + (merged.column_outer_end - local.column_outer_end).abs()
                + (merged.row_outer_start - local.row_outer_start).abs()
                + (merged.row_outer_end - local.row_outer_end).abs()
                + (merged.guide_slot_gap_px - local.guide_slot_gap_px).abs()
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

    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    {
        if let Some(node) = facet_band_layout_coordination_node(measurement, facet_band)? {
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

fn apply_child_frame_layout_alignment_recursive(
    measurement: &mut ComponentsMeasurement,
    plans: &IndexMap<LayoutAlignmentKey, ChildFrameLayoutRequirements>,
    trace: &mut ChildFrameLayoutAlignmentApplyTrace,
) -> Result<(), AvengerChartError> {
    if let Some(alignment_key) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
        .and_then(|concat| grid_layout_coordination_node(concat).transpose())
        .transpose()?
        .map(|node| node.alignment_key())
    {
        if let Some(ChildFrameLayoutRequirements::Grid(requirements)) = plans.get(&alignment_key) {
            if let Some(concat) = measurement
                .coord_measurement
                .as_any_mut()
                .downcast_mut::<ConcatCoordMeasurement>()
            {
                if concat.apply_grid_track_requirements(requirements)? {
                    trace.applied_container_count += 1;
                }
            }
        }
    }

    if let Some(concat) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<ConcatCoordMeasurement>()
    {
        for child in &mut concat.children {
            apply_child_frame_layout_alignment_recursive(&mut child.measurement, plans, trace)?;
        }
        return Ok(());
    }

    let facet_alignment_key = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
        .and_then(|facet_band| {
            facet_band_layout_coordination_node(measurement, facet_band).transpose()
        })
        .transpose()?
        .map(|node| node.alignment_key());

    if let Some(ChildFrameLayoutRequirements::Grid(requirements)) =
        facet_alignment_key.as_ref().and_then(|key| plans.get(key))
    {
        if apply_facet_band_grid_track_requirements(measurement, requirements)? {
            trace.applied_container_count += 1;
        }
    }

    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
    {
        for cell in &mut facet_band.cells {
            apply_child_frame_layout_alignment_recursive(&mut cell.measurement, plans, trace)?;
        }
        return Ok(());
    }

    if let Some(positioned) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<PositionedCoordMeasurement>()
    {
        for child in &mut positioned.children {
            apply_child_frame_layout_alignment_recursive(&mut child.measurement, plans, trace)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::placement::FacetCellPlacement;

    fn test_alignment_key(kind: ChildFrameContainerKind) -> LayoutAlignmentKey {
        LayoutAlignmentKey {
            template_key: ChildFrameContainerTemplateKey {
                container_path_template: Vec::new(),
                semantic_tag: Some(format!("{kind:?}")),
            },
            kind,
            topology: ChildFrameLayoutTopology::Grid {
                shape: GridShape {
                    rows: 1,
                    columns: 1,
                },
                slots: Vec::new(),
            },
        }
    }

    fn test_requirements(width: f32) -> ChildFrameLayoutRequirements {
        ChildFrameLayoutRequirements::Grid(GridTrackRequirements {
            shape: GridShape {
                rows: 1,
                columns: 1,
            },
            guide_slot_gap_px: 0.0,
            column_outer_start: 0.0,
            column_outer_end: 0.0,
            row_outer_start: 0.0,
            row_outer_end: 0.0,
            column_widths: vec![width],
            row_heights: vec![10.0],
            column_left: vec![0.0],
            column_right: vec![0.0],
            row_top: vec![0.0],
            row_bottom: vec![0.0],
        })
    }

    fn test_delta() -> ChildFrameLayoutRequirementDelta {
        ChildFrameLayoutRequirementDelta {
            instance_key: ChildFrameContainerInstanceKey::new(Vec::new()),
            track_delta: 1.0,
            slab_delta: 0.0,
        }
    }

    #[test]
    fn grid_requirement_merge_preserves_guide_slot_gap() {
        let mut left = test_requirements(10.0);
        let mut right = test_requirements(10.0);
        let ChildFrameLayoutRequirements::Grid(left_grid) = &mut left;
        left_grid.guide_slot_gap_px = 6.0;
        let ChildFrameLayoutRequirements::Grid(right_grid) = &mut right;
        right_grid.guide_slot_gap_px = 18.0;

        let merged =
            merge_child_frame_layout_requirements([&left, &right]).expect("compatible grids");
        let ChildFrameLayoutRequirements::Grid(merged_grid) = &merged;

        assert_eq!(merged_grid.guide_slot_gap_px, 18.0);
        assert_eq!(requirement_slab_delta(&left, &merged), 12.0);
        assert_eq!(requirement_slab_delta(&right, &merged), 0.0);
    }

    #[test]
    fn alignment_solution_plans_include_facet_groups_after_adapter_registration() {
        let facet_key = test_alignment_key(ChildFrameContainerKind::FacetColumn);
        let grid_key = test_alignment_key(ChildFrameContainerKind::GridConcat);
        let diagnostics = ChildFrameLayoutAlignmentDiagnostics {
            exported_node_count: 4,
            alignment_group_count: 2,
            merged_groups: vec![
                LayoutAlignmentGroupDiagnostics {
                    key: facet_key.clone(),
                    node_count: 2,
                    merged_requirements: test_requirements(20.0),
                    node_deltas: vec![test_delta()],
                },
                LayoutAlignmentGroupDiagnostics {
                    key: grid_key.clone(),
                    node_count: 2,
                    merged_requirements: test_requirements(30.0),
                    node_deltas: vec![test_delta()],
                },
            ],
            skipped_groups: Vec::new(),
        };

        let plans = alignment_solution_plans(&diagnostics);

        assert_eq!(plans.len(), 2);
        assert!(plans.contains_key(&facet_key));
        assert!(plans.contains_key(&grid_key));
    }

    #[test]
    fn facet_grid_requirements_preserve_outer_band_offsets() -> Result<(), AvengerChartError> {
        let placement = FacetBandPlacement::new(
            FacetAxis::Column,
            vec![
                FacetCellPlacement {
                    cell_index: 0,
                    main_axis_start: 3.0,
                    main_axis_size: 10.0,
                },
                FacetCellPlacement {
                    cell_index: 1,
                    main_axis_start: 20.0,
                    main_axis_size: 10.0,
                },
            ],
            35.0,
            Some(40.0),
        );
        let requirements = facet_grid_track_requirements_from_placement(
            GridShape {
                rows: 1,
                columns: 2,
            },
            &placement,
        )?;

        assert_eq!(requirements.column_outer_start, 3.0);
        assert_eq!(requirements.column_outer_end, 5.0);
        assert_eq!(requirements.guide_slot_gap_px, 0.0);
        assert_eq!(requirements.row_outer_start, 0.0);
        assert_eq!(requirements.row_outer_end, 0.0);
        assert_eq!(requirements.column_widths, vec![10.0, 10.0]);
        assert_eq!(requirements.row_heights, vec![40.0]);
        assert_eq!(requirements.column_right, vec![7.0, 0.0]);
        Ok(())
    }
}
