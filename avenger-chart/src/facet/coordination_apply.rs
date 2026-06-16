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
    layout::Size,
    plot::compiled::ComponentsMeasurement,
    render::{EvaluationContext, context::FacetRuntimeSizingMode},
};

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
    let geometry = facet_band
        .current_geometry_for_parent(measurement.plot_area_width, measurement.plot_area_height);
    let child_frames = current_facet_band_frames_from_geometry(measurement, facet_band, &geometry)?;
    for cell_geometry in &geometry.cells {
        let Some(cell) = facet_band.cells.get(cell_geometry.cell_index) else {
            return Err(AvengerChartError::InternalError(format!(
                "current facet geometry cell index {} exceeded cell count {}",
                cell_geometry.cell_index,
                facet_band.cells.len()
            )));
        };
        overrides.insert(
            (node_id.clone(), cell_geometry.cell_index),
            render_sized_facet_cell(facet_band.axis, cell_geometry.main_size, cell),
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

fn current_facet_band_frames_from_geometry(
    measurement: &ComponentsMeasurement,
    facet_band: &FacetBandCoordMeasurement,
    geometry: &crate::facet::placement::FacetBandGeometry,
) -> Result<CurrentFacetBandFrames, AvengerChartError> {
    if geometry.axis != facet_band.axis {
        return Err(AvengerChartError::InternalError(format!(
            "Facet current geometry axis mismatch: geometry={:?}, measurement={:?}",
            geometry.axis, facet_band.axis
        )));
    }
    if geometry.cells.len() != facet_band.cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet current geometry count mismatch: geometry={}, cells={}",
            geometry.cells.len(),
            facet_band.cells.len()
        )));
    }
    if facet_band.cells.is_empty() {
        return Ok(CurrentFacetBandFrames {
            content_size: Size::default(),
            children: Vec::new(),
        });
    }

    let cells = geometry
        .cells
        .iter()
        .map(|cell_geometry| {
            let cell = facet_band
                .cells
                .get(cell_geometry.cell_index)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Facet current geometry cell index {} exceeded cell count {}",
                        cell_geometry.cell_index,
                        facet_band.cells.len()
                    ))
                })?;
            let size = render_sized_facet_cell(facet_band.axis, cell_geometry.main_size, cell);
            Ok((cell_geometry.cell_index, cell_geometry.main_start, size))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    // Child-frame content follows the resolved band packing exactly. The
    // settled tree solve still owns edge grants; those are attached later as
    // render targets instead of shifting child origins here.
    let main_extent = geometry.main_extent;
    let natural_cross_extent = cells
        .iter()
        .map(|(_, _, size)| match facet_band.axis {
            FacetAxis::Column => size.height,
            FacetAxis::Row => size.width,
        })
        .fold(0.0f32, f32::max);
    let target_cross_extent = match facet_band.axis {
        FacetAxis::Column => measurement.plot_area_height,
        FacetAxis::Row => measurement.plot_area_width,
    }
    .max(0.0);
    let cross_extent = geometry
        .cross_extent
        .unwrap_or(natural_cross_extent)
        .max(natural_cross_extent)
        .max(target_cross_extent);
    let (offset_x, offset_y) = facet_cell_cross_axis_offset(facet_band);
    let children = cells
        .into_iter()
        .map(|(child_index, main_start, size)| {
            let main_size = match facet_band.axis {
                FacetAxis::Column => size.width,
                FacetAxis::Row => size.height,
            };
            let rect = match facet_band.axis {
                FacetAxis::Column => avenger_layout::Rect::new(
                    main_start + offset_x,
                    offset_y,
                    main_size,
                    cross_extent,
                ),
                FacetAxis::Row => avenger_layout::Rect::new(
                    offset_x,
                    main_start + offset_y,
                    cross_extent,
                    main_size,
                ),
            };
            CurrentFacetChildFrame { child_index, rect }
        })
        .collect::<Vec<_>>();

    Ok(CurrentFacetBandFrames {
        content_size: match facet_band.axis {
            FacetAxis::Column => Size::new(main_extent, cross_extent),
            FacetAxis::Row => Size::new(cross_extent, main_extent),
        },
        children,
    })
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
/// implies. Per band: uniform cell sizes come from the active layout and
/// parent plot extent, and each cell's plot area adopts that band-axis
/// size plus the legend-shrunk orthogonal extent. Content-driven axes keep
/// their measured sizes. Measured chrome stays frozen (the staleness law):
/// adoption moves geometry, never re-measures.
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
    let plot_area_width = measurement.plot_area_width;
    let plot_area_height = measurement.plot_area_height;
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
