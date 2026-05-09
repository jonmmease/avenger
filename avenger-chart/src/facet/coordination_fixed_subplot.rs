//! Fixed-subplot facet coordination pipeline.
//!
//! Runs the same four AG-style coordination rounds as canvas-fit mode while
//! preserving fixed leaf plot-area sizing.

use tracing::debug;

use crate::{
    error::AvengerChartError,
    facet::{
        coord::facet_band_canvas_ref,
        coordination_attributes::{
            CollectionRoundNodeSnapshot, CollectionRoundSnapshot, InheritedApplyIntent,
            RecollectionRoundNodeSnapshot, RecollectionRoundSnapshot, build_collection_round_a,
            build_recollection_round,
        },
        coordination_sidecar_fixed::{
            apply_collection_round_a_fixed, apply_recollection_round_fixed,
            derive_inherited_apply_intent_fixed, derive_inherited_propagation_intent_fixed,
            run_inherited_apply_with_trace_fixed, run_inherited_propagation_with_trace_fixed,
            visit_fixed_facet_bands_with_node_id,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::{EvaluationContext, context::FacetRuntimeSizingMode},
};
use std::collections::HashMap;

fn collect_collection_round_snapshot_fixed(
    measurement: &ComponentsMeasurement,
) -> CollectionRoundSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            let mut domain_infos = Vec::new();
            facet_band.collect_cell_domain_infos(&mut domain_infos);
            nodes.push(CollectionRoundNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
                domain_infos,
            });
        },
    );
    CollectionRoundSnapshot { nodes }
}

fn collect_recollection_round_snapshot_fixed(
    measurement: &ComponentsMeasurement,
) -> RecollectionRoundSnapshot {
    let mut nodes = Vec::new();
    let mut node_path = Vec::new();
    visit_fixed_facet_bands_with_node_id(
        measurement,
        0,
        &mut node_path,
        &mut |node_id, depth, facet_band| {
            nodes.push(RecollectionRoundNodeSnapshot {
                node_id: node_id.clone(),
                key: facet_band.coordination_group_key_for_depth(depth),
                local_overflow: facet_band.local_overflow_value(),
                local_layout: facet_band.local_layout_value(),
            });
        },
    );
    RecollectionRoundSnapshot { nodes }
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

fn assert_inherited_apply_trace_invariants_fixed(
    derivation: &InheritedApplyIntent,
    trace: &crate::facet::coordination_attributes::InheritedApplyTrace,
) {
    let derivation_by_node: HashMap<_, _> = derivation
        .node_derivations
        .iter()
        .map(|node| (node.node_id.clone(), node))
        .collect();

    for node_result in &trace.node_results {
        if let Some(derived) = derivation_by_node.get(&node_result.node_id) {
            let intent_count = derived
                .remeasure_plan
                .as_ref()
                .map(|plan| plan.cell_intents.len())
                .unwrap_or(0);
            debug_assert_eq!(
                node_result.remeasured_cell_count + node_result.remeasure_skipped_cell_count,
                intent_count,
                "fixed inherited-apply invariant: remeasured + skipped must match intent count"
            );
        }
    }
}

/// Coordinate fixed-subplot facet measurements with full AG-style rounds.
pub async fn coordinate_facet_measurement_tree_fixed_subplot(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    assert_no_canvas_fit_measurements_in_fixed_tree(measurement);

    let collection_round_a =
        build_collection_round_a(collect_collection_round_snapshot_fixed(measurement));
    debug!(
        overflow_groups = collection_round_a.aggregates.merged_overflow_by_key.len(),
        layout_groups = collection_round_a.aggregates.merged_layout_by_key.len(),
        domain_groups = collection_round_a.aggregates.unified_domain_extents.len(),
        "fixed-subplot coordination collection round A complete"
    );
    apply_collection_round_a_fixed(measurement, &collection_round_a);

    let inherited_apply_derivation = derive_inherited_apply_intent_fixed(measurement);
    let inherited_apply =
        run_inherited_apply_with_trace_fixed(measurement, eval_ctx, &inherited_apply_derivation)
            .await?;
    assert_inherited_apply_trace_invariants_fixed(&inherited_apply_derivation, &inherited_apply);
    if let FacetRuntimeSizingMode::FixedSubplot {
        leaf_plot_width,
        leaf_plot_height,
    } = eval_ctx.facet_runtime_sizing_mode()
    {
        assert_fixed_leaf_plot_sizes(
            measurement,
            leaf_plot_width,
            leaf_plot_height,
            "inherited-apply",
        );
    }
    debug!(
        node_count = inherited_apply.node_results.len(),
        remeasured_nodes = inherited_apply
            .node_results
            .iter()
            .filter(|result| result.remeasure_triggered)
            .count(),
        "fixed-subplot coordination inherited apply complete"
    );

    let recollection_round =
        build_recollection_round(collect_recollection_round_snapshot_fixed(measurement));
    apply_recollection_round_fixed(measurement, &recollection_round);
    debug!(
        overflow_groups = recollection_round.aggregates.merged_overflow_by_key.len(),
        layout_groups = recollection_round.aggregates.merged_layout_by_key.len(),
        "fixed-subplot coordination recollection round complete"
    );

    let inherited_propagation_derivation = derive_inherited_propagation_intent_fixed(measurement);
    let inherited_propagation =
        run_inherited_propagation_with_trace_fixed(measurement, &inherited_propagation_derivation);
    if let FacetRuntimeSizingMode::FixedSubplot {
        leaf_plot_width,
        leaf_plot_height,
    } = eval_ctx.facet_runtime_sizing_mode()
    {
        assert_fixed_leaf_plot_sizes(
            measurement,
            leaf_plot_width,
            leaf_plot_height,
            "inherited-propagation",
        );
    }
    debug!(
        node_count = inherited_propagation.node_results.len(),
        scale_range_retargets = inherited_propagation
            .node_results
            .iter()
            .map(|result| result.scale_range_retarget_count)
            .sum::<usize>(),
        plot_area_adjustments = inherited_propagation
            .node_results
            .iter()
            .map(|result| result.child_plot_area_adjustments_count)
            .sum::<usize>(),
        "fixed-subplot coordination inherited propagation complete"
    );

    Ok(())
}
