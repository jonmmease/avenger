//! Plot-area-sized facet coordination wrappers.

use crate::{
    error::AvengerChartError,
    facet::{coordination, coordination_strategy::PlotAreaSizedCoordinationStrategy},
    plot::compiled::ComponentsMeasurement,
    render::{CoordinationCheckpoint, EvaluationContext},
};

/// Coordinate plot-area-sized facet measurements with the full requirement pipeline.
pub async fn coordinate_facet_measurement_tree_plot_area_sized(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    coordination::coordinate_facet_measurement_tree_with_strategy::<PlotAreaSizedCoordinationStrategy>(
        measurement,
        eval_ctx,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn coordinate_facet_measurement_tree_plot_area_sized_until(
    measurement: &mut ComponentsMeasurement,
    eval_ctx: &EvaluationContext,
    checkpoint: CoordinationCheckpoint,
) -> Result<(), AvengerChartError> {
    coordination::coordinate_facet_measurement_tree_until_with_strategy::<
        PlotAreaSizedCoordinationStrategy,
    >(measurement, eval_ctx, checkpoint)
    .await
}
