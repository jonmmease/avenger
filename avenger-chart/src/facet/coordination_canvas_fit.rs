//! Canvas-fit facet coordination pipeline.
//!
//! Runs the full requirement collection, retargeting, and final propagation
//! cycle for canvas-fit mode.

use tracing::{debug, trace};

use crate::{
    error::AvengerChartError,
    facet::{
        coordination::{self, FacetCoordinationStage},
        coordination_apply::{
            apply_initial_requirement_pass, apply_retargeted_requirement_pass,
            build_final_propagation_plan, build_retarget_plan, run_final_propagation_with_trace,
            run_retarget_with_trace,
        },
        coordination_attributes::{
            CoordinationRunArtifacts, build_initial_requirement_pass,
            build_retargeted_requirement_pass,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
};

pub async fn coordinate_facet_measurement_tree_canvas_fit(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let artifacts =
        coordinate_facet_measurement_tree_canvas_fit_with_artifacts(measurement, eval_ctx).await?;
    trace!(
        initial_requirement_pass_nodes = artifacts.initial_requirement_pass.snapshot.nodes.len(),
        retarget_trace_nodes = artifacts.retarget_trace.node_results.len(),
        retargeted_requirement_pass_nodes =
            artifacts.retargeted_requirement_pass.snapshot.nodes.len(),
        final_propagation_trace_nodes = artifacts.final_propagation_trace.node_results.len(),
        "coordinate_facet_measurement_tree complete"
    );
    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_canvas_fit_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    let initial_requirement_pass = build_initial_requirement_pass(
        coordination::collect_initial_requirement_snapshot(measurement),
    );
    coordination::debug_assert_initial_requirement_coverage(&initial_requirement_pass);
    apply_initial_requirement_pass(measurement, &initial_requirement_pass);
    if checkpoint == CoordinationCheckpoint::InitialRequirementsApplied {
        return Ok(());
    }

    let retarget_plan = build_retarget_plan(measurement);
    coordination::debug_assert_retarget_plan_coverage(measurement, &retarget_plan);
    let retarget_trace = run_retarget_with_trace(measurement, eval_ctx, &retarget_plan).await?;
    coordination::debug_assert_retarget_trace_alignment(&retarget_plan, &retarget_trace);
    if checkpoint == CoordinationCheckpoint::RetargetComplete {
        return Ok(());
    }

    let retargeted_requirement_pass = build_retargeted_requirement_pass(
        coordination::collect_retargeted_requirement_snapshot(measurement),
    );
    coordination::debug_assert_retargeted_requirement_coverage(&retargeted_requirement_pass);
    apply_retargeted_requirement_pass(measurement, &retargeted_requirement_pass);
    if checkpoint == CoordinationCheckpoint::RetargetedRequirementsApplied {
        return Ok(());
    }

    let final_propagation_plan = build_final_propagation_plan(measurement);
    coordination::debug_assert_final_propagation_plan_coverage(
        measurement,
        &final_propagation_plan,
    );
    let final_propagation_trace =
        run_final_propagation_with_trace(measurement, eval_ctx, &final_propagation_plan)?;
    coordination::debug_assert_final_propagation_trace_alignment(
        &final_propagation_plan,
        &final_propagation_trace,
    );

    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_canvas_fit_with_artifacts(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<CoordinationRunArtifacts, AvengerChartError> {
    let mut epoch = None;
    coordination::debug_assert_stage_transition(epoch, FacetCoordinationStage::InitialRequirements);
    epoch = Some(FacetCoordinationStage::InitialRequirements);

    let initial_requirement_pass = build_initial_requirement_pass(
        coordination::collect_initial_requirement_snapshot(measurement),
    );
    coordination::debug_assert_initial_requirement_coverage(&initial_requirement_pass);
    debug!(
        overflow_groups = initial_requirement_pass
            .aggregates
            .merged_overflow_by_key
            .len(),
        layout_groups = initial_requirement_pass
            .aggregates
            .merged_layout_by_key
            .len(),
        domain_groups = initial_requirement_pass
            .aggregates
            .unified_domain_extents
            .len(),
        "coordinate_facet_measurement_tree initial requirements global aggregate + distribution"
    );
    apply_initial_requirement_pass(measurement, &initial_requirement_pass);
    debug!("coordinate_facet_measurement_tree initial requirements complete");

    coordination::debug_assert_stage_transition(epoch, FacetCoordinationStage::Retarget);
    epoch = Some(FacetCoordinationStage::Retarget);

    let retarget_plan = build_retarget_plan(measurement);
    coordination::debug_assert_retarget_plan_coverage(measurement, &retarget_plan);
    let retarget_trace = run_retarget_with_trace(measurement, eval_ctx, &retarget_plan).await?;
    coordination::debug_assert_retarget_trace_alignment(&retarget_plan, &retarget_trace);
    let parent_cross_propagations = retarget_trace
        .node_results
        .iter()
        .filter(|result| result.parent_cross_size_propagated)
        .count();
    let cross_size_changes = retarget_trace
        .node_results
        .iter()
        .filter(|result| {
            (result.subplot_cross_size_after - result.subplot_cross_size_before).abs() > 0.01
        })
        .count();
    debug!(
        parent_cross_propagations,
        cross_size_changes,
        remeasured_nodes = retarget_trace
            .node_results
            .iter()
            .filter(|result| result.remeasure_triggered)
            .count(),
        "coordinate_facet_measurement_tree retarget complete"
    );

    coordination::debug_assert_stage_transition(
        epoch,
        FacetCoordinationStage::RetargetedRequirements,
    );
    epoch = Some(FacetCoordinationStage::RetargetedRequirements);

    let retargeted_requirement_pass = build_retargeted_requirement_pass(
        coordination::collect_retargeted_requirement_snapshot(measurement),
    );
    coordination::debug_assert_retargeted_requirement_coverage(&retargeted_requirement_pass);
    debug!(
        overflow_groups = retargeted_requirement_pass
            .aggregates
            .merged_overflow_by_key
            .len(),
        layout_groups = retargeted_requirement_pass
            .aggregates
            .merged_layout_by_key
            .len(),
        "coordinate_facet_measurement_tree retargeted requirements post-remeasure reconciliation"
    );
    apply_retargeted_requirement_pass(measurement, &retargeted_requirement_pass);
    debug!("coordinate_facet_measurement_tree retargeted requirements complete");

    coordination::debug_assert_stage_transition(epoch, FacetCoordinationStage::FinalPropagation);

    let final_propagation_plan = build_final_propagation_plan(measurement);
    coordination::debug_assert_final_propagation_plan_coverage(
        measurement,
        &final_propagation_plan,
    );
    let final_propagation_trace =
        run_final_propagation_with_trace(measurement, eval_ctx, &final_propagation_plan)?;
    coordination::debug_assert_final_propagation_trace_alignment(
        &final_propagation_plan,
        &final_propagation_trace,
    );
    debug!(
        scale_range_retargets = final_propagation_trace
            .node_results
            .iter()
            .map(|result| result.scale_range_retarget_count)
            .sum::<usize>(),
        plot_area_adjustments = final_propagation_trace
            .node_results
            .iter()
            .map(|result| result.child_plot_area_adjustments_count)
            .sum::<usize>(),
        "coordinate_facet_measurement_tree final propagation complete"
    );

    Ok(CoordinationRunArtifacts {
        initial_requirement_pass,
        retarget_trace,
        retargeted_requirement_pass,
        final_propagation_trace,
    })
}
