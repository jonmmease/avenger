//! Utility functions for extracting channel values from batches

use crate::error::AvengerChartError;
use avenger_common::types::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::coerce::Coercer;
use datafusion::arrow::array::{
    Array, ArrayRef, Float32Array, Float64Array, ListArray, StringArray,
};
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
            return coerce_fn(&coercer, array)
                .map_err(|e| AvengerChartError::InternalError(e.to_string()));
        }
    }

    // Then check scalar batch
    if let Some(array) = scalars.column_by_name(channel) {
        coerce_fn(&coercer, array)
            .map(|v| v.to_scalar_if_len_one())
            .map_err(|e| AvengerChartError::InternalError(e.to_string()))
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

/// Get color channel values
pub fn get_color_channel(
    channel: &str,
    batch: Option<&RecordBatch>,
    scalars: &HashMap<String, ScalarValue>,
    default: [f32; 4],
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
    // Check if we have a scalar color
    if let Some(scalar) = scalars.get(channel) {
        let color = scalar_to_color(scalar).unwrap_or(default);
        return Ok(ScalarOrArray::new_scalar(ColorOrGradient::Color(color)));
    }

    // Otherwise try to get array from batch
    if let Some(batch) = batch {
        if let Some(array) = batch.column_by_name(channel) {
            return extract_color_array(array, default);
        }
    }

    // Default scalar
    Ok(ScalarOrArray::new_scalar(ColorOrGradient::Color(default)))
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

/// Extract color values from an Arrow array (handles color scale outputs)
pub fn extract_color_array(
    array: &dyn Array,
    default: [f32; 4],
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
    if let Some(list_array) = array.as_any().downcast_ref::<ListArray>() {
        // Handle ListArray output from color scales
        if list_array.len() == 1 && !list_array.is_null(0) {
            // Single color for all instances
            let rgba_array = list_array.value(0);
            if let Some(f32_array) = rgba_array.as_any().downcast_ref::<Float32Array>() {
                if f32_array.len() >= 4 {
                    let color = [
                        f32_array.value(0),
                        f32_array.value(1),
                        f32_array.value(2),
                        f32_array.value(3),
                    ];
                    return Ok(ScalarOrArray::new_scalar(ColorOrGradient::Color(color)));
                }
            }
        } else if list_array.len() > 1 {
            // Multiple colors - one per instance
            let mut colors = Vec::with_capacity(list_array.len());
            for i in 0..list_array.len() {
                if list_array.is_null(i) {
                    colors.push(default);
                } else {
                    let rgba_array = list_array.value(i);
                    if let Some(f32_array) = rgba_array.as_any().downcast_ref::<Float32Array>() {
                        if f32_array.len() >= 4 {
                            let color = [
                                f32_array.value(0),
                                f32_array.value(1),
                                f32_array.value(2),
                                f32_array.value(3),
                            ];
                            colors.push(color);
                        } else {
                            colors.push(default);
                        }
                    } else {
                        colors.push(default);
                    }
                }
            }
            return Ok(ScalarOrArray::new_array(
                colors.into_iter().map(ColorOrGradient::Color).collect(),
            ));
        }
    }

    Ok(ScalarOrArray::new_scalar(ColorOrGradient::Color(default)))
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

/// Convert ScalarValue to color array
pub fn scalar_to_color(scalar: &ScalarValue) -> Option<[f32; 4]> {
    match scalar {
        ScalarValue::Utf8(Some(s)) => parse_color_string(s),
        // TODO: Handle other color representations
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

/// Parse color string to RGBA array
fn parse_color_string(_color: &str) -> Option<[f32; 4]> {
    // TODO: Implement color parsing (hex, named colors, etc.)
    // For now, return None - marks will use defaults
    None
}
