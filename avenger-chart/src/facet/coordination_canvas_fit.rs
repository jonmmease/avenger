//! Canvas-fit facet coordination wrappers.

use tracing::trace;

use crate::{
    error::AvengerChartError,
    facet::{
        coordination, coordination_plans::CoordinationRunArtifacts,
        coordination_strategy::CanvasFitCoordinationStrategy,
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
    coordination::coordinate_facet_measurement_tree_until_with_strategy::<
        CanvasFitCoordinationStrategy,
    >(measurement, eval_ctx, checkpoint)
    .await
}

pub(crate) async fn coordinate_facet_measurement_tree_canvas_fit_with_artifacts(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<CoordinationRunArtifacts, AvengerChartError> {
    coordination::coordinate_facet_measurement_tree_with_strategy::<CanvasFitCoordinationStrategy>(
        measurement,
        eval_ctx,
    )
    .await
}
