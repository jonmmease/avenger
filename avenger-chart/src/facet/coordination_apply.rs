use std::{collections::HashMap, future::Future, pin::Pin};

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coordination_plans::{
            CoordinationNodeKey, FinalPropagationChildPlan, FinalPropagationNodePlan,
            FinalPropagationNodeTrace, FinalPropagationTrace, RequirementPass, RetargetNodePlan,
            RetargetNodeTrace, RetargetTrace,
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
                    "{} pass did not include node path {:?}",
                    requirement_pass.stage.label(),
                    node_id.path
                )));
                return;
            }
            facet_band.base_mut().clear_realized_owned_legend_slabs();
            facet_band
                .base_mut()
                .set_coordination_solution(std::sync::Arc::clone(solution), node_id.clone());
        },
    );

    if let Some(error) = error {
        return Err(error);
    }

    Ok(())
}

/// The retarget walk: derive every node's decisions from the
/// PRE-retarget state in one read-only pass, then apply them
/// parent-first. Node coverage and trace alignment are by construction
/// (the apply walk visits exactly the derive walk's nodes), which is what
/// retired the former plan/trace validators.
pub(crate) async fn run_retarget(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<RetargetTrace, AvengerChartError> {
    let decisions = derive_retarget_decisions(measurement, eval_ctx)?;
    apply_retarget_decisions(measurement, eval_ctx, &decisions).await
}

/// Derive every facet band's retarget decisions (requirements + actions)
/// from the PRE-retarget state, children first. Read-only by contract:
/// decisions must not observe earlier applications — the apply walk runs
/// parent-first afterwards.
pub(crate) fn derive_retarget_decisions(
    measurement: &ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<Vec<RetargetNodePlan>, AvengerChartError> {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    derive_retarget_decisions_recursive(measurement, eval_ctx, &mut node_path, &mut node_plans)?;
    Ok(node_plans)
}

fn derive_retarget_decisions_recursive(
    measurement: &ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<RetargetNodePlan>,
) -> Result<(), AvengerChartError> {
    if let Some(facet_band) = FacetCoordinationPolicy::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_retarget_decisions_recursive(child, eval_ctx, node_path, node_plans)?;
            node_path.pop();
        }

        let base = facet_band.base();
        let node_id = CoordinationNodeKey::new(node_path.clone());
        let requirements = base.derive_retarget_requirements(node_id.clone())?;
        let actions =
            FacetCoordinationPolicy::build_retarget_actions(facet_band, &requirements, eval_ctx);
        // Derivation-coherence invariants (formerly the plan-coverage
        // validator's per-node checks; node-set coverage itself is by
        // construction now that decide and apply share one walk).
        if requirements.child_count != requirements.child_plot_areas.len() {
            return Err(AvengerChartError::InternalError(format!(
                "retarget requirements for node path {:?} have child_count={} but {} child plot-area sizes",
                node_id.path,
                requirements.child_count,
                requirements.child_plot_areas.len()
            )));
        }
        if requirements.child_count != actions.child_actions.len() {
            return Err(AvengerChartError::InternalError(format!(
                "retarget actions for node path {:?} have child_count={} but {} child actions",
                node_id.path,
                requirements.child_count,
                actions.child_actions.len()
            )));
        }
        let slabs = crate::facet::overflow_projection::FacetOverflowSlabs::from_coordinated(
            &requirements.coordinated_overflow,
        );
        let (legend_start, legend_end) = match requirements.axis {
            FacetAxis::Column => slabs.legend_vertical(),
            FacetAxis::Row => slabs.legend_horizontal(),
        };
        if (requirements.legend_main_axis_slab.start - legend_start).abs() > 0.01
            || (requirements.legend_main_axis_slab.end - legend_end).abs() > 0.01
        {
            return Err(AvengerChartError::InternalError(format!(
                "retarget requirements for node path {:?} have stale legend slab: planned=({:.3}, {:.3}), derived=({:.3}, {:.3})",
                node_id.path,
                requirements.legend_main_axis_slab.start,
                requirements.legend_main_axis_slab.end,
                legend_start,
                legend_end
            )));
        }
        if requirements.layout_changed && requirements.coordinated_layout.is_none() {
            return Err(AvengerChartError::InternalError(format!(
                "retarget requirements for node path {:?} changed layout without a coordinated layout",
                node_id.path
            )));
        }
        if !requirements.ownership.has_holes
            && requirements.ownership.axis_owner_ignore_empty_cells
            && !matches!(
                requirements.ownership.empty_cell_policy,
                avenger_chart_core::FacetEmptyCellPolicy::Hole
            )
        {
            return Err(AvengerChartError::InternalError(format!(
                "retarget requirements for node path {:?} ignore empty cells without facet holes",
                node_id.path
            )));
        }
        node_plans.push(RetargetNodePlan {
            node_id,
            requirements,
            actions,
        });
    }
    Ok(())
}

/// Apply derived retarget decisions parent-first, propagating parent
/// cross sizes to children between apply and recurse.
pub(crate) fn apply_retarget_decisions<'a>(
    measurement: &'a mut ComponentsMeasurement,
    eval_ctx: &'a EvaluationContext,
    decisions: &'a [RetargetNodePlan],
) -> Pin<Box<dyn Future<Output = Result<RetargetTrace, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let plan_by_node: HashMap<CoordinationNodeKey, RetargetNodePlan> = decisions
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
                planned_child_action_counts: planned.actions.child_action_counts(),
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

/// The final-propagation walk: derive every node's decisions from the
/// pre-propagation state, then apply them. Coverage and trace alignment
/// are by construction (decide and apply share one traversal), which is
/// what retired the former plan/trace validators.
pub(crate) fn run_final_propagation(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<FinalPropagationTrace, AvengerChartError> {
    let decisions = derive_final_propagation_decisions(measurement, Some(eval_ctx));
    apply_final_propagation_decisions(measurement, eval_ctx, &decisions)
}

#[cfg(test)]
pub(crate) fn build_final_propagation_plan(
    measurement: &ComponentsMeasurement,
) -> Vec<FinalPropagationNodePlan> {
    derive_final_propagation_decisions(measurement, None)
}

/// Derive every facet band's final-propagation decisions (uniform child
/// plot-area targets from a band solve, plus band-range updates),
/// children first, read-only.
pub(crate) fn derive_final_propagation_decisions(
    measurement: &ComponentsMeasurement,
    eval_ctx: Option<&EvaluationContext>,
) -> Vec<FinalPropagationNodePlan> {
    let mut node_plans = Vec::new();
    let mut node_path = Vec::new();
    derive_final_propagation_decisions_recursive(
        measurement,
        eval_ctx,
        &mut node_path,
        &mut node_plans,
    );
    node_plans
}

fn derive_final_propagation_decisions_recursive(
    measurement: &ComponentsMeasurement,
    eval_ctx: Option<&EvaluationContext>,
    node_path: &mut Vec<usize>,
    node_plans: &mut Vec<FinalPropagationNodePlan>,
) {
    if let Some(facet_band) = FacetCoordinationPolicy::facet_band_ref(measurement) {
        for (idx, child) in facet_band.base().child_measurements_iter().enumerate() {
            node_path.push(idx);
            derive_final_propagation_decisions_recursive(child, eval_ctx, node_path, node_plans);
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

/// Apply derived final-propagation decisions, depth-first, refreshing
/// coordinated scale adjustments along the way.
pub(crate) fn apply_final_propagation_decisions(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    decisions: &[FinalPropagationNodePlan],
) -> Result<FinalPropagationTrace, AvengerChartError> {
    let plan_by_node: HashMap<CoordinationNodeKey, FinalPropagationNodePlan> = decisions
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
