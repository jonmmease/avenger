use std::sync::Arc;

use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::{
        coord::FacetBandCoordMeasurementPlotAreaSized,
        empty_cell_policy::FacetEmptyCellPolicy,
        ownership_policy::{
            cell_requires_invalid_path_axis_fallback_hidden, has_holes_from_cells,
            resolve_facet_ownership_policy,
        },
    },
    plot::CompiledPlot,
    render::RenderContext,
};

use super::{FacetBandRenderOps, facet_cell_main_axis_start_offset, facet_subplot_eval_ctx};

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

    if facet_measurement.cells.is_empty() {
        return Ok(Vec::new());
    }

    let cell_positions = &facet_measurement
        .plot_area_sized_placement
        .main_axis_positions;
    if cell_positions.len() != facet_measurement.cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet plot-area-sized render position count mismatch: positions={}, cells={}",
            cell_positions.len(),
            facet_measurement.cells.len()
        )));
    }

    let ownership_policy = resolve_facet_ownership_policy(
        facet_empty_cell_policy,
        has_holes_from_cells(
            facet_measurement
                .cells
                .iter()
                .map(|cell| cell.plan.is_empty),
        ),
    );
    let subplot_eval_ctx = facet_subplot_eval_ctx(
        compiled_subplot,
        context,
        ownership_policy.axis_owner_ignore_empty_cells,
    );

    let (origin_offset_x, origin_offset_y) = facet_cell_main_axis_start_offset(facet_measurement);
    ops.trace_main_axis_start_offset(origin_offset_x, origin_offset_y);

    let mut scene_marks = Vec::with_capacity(facet_measurement.cells.len());
    for (idx, cell) in facet_measurement.cells.iter().enumerate() {
        let position = cell_positions[idx];
        let subplot_origin = ops.subplot_origin(position, origin_offset_x, origin_offset_y);
        let is_empty_cell = cell.plan.is_empty;
        let band_size = match facet_measurement.axis {
            FacetAxis::Column => cell.measurement.plot_area_width,
            FacetAxis::Row => cell.measurement.plot_area_height,
        }
        .max(0.0);

        if is_empty_cell
            && matches!(
                ownership_policy.effective_empty_cell_policy,
                FacetEmptyCellPolicy::Hole
            )
        {
            let empty_group = SceneGroup {
                name: ops.group_name(idx, true),
                origin: subplot_origin,
                clip: avenger_scenegraph::marks::group::Clip::None,
                marks: Vec::new(),
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
            };
            scene_marks.push(SceneMark::Group(empty_group));
            continue;
        }

        let cell_eval_ctx = if cell_requires_invalid_path_axis_fallback_hidden(
            is_empty_cell,
            cell.plan.in_domain_slot,
        ) {
            subplot_eval_ctx.with_invalid_facet_path_axis_fallback_hidden(true)
        } else {
            subplot_eval_ctx.clone()
        }
        .with_facet_coord_node_path_appended(idx);

        let components = compiled_subplot
            .build_plot_components(
                &cell_eval_ctx,
                &cell.measurement,
                Some(&cell.data_override),
                true,
                &cell.plan.full_path,
            )
            .await?;

        let data_marks_group = SceneGroup {
            origin: [0.0, 0.0],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        let mut all_marks = vec![SceneMark::Group(data_marks_group)];
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        let subplot_group = SceneGroup {
            name: ops.group_name(idx, false),
            origin: subplot_origin,
            clip: avenger_scenegraph::marks::group::Clip::None,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        };
        scene_marks.push(SceneMark::Group(subplot_group));
        ops.trace_position(idx, subplot_origin, position, band_size);
    }

    Ok(scene_marks)
}
