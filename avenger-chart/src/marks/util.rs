//! Utility functions for extracting channel values from batches

use crate::error::AvengerChartError;
use avenger_common::types::{ColorOrGradient, StrokeCap, StrokeJoin};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::coerce::Coercer;
use datafusion::arrow::array::{Array, ArrayRef, Float32Array, Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::scalar::ScalarValue;
use std::collections::HashMap;

/// Coerce a channel from either data or scalar batch using the provided coercion function
/// First checks data batch for array values, then scalar batch for scalar values
pub fn coerce_channel<T, F>(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    coerce_fn: F,
    default: T,
) -> Result<ScalarOrArray<T>, AvengerChartError>
where
    T: Clone + Sync,
    F: Fn(
        &Coercer,
        &ArrayRef,
    ) -> Result<ScalarOrArray<T>, avenger_scales::error::AvengerScaleError>,
{
    let coercer = Coercer::default();

    // First check data batch for array values
    if let Some(data_batch) = data {
        if let Some(array) = data_batch.column_by_name(channel) {
            return coerce_fn(&coercer, array).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Error coercing channel '{}': {}",
                    channel, e
                ))
            });
        }
    }

    // Then check scalar batch
    if let Some(array) = scalars.column_by_name(channel) {
        coerce_fn(&coercer, array)
            .map(|v| v.to_scalar_if_len_one())
            .map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Error coercing channel '{}': {}",
                    channel, e
                ))
            })
    } else {
        Ok(ScalarOrArray::new_scalar(default))
    }
}

/// Get numeric channel values using Coercer
pub fn coerce_numeric_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: f32,
) -> Result<ScalarOrArray<f32>, AvengerChartError> {
    coerce_channel(
        data,
        scalars,
        channel,
        |c, a| c.to_numeric(a, Some(default)),
        default,
    )
}

/// Get color channel values using Coercer
pub fn coerce_color_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: [f32; 4],
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
    let default_color = ColorOrGradient::Color(default);
    coerce_channel(
        data,
        scalars,
        channel,
        |c, a| c.to_color(a, Some(ColorOrGradient::Color(default))),
        default_color,
    )
}

/// Get boolean channel values using Coercer
pub fn coerce_bool_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: bool,
) -> Result<ScalarOrArray<bool>, AvengerChartError> {
    coerce_channel(data, scalars, channel, |c, a| c.to_boolean(a), default)
}

/// Get stroke cap channel value using Coercer
pub fn coerce_stroke_cap_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: StrokeCap,
) -> Result<StrokeCap, AvengerChartError> {
    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_cap(a), default).map(|v| {
        // Since stroke_cap must be scalar, extract the scalar value
        match v.value() {
            ScalarOrArrayValue::Scalar(cap) => *cap,
            ScalarOrArrayValue::Array(caps) => {
                // Take first value or default if empty
                caps.first().cloned().unwrap_or(default)
            }
        }
    })
}

/// Get stroke join channel value using Coercer
pub fn coerce_stroke_join_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: StrokeJoin,
) -> Result<StrokeJoin, AvengerChartError> {
    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_join(a), default).map(|v| {
        // Since stroke_join must be scalar, extract the scalar value
        match v.value() {
            ScalarOrArrayValue::Scalar(join) => *join,
            ScalarOrArrayValue::Array(joins) => {
                // Take first value or default if empty
                joins.first().cloned().unwrap_or(default)
            }
        }
    })
}

/// Get stroke dash channel value using Coercer
pub fn coerce_stroke_dash_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
) -> Result<ScalarOrArray<Option<Vec<f32>>>, AvengerChartError> {
    // First check if the channel exists
    if data.and_then(|d| d.column_by_name(channel)).is_none()
        && scalars.column_by_name(channel).is_none()
    {
        return Ok(ScalarOrArray::new_scalar(None));
    }

    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_dash(a), vec![]).map(|v| {
        // Convert Vec<f32> to Option<Vec<f32>> where empty vec means None
        match v.value() {
            ScalarOrArrayValue::Scalar(dash) => {
                if dash.is_empty() {
                    ScalarOrArray::new_scalar(None)
                } else {
                    ScalarOrArray::new_scalar(Some(dash.clone()))
                }
            }
            ScalarOrArrayValue::Array(dashes) => {
                let options: Vec<Option<Vec<f32>>> = dashes
                    .iter()
                    .map(|d| if d.is_empty() { None } else { Some(d.clone()) })
                    .collect();
                ScalarOrArray::new_array(options)
            }
        }
    })
}

/// Get numeric channel values as a vector
pub fn get_numeric_channel(
    channel: &str,
    batch: Option<&RecordBatch>,
    scalars: &HashMap<String, ScalarValue>,
    default: f32,
) -> Result<Vec<f32>, AvengerChartError> {
    if let Some(batch) = batch {
        if let Some(array) = batch.column_by_name(channel) {
            extract_numeric_array(array)
        } else if let Some(scalar) = scalars.get(channel) {
            let value = scalar_to_f32(scalar).unwrap_or(default);
            Ok(vec![value; batch.num_rows()])
        } else {
            Ok(vec![default; batch.num_rows()])
        }
    } else {
        // Pure scalar case - single mark
        let value = scalars
            .get(channel)
            .and_then(scalar_to_f32)
            .unwrap_or(default);
        Ok(vec![value])
    }
}

/// Get numeric channel values as ScalarOrArray for memory efficiency
pub fn get_numeric_channel_scalar_or_array(
    channel: &str,
    batch: Option<&RecordBatch>,
    scalars: &HashMap<String, ScalarValue>,
    default: f32,
) -> Result<ScalarOrArray<f32>, AvengerChartError> {
    // Check if we have a scalar value
    if let Some(scalar) = scalars.get(channel) {
        let value = scalar_to_f32(scalar).unwrap_or(default);
        return Ok(ScalarOrArray::new_scalar(value));
    }

    // Otherwise try to get array from batch
    if let Some(batch) = batch {
        if let Some(array) = batch.column_by_name(channel) {
            let values = extract_numeric_array(array)?;
            return Ok(ScalarOrArray::new_array(values));
        }
    }

    // Default scalar
    Ok(ScalarOrArray::new_scalar(default))
}

/// Get text channel values
pub fn get_text_channel(
    channel: &str,
    batch: Option<&RecordBatch>,
    scalars: &HashMap<String, ScalarValue>,
    default: &str,
) -> Result<Vec<String>, AvengerChartError> {
    if let Some(batch) = batch {
        if let Some(array) = batch.column_by_name(channel) {
            extract_text_array(array)
        } else if let Some(scalar) = scalars.get(channel) {
            let value = scalar_to_string(scalar).unwrap_or_else(|| default.to_string());
            Ok(vec![value; batch.num_rows()])
        } else {
            Ok(vec![default.to_string(); batch.num_rows()])
        }
    } else {
        // Pure scalar case
        let value = scalars
            .get(channel)
            .and_then(scalar_to_string)
            .unwrap_or_else(|| default.to_string());
        Ok(vec![value])
    }
}

/// Extract numeric values from an Arrow array
pub fn extract_numeric_array(array: &dyn Array) -> Result<Vec<f32>, AvengerChartError> {
    let num_rows = array.len();

    if let Some(float_array) = array.as_any().downcast_ref::<Float32Array>() {
        Ok((0..num_rows).map(|i| float_array.value(i)).collect())
    } else if let Some(float64_array) = array.as_any().downcast_ref::<Float64Array>() {
        Ok((0..num_rows)
            .map(|i| float64_array.value(i) as f32)
            .collect())
    } else {
        Err(AvengerChartError::InternalError(format!(
            "Expected numeric array but got {:?}",
            array.data_type()
        )))
    }
}

/// Extract text values from an Arrow array
pub fn extract_text_array(array: &dyn Array) -> Result<Vec<String>, AvengerChartError> {
    let num_rows = array.len();

    if let Some(string_array) = array.as_any().downcast_ref::<StringArray>() {
        Ok((0..num_rows)
            .map(|i| {
                if string_array.is_null(i) {
                    String::new()
                } else {
                    string_array.value(i).to_string()
                }
            })
            .collect())
    } else {
        Err(AvengerChartError::InternalError(format!(
            "Expected string array but got {:?}",
            array.data_type()
        )))
    }
}

/// Convert ScalarValue to f32
pub fn scalar_to_f32(scalar: &ScalarValue) -> Option<f32> {
    match scalar {
        ScalarValue::Float32(Some(v)) => Some(*v),
        ScalarValue::Float64(Some(v)) => Some(*v as f32),
        ScalarValue::Int32(Some(v)) => Some(*v as f32),
        ScalarValue::Int64(Some(v)) => Some(*v as f32),
        ScalarValue::UInt32(Some(v)) => Some(*v as f32),
        ScalarValue::UInt64(Some(v)) => Some(*v as f32),
        _ => None,
    }
}

/// Convert ScalarValue to string
pub fn scalar_to_string(scalar: &ScalarValue) -> Option<String> {
    match scalar {
        ScalarValue::Utf8(Some(s)) => Some(s.clone()),
        ScalarValue::LargeUtf8(Some(s)) => Some(s.clone()),
        _ => None,
    }
}
