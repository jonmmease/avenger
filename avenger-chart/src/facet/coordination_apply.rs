use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
};

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coordination_plans::{
            CoordinationNodeKey, FinalPropagationChildPlan, FinalPropagationNodePlan,
            FinalPropagationNodeTrace, FinalPropagationPlan, FinalPropagationTrace,
            RequirementPass, RetargetNodePlan, RetargetNodeTrace, RetargetPlan, RetargetTrace,
        },
        coordination_policy::{FacetBandMut, FacetBandRef, FacetCoordinationPolicy},
    },
    layout::Size as LayoutSize,
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

pub(crate) fn visit_facet_bands_with_node_id<F>(
    measurement: &ComponentsMeasurement,
    depth: usize,
    node_path: &mut Vec<usize>,
    visit: &mut F,
) where
    F: FnMut(&CoordinationNodeKey, usize, &FacetBandRef<'_>),
{
    if let Some(facet_band) = FacetCoordinationPolicy::facet_band_ref(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, &facet_band);
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
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
    F: FnMut(&CoordinationNodeKey, usize, &mut FacetBandMut<'_>),
{
    if let Some(mut facet_band) = FacetCoordinationPolicy::facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        visit(&node_id, depth, &mut facet_band);
        for (idx, child) in facet_band
            .base_mut()
            .child_measurements_iter_mut()
            .enumerate()
        {
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
    let snapshot_keys_by_node = requirement_pass
        .snapshot
        .nodes
        .iter()
        .map(|node| (node.node_id.clone(), node.key.clone()))
        .collect::<HashMap<_, _>>();
    let snapshot_nodes = snapshot_keys_by_node
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    let nodes_requiring_overflow = requirement_pass
        .snapshot
        .nodes
        .iter()
        .filter(|node| node.overflow_cells.is_some())
        .map(|node| node.node_id.clone())
        .collect::<HashSet<_>>();
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
            if !snapshot_nodes.contains(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "{} pass did not include node path {:?}",
                    requirement_pass.stage.label(),
                    node_id.path
                )));
                return;
            }
            if let Some(overflow) = solution.overflow_by_node.get(node_id).cloned() {
                facet_band
                    .base_mut()
                    .set_coordinated_overflow_value(overflow);
            } else if nodes_requiring_overflow.contains(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "{} pass missing coordinated overflow patch for node path {:?}, group {:?}",
                    requirement_pass.stage.label(),
                    node_id.path,
                    snapshot_keys_by_node.get(node_id)
                )));
                return;
            }
            if let Some(boundary_overflow) =
                solution.boundary_overflow_by_node.get(node_id).cloned()
            {
                facet_band
                    .base_mut()
                    .set_coordinated_boundary_overflow_value(boundary_overflow);
            } else if nodes_requiring_overflow.contains(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "{} pass missing boundary overflow patch for node path {:?}, group {:?}",
                    requirement_pass.stage.label(),
                    node_id.path,
                    snapshot_keys_by_node.get(node_id)
                )));
                return;
            }
            if let Some(guide_anchor_overflow) =
                solution.guide_anchor_overflow_by_node.get(node_id).cloned()
            {
                facet_band
                    .base_mut()
                    .set_coordinated_guide_anchor_overflow_value(guide_anchor_overflow);
            } else if nodes_requiring_overflow.contains(node_id) {
                error = Some(AvengerChartError::InternalError(format!(
                    "{} pass missing guide-anchor overflow patch for node path {:?}, group {:?}",
                    requirement_pass.stage.label(),
                    node_id.path,
                    snapshot_keys_by_node.get(node_id)
                )));
                return;
            }
            let Some(layout) = solution.layout_by_node.get(node_id).cloned() else {
                error = Some(AvengerChartError::InternalError(format!(
                    "{} pass missing coordinated layout patch for node path {:?}, group {:?}",
                    requirement_pass.stage.label(),
                    node_id.path,
                    snapshot_keys_by_node.get(node_id)
                )));
                return;
            };
            facet_band.base_mut().set_coordinated_layout_value(layout);
            facet_band
                .base_mut()
                .set_coordination_solution(std::sync::Arc::clone(solution), node_id.clone());
            FacetCoordinationPolicy::refresh_placement_after_requirement_patch(facet_band, true);
        },
    );

    if let Some(error) = error {
        return Err(error);
    }

    Ok(())
}

pub(crate) fn build_retarget_plan(
    measurement: &ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<RetargetPlan, AvengerChartError> {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_retarget_plan_recursive(measurement, eval_ctx, &mut node_path, &mut node_plans)?;
    Ok(RetargetPlan { node_plans })
}

fn build_retarget_plan_recursive(
    measurement: &ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<RetargetNodePlan>,
) -> Result<(), AvengerChartError> {
    if let Some(facet_band) = FacetCoordinationPolicy::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_retarget_plan_recursive(child, eval_ctx, node_path, node_plans)?;
            node_path.pop();
        }

        let base = facet_band.base();
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let requirements = base.derive_retarget_requirements(node_id.clone())?;
        let actions =
            FacetCoordinationPolicy::build_retarget_actions(facet_band, &requirements, eval_ctx);
        node_plans.push(RetargetNodePlan {
            node_id,
            requirements,
            actions,
        });
    }
    Ok(())
}

pub(crate) fn run_retarget_with_trace<'a>(
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
        if let Some(mut facet_band) = FacetCoordinationPolicy::facet_band_mut(measurement) {
            let node_id = CoordinationNodeKey::new(node_path.clone());
            let planned = plan_by_node.get(&node_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing retarget plan for node path {:?}",
                    node_id.path
                ))
            })?;
            let outcome = facet_band
                .base_mut()
                .apply_retarget_actions(eval_ctx, &planned.actions)
                .await?;
            FacetCoordinationPolicy::refresh_placement_after_retarget_node(&mut facet_band);

            let parent_cross_size = facet_band.base().coordinated_subplot_cross_size();
            let parent_axis = facet_band.base().axis;
            let mut parent_cross_size_propagated = false;

            for (idx, child) in facet_band
                .base_mut()
                .child_measurements_iter_mut()
                .enumerate()
            {
                if let Some(cross_size) = parent_cross_size
                    && FacetCoordinationPolicy::set_child_parent_bandwidth_if_same_axis(
                        child,
                        parent_axis,
                        cross_size,
                    )
                {
                    parent_cross_size_propagated = true;
                }
                node_path.push(idx);
                run_retarget_recursive(child, eval_ctx, plan_by_node, node_path, node_results)
                    .await?;
                node_path.pop();
            }

            node_results.push(RetargetNodeTrace {
                node_id,
                axis: planned.requirements.axis,
                planned_has_legend_overflow: planned.requirements.has_legend_overflow,
                planned_layout_changed: planned.requirements.layout_changed,
                planned_axis_owner_ignore_empty_cells: planned
                    .requirements
                    .ownership
                    .axis_owner_ignore_empty_cells,
                planned_band_action: planned.actions.band_action,
                planned_child_action_counts: planned.actions.child_action_counts(),
                planned_child_count: planned.requirements.child_count,
                parent_cross_size_propagated,
                subplot_cross_size_before: outcome.subplot_cross_size_before,
                subplot_cross_size_after: outcome.subplot_cross_size_after,
                band_layout_applied: outcome.band_layout_applied,
                plot_area_retarget_count: outcome.plot_area_retarget_count,
                width_retarget_count: outcome.width_retarget_count,
                height_retarget_count: outcome.height_retarget_count,
            });
        }
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn build_final_propagation_plan(
    measurement: &ComponentsMeasurement,
) -> FinalPropagationPlan {
    build_final_propagation_plan_for_eval(measurement, None)
}

pub(crate) fn build_final_propagation_plan_for_eval(
    measurement: &ComponentsMeasurement,
    eval_ctx: Option<&EvaluationContext>,
) -> FinalPropagationPlan {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    build_final_propagation_plan_recursive(measurement, eval_ctx, &mut node_path, &mut node_plans);
    FinalPropagationPlan { node_plans }
}

fn build_final_propagation_plan_recursive(
    measurement: &ComponentsMeasurement,
    eval_ctx: Option<&EvaluationContext>,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<FinalPropagationNodePlan>,
) {
    if let Some(facet_band) = FacetCoordinationPolicy::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            build_final_propagation_plan_recursive(child, eval_ctx, node_path, node_plans);
            node_path.pop();
        }

        let base = facet_band.base();
        let parent_cross_size_target = base.coordinated_subplot_cross_size();
        let solved_targets = solved_final_propagation_targets(
            base.axis,
            parent_cross_size_target,
            base.child_measurements_iter(),
        );
        let child_plans = build_final_propagation_child_plans(
            base.axis,
            parent_cross_size_target,
            solved_targets.as_deref(),
            base.child_measurements_iter(),
            eval_ctx,
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

/// Derive each cell's uniform plot-area target by solving the band as a
/// layout tree: every leaf takes the coordinated uniform size on the band
/// axis, and each child's target is its solved slot extent. Today the band
/// model is uniform single-span tracks, so the solved extents equal the
/// coordinated size (debug-asserted at the consumer); routing them through
/// `solve_tree` makes the targets allocation geometry rather than copied
/// policy, so spans or ragged slots would be handled by the solver.
fn solved_final_propagation_targets<'a, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    child_measurements: I,
) -> Option<Vec<f32>>
where
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    let target = parent_cross_size_target?;
    let leaves = child_measurements
        .map(|child| {
            let content_size = match axis {
                FacetAxis::Column => LayoutSize::new(target, child.plot_area_height),
                FacetAxis::Row => LayoutSize::new(child.plot_area_width, target),
            };
            avenger_layout::Layout::<usize>::leaf(content_size)
        })
        .collect::<Vec<_>>();
    if leaves.is_empty() {
        return None;
    }
    let child_count = leaves.len();
    let band = match axis {
        FacetAxis::Column => avenger_layout::Layout::row(leaves),
        FacetAxis::Row => avenger_layout::Layout::column(leaves),
    };
    let solved = band.solve(&avenger_layout::SolveOptions::default()).ok()?;
    Some(
        (0..child_count)
            .map(|index| {
                let region = solved.at_path(&[index]).expect("band child region exists");
                match axis {
                    FacetAxis::Column => region.slot.width,
                    FacetAxis::Row => region.slot.height,
                }
            })
            .collect(),
    )
}

fn build_final_propagation_child_plans<'a, I>(
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    solved_targets: Option<&[f32]>,
    child_measurements: I,
    eval_ctx: Option<&EvaluationContext>,
) -> Vec<FinalPropagationChildPlan>
where
    I: Iterator<Item = &'a ComponentsMeasurement>,
{
    child_measurements
        .enumerate()
        .map(|(idx, child)| {
            let solved_target = solved_targets.and_then(|targets| targets.get(idx).copied());
            build_final_propagation_child_plan(
                idx,
                axis,
                parent_cross_size_target,
                solved_target,
                child,
                eval_ctx,
            )
        })
        .collect()
}

fn build_final_propagation_child_plan(
    child_index: usize,
    axis: FacetAxis,
    parent_cross_size_target: Option<f32>,
    solved_target: Option<f32>,
    child: &ComponentsMeasurement,
    eval_ctx: Option<&EvaluationContext>,
) -> FinalPropagationChildPlan {
    // The tree-solved slot extent is the target; with today's uniform
    // single-span band model it must equal the coordinated size verbatim.
    if let (Some(solved), Some(direct)) = (solved_target, parent_cross_size_target) {
        debug_assert!(
            (solved - direct).abs() <= 0.01,
            "tree-solved final propagation target diverged: solved={solved}, direct={direct}"
        );
    }
    let parent_cross_size_target = solved_target.or(parent_cross_size_target);
    let policy = if let Some(eval_ctx) = eval_ctx {
        FacetCoordinationPolicy::final_child_resize_policy_for_eval(
            axis,
            parent_cross_size_target,
            child,
            eval_ctx,
        )
    } else {
        FacetCoordinationPolicy::final_child_resize_policy(axis, parent_cross_size_target, child)
    };
    let (target_plot_area_width, target_plot_area_height) = match (axis, parent_cross_size_target) {
        (FacetAxis::Column, Some(target_width))
            if policy.allow_width_resize && (child.plot_area_width - target_width).abs() > 0.01 =>
        {
            (Some(target_width), None)
        }
        (FacetAxis::Row, Some(target_height))
            if policy.allow_height_resize
                && (child.plot_area_height - target_height).abs() > 0.01 =>
        {
            (None, Some(target_height))
        }
        _ => (None, None),
    };
    let adjust_plot_area = target_plot_area_width.is_some() || target_plot_area_height.is_some();

    let has_band_scale = child.scales.contains_key(axis.scale_name());
    let target_band_range_end = if policy.allow_band_range_retarget(axis) && has_band_scale {
        parent_cross_size_target
    } else {
        None
    };
    let update_band_range = target_band_range_end.is_some();

    FinalPropagationChildPlan {
        child_index,
        target_plot_area_width,
        target_plot_area_height,
        target_band_range_end,
        adjust_plot_area,
        update_band_range,
    }
}

pub(crate) fn run_final_propagation_with_trace(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    plan: &FinalPropagationPlan,
) -> Result<FinalPropagationTrace, AvengerChartError> {
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
        eval_ctx,
        &plan_by_node,
        &mut node_path,
        &mut node_results,
    )?;
    Ok(FinalPropagationTrace { node_results })
}

fn run_final_propagation_recursive(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    plan_by_node: &HashMap<CoordinationNodeKey, FinalPropagationNodePlan>,
    node_path: &mut Vec<usize>,
    node_results: &mut Vec<FinalPropagationNodeTrace>,
) -> Result<(), AvengerChartError> {
    crate::coords::apply_coord_measurement_scale_adjustments(
        measurement.coord_measurement.as_ref(),
        &mut measurement.scales,
    );

    if let Some(mut facet_band) = FacetCoordinationPolicy::facet_band_mut(measurement) {
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let axis = facet_band.base().axis;
        let planned = plan_by_node.get(&node_id).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing final propagation plan for node path {:?}",
                node_id.path
            ))
        })?;
        let child_plans = &planned.child_plans;
        let planned_parent_cross_size_target = planned.parent_cross_size_target;
        let planned_child_count = planned.child_count;
        let planned_plot_area_adjustments_count = planned.expected_plot_area_adjustments_count;
        let actual_child_count = facet_band.base().child_measurements_iter().count();
        if planned_child_count != actual_child_count || child_plans.len() != actual_child_count {
            return Err(AvengerChartError::InternalError(format!(
                "Final propagation plan child count mismatch for node path {:?}: planned child_count={}, child_plans={}, actual_children={}",
                node_id.path,
                planned_child_count,
                child_plans.len(),
                actual_child_count
            )));
        }
        let mut child_plot_area_adjustments_count = 0usize;
        let mut scale_range_retarget_count = 0usize;
        let compiled_subplot = facet_band.base().compiled_subplot.clone();

        for (idx, cell) in facet_band.base_mut().cells.iter_mut().enumerate() {
            let child_plan = &child_plans[idx];

            let (plot_area_adjusted, retarget_count) = {
                let child = &mut cell.measurement;
                FacetCoordinationPolicy::apply_final_propagation_child_update(
                    axis,
                    child,
                    Some(&cell.plan),
                    compiled_subplot.as_ref(),
                    eval_ctx,
                    child_plan,
                )?
            };
            if plot_area_adjusted {
                child_plot_area_adjustments_count += 1;
            }
            scale_range_retarget_count += retarget_count;
            node_path.push(idx);
            run_final_propagation_recursive(
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
            .realize_coordinated_child_frame_allocations();
        FacetCoordinationPolicy::refresh_placement_after_final_propagation_node(&mut facet_band);

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
