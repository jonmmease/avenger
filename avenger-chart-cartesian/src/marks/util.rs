use std::collections::HashMap;

use avenger_chart_core::{
    AdjustmentTransformContext, AvengerChartError, CompiledMarkCore, CoordinateSystemTransformCore,
    MarkRuntimeContext, PlotAreaInfo, PointGeometry, coerce_numeric_channel_with_renderer,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::coerce::Coercer;
use datafusion::arrow::{
    array::{ArrayRef, StringArray},
    record_batch::RecordBatch,
};
use std::sync::Arc;

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

pub(crate) fn adjustment_transform_context(
    context: &dyn MarkRuntimeContext,
) -> AdjustmentTransformContext<'_> {
    AdjustmentTransformContext::new().with_plot_area(PlotAreaInfo {
        facet_path: context.facet_path(),
        width: context.plot_width(),
        height: context.plot_height(),
        origin: context.plot_area_origin(),
        clip: context.plot_area_clip(),
    })
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

pub(crate) fn coerce_color_strings(
    values: &[String],
    channel: &str,
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
    let array = Arc::new(StringArray::from(values.to_vec())) as ArrayRef;
    Coercer::default()
        .to_color(&array, None)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Error coercing adjusted color channel '{channel}': {error}"
            ))
        })
}

pub(crate) fn color_channel_strings(
    values: &ScalarOrArray<ColorOrGradient>,
    len: usize,
) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(color_or_gradient_string)
        .collect()
}

pub(crate) fn stroke_cap_strings(values: &ScalarOrArray<StrokeCap>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(stroke_cap_name)
        .collect()
}

pub(crate) fn coerce_stroke_cap_strings(
    values: &[String],
    channel: &str,
) -> Result<ScalarOrArray<StrokeCap>, AvengerChartError> {
    let array = Arc::new(StringArray::from(values.to_vec())) as ArrayRef;
    Coercer::default()
        .to_stroke_cap(&array)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Error coercing adjusted stroke cap channel '{channel}': {error}"
            ))
        })
}

pub(crate) fn stroke_join_strings(values: &ScalarOrArray<StrokeJoin>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(stroke_join_name)
        .collect()
}

pub(crate) fn coerce_stroke_join_strings(
    values: &[String],
    channel: &str,
) -> Result<ScalarOrArray<StrokeJoin>, AvengerChartError> {
    let array = Arc::new(StringArray::from(values.to_vec())) as ArrayRef;
    Coercer::default()
        .to_stroke_join(&array)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Error coercing adjusted stroke join channel '{channel}': {error}"
            ))
        })
}

pub(crate) fn stroke_dash_strings(
    values: &Option<ScalarOrArray<Vec<f32>>>,
    len: usize,
) -> Vec<String> {
    match values {
        None => vec!["solid".to_string(); len],
        Some(values) => values
            .as_vec(len, None)
            .into_iter()
            .map(stroke_dash_string)
            .collect(),
    }
}

pub(crate) fn coerce_stroke_dash_strings(
    values: &[String],
    channel: &str,
) -> Result<Option<ScalarOrArray<Vec<f32>>>, AvengerChartError> {
    let array = Arc::new(StringArray::from(values.to_vec())) as ArrayRef;
    let dashes = Coercer::default().to_stroke_dash(&array).map_err(|error| {
        AvengerChartError::InvalidArgument(format!(
            "Error coercing adjusted stroke dash channel '{channel}': {error}"
        ))
    })?;
    Ok(optional_stroke_dash(match dashes.value() {
        ScalarOrArrayValue::Scalar(dash) => {
            if dash.is_empty() {
                ScalarOrArray::new_scalar(None)
            } else {
                ScalarOrArray::new_scalar(Some(dash.clone()))
            }
        }
        ScalarOrArrayValue::Array(dashes) => ScalarOrArray::new_array(
            dashes
                .iter()
                .map(|dash| {
                    if dash.is_empty() {
                        None
                    } else {
                        Some(dash.clone())
                    }
                })
                .collect(),
        ),
    }))
}

fn color_or_gradient_string(value: ColorOrGradient) -> String {
    match value {
        ColorOrGradient::Color(color) => {
            let r = (color[0].clamp(0.0, 1.0) * 255.0).round();
            let g = (color[1].clamp(0.0, 1.0) * 255.0).round();
            let b = (color[2].clamp(0.0, 1.0) * 255.0).round();
            let a = color[3].clamp(0.0, 1.0);
            format!("rgba({r}, {g}, {b}, {a})")
        }
        ColorOrGradient::GradientIndex(index) => format!("gradient({index})"),
    }
}

fn stroke_cap_name(value: StrokeCap) -> String {
    match value {
        StrokeCap::Butt => "butt",
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
    }
    .to_string()
}

fn stroke_join_name(value: StrokeJoin) -> String {
    match value {
        StrokeJoin::Bevel => "bevel",
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
    }
    .to_string()
}

fn stroke_dash_string(value: Vec<f32>) -> String {
    if value.is_empty() || value == [f32::MAX] {
        "solid".to_string()
    } else {
        value
            .into_iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(crate) fn gather_by_indices<T: Sync + Clone>(
    values: &ScalarOrArray<T>,
    len: usize,
    indices: &[usize],
) -> ScalarOrArray<T> {
    let values = values.as_vec(len, None);
    ScalarOrArray::new_array(
        indices
            .iter()
            .filter_map(|index| values.get(*index).cloned())
            .collect(),
    )
}
