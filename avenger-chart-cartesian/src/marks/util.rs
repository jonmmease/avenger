use std::collections::HashMap;

use avenger_chart_core::{
    AvengerChartError, CompiledMarkCore, CoordinateSystemTransformCore, MarkRuntimeContext,
    PointGeometry, coerce_numeric_channel_with_renderer,
};
use avenger_common::value::ScalarOrArray;
use datafusion::arrow::record_batch::RecordBatch;

pub(crate) fn transform_cartesian_point_channels<M>(
    mark: &M,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    context: &dyn MarkRuntimeContext,
    coord: &dyn CoordinateSystemTransformCore,
    x_channel: &str,
    y_channel: &str,
) -> Result<PointGeometry, AvengerChartError>
where
    M: CompiledMarkCore + ?Sized,
{
    let mark_context = context.core_view();
    let mut position_channels = HashMap::new();
    position_channels.insert(
        "x",
        coerce_numeric_channel_with_renderer(mark, data, scalars, x_channel, &mark_context, 0.0)?,
    );
    position_channels.insert(
        "y",
        coerce_numeric_channel_with_renderer(mark, data, scalars, y_channel, &mark_context, 0.0)?,
    );

    let geometry = coord.transform(
        &position_channels,
        None,
        context.plot_width(),
        context.plot_height(),
    )?;

    geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .cloned()
        .ok_or_else(|| {
            AvengerChartError::CoordinateSystemError(
                "Failed to downcast transformed Cartesian point to PointGeometry".to_string(),
            )
        })
}

pub(crate) fn scene_len(data: Option<&RecordBatch>) -> u32 {
    data.map_or(1, |data| data.num_rows()) as u32
}

pub(crate) fn optional_stroke_dash(
    dash: ScalarOrArray<Option<Vec<f32>>>,
) -> Option<ScalarOrArray<Vec<f32>>> {
    match dash.value() {
        avenger_common::value::ScalarOrArrayValue::Scalar(None) => None,
        avenger_common::value::ScalarOrArrayValue::Scalar(Some(dash)) => {
            Some(ScalarOrArray::new_scalar(dash.clone()))
        }
        avenger_common::value::ScalarOrArrayValue::Array(dashes) => {
            if dashes.iter().all(Option::is_none) {
                None
            } else {
                Some(ScalarOrArray::new_array(
                    dashes
                        .iter()
                        .map(|dash| {
                            let dash = dash.clone().unwrap_or_default();
                            if dash.is_empty() {
                                vec![f32::MAX]
                            } else {
                                dash
                            }
                        })
                        .collect(),
                ))
            }
        }
    }
}
