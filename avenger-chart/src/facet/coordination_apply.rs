use std::{collections::HashMap, future::Future, pin::Pin};

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coordination_plans::{
            CoordinationNodeKey, FinalPropagationChildPlan, FinalPropagationNodePlan,
            FinalPropagationNodeTrace, FinalPropagationPlan, FinalPropagationTrace,
            InitialRequirementPass, RetargetNodePlan, RetargetNodeTrace, RetargetPlan,
            RetargetTrace, RetargetedRequirementPass,
        },
        coordination_strategy::{FacetBandMut, FacetBandRef, FacetSizingCoordinationStrategy},
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

#[cfg(test)]
use crate::facet::coord::FacetBandCoordMeasurement;
#[cfg(test)]
use crate::facet::coordination_strategy::CanvasFitCoordinationStrategy;

pub(crate) fn visit_facet_bands_with_node_id_for_strategy<S, F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    S: FacetSizingCoordinationStrategy,
    F: FnMut(&CoordinationNodeKey, usize, &FacetBandRef<'_>),
{
    if let Some(facet_band) = S::facet_band_ref(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, &facet_band);
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            visit_facet_bands_with_node_id_for_strategy::<S, F>(child, depth + 1, node_path, visit);
            node_path.pop();
        }
    }
}

pub(crate) fn visit_facet_bands_with_node_id_mut_for_strategy<S, F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    S: FacetSizingCoordinationStrategy,
    F: FnMut(&CoordinationNodeKey, usize, &mut FacetBandMut<'_>),
{
    if let Some(mut facet_band) = S::facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, &mut facet_band);
        for (idx, child) in facet_band
            .base_mut()
            .child_measurements_iter_mut()
            .enumerate()
        {
            node_path.push(idx);
            visit_facet_bands_with_node_id_mut_for_strategy::<S, F>(
                child,
                depth + 1,
                node_path,
                visit,
            );
            node_path.pop();
        }
    }
}

#[cfg(test)]
pub(crate) fn visit_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &FacetBandCoordMeasurement),
{
    visit_facet_bands_with_node_id_for_strategy::<CanvasFitCoordinationStrategy, _>(
        measurement,
        depth,
        node_path,
        &mut |node_id, depth, facet_band| visit(node_id, depth, facet_band.base()),
    );
}

#[cfg(test)]
pub(crate) fn visit_facet_bands_with_node_id_mut<F>(
    measurement: &mut ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &mut FacetBandCoordMeasurement),
{
    visit_facet_bands_with_node_id_mut_for_strategy::<CanvasFitCoordinationStrategy, _>(
        measurement,
        depth,
        node_path,
        &mut |node_id, depth, facet_band| visit(node_id, depth, facet_band.base_mut()),
    );
}

pub(crate) fn apply_initial_requirement_pass_with_strategy<S>(
    measurement: &mut ComponentsMeasurement,
    initial_requirement_pass: &InitialRequirementPass,
) where
    S: FacetSizingCoordinationStrategy,
{
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut_for_strategy::<S, _>(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            let mut patch_applied = false;
            if let Some(overflow) = initial_requirement_pass
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band
                    .base_mut()
                    .set_coordinated_overflow_value(overflow);
                patch_applied = true;
            }
            if let Some(layout) = initial_requirement_pass
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.base_mut().set_coordinated_layout_value(layout);
                patch_applied = true;
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
                facet_band.base_mut().distribute_coordinated_domain_extents(
                    &initial_requirement_pass.distribution.unified_domain_extents,
                );
            }
            S::refresh_placement_after_requirement_patch(facet_band, patch_applied);
        },
    );
}

#[cfg(test)]
pub(crate) fn apply_initial_requirement_pass(
    measurement: &mut ComponentsMeasurement,
    initial_requirement_pass: &InitialRequirementPass,
) {
    apply_initial_requirement_pass_with_strategy::<CanvasFitCoordinationStrategy>(
        measurement,
        initial_requirement_pass,
    );
}

pub(crate) fn apply_retargeted_requirement_pass_with_strategy<S>(
    measurement: &mut ComponentsMeasurement,
    retargeted_requirement_pass: &RetargetedRequirementPass,
) where
    S: FacetSizingCoordinationStrategy,
{
    let mut node_path = Vec::new();
    visit_facet_bands_with_node_id_mut_for_strategy::<S, _>(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, _depth, facet_band| {
            let mut patch_applied = false;
            if let Some(overflow) = retargeted_requirement_pass
                .distribution
                .overflow_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band
                    .base_mut()
                    .set_coordinated_overflow_value(overflow);
                patch_applied = true;
            }
            if let Some(layout) = retargeted_requirement_pass
                .distribution
                .layout_patches_by_node
                .get(node_id)
                .cloned()
            {
                facet_band.base_mut().set_coordinated_layout_value(layout);
                patch_applied = true;
            }
            S::refresh_placement_after_requirement_patch(facet_band, patch_applied);
        },
    );
}

#[cfg(test)]
pub(crate) fn apply_retargeted_requirement_pass(
    measurement: &mut ComponentsMeasurement,
    retargeted_requirement_pass: &RetargetedRequirementPass,
) {
    apply_retargeted_requirement_pass_with_strategy::<CanvasFitCoordinationStrategy>(
        measurement,
        retargeted_requirement_pass,
    );
}

pub(crate) fn build_retarget_plan_with_strategy<S>(
    measurement: &ComponentsMeasurement,
) -> RetargetPlan
where
    S: FacetSizingCoordinationStrategy,
{
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_retarget_plan_recursive::<S>(measurement, &mut node_path, &mut node_plans);
    RetargetPlan { node_plans }
}

fn build_retarget_plan_recursive<S>(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<RetargetNodePlan>,
) where
    S: FacetSizingCoordinationStrategy,
{
    if let Some(facet_band) = S::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_retarget_plan_recursive::<S>(child, node_path, node_plans);
            node_path.pop();
        }

        let base = facet_band.base();
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let mut apply_plan = base.derive_coordinated_apply_plan();
        S::prepare_retarget_apply_plan(facet_band, &mut apply_plan);
        let child_count = base.child_measurements_iter().count();
        node_plans.push(RetargetNodePlan {
            node_id,
            axis: apply_plan.axis,
            has_legend_overflow: apply_plan.has_legend_overflow,
            has_coordinated_extents: apply_plan.has_coordinated_extents,
            cell_retarget_required: apply_plan.cell_retarget_required,
            apply_plan,
            child_count,
        });
    }
}

#[cfg(test)]
pub(crate) fn build_retarget_plan(measurement: &ComponentsMeasurement) -> RetargetPlan {
    build_retarget_plan_with_strategy::<CanvasFitCoordinationStrategy>(measurement)
}

pub(crate) fn run_retarget_with_trace_with_strategy<'a, S>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    plan: &'a RetargetPlan,
) -> Pin<Box<dyn Future<Output = Result<RetargetTrace, AvengerChartError>> + Send + 'a>>
where
    S: FacetSizingCoordinationStrategy + Send + Sync + 'static,
{
    Box::pin(async move {
        let plan_by_node: HashMap<CoordinationNodeKey, RetargetNodePlan> = plan
            .node_plans
            .iter()
            .cloned()
            .map(|node| (node.node_id.clone(), node))
            .collect();

        let mut node_results = Vec::new();
        let mut node_path = Vec::new();
        run_retarget_recursive::<S>(
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

fn run_retarget_recursive<'a, S>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    plan_by_node: &'a HashMap<CoordinationNodeKey, RetargetNodePlan>,
    node_path: &'a mut Vec<usize>,
    node_results: &'a mut Vec<RetargetNodeTrace>,
) -> Pin<Box<dyn Future<Output = Result<(), AvengerChartError>> + Send + 'a>>
where
    S: FacetSizingCoordinationStrategy + Send + Sync + 'static,
{
    Box::pin(async move {
        if let Some(mut facet_band) = S::facet_band_mut(measurement) {
            let node_id = CoordinationNodeKey::new(node_path.clone());
            let planned = plan_by_node.get(&node_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing retarget plan for node path {:?}",
                    node_id.path
                ))
            })?;
            let execution_plan = S::execution_retarget_apply_plan(&facet_band, &planned.apply_plan);
            let outcome = facet_band
                .base_mut()
                .apply_coordinated_overflow_with_plan(eval_ctx, &execution_plan)
                .await?;
            S::refresh_placement_after_retarget_node(&mut facet_band);

            let parent_cross_size = facet_band.base().coordinated_subplot_cross_size();
            let parent_axis = facet_band.base().axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band
                .base_mut()
                .child_measurements_iter_mut()
                .enumerate()
            {
                if let Some(cross_size) = parent_cross_size
                    && S::set_child_parent_bandwidth_if_same_axis(child, parent_axis, cross_size)
                {
                    parent_cross_size_propagated = true;
                }
                node_path.push(idx);
                run_retarget_recursive::<S>(child, eval_ctx, plan_by_node, node_path, node_results)
                    .await?;
                node_path.pop();
            }

            node_results.push(RetargetNodeTrace {
                node_id,
                axis: planned.axis,
                planned_has_legend_overflow: planned.has_legend_overflow,
                planned_has_coordinated_extents: planned.has_coordinated_extents,
                planned_cell_retarget_required: planned.cell_retarget_required,
                planned_axis_owner_ignore_empty_cells: planned
                    .apply_plan
                    .axis_owner_ignore_empty_cells,
                planned_adjusted_main_size: planned.apply_plan.adjusted_main_size,
                planned_child_count: planned.child_count,
                parent_cross_size_propagated,
                subplot_cross_size_before: outcome.subplot_cross_size_before,
                subplot_cross_size_after: outcome.subplot_cross_size_after,
                cell_retarget_applied: outcome.cell_retarget_applied,
                retargeted_cell_count: outcome.retargeted_cell_count,
            });
        }
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn run_retarget_with_trace<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    plan: &'a RetargetPlan,
) -> Pin<Box<dyn Future<Output = Result<RetargetTrace, AvengerChartError>> + Send + 'a>> {
    run_retarget_with_trace_with_strategy::<CanvasFitCoordinationStrategy>(
        measurement,
        eval_ctx,
        plan,
    )
}

pub(crate) fn build_final_propagation_plan_with_strategy<S>(
    measurement: &ComponentsMeasurement,
) -> FinalPropagationPlan
where
    S: FacetSizingCoordinationStrategy,
{
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_final_propagation_plan_recursive::<S>(measurement, &mut node_path, &mut node_plans);
    FinalPropagationPlan { node_plans }
}

fn build_final_propagation_plan_recursive<S>(
    measurement: &ComponentsMeasurement,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<FinalPropagationNodePlan>,
) where
    S: FacetSizingCoordinationStrategy,
{
    if let Some(facet_band) = S::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_final_propagation_plan_recursive::<S>(child, node_path, node_plans);
            node_path.pop();
        }

        let base = facet_band.base();
        let parent_cross_size_target = base.coordinated_subplot_cross_size();
        let child_plans = build_final_propagation_child_plans_with_strategy::<S, _>(
            base.axis,
            parent_cross_size_target,
            base.child_measurements_iter(),
        );
        let expected_plot_area_adjustments_count = child_plans
            .iter()
            .filter(|child_plan| child_plan.adjust_plot_area)
            .count();

        node_plans.push(FinalPropagationNodePlan {
            node_id: CoordinationNodeKey::new(node_path.clone()),
            axis: base.axis,
            parent_cross_size_target,
            child_count: child_plans.len(),
            child_plans,
            expected_plot_area_adjustments_count,
        });
    }
}

fn build_final_propagation_child_plans_with_strategy<'a, S, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child_measurements: I,
) -> Vec<FinalPropagationChildPlan>
where
    S: FacetSizingCoordinationStrategy,
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    child_measurements
        .enumerate()
        .map(|(idx, child)| {
            build_final_propagation_child_plan_with_strategy::<S>(
                idx,
                axis,
                parent_cross_size_target,
                child,
            )
        })
        .collect()
}

fn build_final_propagation_child_plan_with_strategy<S>(
    child_index: usize,
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child: &ComponentsMeasurement,
) -> FinalPropagationChildPlan
where
    S: FacetSizingCoordinationStrategy,
{
    let policy = S::final_child_resize_policy(axis, parent_cross_size_target, child);
    let (target_plot_area_width, target_plot_area_height, adjust_plot_area) =
        if policy.allow_plot_area_resize {
            match (axis, parent_cross_size_target) {
                (FacetAxis::Column, Some(target_width))
                    if (child.plot_area_width - target_width).abs() > 0.01 =>
                {
                    (Some(target_width), None, true)
                }
                (FacetAxis::Row, Some(target_height))
                    if (child.plot_area_height - target_height).abs() > 0.01 =>
                {
                    (None, Some(target_height), true)
                }
                _ => (None, None, false),
            }
        } else {
            (None, None, false)
        };

    let has_band_scale = child.scales.contains_key(axis.scale_name());
    let target_band_range_end = if policy.allow_scale_range_retarget && has_band_scale {
        parent_cross_size_target
    } else {
        None
    };
    let update_band_range = target_band_range_end.is_some();

    FinalPropagationChildPlan {
        child_index,
        old_plot_area_width: child.plot_area_width,
        old_plot_area_height: child.plot_area_height,
        target_plot_area_width,
        target_plot_area_height,
        target_band_range_end,
        adjust_plot_area,
        update_band_range,
    }
}

#[cfg(test)]
pub(crate) fn build_final_propagation_plan(
    measurement: &ComponentsMeasurement,
) -> FinalPropagationPlan {
    build_final_propagation_plan_with_strategy::<CanvasFitCoordinationStrategy>(measurement)
}

pub(crate) fn run_final_propagation_with_trace_with_strategy<S>(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    plan: &FinalPropagationPlan,
) -> Result<FinalPropagationTrace, AvengerChartError>
where
    S: FacetSizingCoordinationStrategy,
{
    let plan_by_node: HashMap<CoordinationNodeKey, FinalPropagationNodePlan> = plan
        .node_plans
        .iter()
        .cloned()
        .map(|node| (node.node_id.clone(), node))
        .collect();

    let mut node_results = Vec::new();
    let mut node_path = Vec::new();
    run_final_propagation_recursive::<S>(
        measurement,
        eval_ctx,
        &plan_by_node,
        &mut node_path,
        &mut node_results,
    )?;
    Ok(FinalPropagationTrace { node_results })
}

fn run_final_propagation_recursive<S>(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    plan_by_node: &HashMap<CoordinationNodeKey, FinalPropagationNodePlan>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<FinalPropagationNodeTrace>,
) -> Result<(), AvengerChartError>
where
    S: FacetSizingCoordinationStrategy,
{
    measurement
        .coord_measurement
        .apply_scale_adjustments(&mut measurement.scales);

    if let Some(mut facet_band) = S::facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let axis = facet_band.base().axis;
        let planned = plan_by_node.get(&node_id);
        debug_assert!(
            planned.is_some(),
            "Missing final propagation plan for node path {:?}",
            node_id.path
        );
        let fallback_child_plans = build_final_propagation_child_plans_with_strategy::<S, _>(
            axis,
            facet_band.base().coordinated_subplot_cross_size(),
            facet_band.base().child_measurements_iter(),
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
                facet_band.base().coordinated_subplot_cross_size(),
                facet_band.base().child_measurements_iter().count(),
                fallback_expected_plot_area_adjustments_count,
            )
        };
        let mut child_plot_area_adjustments_count = 0usize;
        let mut scale_range_retarget_count = 0usize;
        let compiled_subplot = facet_band.base().compiled_subplot.clone();

        for (idx, cell) in facet_band.base_mut().cells.iter_mut().enumerate() {
            let child_plan = child_plans.get(idx).cloned().unwrap_or_else(|| {
                build_final_propagation_child_plan_with_strategy::<S>(
                    idx,
                    axis,
                    planned_parent_cross_size_target,
                    &cell.measurement,
                )
            });

            let (plot_area_adjusted, retarget_count) = {
                let child = &mut cell.measurement;
                S::apply_final_propagation_child_update(
                    axis,
                    child,
                    Some(&cell.plan),
                    compiled_subplot.as_ref(),
                    eval_ctx,
                    &child_plan,
                )?
            };
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count += retarget_count;
            node_path.push(idx);
            run_final_propagation_recursive::<S>(
                &mut cell.measurement,
                eval_ctx,
                plan_by_node,
                node_path,
                node_results,
            )?;
            node_path.pop();
        }

        facet_band
            .base_mut()
            .apply_coordinated_alignment_slabs_to_child_layouts();
        S::refresh_placement_after_final_propagation_node(&mut facet_band);

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

    Ok(())
}

#[cfg(test)]
pub(crate) fn run_final_propagation_with_trace(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    plan: &FinalPropagationPlan,
) -> Result<FinalPropagationTrace, AvengerChartError> {
    run_final_propagation_with_trace_with_strategy::<CanvasFitCoordinationStrategy>(
        measurement,
        eval_ctx,
        plan,
    )
}
