use crate::{
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, facet_band_mut as facet_band_mut_from_coord,
            facet_band_ref as facet_band_ref_from_coord,
        },
        coordination_plans::{CoordinationNodeKey, RequirementPass},
    },
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

/// Replace the installed coordination solution's retained geometry with a
/// solve from the post-adoption measurement state.
///
/// The channel values remain the requirement pass values installed before
/// adoption; only the retained `LayoutSolution` used for geometry readback is
/// refreshed. This keeps render readback aligned with the scales and child
/// plot areas that adoption just retargeted.
pub(crate) fn refresh_retained_geometry_after_adopt(
    measurement: &mut ComponentsMeasurement,
    sizing: FacetRuntimeSizingMode,
) -> Result<(), AvengerChartError> {
    let Some(lowered) = crate::facet::tree_solve::lower_settled_facet_tree(measurement, sizing)
    else {
        return Ok(());
    };
    let solved = lowered.solve().map_err(|error| {
        AvengerChartError::InternalError(format!("facet settled geometry solve failed: {error}"))
    })?;
    let retained = std::sync::Arc::new(crate::facet::tree_solve::RetainedFacetSolve {
        lowered,
        solution: solved,
    });

    let mut base_solution = None;
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |_node_id, _depth, facet_band| {
            if base_solution.is_none()
                && let Some((solution, _)) = facet_band.coordination_solution_handle()
            {
                base_solution = Some(solution);
            }
        },
    );

    let Some(base_solution) = base_solution else {
        return Ok(());
    };
    let mut updated_solution = (*base_solution).clone();
    updated_solution.retained = Some(retained);
    let updated_solution = std::sync::Arc::new(updated_solution);

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
            if !updated_solution.layout_by_node.contains_key(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "settled geometry solution did not include node path {:?}",
                    node_id.path
                )));
                return;
            }
            facet_band.set_coordination_solution(
                std::sync::Arc::clone(&updated_solution),
                node_id.clone(),
            );
        },
    );

    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
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
