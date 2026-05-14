use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;

use crate::{
    error::AvengerChartError,
    facet::{
        coord::FacetBandCoordMeasurementPlotAreaSized, empty_cell_policy::FacetEmptyCellPolicy,
    },
    plot::CompiledPlot,
    render::RenderContext,
};

use super::{FacetBandRenderOps, render_facet_band_with_placement};

pub(super) async fn render_facet_band_plot_area_sized(
    ops: FacetBandRenderOps,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let facet_measurement = context
        .coord_measurement()
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementPlotAreaSized>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected FacetBandCoordMeasurementPlotAreaSized in coord_measurement".into(),
            )
        })?;

    let placement = facet_measurement.resolved_placement()?;
    render_facet_band_with_placement(
        ops,
        compiled_subplot,
        facet_empty_cell_policy,
        context,
        facet_measurement,
        placement,
    )
    .await
}
