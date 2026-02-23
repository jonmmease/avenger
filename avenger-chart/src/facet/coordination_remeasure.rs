use std::sync::Arc;

use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::coord::{FacetCellRuntime, measure_facet_cell_with_explicit_builder},
    layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
    plot::compiled::CompiledPlot,
    render::EvaluationContext,
    scales::ScaleBuilder,
};

#[derive(Debug, Clone)]
pub(crate) struct FacetCoordRemeasureRequest {
    pub(crate) axis: FacetAxis,
    pub(crate) subplot_cross_size: f32,
    pub(crate) adjusted_main_size: f32,
    pub(crate) axis_owner_ignore_empty_cells: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FacetCoordRemeasureCellOutcome {
    pub(crate) cell_index: usize,
    pub(crate) has_data_rows: bool,
    pub(crate) used_coordinated_extents: bool,
    pub(crate) plot_area_width: f32,
    pub(crate) plot_area_height: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FacetCoordRemeasureOutcome {
    pub(crate) cell_outcomes: Vec<FacetCoordRemeasureCellOutcome>,
}

pub(crate) fn subplot_plot_dimensions(request: &FacetCoordRemeasureRequest) -> (f32, f32) {
    match request.axis {
        FacetAxis::Column => (request.subplot_cross_size, request.adjusted_main_size),
        FacetAxis::Row => (request.adjusted_main_size, request.subplot_cross_size),
    }
}

pub(crate) fn build_subplot_eval_ctx_for_coord_remeasure(
    eval_ctx: &EvaluationContext,
    subplot_default_params: &IndexMap<String, ScalarValue>,
    axis_owner_ignore_empty_cells: bool,
) -> EvaluationContext {
    let mut params = subplot_default_params.clone();
    for (key, value) in &eval_ctx.params {
        params.insert(key.clone(), value.clone());
    }
    let eval_ctx = eval_ctx.with_params(params);
    eval_ctx.with_axis_owner_ignore_empty_cells(axis_owner_ignore_empty_cells)
}

fn fixed_plot_area_layout_spec(width: f32, height: f32) -> EvaluatedLayoutSpec {
    EvaluatedLayoutSpec {
        canvas: EvaluatedSizeMode::Auto,
        plot_area: EvaluatedSizeMode::Fixed { width, height },
        margins: EvaluatedMargins {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
    }
}

pub(crate) async fn run_facet_coord_remeasure(
    cells: &mut [FacetCellRuntime],
    compiled_subplot: &Arc<CompiledPlot>,
    shared_scale_builder: &ScaleBuilder,
    eval_ctx: &EvaluationContext,
    request: &FacetCoordRemeasureRequest,
) -> Result<FacetCoordRemeasureOutcome, AvengerChartError> {
    let subplot_eval_ctx = build_subplot_eval_ctx_for_coord_remeasure(
        eval_ctx,
        compiled_subplot.get_default_params(),
        request.axis_owner_ignore_empty_cells,
    );
    let (subplot_plot_width, subplot_plot_height) = subplot_plot_dimensions(request);
    let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_plot_width, subplot_plot_height);

    let mut cell_outcomes = Vec::with_capacity(cells.len());
    for (idx, cell) in cells.iter_mut().enumerate() {
        let used_coordinated_extents = !cell.coordinated_domain_extents.is_empty();
        let mut builder = shared_scale_builder.clone();
        if used_coordinated_extents {
            builder.extend_with_domain_extents(&cell.coordinated_domain_extents);
        }

        let measurement = measure_facet_cell_with_explicit_builder(
            &cell.plan,
            &cell.data_override,
            compiled_subplot,
            &subplot_eval_ctx,
            &subplot_layout_spec,
            &builder,
        )
        .await?;

        trace!(
            cell_index = idx,
            cell_value = ?cell.plan.value,
            axis = ?request.axis,
            subplot_cross_size = request.subplot_cross_size,
            adjusted_main_size = request.adjusted_main_size,
            used_coordinated_extents,
            is_empty = !cell.plan.has_data_rows,
            "FacetBand coordinated remeasure kernel re-measured cell"
        );

        let outcome = FacetCoordRemeasureCellOutcome {
            cell_index: idx,
            has_data_rows: cell.plan.has_data_rows,
            used_coordinated_extents,
            plot_area_width: measurement.plot_area_width,
            plot_area_height: measurement.plot_area_height,
        };
        cell.measurement = measurement;
        cell_outcomes.push(outcome);
    }

    Ok(FacetCoordRemeasureOutcome { cell_outcomes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        facet::evaluated_facet_tree::EvaluatedFacetTree,
        render::context::AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM,
        theme::Theme,
    };
    use datafusion::prelude::SessionContext;

    #[test]
    fn coord_remeasure_request_maps_axis_dimensions_correctly() {
        let column = FacetCoordRemeasureRequest {
            axis: FacetAxis::Column,
            subplot_cross_size: 80.0,
            adjusted_main_size: 240.0,
            axis_owner_ignore_empty_cells: false,
        };
        assert_eq!(subplot_plot_dimensions(&column), (80.0, 240.0));

        let row = FacetCoordRemeasureRequest {
            axis: FacetAxis::Row,
            subplot_cross_size: 60.0,
            adjusted_main_size: 320.0,
            axis_owner_ignore_empty_cells: true,
        };
        assert_eq!(subplot_plot_dimensions(&row), (320.0, 60.0));
    }

    #[test]
    fn coord_remeasure_sets_axis_owner_ignore_empty_cells_in_eval_context() {
        let session = Arc::new(SessionContext::new());
        let eval_ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            session,
            IndexMap::new(),
            Arc::new(EvaluatedFacetTree::empty()),
        );

        let mut subplot_params = IndexMap::new();
        subplot_params.insert("subplot_flag".to_string(), ScalarValue::Boolean(Some(true)));

        let subplot_eval_ctx =
            build_subplot_eval_ctx_for_coord_remeasure(&eval_ctx, &subplot_params, true);

        assert_eq!(
            subplot_eval_ctx
                .params
                .get(AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM),
            Some(&ScalarValue::Boolean(Some(true)))
        );
        assert_eq!(
            subplot_eval_ctx.params.get("subplot_flag"),
            Some(&ScalarValue::Boolean(Some(true)))
        );
    }
}
