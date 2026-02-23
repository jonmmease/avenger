use std::sync::Arc;

use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use tracing::trace;

use crate::{
    coords::FacetAxis,
    error::AvengerChartError,
    facet::coord::{
        FacetBandCoordApplyPlan, FacetCellRuntime, measure_facet_cell_with_explicit_builder,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FacetCoordRemeasureCellIntent {
    pub(crate) cell_index: usize,
    pub(crate) has_data_rows: bool,
    pub(crate) use_coordinated_extents: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FacetCoordRemeasurePlan {
    pub(crate) request: FacetCoordRemeasureRequest,
    pub(crate) cell_intents: Vec<FacetCoordRemeasureCellIntent>,
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

pub(crate) fn derive_facet_coord_remeasure_plan(
    cells: &[FacetCellRuntime],
    plan: &FacetBandCoordApplyPlan,
    subplot_cross_size: f32,
) -> FacetCoordRemeasurePlan {
    let request = FacetCoordRemeasureRequest {
        axis: plan.axis,
        subplot_cross_size,
        adjusted_main_size: plan.adjusted_main_size,
        axis_owner_ignore_empty_cells: plan.axis_owner_ignore_empty_cells,
    };
    let cell_intents = cells
        .iter()
        .enumerate()
        .map(|(cell_index, cell)| FacetCoordRemeasureCellIntent {
            cell_index,
            has_data_rows: cell.plan.has_data_rows,
            use_coordinated_extents: !cell.coordinated_domain_extents.is_empty(),
        })
        .collect();

    FacetCoordRemeasurePlan {
        request,
        cell_intents,
    }
}

pub(crate) async fn run_facet_coord_remeasure(
    cells: &mut [FacetCellRuntime],
    compiled_subplot: &Arc<CompiledPlot>,
    shared_scale_builder: &ScaleBuilder,
    eval_ctx: &EvaluationContext,
    request: &FacetCoordRemeasureRequest,
) -> Result<FacetCoordRemeasureOutcome, AvengerChartError> {
    let fallback_plan = FacetCoordRemeasurePlan {
        request: request.clone(),
        cell_intents: cells
            .iter()
            .enumerate()
            .map(|(cell_index, cell)| FacetCoordRemeasureCellIntent {
                cell_index,
                has_data_rows: cell.plan.has_data_rows,
                use_coordinated_extents: !cell.coordinated_domain_extents.is_empty(),
            })
            .collect(),
    };
    run_facet_coord_remeasure_with_plan(
        cells,
        compiled_subplot,
        shared_scale_builder,
        eval_ctx,
        &fallback_plan,
    )
    .await
}

pub(crate) async fn run_facet_coord_remeasure_with_plan(
    cells: &mut [FacetCellRuntime],
    compiled_subplot: &Arc<CompiledPlot>,
    shared_scale_builder: &ScaleBuilder,
    eval_ctx: &EvaluationContext,
    remeasure_plan: &FacetCoordRemeasurePlan,
) -> Result<FacetCoordRemeasureOutcome, AvengerChartError> {
    if remeasure_plan.cell_intents.len() != cells.len() {
        return Err(AvengerChartError::InternalError(format!(
            "Facet coordinated remeasure plan size mismatch: intents={}, cells={}",
            remeasure_plan.cell_intents.len(),
            cells.len()
        )));
    }

    let subplot_eval_ctx = build_subplot_eval_ctx_for_coord_remeasure(
        eval_ctx,
        compiled_subplot.get_default_params(),
        remeasure_plan.request.axis_owner_ignore_empty_cells,
    );
    let (subplot_plot_width, subplot_plot_height) =
        subplot_plot_dimensions(&remeasure_plan.request);
    let subplot_layout_spec = fixed_plot_area_layout_spec(subplot_plot_width, subplot_plot_height);

    let mut cell_outcomes = Vec::with_capacity(remeasure_plan.cell_intents.len());
    for (expected_index, intent) in remeasure_plan.cell_intents.iter().enumerate() {
        if intent.cell_index != expected_index {
            return Err(AvengerChartError::InternalError(format!(
                "Facet coordinated remeasure intents must be in-order; expected cell_index={}, found={}",
                expected_index, intent.cell_index
            )));
        }
        let cell = cells.get_mut(intent.cell_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Facet coordinated remeasure intent references invalid cell index {}",
                intent.cell_index
            ))
        })?;

        debug_assert_eq!(
            intent.has_data_rows, cell.plan.has_data_rows,
            "remeasure plan must preserve has_data_rows classification"
        );
        let live_uses_coordinated_extents = !cell.coordinated_domain_extents.is_empty();
        if intent.use_coordinated_extents != live_uses_coordinated_extents {
            trace!(
                cell_index = intent.cell_index,
                intent_use_coordinated_extents = intent.use_coordinated_extents,
                live_use_coordinated_extents = live_uses_coordinated_extents,
                "FacetBand coordinated remeasure intent/live extents usage drift"
            );
        }

        let use_coordinated_extents_for_builder =
            intent.use_coordinated_extents || live_uses_coordinated_extents;
        let mut builder = shared_scale_builder.clone();
        if use_coordinated_extents_for_builder {
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
            cell_index = intent.cell_index,
            cell_value = ?cell.plan.value,
            axis = ?remeasure_plan.request.axis,
            subplot_cross_size = remeasure_plan.request.subplot_cross_size,
            adjusted_main_size = remeasure_plan.request.adjusted_main_size,
            used_coordinated_extents = intent.use_coordinated_extents,
            used_coordinated_extents_for_builder = use_coordinated_extents_for_builder,
            is_empty = !cell.plan.has_data_rows,
            "FacetBand coordinated remeasure kernel re-measured cell"
        );

        let outcome = FacetCoordRemeasureCellOutcome {
            cell_index: intent.cell_index,
            has_data_rows: intent.has_data_rows,
            used_coordinated_extents: intent.use_coordinated_extents,
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
        facet::coord::FacetBandCoordApplyPlan, facet::evaluated_facet_tree::EvaluatedFacetTree,
        render::context::AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, theme::Theme,
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

    #[test]
    fn coord_remeasure_plan_derivation_empty_cells_uses_apply_plan_request() {
        let plan = FacetBandCoordApplyPlan {
            axis: FacetAxis::Column,
            legend_start: 0.0,
            legend_end: 0.0,
            legend_slab_applied: 0.0,
            has_legend_overflow: false,
            has_coordinated_extents: false,
            has_coordinated_layout: false,
            remeasure_required: true,
            has_holes: false,
            axis_owner_ignore_empty_cells: true,
            original_main_size: 240.0,
            adjusted_main_size: 220.0,
            legend_main_axis_shrink: 20.0,
        };

        let derived = derive_facet_coord_remeasure_plan(&[], &plan, 64.0);
        assert_eq!(derived.request.axis, FacetAxis::Column);
        assert_eq!(derived.request.subplot_cross_size, 64.0);
        assert_eq!(derived.request.adjusted_main_size, 220.0);
        assert!(derived.request.axis_owner_ignore_empty_cells);
        assert!(derived.cell_intents.is_empty());
    }
}
