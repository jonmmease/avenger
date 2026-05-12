//! Fixed-subplot facet coordination pipeline.
//!
//! Runs the same requirement collection and propagation steps as canvas-fit mode while
//! preserving fixed leaf plot-area sizing.

use tracing::debug;

use crate::{
    error::AvengerChartError,
    facet::{
        coord::facet_band_canvas_ref,
        coordination_apply_fixed::{
            apply_initial_requirement_pass_fixed, apply_retargeted_requirement_pass_fixed,
            build_final_propagation_plan_fixed, build_retarget_plan_fixed,
            run_final_propagation_with_trace_fixed, run_retarget_with_trace_fixed,
            visit_fixed_facet_bands_with_node_id,
        },
        coordination_attributes::{
            InitialRequirementNodeSnapshot, InitialRequirementSnapshot, RetargetPlan,
            RetargetedRequirementNodeSnapshot, RetargetedRequirementSnapshot,
            build_initial_requirement_pass, build_retargeted_requirement_pass,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext, context::FacetRuntimeSizingMode},
};

fn collect_initial_requirement_snapshot_fixed(
    measurement: &ComponentsMeasurement,
) -> InitialRequirementSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let mut domain_infos = Vec::new();
            facet_band.collect_cell_domain_infos(&mut domain_infos);
            nodes.push(InitialRequirementNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
                domain_infos,
            });
        },
    );
    InitialRequirementSnapshot { nodes }
}

fn collect_retargeted_requirement_snapshot_fixed(
    measurement: &ComponentsMeasurement,
) -> RetargetedRequirementSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            nodes.push(RetargetedRequirementNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
            });
        },
    );
    RetargetedRequirementSnapshot { nodes }
}

fn assert_no_canvas_fit_measurements_in_fixed_tree(measurement: &ComponentsMeasurement) {
    debug_assert!(
        facet_band_canvas_ref(measurement.coord_measurement.as_ref()).is_none(),
        "fixed-subplot coordinator must not traverse canvas-fit facet measurements"
    );

    if let Some(fixed_facet) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<crate::facet::coord::FacetBandCoordMeasurementFixed>(
    ) {
        for child in fixed_facet.child_measurements_iter() {
            assert_no_canvas_fit_measurements_in_fixed_tree(child);
        }
    }
}

fn assert_fixed_leaf_plot_sizes(
    measurement: &ComponentsMeasurement,
    expected_leaf_plot_width: f32,
    expected_leaf_plot_height: f32,
    stage: &str,
) {
    if let Some(fixed_facet) = measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<crate::facet::coord::FacetBandCoordMeasurementFixed>(
    ) {
        for child in fixed_facet.child_measurements_iter() {
            assert_fixed_leaf_plot_sizes(
                child,
                expected_leaf_plot_width,
                expected_leaf_plot_height,
                stage,
            );
        }
    } else {
        debug_assert!(
            (measurement.plot_area_width - expected_leaf_plot_width).abs() <= 0.01,
            "fixed-subplot leaf width drifted at stage {stage}: width={}, expected={}",
            measurement.plot_area_width,
            expected_leaf_plot_width
        );
        debug_assert!(
            (measurement.plot_area_height - expected_leaf_plot_height).abs() <= 0.01,
            "fixed-subplot leaf height drifted at stage {stage}: height={}, expected={}",
            measurement.plot_area_height,
            expected_leaf_plot_height
        );
    }
}

fn assert_retarget_trace_invariants_fixed(
    _plan: &RetargetPlan,
    trace: &crate::facet::coordination_attributes::RetargetTrace,
) {
    for node_result in &trace.node_results {
        debug_assert_eq!(
            node_result.remeasured_cell_count + node_result.remeasure_skipped_cell_count,
            0,
            "fixed retarget invariant: coordinated apply no longer remeasures cells"
        );
    }
}

/// Coordinate fixed-subplot facet measurements with the full requirement pipeline.
pub async fn coordinate_facet_measurement_tree_fixed_subplot(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    assert_no_canvas_fit_measurements_in_fixed_tree(measurement);

    let initial_requirement_pass =
        build_initial_requirement_pass(collect_initial_requirement_snapshot_fixed(measurement));
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
        "fixed-subplot coordination initial requirements complete"
    );
    apply_initial_requirement_pass_fixed(measurement, &initial_requirement_pass);

    let retarget_plan = build_retarget_plan_fixed(measurement);
    let retarget_trace =
        run_retarget_with_trace_fixed(measurement, eval_ctx, &retarget_plan).await?;
    assert_retarget_trace_invariants_fixed(&retarget_plan, &retarget_trace);
    if let FacetRuntimeSizingMode::FixedSubplot {
        leaf_plot_width,
        leaf_plot_height,
    } = eval_ctx.facet_runtime_sizing_mode()
    {
        assert_fixed_leaf_plot_sizes(measurement, leaf_plot_width, leaf_plot_height, "retarget");
    }
    debug!(
        node_count = retarget_trace.node_results.len(),
        remeasured_nodes = retarget_trace
            .node_results
            .iter()
            .filter(|result| result.remeasure_triggered)
            .count(),
        "fixed-subplot coordination retarget complete"
    );

    let retargeted_requirement_pass = build_retargeted_requirement_pass(
        collect_retargeted_requirement_snapshot_fixed(measurement),
    );
    apply_retargeted_requirement_pass_fixed(measurement, &retargeted_requirement_pass);
    debug!(
        overflow_groups = retargeted_requirement_pass
            .aggregates
            .merged_overflow_by_key
            .len(),
        layout_groups = retargeted_requirement_pass
            .aggregates
            .merged_layout_by_key
            .len(),
        "fixed-subplot coordination retargeted requirements complete"
    );

    let final_propagation_plan = build_final_propagation_plan_fixed(measurement);
    let final_propagation_trace =
        run_final_propagation_with_trace_fixed(measurement, &final_propagation_plan);
    if let FacetRuntimeSizingMode::FixedSubplot {
        leaf_plot_width,
        leaf_plot_height,
    } = eval_ctx.facet_runtime_sizing_mode()
    {
        assert_fixed_leaf_plot_sizes(
            measurement,
            leaf_plot_width,
            leaf_plot_height,
            "final propagation",
        );
    }
    debug!(
        node_count = final_propagation_trace.node_results.len(),
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
        "fixed-subplot coordination final propagation complete"
    );

    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_fixed_subplot_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    assert_no_canvas_fit_measurements_in_fixed_tree(measurement);

    let initial_requirement_pass =
        build_initial_requirement_pass(collect_initial_requirement_snapshot_fixed(measurement));
    apply_initial_requirement_pass_fixed(measurement, &initial_requirement_pass);
    if checkpoint == CoordinationCheckpoint::InitialRequirementsApplied {
        return Ok(());
    }

    let retarget_plan = build_retarget_plan_fixed(measurement);
    let retarget_trace =
        run_retarget_with_trace_fixed(measurement, eval_ctx, &retarget_plan).await?;
    assert_retarget_trace_invariants_fixed(&retarget_plan, &retarget_trace);
    if checkpoint == CoordinationCheckpoint::RetargetComplete {
        return Ok(());
    }

    let retargeted_requirement_pass = build_retargeted_requirement_pass(
        collect_retargeted_requirement_snapshot_fixed(measurement),
    );
    apply_retargeted_requirement_pass_fixed(measurement, &retargeted_requirement_pass);
    if checkpoint == CoordinationCheckpoint::RetargetedRequirementsApplied {
        return Ok(());
    }

    let final_propagation_plan = build_final_propagation_plan_fixed(measurement);
    let _ = run_final_propagation_with_trace_fixed(measurement, &final_propagation_plan);

    Ok(())
}
