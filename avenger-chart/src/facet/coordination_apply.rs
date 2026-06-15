use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, facet_band_mut as facet_band_mut_from_coord,
            facet_band_ref as facet_band_ref_from_coord,
        },
        coordination_plans::{CoordinationNodeKey, RequirementPass},
        tree_solve::{CellSizeOverrides, CurrentFacetBandFrames, CurrentFacetChildFrame},
    },
    layout::{Size, TrackSpacing},
    plot::compiled::ComponentsMeasurement,
    render::{EvaluationContext, context::FacetRuntimeSizingMode},
};
use avenger_layout::{Distribute, Layout, SolveOptions, TrackSize};

pub(crate) fn visit_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_ref_from_coord(measurement.coord_measurement.as_ref()) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            visit_facet_bands_with_node_id(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn visit_facet_bands_with_node_id_mut<F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &mut FacetBandCoordMeasurement),
{
    if let Some(facet_band) = facet_band_mut_from_coord(measurement.coord_measurement.as_mut()) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            node_path.push(idx);
            visit_facet_bands_with_node_id_mut(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn apply_requirement_pass(
    measurement: &mut ComponentsMeasurement,
    requirement_pass: &RequirementPass,
) -> Result<(), AvengerChartError> {
    let solution = &requirement_pass.solution;
    let mut error = None;
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if error.is_some() {
                return;
            }
            // Construction guarantees the layout channel covers every
            // snapshot node, so a missing entry means this tree node was
            // not in the pass's snapshot.
            if !solution.layout_by_node.contains_key(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "requirement pass did not include node path {:?}",
                    node_id.path
                )));
                return;
            }
            facet_band.clear_realized_owned_legend_slabs();
            facet_band.set_coordination_solution(std::sync::Arc::clone(solution), node_id.clone());
        },
    );

    if let Some(error) = error {
        return Err(error);
    }

    Ok(())
}

/// Solve and install the current settled facet geometry.
///
/// Coordination channel values remain unchanged. The geometry handle is the
/// render/readback source for facet child frames, guide positions, and debug
/// projections, so every no-remeasure mutation boundary refreshes it once.
pub(crate) fn refresh_current_facet_geometry(
    measurement: &mut ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
) -> Result<(), AvengerChartError> {
    let mut overrides = CellSizeOverrides::new();
    let mut child_frames_by_node = std::collections::HashMap::new();
    let mut node_path = Vec::new();
    collect_current_facet_cell_geometry(
        measurement,
        &mut node_path,
        &mut overrides,
        &mut child_frames_by_node,
    )?;

    let Some(lowered) = crate::facet::tree_solve::lower_settled_facet_tree_with_overrides(
        measurement,
        sizing,
        Some(&overrides),
    ) else {
        return Ok(());
    };
    let solved = lowered.solve().map_err(|error| {
        AvengerChartError::InternalError(format!("facet settled geometry solve failed: {error}"))
    })?;
    let geometry = std::sync::Arc::new(crate::facet::tree_solve::CurrentFacetGeometry {
        lowered,
        solution: solved,
        child_frames_by_node,
    });

    let mut error = None;
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if error.is_some() {
                return;
            }
            if geometry.lowered_band(node_id).is_none() {
                error = Some(AvengerChartError::InternalError(format!(
                    "settled geometry did not include node path {:?}",
                    node_id.path
                )));
                return;
            }
            facet_band.set_current_geometry(std::sync::Arc::clone(&geometry), node_id.clone());
        },
    );

    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

fn collect_current_facet_cell_geometry(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    overrides: &mut CellSizeOverrides,
    child_frames_by_node: &mut std::collections::HashMap<
        CoordinationNodeKey,
        CurrentFacetBandFrames,
    >,
) -> Result<(), AvengerChartError> {
    let Some(facet_band) = facet_band_ref_from_coord(measurement.coord_measurement.as_ref()) else {
        return Ok(());
    };

    let node_id = CoordinationNodeKey::new(node_path.clone());
    let placement = facet_band.resolved_placement_from_scale_specs(&measurement.scales)?;
    let child_frames =
        current_facet_band_frames_from_placement(measurement, facet_band, &placement)?;
    for cell_placement in &placement.cells {
        let Some(cell) = facet_band.cells.get(cell_placement.cell_index) else {
            return Err(AvengerChartError::InternalError(format!(
                "current facet geometry placement cell index {} exceeded cell count {}",
                cell_placement.cell_index,
                facet_band.cells.len()
            )));
        };
        overrides.insert(
            (node_id.clone(), cell_placement.cell_index),
            render_sized_facet_cell(facet_band.axis, cell_placement.main_size, cell),
        );
    }
    child_frames_by_node.insert(node_id, child_frames);

    for (idx, child) in facet_band.child_measurements_iter().enumerate() {
        node_path.push(idx);
        collect_current_facet_cell_geometry(child, node_path, overrides, child_frames_by_node)?;
        node_path.pop();
    }

    Ok(())
}

fn render_sized_facet_cell(
    axis: FacetAxis,
    band_size: f32,
    cell: &crate::facet::coord::FacetCellRuntime,
) -> Size {
    match axis {
        FacetAxis::Column => Size::new(
            band_size.max(0.0),
            cell.measurement.plot_area_height.max(0.0),
        ),
        FacetAxis::Row => Size::new(
            cell.measurement.plot_area_width.max(0.0),
            band_size.max(0.0),
        ),
    }
}

fn current_facet_band_frames_from_placement(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
    placement: &crate::facet::placement::FacetBandPlacement,
) -> Result<CurrentFacetBandFrames, AvengerChartError> {
    if placement.axis != facet_band.axis {
        return Err(AvengerChartError::InternalError(format!(
            "Facet current geometry axis mismatch: placement={:?}, measurement={:?}",
            placement.axis, facet_band.axis
        )));
    }
    if placement.cells.len() != facet_band.cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet current geometry count mismatch: placement={}, cells={}",
            placement.cells.len(),
            facet_band.cells.len()
        )));
    }
    if facet_band.cells.is_empty() {
        return Ok(CurrentFacetBandFrames {
            content_size: Size::default(),
            children: Vec::new(),
        });
    }

    let children = placement
        .cells
        .iter()
        .map(|cell_placement| {
            let cell = facet_band
                .cells
                .get(cell_placement.cell_index)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Facet current geometry placement cell index {} exceeded cell count {}",
                        cell_placement.cell_index,
                        facet_band.cells.len()
                    ))
                })?;
            Ok(Layout::<usize, ()>::leaf(render_sized_facet_cell(
                facet_band.axis,
                cell_placement.main_size,
                cell,
            )))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    let spacing = TrackSpacing {
        outer_start: facet_band.active_layout().outer_start,
        outer_end: facet_band.active_layout().outer_end,
        min_gap: facet_band.active_layout().padding_inner_px,
    };
    let fixed_tracks = placement
        .cells
        .iter()
        .map(|cell| TrackSize::Fixed(cell.main_size.max(0.0)))
        .collect::<Vec<_>>();
    // Child-frame content follows the resolved band packing exactly. The
    // settled tree solve still owns edge grants; those are attached later as
    // render targets instead of shifting child origins here.
    let render_layout = match facet_band.axis {
        FacetAxis::Column => Layout::row(children)
            .column_spacing(spacing)
            .columns(fixed_tracks)
            .distribute_x(Distribute::Start),
        FacetAxis::Row => Layout::column(children)
            .row_spacing(spacing)
            .rows(fixed_tracks)
            .distribute_y(Distribute::Start),
    };
    let solution = render_layout
        .solve(&SolveOptions {
            width: Some(measurement.plot_area_width.max(0.0)),
            height: Some(measurement.plot_area_height.max(0.0)),
        })
        .map_err(|error| {
            AvengerChartError::InternalError(format!(
                "Facet current child-frame projection solve failed: {error}"
            ))
        })?;
    let root = solution.regions().next().ok_or_else(|| {
        AvengerChartError::InternalError("Facet current child-frame projection was empty".into())
    })?;
    let avenger_layout::RegionDetail::Grid { tracks } = &root.detail else {
        return Err(AvengerChartError::InternalError(
            "Facet current child-frame projection root was not a grid".into(),
        ));
    };
    let content_rect = tracks
        .content_rect_for_slot(full_current_frame_grid_slot(tracks))
        .unwrap_or(root.content);
    let (offset_x, offset_y) = facet_cell_cross_axis_offset(facet_band);
    let children = placement
        .cells
        .iter()
        .map(|cell| {
            let rect = tracks
                .content_rect_for_slot(current_frame_cell_grid_slot(
                    facet_band.axis,
                    cell.cell_index,
                ))
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Facet current child-frame projection missing cell {}",
                        cell.cell_index
                    ))
                })?;
            Ok(CurrentFacetChildFrame {
                child_index: cell.cell_index,
                rect: avenger_layout::Rect::new(
                    rect.x + offset_x,
                    rect.y + offset_y,
                    rect.width,
                    rect.height,
                ),
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    Ok(CurrentFacetBandFrames {
        content_size: Size::new(content_rect.width, content_rect.height),
        children,
    })
}

fn full_current_frame_grid_slot(tracks: &avenger_layout::SolvedTracks) -> avenger_layout::GridSlot {
    avenger_layout::GridSlot {
        row: 0,
        column: 0,
        row_span: tracks.row_sizes.len(),
        column_span: tracks.column_sizes.len(),
    }
}

fn current_frame_cell_grid_slot(axis: FacetAxis, cell_index: usize) -> avenger_layout::GridSlot {
    match axis {
        FacetAxis::Column => avenger_layout::GridSlot {
            row: 0,
            column: cell_index,
            row_span: 1,
            column_span: 1,
        },
        FacetAxis::Row => avenger_layout::GridSlot {
            row: cell_index,
            column: 0,
            row_span: 1,
            column_span: 1,
        },
    }
}

fn facet_cell_cross_axis_offset(facet_band: &FacetBandCoordMeasurement) -> (f32, f32) {
    let slabs = crate::facet::overflow_projection::FacetOverflowSlabs::from_coordinated(
        facet_band.active_overflow(),
    );
    match facet_band.axis {
        FacetAxis::Column => (0.0, slabs.legend.top),
        FacetAxis::Row => (slabs.legend.left, 0.0),
    }
}

/// Geometry adoption: walk the measurement tree parent-first and move
/// every band to the operating point its installed coordination solution
/// implies. Per band: the band scale range follows the container's main
/// extent, `subplot_cross_size` becomes the bandwidth of the band scale
/// at the active coordinated layout, and each cell's plot area adopts
/// the band-axis bandwidth plus the legend-shrunk orthogonal extent —
/// all policy-gated (content-driven axes keep their measured sizes) and
/// applied through the no-remeasure substrate, which also rewrites
/// nested scale-backed band scales when a cell's size moves. Measured
/// chrome stays frozen (the staleness law): adoption moves geometry,
/// never re-measures.
pub(crate) fn run_adopt(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<AdoptTrace, AvengerChartError> {
    let mut trace = AdoptTrace::default();
    adopt_recursive(measurement, eval_ctx, &mut trace)?;
    Ok(trace)
}

/// Per-run adoption counters for driver logs and tests.
#[derive(Debug, Clone, Default)]
pub(crate) struct AdoptTrace {
    pub(crate) bands: usize,
    pub(crate) cells_adopted: usize,
    pub(crate) cells_unchanged: usize,
}

/// Domain-recompute seam (the staleness law, D5): scale domains are
/// data-driven today, so moving plot areas and ranges recomputes
/// nothing — ranges follow geometry, domains hold. If a domain ever
/// depends on adopted geometry (e.g. pixel-snapped or density-driven
/// domains), the recompute belongs here: after this band's geometry
/// adoption, before its cells' subtrees adopt. Chrome and measured
/// overflow stay frozen at their epoch measurements within an
/// iteration; `max_iterations > 0` exists to re-measure them.
fn adopt_domain_recompute_seam(_facet_band: &mut FacetBandCoordMeasurement) {}

fn adopt_recursive(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    trace: &mut AdoptTrace,
) -> Result<(), AvengerChartError> {
    crate::coords::apply_coord_measurement_scale_adjustments(
        measurement.coord_measurement.as_ref(),
        &mut measurement.scales,
    );

    let plot_area_width = measurement.plot_area_width;
    let plot_area_height = measurement.plot_area_height;
    let scales = &mut measurement.scales;
    if let Some(facet_band) = facet_band_mut_from_coord(measurement.coord_measurement.as_mut()) {
        trace.bands += 1;
        let cell_sizes_before: Vec<(f32, f32)> = facet_band
            .cells
            .iter()
            .map(|cell| {
                (
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                )
            })
            .collect();
        facet_band.retarget_parent_plot_area_policy_no_remeasure(
            scales,
            eval_ctx,
            plot_area_width,
            plot_area_height,
        )?;
        adopt_domain_recompute_seam(facet_band);
        for (idx, (before_width, before_height)) in cell_sizes_before.into_iter().enumerate() {
            let cell = &mut facet_band.cells[idx];
            let changed = (cell.measurement.plot_area_width - before_width).abs() > 0.01
                || (cell.measurement.plot_area_height - before_height).abs() > 0.01;
            if changed {
                trace.cells_adopted += 1;
            } else {
                trace.cells_unchanged += 1;
            }
            adopt_recursive(&mut cell.measurement, eval_ctx, trace)?;
        }
        facet_band.realize_coordinated_child_frame_allocations();
    }
    Ok(())
}
