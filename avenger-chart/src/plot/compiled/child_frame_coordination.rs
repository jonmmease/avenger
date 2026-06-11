//! Layout coordination nodes for measured child-frame containers.
//!
//! These nodes describe measured container requirements. The alignment pass can
//! group compatible nodes, merge their requirements, and apply aligned
//! solutions through concrete container adapters.

use indexmap::IndexMap;
use tracing::debug;

use crate::{
    concat::ConcatCoordMeasurement,
    coords::FacetAxis,
    facet::{
        coord::FacetBandCoordMeasurement,
        overflow_projection::{FacetBoundaryDemand, rendered_boundary_demand_for_measurement},
        placement::{FacetBandPlacement, resolve_facet_band_placement},
    },
    layout::{
        AlignmentNode, EdgeDemand, GridShape, GridSlot, SingletonPolicy, SkippedGroupReason,
        TrackSpacing, align_by,
    },
    plot::compiled::{ChildFrameKey, ComponentsMeasurement, ContainerPathSegment},
    positioned_subplot::PositionedCoordMeasurement,
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

    fn from_facet_band_instance(
        instance_key: &ChildFrameContainerInstanceKey,
        scope: LayoutCoordinationScope,
        semantic_tag: String,
    ) -> Self {
        let container_path_template = match scope {
            LayoutCoordinationScope::TemplatePathWithoutFacetSegments => {
                facet_band_layout_alignment_template_path(&instance_key.container_path)
            }
        };
        Self {
            container_path_template,
            semantic_tag: Some(semantic_tag),
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

fn facet_band_layout_alignment_template_path(
    path: &[ContainerPathSegment],
) -> Vec<ContainerPathSegment> {
    let mut template = layout_alignment_template_path(path);
    if matches!(
        path.last(),
        Some(ContainerPathSegment::ConcatChild { key, .. })
            if key.as_deref().and_then(repeat_template_key).is_none()
    ) {
        template.pop();
    }
    template
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
    pub(crate) slot: GridSlot,
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
    pub(crate) slot: GridSlot,
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
    /// The member grid for a coordination group solve. Present for concat
    /// containers (whose alignment solves cousins together on a share key);
    /// `None` for facet bands, which apply merged requirement values.
    pub(crate) member: Option<GridMemberSpec>,
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

/// Neutral grid requirements plus chart-only side-car state.
use crate::layout::concat_grid::{
    ChartGridData, GridMemberSpec, SolvedConcatGrid, solve_concat_grid_group,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChartGridRequirements {
    pub(crate) grid: ChartGridData,
    /// Chart-adapter facet guide slot gap needed when generic grid
    /// requirements are projected back into `CoordinatedLayout`.
    ///
    /// Concat containers do not use this value, so their local requirements
    /// keep it at zero. Keeping it here avoids losing chart guide-spacing
    /// state when facet bands participate in the generic layout model.
    pub(crate) guide_slot_gap_px: f32,
}

/// Local requirements exported by a measured child-frame container.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChildFrameLayoutRequirements {
    Grid(ChartGridRequirements),
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
    let requirements = ChildFrameLayoutRequirements::Grid(ChartGridRequirements {
        grid: concat.grid_requirements()?,
        guide_slot_gap_px: 0.0,
    });
    let kind = match concat.band_direction() {
        Some(crate::layout::Orientation::Horizontal) => ChildFrameContainerKind::HConcat,
        Some(crate::layout::Orientation::Vertical) => ChildFrameContainerKind::VConcat,
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
        member: Some(concat.grid_member_spec()?),
    }))
}

pub(crate) fn facet_band_layout_coordination_node(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
) -> Result<Option<ChildFrameLayoutCoordinationNode>, AvengerChartError> {
    let shape = facet_layout_coordination_shape(facet_band);
    let instance_key = ChildFrameContainerInstanceKey::new(facet_band.scope_path_prefix.clone());
    let alignment_scope = LayoutCoordinationScope::TemplatePathWithoutFacetSegments;
    let template_key = ChildFrameContainerTemplateKey::from_facet_band_instance(
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
    let requirements = ChildFrameLayoutRequirements::Grid(facet_grid_requirements(
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
        member: None,
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

fn facet_layout_slot(axis: FacetAxis, child_index: usize) -> GridSlot {
    match axis {
        FacetAxis::Column => GridSlot {
            row: 0,
            column: child_index,
            row_span: 1,
            column_span: 1,
        },
        FacetAxis::Row => GridSlot {
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

fn facet_grid_requirements(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
    shape: GridShape,
) -> Result<ChartGridRequirements, AvengerChartError> {
    let placement = resolve_facet_band_placement(measurement)?.ok_or_else(|| {
        AvengerChartError::InternalError(
            "Facet grid track requirements requested for non-facet measurement".to_string(),
        )
    })?;
    let active_layout = facet_band.active_layout();
    let cell_boundaries = facet_band
        .cells
        .iter()
        .map(|cell| rendered_boundary_demand_for_measurement(facet_band.axis, &cell.measurement))
        .collect::<Vec<_>>();
    let grid = facet_grid_requirements_from_placement(
        shape,
        &placement,
        active_layout.padding_inner_px,
        &cell_boundaries,
    )?;
    Ok(ChartGridRequirements {
        grid,
        guide_slot_gap_px: active_layout
            .guide_slot_gap_px
            .max(facet_band.guide_padding_inner_px),
    })
}

/// Export facet-band layout state as grid requirements.
///
/// The export keeps padding policy and boundary chrome separate: the band's
/// `padding_inner_px` becomes the main-axis `min_gap`, and each cell's raw
/// rendered boundary demand becomes that track's edge demand. The exported
/// `min_gap` is deliberately not floored by `MIN_SUBPLOT_MAIN_GAP`: facet
/// requirements are only merged and projected back into `CoordinatedLayout`
/// (never solved), and the floor is applied where placement is computed, in
/// `compute_explicit_facet_band_placement`. Exporting the raw value keeps
/// export -> apply -> export a fixed point.
fn facet_grid_requirements_from_placement(
    shape: GridShape,
    placement: &FacetBandPlacement,
    min_gap: f32,
    cell_boundaries: &[FacetBoundaryDemand],
) -> Result<ChartGridData, AvengerChartError> {
    let mut requirements = ChartGridData {
        shape,
        column_spacing: TrackSpacing::default(),
        row_spacing: TrackSpacing::default(),
        column_widths: vec![0.0; shape.columns],
        row_heights: vec![0.0; shape.rows],
        column_left: vec![EdgeDemand::default(); shape.columns],
        column_right: vec![EdgeDemand::default(); shape.columns],
        row_top: vec![EdgeDemand::default(); shape.rows],
        row_bottom: vec![EdgeDemand::default(); shape.rows],
    };

    match placement.axis {
        FacetAxis::Column => {
            requirements.column_spacing.min_gap = min_gap;
            if let Some(first) = placement.cells.first() {
                requirements.column_spacing.outer_start = first.main_start.max(0.0);
            }
            if let Some(last) = placement.cells.last() {
                requirements.column_spacing.outer_end =
                    (placement.main_extent - last.main_start - last.main_size).max(0.0);
            }
            if let Some(height) = placement.cross_extent {
                requirements.row_heights[0] = height;
            }
            for cell in &placement.cells {
                if cell.cell_index >= shape.columns {
                    return Err(AvengerChartError::InternalError(format!(
                        "Facet column placement cell index {} exceeded coordination columns {}",
                        cell.cell_index, shape.columns
                    )));
                }
                requirements.column_widths[cell.cell_index] = cell.main_size;
            }
            for (index, boundary) in cell_boundaries.iter().enumerate().take(shape.columns) {
                requirements.column_left[index] = EdgeDemand::total(boundary.before);
                requirements.column_right[index] = EdgeDemand::total(boundary.after);
            }
        }
        FacetAxis::Row => {
            requirements.row_spacing.min_gap = min_gap;
            if let Some(first) = placement.cells.first() {
                requirements.row_spacing.outer_start = first.main_start.max(0.0);
            }
            if let Some(last) = placement.cells.last() {
                requirements.row_spacing.outer_end =
                    (placement.main_extent - last.main_start - last.main_size).max(0.0);
            }
            if let Some(width) = placement.cross_extent {
                requirements.column_widths[0] = width;
            }
            for cell in &placement.cells {
                if cell.cell_index >= shape.rows {
                    return Err(AvengerChartError::InternalError(format!(
                        "Facet row placement cell index {} exceeded coordination rows {}",
                        cell.cell_index, shape.rows
                    )));
                }
                requirements.row_heights[cell.cell_index] = cell.main_size;
            }
            for (index, boundary) in cell_boundaries.iter().enumerate().take(shape.rows) {
                requirements.row_top[index] = EdgeDemand::total(boundary.before);
                requirements.row_bottom[index] = EdgeDemand::total(boundary.after);
            }
        }
    }

    Ok(requirements)
}

// Facet bands export layout coordination nodes for alignment DIAGNOSTICS
// only. The value-apply adapter that once pushed merged grid requirements
// back into a band (`apply_facet_band_grid_requirements` and friends) was
// unreachable from any end-to-end chart spec — explicit placement exists
// only under a leaf-plot-sized facet root, which cannot also be a concat
// child, and in-chart facet cousins are equalized by the coordination
// rounds before alignment runs — so it was deleted with the coordinated
// value stores. The `concat_grid_facet_track_alignment` visual baseline
// pins the boundary rendering.

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
    let concat_solutions = solve_planned_concat_groups(&nodes, &plans)?;

    let mut trace = ChildFrameLayoutAlignmentApplyTrace {
        exported_node_count: diagnostics.exported_node_count,
        alignment_group_count: diagnostics.alignment_group_count,
        planned_group_count: plans.len(),
        applied_container_count: 0,
    };

    if !plans.is_empty() {
        apply_child_frame_layout_alignment_recursive(
            measurement,
            &plans,
            &concat_solutions,
            &mut trace,
        )?;
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
    let alignment_nodes = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let ChildFrameLayoutRequirements::Grid(chart_grid) = &node.requirements;
            AlignmentNode {
                id: index,
                group_key: node.alignment_key(),
                requirements: chart_grid.clone(),
            }
        })
        .collect::<Vec<_>>();

    // The payload is the chart requirement (grid plus guide-gap side-car);
    // the merge and delta closures own the side-car laws so no per-group
    // member fold is needed afterwards.
    let plan = align_by(
        &alignment_nodes,
        SingletonPolicy::Skip,
        |members: &[&ChartGridRequirements]| {
            let grid = crate::layout::alignment::merged_grid_requirements(
                members.iter().map(|member| &member.grid),
            )?;
            let guide_slot_gap_px = members
                .iter()
                .map(|member| member.guide_slot_gap_px)
                .fold(0.0f32, f32::max);
            Some(ChartGridRequirements {
                grid,
                guide_slot_gap_px,
            })
        },
        |local, merged| {
            (
                crate::layout::alignment::grid_content_delta(&local.grid, &merged.grid),
                crate::layout::alignment::grid_edge_delta(&local.grid, &merged.grid)
                    + (merged.guide_slot_gap_px - local.guide_slot_gap_px).abs(),
            )
        },
    );
    let exported_node_count = plan.node_count;
    let alignment_group_count = plan.group_count();

    let merged_groups = plan
        .groups
        .iter()
        .map(|group| {
            let node_deltas = group
                .deltas
                .iter()
                .map(|delta| ChildFrameLayoutRequirementDelta {
                    instance_key: nodes[delta.id].instance_key.clone(),
                    track_delta: delta.content_delta,
                    slab_delta: delta.edge_delta,
                })
                .collect();

            LayoutAlignmentGroupDiagnostics {
                key: group.key.clone(),
                node_count: group.deltas.len(),
                merged_requirements: ChildFrameLayoutRequirements::Grid(group.merged.clone()),
                node_deltas,
            }
        })
        .collect();

    let skipped_groups = plan
        .skipped
        .into_iter()
        .map(|skipped| LayoutAlignmentSkippedGroup {
            key: skipped.key,
            node_count: skipped.node_count,
            reason: match skipped.reason {
                SkippedGroupReason::Singleton => LayoutAlignmentSkippedReason::Singleton,
                SkippedGroupReason::IncompatibleRequirements => {
                    LayoutAlignmentSkippedReason::IncompatibleRequirements
                }
            },
        })
        .collect();

    ChildFrameLayoutAlignmentDiagnostics {
        exported_node_count,
        alignment_group_count,
        merged_groups,
        skipped_groups,
    }
}

/// Solve every planned concat-kind group on a real share key: cousins are
/// rebuilt from their member specs and solved together, and each member's
/// extracted solution is keyed by its instance for the apply walk. Facet
/// bands stay on the merged-requirements path (they apply values, not
/// placements).
fn solve_planned_concat_groups(
    nodes: &[ChildFrameLayoutCoordinationNode],
    plans: &IndexMap<LayoutAlignmentKey, ChildFrameLayoutRequirements>,
) -> Result<IndexMap<ChildFrameContainerInstanceKey, SolvedConcatGrid>, AvengerChartError> {
    let mut solutions = IndexMap::new();
    for key in plans.keys() {
        if !matches!(
            key.kind,
            ChildFrameContainerKind::HConcat
                | ChildFrameContainerKind::VConcat
                | ChildFrameContainerKind::GridConcat
        ) {
            continue;
        }
        let members = nodes
            .iter()
            .filter(|node| &node.alignment_key() == key)
            .collect::<Vec<_>>();
        let specs = members
            .iter()
            .map(|node| {
                node.member.clone().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Concat coordination node is missing its grid member spec".to_string(),
                    )
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let solved = solve_concat_grid_group(&specs).map_err(AvengerChartError::InternalError)?;
        for (node, solution) in members.iter().zip(solved) {
            solutions.insert(node.instance_key.clone(), solution);
        }
    }
    Ok(solutions)
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

/// Only concat containers have an alignment apply adapter (their grid
/// solve installs solutions directly); facet bands export nodes for
/// diagnostics only.
fn layout_alignment_key_has_apply_adapter(key: &LayoutAlignmentKey) -> bool {
    matches!(
        key.kind,
        ChildFrameContainerKind::HConcat
            | ChildFrameContainerKind::VConcat
            | ChildFrameContainerKind::GridConcat
    )
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
        for placement in container.placement().placements() {
            let child = container.child_measurement(placement.id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame measurement for child index {} while collecting layout coordination nodes",
                    placement.id
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
    concat_solutions: &IndexMap<ChildFrameContainerInstanceKey, SolvedConcatGrid>,
    trace: &mut ChildFrameLayoutAlignmentApplyTrace,
) -> Result<(), AvengerChartError> {
    if let Some(instance_key) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
        .filter(|concat| concat.layout_coordination_shape().is_some())
        .map(|concat| Ok::<_, AvengerChartError>(concat.container_path()?))
        .transpose()?
        .map(ChildFrameContainerInstanceKey::new)
    {
        if let Some(solution) = concat_solutions.get(&instance_key) {
            if let Some(concat) = measurement
                .coord_measurement
                .as_any_mut()
                .downcast_mut::<ConcatCoordMeasurement>()
            {
                if concat.install_grid_solution(solution)? {
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
            apply_child_frame_layout_alignment_recursive(
                &mut child.measurement,
                plans,
                concat_solutions,
                trace,
            )?;
        }
        return Ok(());
    }

    if let Some(facet_band) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<FacetBandCoordMeasurement>()
    {
        for cell in &mut facet_band.cells {
            apply_child_frame_layout_alignment_recursive(
                &mut cell.measurement,
                plans,
                concat_solutions,
                trace,
            )?;
        }
        return Ok(());
    }

    if let Some(positioned) = measurement
        .coord_measurement
        .as_any_mut()
        .downcast_mut::<PositionedCoordMeasurement>()
    {
        for child in &mut positioned.children {
            apply_child_frame_layout_alignment_recursive(
                &mut child.measurement,
                plans,
                concat_solutions,
                trace,
            )?;
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
        ChildFrameLayoutRequirements::Grid(ChartGridRequirements {
            grid: ChartGridData {
                shape: GridShape {
                    rows: 1,
                    columns: 1,
                },
                column_spacing: TrackSpacing::default(),
                row_spacing: TrackSpacing::default(),
                column_widths: vec![width],
                row_heights: vec![10.0],
                column_left: vec![EdgeDemand::default(); 1],
                column_right: vec![EdgeDemand::default(); 1],
                row_top: vec![EdgeDemand::default(); 1],
                row_bottom: vec![EdgeDemand::default(); 1],
            },
            guide_slot_gap_px: 0.0,
        })
    }

    fn test_delta() -> ChildFrameLayoutRequirementDelta {
        ChildFrameLayoutRequirementDelta {
            instance_key: ChildFrameContainerInstanceKey::new(Vec::new()),
            track_delta: 1.0,
            slab_delta: 0.0,
        }
    }

    fn test_node(width: f32, guide_slot_gap_px: f32) -> ChildFrameLayoutCoordinationNode {
        let ChildFrameLayoutRequirements::Grid(mut chart_grid) = test_requirements(width);
        chart_grid.guide_slot_gap_px = guide_slot_gap_px;
        ChildFrameLayoutCoordinationNode {
            instance_key: ChildFrameContainerInstanceKey::new(Vec::new()),
            template_key: ChildFrameContainerTemplateKey {
                container_path_template: Vec::new(),
                semantic_tag: Some("test".to_string()),
            },
            kind: ChildFrameContainerKind::GridConcat,
            alignment_scope: LayoutCoordinationScope::TemplatePathWithoutFacetSegments,
            topology: ChildFrameLayoutTopology::Grid {
                shape: GridShape {
                    rows: 1,
                    columns: 1,
                },
                slots: Vec::new(),
            },
            slots: Vec::new(),
            requirements: ChildFrameLayoutRequirements::Grid(chart_grid),
            member: None,
        }
    }

    #[test]
    fn grid_requirement_merge_preserves_guide_slot_gap() {
        let nodes = vec![test_node(10.0, 6.0), test_node(10.0, 18.0)];

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);

        assert_eq!(diagnostics.exported_node_count, 2);
        assert_eq!(diagnostics.alignment_group_count, 1);
        assert_eq!(diagnostics.merged_groups.len(), 1);
        let group = &diagnostics.merged_groups[0];
        let ChildFrameLayoutRequirements::Grid(merged) = &group.merged_requirements;
        assert_eq!(
            merged.guide_slot_gap_px, 18.0,
            "side-car gap folds by max over the group's member IDs"
        );
        assert_eq!(group.node_deltas[0].slab_delta, 12.0);
        assert_eq!(group.node_deltas[1].slab_delta, 0.0);
    }

    #[test]
    fn alignment_solution_plans_skip_facet_groups_without_apply_adapter() {
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

        // Facet bands export nodes for diagnostics only; concat containers
        // are the only kinds with an apply adapter.
        assert_eq!(plans.len(), 1);
        assert!(!plans.contains_key(&facet_key));
        assert!(plans.contains_key(&grid_key));
    }

    fn two_cell_column_placement() -> FacetBandPlacement {
        FacetBandPlacement::new(
            FacetAxis::Column,
            vec![
                FacetCellPlacement {
                    cell_index: 0,
                    main_start: 3.0,
                    main_size: 10.0,
                },
                FacetCellPlacement {
                    cell_index: 1,
                    main_start: 20.0,
                    main_size: 10.0,
                },
            ],
            35.0,
            Some(40.0),
        )
    }

    #[test]
    fn facet_grid_requirements_preserve_outer_band_offsets() -> Result<(), AvengerChartError> {
        let placement = two_cell_column_placement();
        let boundaries = [
            FacetBoundaryDemand {
                before: 0.0,
                after: 7.0,
            },
            FacetBoundaryDemand {
                before: 2.0,
                after: 0.0,
            },
        ];
        let requirements = facet_grid_requirements_from_placement(
            GridShape {
                rows: 1,
                columns: 2,
            },
            &placement,
            9.0,
            &boundaries,
        )?;

        assert_eq!(
            requirements.column_spacing,
            TrackSpacing {
                outer_start: 3.0,
                outer_end: 5.0,
                min_gap: 9.0,
            }
        );
        assert_eq!(requirements.row_spacing, TrackSpacing::default());
        assert_eq!(requirements.column_widths, vec![10.0, 10.0]);
        assert_eq!(requirements.row_heights, vec![40.0]);
        assert_eq!(
            requirements.column_left,
            vec![EdgeDemand::total(0.0), EdgeDemand::total(2.0)]
        );
        assert_eq!(
            requirements.column_right,
            vec![EdgeDemand::total(7.0), EdgeDemand::total(0.0)]
        );
        Ok(())
    }

    #[test]
    fn facet_grid_requirements_round_trip_preserves_coordinated_layout()
    -> Result<(), AvengerChartError> {
        let placement = two_cell_column_placement();
        let boundaries = [
            FacetBoundaryDemand {
                before: 1.0,
                after: 30.0,
            },
            FacetBoundaryDemand {
                before: 6.0,
                after: 0.0,
            },
        ];
        let requirements = facet_grid_requirements_from_placement(
            GridShape {
                rows: 1,
                columns: 2,
            },
            &placement,
            8.0,
            &boundaries,
        )?;

        assert_eq!(requirements.shape.columns, 2);
        assert_eq!(requirements.column_spacing.outer_start, 3.0);
        assert_eq!(requirements.column_spacing.outer_end, 5.0);
        assert_eq!(
            requirements.column_spacing.min_gap, 8.0,
            "padding policy must round trip exactly and not absorb boundary chrome"
        );
        Ok(())
    }

    #[test]
    fn merged_facet_requirements_take_max_min_gap_without_boundary_inflation()
    -> Result<(), AvengerChartError> {
        let placement = two_cell_column_placement();
        let shape = GridShape {
            rows: 1,
            columns: 2,
        };
        let narrow_boundaries = [FacetBoundaryDemand::default(); 2];
        let wide_boundaries = [
            FacetBoundaryDemand {
                before: 0.0,
                after: 18.0,
            },
            FacetBoundaryDemand {
                before: 12.0,
                after: 0.0,
            },
        ];
        let first =
            facet_grid_requirements_from_placement(shape, &placement, 14.0, &narrow_boundaries)?;
        let second =
            facet_grid_requirements_from_placement(shape, &placement, 8.0, &wide_boundaries)?;

        let merged = crate::layout::alignment::merged_grid_requirements([&first, &second])
            .expect("compatible facet grids");

        assert_eq!(
            merged.column_spacing.min_gap, 14.0,
            "merged padding is the max of paddings, not the adjacent boundary sum (30.0)"
        );
        assert_eq!(
            merged.column_right,
            vec![EdgeDemand::total(18.0), EdgeDemand::total(0.0)],
            "raw boundary demands merge independently of padding policy"
        );
        Ok(())
    }
}
