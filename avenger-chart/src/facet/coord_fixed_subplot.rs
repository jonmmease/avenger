use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, dataframe::DataFrame};

use crate::{
    coords::{CoordMeasurement, FacetAxis},
    error::AvengerChartError,
    facet::coord::{FacetAxisOps, FacetBandMeasurePipeline},
    marks::CompiledMark,
    render::EvaluationContext,
    scales::ConfiguredScaleWithSpec,
};

/// Fixed-subplot facet measurement path.
///
/// This path is intentionally separated from canvas-fit orchestration so fixed-subplot
/// behavior can evolve independently while sharing the core facet-band synthesis engine.
pub(crate) async fn measure_facet_row_fixed_subplot(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_other_axis_size: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    FacetBandMeasurePipeline::new(
        FacetAxisOps::for_axis(FacetAxis::Row),
        scales,
        plot_other_axis_size,
        eval_ctx,
        data,
        compiled_marks,
        facet_path,
    )
    .run()
    .await
}

pub(crate) async fn measure_facet_column_fixed_subplot(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_other_axis_size: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    FacetBandMeasurePipeline::new(
        FacetAxisOps::for_axis(FacetAxis::Column),
        scales,
        plot_other_axis_size,
        eval_ctx,
        data,
        compiled_marks,
        facet_path,
    )
    .run()
    .await
}
