use std::{collections::HashMap, future::Future, pin::Pin};

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurementPlotAreaSized,
            facet_band_plot_area_sized_mut as facet_band_plot_area_sized_mut_from_coord,
            facet_band_plot_area_sized_ref as facet_band_plot_area_sized_ref_from_coord,
            retarget_scale_ranges_for_plot_area,
        },
        coordination_plans::{
            CoordinationNodeKey, FinalPropagationChildPlan, FinalPropagationNodePlan,
            FinalPropagationNodeTrace, FinalPropagationPlan, FinalPropagationTrace,
            InitialRequirementPass, RetargetNodePlan, RetargetNodeTrace, RetargetPlan,
            RetargetTrace, RetargetedRequirementPass,
        },
        layout_slabs::LayoutSlabs,
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

fn facet_band_ref(
    measurement: &ComponentsMeasurement,
) -> Option<&FacetBandCoordMeasurementPlotAreaSized> {
    facet_band_plot_area_sized_ref_from_coord(measurement.coord_measurement.as_ref())
}

fn facet_band_mut(
    measurement: &mut ComponentsMeasurement,
) -> Option<&mut FacetBandCoordMeasurementPlotAreaSized> {
    facet_band_plot_area_sized_mut_from_coord(measurement.coord_measurement.as_mut())
}

pub(crate) fn visit_plot_area_sized_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &FacetBandCoordMeasurementPlotAreaSized),
{
    if let Some(facet_band) = facet_band_ref(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            visit_plot_area_sized_facet_bands_with_node_id(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn visit_plot_area_sized_facet_bands_with_node_id_mut<F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &mut FacetBandCoordMeasurementPlotAreaSized),
{
    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, facet_band);
        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            node_path.push(idx);
            visit_plot_area_sized_facet_bands_with_node_id_mut(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn apply_initial_requirement_pass_plot_area_sized(
    measurement: &mut ComponentsMeasurement,
    initial_requirement_pass: &InitialRequirementPass,
) {
    let mut node_path = Vec::new();
    visit_plot_area_sized_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = initial_requirement_pass
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
                facet_band.recompute_plot_area_sized_placement();
            }
            if let Some(layout) = initial_requirement_pass
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_layout_value(layout);
                facet_band.recompute_plot_area_sized_placement();
            }
            if initial_requirement_pass
                .distribution
                .domain_target_nodes
                .contains(node_id)
                && !initial_requirement_pass
                    .distribution
                    .unified_domain_extents
                    .is_empty()
            {
                facet_band.distribute_coordinated_domain_extents(
                    &initial_requirement_pass.distribution.unified_domain_extents,
                );
            }
        },
    );
}

pub(crate) fn apply_retargeted_requirement_pass_plot_area_sized(
    measurement: &mut ComponentsMeasurement,
    retargeted_requirement_pass: &RetargetedRequirementPass,
) {
    let mut node_path = Vec::new();
    visit_plot_area_sized_facet_bands_with_node_id_mut(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            if let Some(overflow) = retargeted_requirement_pass
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_overflow_value(overflow);
                facet_band.recompute_plot_area_sized_placement();
            }
            if let Some(layout) = retargeted_requirement_pass
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.set_coordinated_layout_value(layout);
                facet_band.recompute_plot_area_sized_placement();
            }
        },
    );
}

pub(crate) fn build_retarget_plan_plot_area_sized(
    measurement: &ComponentsMeasurement,
) -> RetargetPlan {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_retarget_plan_plot_area_sized_recursive(measurement, &mut node_path, &mut node_plans);
    RetargetPlan { node_plans }
}

fn build_retarget_plan_plot_area_sized_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<RetargetNodePlan>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_retarget_plan_plot_area_sized_recursive(child, node_path, node_plans);
            node_path.pop();
        }

        let node_id = CoordinationNodeKey::new(node_path.clone());
        let mut apply_plan = facet_band.derive_coordinated_apply_plan();
        let slabs = LayoutSlabs::from_coordinated(&facet_band.coordinated_overflow);
        let (legend_main_start, legend_main_end) = match facet_band.axis {
            FacetAxis::Column => slabs.legend_vertical(),
            FacetAxis::Row => slabs.legend_horizontal(),
        };
        let has_main_axis_legend_slab = legend_main_start > 0.0 || legend_main_end > 0.0;
        debug_assert_eq!(
            apply_plan.has_legend_overflow, has_main_axis_legend_slab,
            "fixed retarget invariant: apply-plan legend-overflow flag must match coordinated main-axis legend slabs"
        );
        // Fixed leaf plot-area mode keeps per-cell plot area dimensions locked.
        // Keep coordinated layout/domain semantics intact, but disable
        // legend-driven main-size shrink for fixed leaf plot areas.
        apply_plan.adjusted_main_size = apply_plan.original_main_size;
        apply_plan.legend_main_axis_shrink = 0.0;
        let child_count = facet_band.child_measurements_iter().count();
        node_plans.push(RetargetNodePlan {
            node_id,
            axis: apply_plan.axis,
            has_legend_overflow: apply_plan.has_legend_overflow,
            has_coordinated_extents: apply_plan.has_coordinated_extents,
            remeasure_triggered: false,
            apply_plan,
            child_count,
        });
    }
}

pub(crate) fn run_retarget_with_trace_plot_area_sized<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    plan: &'a RetargetPlan,
) -> Pin<Box<dyn Future<Output = Result<RetargetTrace, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let plan_by_node: HashMap<CoordinationNodeKey, RetargetNodePlan> = plan
            .node_plans
            .iter()
            .cloned()
            .map(|node| (node.node_id.clone(), node))
            .collect();

        let mut node_results = Vec::new();
        let mut node_path = Vec::new();
        run_retarget_recursive(
            measurement,
            eval_ctx,
            &plan_by_node,
            &mut node_path,
            &mut node_results,
        )
        .await?;
        Ok(RetargetTrace { node_results })
    })
}

fn run_retarget_recursive<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    plan_by_node: &'a HashMap<CoordinationNodeKey, RetargetNodePlan>,
    node_path: &'a mut Vec<usize>,
    node_results: &'a mut Vec<RetargetNodeTrace>,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        if let Some(facet_band) = facet_band_mut(measurement) {
            let node_id = CoordinationNodeKey::new(node_path.clone());
            let planned = plan_by_node.get(&node_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing retarget plan for node path {:?}",
                    node_id.path
                ))
            })?;
            // Fixed leaf plot-area mode keeps leaf subplot sizes locked. Preserve planned
            // coordination semantics for diagnostics/tracing, but skip coordinated
            // cross-size rewrites during execution.
            let mut execution_plan = planned.apply_plan.clone();
            execution_plan.has_coordinated_layout = false;
            let outcome = facet_band
                .apply_coordinated_overflow_with_plan(eval_ctx, &execution_plan)
                .await?;
            facet_band.recompute_plot_area_sized_placement();
            let parent_cross_size = facet_band.coordinated_subplot_cross_size();
            let parent_axis = facet_band.axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
                if let Some(cross_size) = parent_cross_size
                    && let Some(child_facet_band) = child
                        .coord_measurement
                        .as_any_mut()
                        .downcast_mut::<FacetBandCoordMeasurementPlotAreaSized>(
                    )
                    && child_facet_band.axis == parent_axis
                {
                    child_facet_band.set_parent_bandwidth_value(cross_size);
                    parent_cross_size_propagated = true;
                }
                node_path.push(idx);
                run_retarget_recursive(child, eval_ctx, plan_by_node, node_path, node_results)
                    .await?;
                node_path.pop();
            }

            node_results.push(RetargetNodeTrace {
                node_id,
                axis: planned.axis,
                planned_has_legend_overflow: planned.has_legend_overflow,
                planned_has_coordinated_extents: planned.has_coordinated_extents,
                planned_remeasure_required: planned.remeasure_triggered,
                planned_axis_owner_ignore_empty_cells: planned
                    .apply_plan
                    .axis_owner_ignore_empty_cells,
                planned_adjusted_main_size: planned.apply_plan.adjusted_main_size,
                planned_child_count: planned.child_count,
                parent_cross_size_propagated,
                subplot_cross_size_before: outcome.subplot_cross_size_before,
                subplot_cross_size_after: outcome.subplot_cross_size_after,
                remeasure_triggered: outcome.remeasure_triggered,
                remeasured_cell_count: outcome.remeasured_cell_count,
                remeasure_skipped_cell_count: outcome.remeasure_skipped_cell_count,
                remeasured_non_empty_cell_count: outcome.remeasured_non_empty_cell_count,
                remeasured_with_coordinated_extents_count: outcome
                    .remeasured_with_coordinated_extents_count,
            });
        }
        Ok(())
    })
}

pub(crate) fn build_final_propagation_plan_plot_area_sized(
    measurement: &ComponentsMeasurement,
) -> FinalPropagationPlan {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_final_propagation_plan_plot_area_sized_recursive(
        measurement,
        &mut node_path,
        &mut node_plans,
    );
    FinalPropagationPlan { node_plans }
}

fn build_final_propagation_plan_plot_area_sized_recursive(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<FinalPropagationNodePlan>,
) {
    if let Some(facet_band) = facet_band_ref(measurement) {
        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_final_propagation_plan_plot_area_sized_recursive(child, node_path, node_plans);
            node_path.pop();
        }

        let parent_cross_size_target = facet_band.coordinated_subplot_cross_size();
        let child_plans = build_final_propagation_child_plans(
            facet_band.axis,
            parent_cross_size_target,
            facet_band.child_measurements_iter(),
        );
        let expected_plot_area_adjustments_count = child_plans
            .iter()
            .filter(|child_plan| child_plan.adjust_plot_area)
            .count();

        node_plans.push(FinalPropagationNodePlan {
            node_id: CoordinationNodeKey::new(node_path.clone()),
            axis: facet_band.axis,
            parent_cross_size_target,
            child_count: child_plans.len(),
            child_plans,
            expected_plot_area_adjustments_count,
        });
    }
}

fn build_final_propagation_child_plans<'a, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child_measurements: I,
) -> Vec<FinalPropagationChildPlan>
where
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    child_measurements
        .enumerate()
        .map(|(idx, child)| {
            let has_band_scale = child.scales.contains_key(axis.scale_name());
            let child_is_facet_band = child
                .coord_measurement
                .as_any()
                .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
                .is_some();
            build_final_propagation_child_plan(
                idx,
                axis,
                parent_cross_size_target,
                child.plot_area_width,
                child.plot_area_height,
                has_band_scale,
                child_is_facet_band,
            )
        })
        .collect()
}

fn build_final_propagation_child_plan(
    child_index: usize,
    _axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    old_plot_area_width: f32,
    old_plot_area_height: f32,
    has_band_scale: bool,
    child_is_facet_band: bool,
) -> FinalPropagationChildPlan {
    // Fixed leaf plot-area mode keeps leaf subplot dimensions locked, but facet child containers may still
    // need cross-size propagation for coordinated parent sizing.
    let (target_plot_area_width, target_plot_area_height, adjust_plot_area) = if child_is_facet_band
    {
        match (_axis, parent_cross_size_target) {
            (FacetAxis::Column, Some(target_width))
                if (old_plot_area_width - target_width).abs() > 0.01 =>
            {
                (Some(target_width), None, true)
            }
            (FacetAxis::Row, Some(target_height))
                if (old_plot_area_height - target_height).abs() > 0.01 =>
            {
                (None, Some(target_height), true)
            }
            _ => (None, None, false),
        }
    } else {
        (None, None, false)
    };

    let target_band_range_end = if has_band_scale {
        parent_cross_size_target
    } else {
        None
    };
    let update_band_range = target_band_range_end.is_some();

    FinalPropagationChildPlan {
        child_index,
        old_plot_area_width,
        old_plot_area_height,
        target_plot_area_width,
        target_plot_area_height,
        target_band_range_end,
        adjust_plot_area,
        update_band_range,
    }
}

fn apply_final_propagation_child_resize(
    axis: FacetAxis,
    child: &mut ComponentsMeasurement,
    child_plan: &FinalPropagationChildPlan,
) -> bool {
    let mut plot_area_adjusted = false;
    if child_plan.adjust_plot_area {
        if let Some(new_width) = child_plan.target_plot_area_width
            && (child.plot_area_width - new_width).abs() > 0.01
        {
            child.plot_area_width = new_width;
            plot_area_adjusted = true;
        }
        if let Some(new_height) = child_plan.target_plot_area_height
            && (child.plot_area_height - new_height).abs() > 0.01
        {
            child.plot_area_height = new_height;
            plot_area_adjusted = true;
        }
    }

    if child_plan.update_band_range
        && let (Some(band_scale), Some(range_end)) = (
            child.scales.get_mut(axis.scale_name()),
            child_plan.target_band_range_end,
        )
    {
        let updated_config = band_scale
            .configured()
            .clone()
            .with_range_interval((0.0, range_end));
        band_scale.set_configured(updated_config);
    }

    plot_area_adjusted
}

fn apply_final_propagation_child_retarget(
    child: &mut ComponentsMeasurement,
    child_plan: &FinalPropagationChildPlan,
) -> usize {
    if (child.plot_area_width - child_plan.old_plot_area_width).abs() <= 0.01
        && (child.plot_area_height - child_plan.old_plot_area_height).abs() <= 0.01
    {
        return 0;
    }

    retarget_child_scales_for_resized_plot_area(child)
}

pub(crate) fn run_final_propagation_with_trace_plot_area_sized(
    measurement: &mut ComponentsMeasurement,
    plan: &FinalPropagationPlan,
) -> FinalPropagationTrace {
    let plan_by_node: HashMap<CoordinationNodeKey, FinalPropagationNodePlan> = plan
        .node_plans
        .iter()
        .cloned()
        .map(|node| (node.node_id.clone(), node))
        .collect();

    let mut node_results = Vec::new();
    let mut node_path = Vec::new();
    run_final_propagation_recursive(
        measurement,
        &plan_by_node,
        &mut node_path,
        &mut node_results,
    );
    FinalPropagationTrace { node_results }
}

fn run_final_propagation_recursive(
    measurement: &mut ComponentsMeasurement,
    plan_by_node: &HashMap<CoordinationNodeKey, FinalPropagationNodePlan>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<FinalPropagationNodeTrace>,
) {
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    if let Some(facet_band) = facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let axis = facet_band.axis;
        let planned = plan_by_node.get(&node_id);
        debug_assert!(
            planned.is_some(),
            "Missing final propagation plan for node path {:?}",
            node_id.path
        );
        let fallback_child_plans = build_final_propagation_child_plans(
            axis,
            facet_band.coordinated_subplot_cross_size(),
            facet_band.child_measurements_iter(),
        );
        let fallback_expected_plot_area_adjustments_count = fallback_child_plans
            .iter()
            .filter(|child_plan| child_plan.adjust_plot_area)
            .count();
        let (
            child_plans,
            planned_parent_cross_size_target,
            planned_child_count,
            planned_plot_area_adjustments_count,
        ) = if let Some(planned) = planned {
            (
                planned.child_plans.clone(),
                planned.parent_cross_size_target,
                planned.child_count,
                planned.expected_plot_area_adjustments_count,
            )
        } else {
            (
                fallback_child_plans,
                facet_band.coordinated_subplot_cross_size(),
                facet_band.child_measurements_iter().count(),
                fallback_expected_plot_area_adjustments_count,
            )
        };
        let mut child_plot_area_adjustments_count = 0usize;
        let mut scale_range_retarget_count = 0usize;

        for (idx, child) in facet_band.child_measurements_iter_mut().enumerate() {
            let child_plan = child_plans.get(idx).cloned().unwrap_or_else(|| {
                let has_band_scale = child.scales.contains_key(axis.scale_name());
                build_final_propagation_child_plan(
                    idx,
                    axis,
                    planned_parent_cross_size_target,
                    child.plot_area_width,
                    child.plot_area_height,
                    has_band_scale,
                    child
                        .coord_measurement
                        .as_any()
                        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
                        .is_some(),
                )
            });

            let plot_area_adjusted = apply_final_propagation_child_resize(axis, child, &child_plan);
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count +=
                apply_final_propagation_child_retarget(child, &child_plan);
            node_path.push(idx);
            run_final_propagation_recursive(child, plan_by_node, node_path, node_results);
            node_path.pop();
        }

        facet_band.apply_coordinated_alignment_slabs_to_child_layouts();
        facet_band.recompute_plot_area_sized_placement();

        node_results.push(FinalPropagationNodeTrace {
            node_id,
            axis,
            planned_parent_cross_size_target,
            planned_child_count,
            planned_child_plan_count: child_plans.len(),
            planned_plot_area_adjustments_count,
            child_plot_area_adjustments_count,
            scale_range_retarget_count,
        });
    }
}

fn retarget_child_scales_for_resized_plot_area(child: &mut ComponentsMeasurement) -> usize {
    let new_plot_area_width = child.plot_area_width;
    let new_plot_area_height = child.plot_area_height;
    retarget_scale_ranges_for_plot_area(
        &mut child.scales,
        new_plot_area_width,
        new_plot_area_height,
    )
}
