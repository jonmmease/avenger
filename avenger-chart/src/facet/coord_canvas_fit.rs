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

pub(crate) async fn measure_facet_row_canvas_fit(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    FacetBandMeasurePipeline::new(
        FacetAxisOps::for_axis(FacetAxis::Row),
        scales,
        plot_width,
        eval_ctx,
        data,
        compiled_marks,
        facet_path,
    )
    .run()
    .await
}

pub(crate) async fn measure_facet_column_canvas_fit(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    FacetBandMeasurePipeline::new(
        FacetAxisOps::for_axis(FacetAxis::Column),
        scales,
        plot_height,
        eval_ctx,
        data,
        compiled_marks,
        facet_path,
    )
    .run()
    .await
}
