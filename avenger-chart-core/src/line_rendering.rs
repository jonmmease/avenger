use std::sync::Arc;

use crate::AvengerChartError;
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::coerce::Coercer;
use datafusion::arrow::array::{ArrayRef, StringArray};

#[doc(hidden)]
pub fn optional_stroke_dash(
    dash: ScalarOrArray<Option<Vec<f32>>>,
) -> Option<ScalarOrArray<Vec<f32>>> {
    match dash.value() {
        ScalarOrArrayValue::Scalar(None) => None,
        ScalarOrArrayValue::Scalar(Some(dash)) => Some(ScalarOrArray::new_scalar(dash.clone())),
        ScalarOrArrayValue::Array(dashes) => {
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

#[doc(hidden)]
pub fn coerce_color_strings(
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

#[doc(hidden)]
pub fn color_channel_strings(values: &ScalarOrArray<ColorOrGradient>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(color_or_gradient_string)
        .collect()
}

#[doc(hidden)]
pub fn stroke_cap_strings(values: &ScalarOrArray<StrokeCap>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(stroke_cap_name)
        .collect()
}

#[doc(hidden)]
pub fn coerce_stroke_cap_strings(
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

#[doc(hidden)]
pub fn stroke_join_strings(values: &ScalarOrArray<StrokeJoin>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(stroke_join_name)
        .collect()
}

#[doc(hidden)]
pub fn coerce_stroke_join_strings(
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

#[doc(hidden)]
pub fn stroke_dash_strings(values: &Option<ScalarOrArray<Vec<f32>>>, len: usize) -> Vec<String> {
    match values {
        None => vec!["solid".to_string(); len],
        Some(values) => values
            .as_vec(len, None)
            .into_iter()
            .map(stroke_dash_string)
            .collect(),
    }
}

#[doc(hidden)]
pub fn coerce_stroke_dash_strings(
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

#[doc(hidden)]
pub fn stroke_dash_from_name(value: &str) -> Result<Option<Vec<f32>>, AvengerChartError> {
    Ok(
        coerce_stroke_dash_strings(&[value.to_string()], "stroke_dash")?
            .and_then(|dash| dash.first().cloned())
            .filter(|dash| !dash.is_empty() && dash.as_slice() != [f32::MAX]),
    )
}

#[doc(hidden)]
pub fn gather_by_indices<T: Sync + Clone>(
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
