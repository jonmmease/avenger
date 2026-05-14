use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;

use crate::{
    error::AvengerChartError,
    facet::{
        coord::FacetBandCoordMeasurementCanvasFit, empty_cell_policy::FacetEmptyCellPolicy,
        placement::resolve_facet_band_placement_from_scale_specs,
    },
    plot::CompiledPlot,
    render::RenderContext,
};

use super::{FacetBandRenderOps, render_facet_band_with_placement};

pub(super) async fn render_facet_band_canvas_fit(
    ops: FacetBandRenderOps,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_empty_cell_policy: FacetEmptyCellPolicy,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let facet_measurement = context
        .coord_measurement()
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurementCanvasFit>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Expected FacetBandCoordMeasurementCanvasFit in coord_measurement".into(),
            )
        })?;

    if facet_measurement.cells.is_empty() {
        return Ok(Vec::new());
    }

    let placement = resolve_facet_band_placement_from_scale_specs(
        context.coord_measurement(),
        context.scales(),
    )?
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            "Expected scale-backed facet placement for canvas-fit facet band".into(),
        )
    })?;

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
