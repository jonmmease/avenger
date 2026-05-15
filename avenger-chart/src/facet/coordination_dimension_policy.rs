//! Per-dimension facet coordination wrappers.

use crate::{
    error::AvengerChartError,
    facet::{coordination, coordination_strategy::DimensionPolicyCoordinationStrategy},
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
};

pub(crate) async fn coordinate_facet_measurement_tree_dimension_policy(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    coordination::coordinate_facet_measurement_tree_with_strategy::<
        DimensionPolicyCoordinationStrategy,
    >(measurement, eval_ctx)
    .await
    .map(|_| ())
}

pub(crate) async fn coordinate_facet_measurement_tree_dimension_policy_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    coordination::coordinate_facet_measurement_tree_until_with_strategy::<
        DimensionPolicyCoordinationStrategy,
    >(measurement, eval_ctx, checkpoint)
    .await
}
