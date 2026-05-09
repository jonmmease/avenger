//! Canvas-fit facet coordination pipeline.
//!
//! Runs the full four-phase AG-style coordination cycle for canvas-fit mode.

use tracing::{debug, trace};

use crate::{
    error::AvengerChartError,
    facet::{
        attribute_context::FacetCircularEpoch,
        coordination,
        coordination_attributes::{
            CoordinationRunArtifacts, build_collection_round_a, build_recollection_round,
        },
        coordination_sidecar::{
            apply_collection_round_a, apply_recollection_round, derive_inherited_apply_intent,
            derive_inherited_propagation_intent, run_inherited_apply_with_trace,
            run_inherited_propagation_with_trace,
        },
    },
    plot::compiled::ComponentsMeasurement,
    render::EvaluationContext,
};

pub async fn coordinate_facet_measurement_tree_canvas_fit(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let artifacts =
        coordinate_facet_measurement_tree_canvas_fit_with_artifacts(measurement, eval_ctx).await?;
    trace!(
        collection_round_a_nodes = artifacts.collection_round_a.snapshot.nodes.len(),
        inherited_apply_nodes = artifacts.inherited_apply.node_results.len(),
        recollection_round_nodes = artifacts.recollection_round.snapshot.nodes.len(),
        inherited_propagation_nodes = artifacts.inherited_propagation.node_results.len(),
        "coordinate_facet_measurement_tree complete"
    );
    Ok(())
}

pub(crate) async fn coordinate_facet_measurement_tree_canvas_fit_with_artifacts(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<CoordinationRunArtifacts, AvengerChartError> {
    let mut epoch = None;
    coordination::debug_assert_epoch_transition(epoch, FacetCircularEpoch::CollectionA);
    epoch = Some(FacetCircularEpoch::CollectionA);

    let collection_round_a =
        build_collection_round_a(coordination::collect_collection_round_snapshot(measurement));
    coordination::debug_assert_collection_round_coverage(&collection_round_a);
    debug!(
        overflow_groups = collection_round_a.aggregates.merged_overflow_by_key.len(),
        layout_groups = collection_round_a.aggregates.merged_layout_by_key.len(),
        domain_groups = collection_round_a.aggregates.unified_domain_extents.len(),
        "coordinate_facet_measurement_tree collection round A global aggregate + distribution"
    );
    apply_collection_round_a(measurement, &collection_round_a);
    debug!("coordinate_facet_measurement_tree collection round A complete");

    coordination::debug_assert_epoch_transition(epoch, FacetCircularEpoch::InheritedApply);
    epoch = Some(FacetCircularEpoch::InheritedApply);

    let inherited_apply_derivation = derive_inherited_apply_intent(measurement);
    coordination::debug_assert_inherited_apply_derivation_coverage(
        measurement,
        &inherited_apply_derivation,
    );
    let inherited_apply =
        run_inherited_apply_with_trace(measurement, eval_ctx, &inherited_apply_derivation).await?;
    coordination::debug_assert_inherited_apply_trace_alignment(
        &inherited_apply_derivation,
        &inherited_apply,
    );
    let parent_cross_propagations = inherited_apply
        .node_results
        .iter()
        .filter(|result| result.parent_cross_size_propagated)
        .count();
    let cross_size_changes = inherited_apply
        .node_results
        .iter()
        .filter(|result| {
            (result.subplot_cross_size_after - result.subplot_cross_size_before).abs() > 0.01
        })
        .count();
    debug!(
        parent_cross_propagations,
        cross_size_changes,
        remeasured_nodes = inherited_apply
            .node_results
            .iter()
            .filter(|result| result.remeasure_triggered)
            .count(),
        "coordinate_facet_measurement_tree inherited apply complete"
    );

    coordination::debug_assert_epoch_transition(epoch, FacetCircularEpoch::Recollection);
    epoch = Some(FacetCircularEpoch::Recollection);

    let recollection_round = build_recollection_round(
        coordination::collect_recollection_round_snapshot(measurement),
    );
    coordination::debug_assert_recollection_round_coverage(&recollection_round);
    debug!(
        overflow_groups = recollection_round.aggregates.merged_overflow_by_key.len(),
        layout_groups = recollection_round.aggregates.merged_layout_by_key.len(),
        "coordinate_facet_measurement_tree recollection round post-remeasure reconciliation"
    );
    apply_recollection_round(measurement, &recollection_round);
    debug!("coordinate_facet_measurement_tree recollection round complete");

    coordination::debug_assert_epoch_transition(epoch, FacetCircularEpoch::InheritedPropagation);

    let inherited_propagation_derivation = derive_inherited_propagation_intent(measurement);
    coordination::debug_assert_inherited_propagation_derivation_coverage(
        measurement,
        &inherited_propagation_derivation,
    );
    let inherited_propagation =
        run_inherited_propagation_with_trace(measurement, &inherited_propagation_derivation);
    coordination::debug_assert_inherited_propagation_trace_alignment(
        &inherited_propagation_derivation,
        &inherited_propagation,
    );
    debug!(
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
        "coordinate_facet_measurement_tree inherited propagation complete"
    );

    Ok(CoordinationRunArtifacts {
        collection_round_a,
        inherited_apply,
        recollection_round,
        inherited_propagation,
    })
}
