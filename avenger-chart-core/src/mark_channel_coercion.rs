//! Utility functions for extracting channel values from batches
//!
//! This module provides functions to coerce channel values from DataFusion RecordBatches
//! into typed values using the avenger-scales Coercer system.

use avenger_common::{
    types::{ColorOrGradient, StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::coerce::Coercer;

use datafusion::arrow::{array::ArrayRef, record_batch::RecordBatch};

use crate::AvengerChartError;

/// Coerce a channel from either data or scalar batch using the provided coercion function
///
/// # Arguments
/// * `data` - Optional data batch containing array values (multiple rows)
/// * `scalars` - Scalar batch containing scalar values (single row)
/// * `channel` - Channel name to extract
/// * `coerce_fn` - Function to coerce the array to the desired type
/// * `default` - Default value if channel not found
///
/// # Returns
/// - If channel found in data batch: returns as array
/// - If channel found in scalar batch: returns as scalar (since it's a single row)
/// - Otherwise: returns default as scalar
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
    if let Some(data_batch) = data
        && let Some(array) = data_batch.column_by_name(channel)
    {
        return coerce_fn(&coercer, array).map_err(|e| {
            AvengerChartError::InternalError(format!("Error coercing channel '{}': {}", channel, e))
        });
    }

    // Then check scalar batch (single row, so return as scalar)
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
/// Note: stroke_cap must be scalar (constant for entire mark)
/// If an array is provided, takes the first value
pub fn coerce_stroke_cap_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: StrokeCap,
) -> Result<StrokeCap, AvengerChartError> {
    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_cap(a), default)
        .map(|v| v.first().cloned().unwrap_or(default))
}

/// Get stroke join channel value using Coercer
/// Note: stroke_join must be scalar (constant for entire mark)
/// If an array is provided, takes the first value
pub fn coerce_stroke_join_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: StrokeJoin,
) -> Result<StrokeJoin, AvengerChartError> {
    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_join(a), default)
        .map(|v| v.first().cloned().unwrap_or(default))
}

/// Get text channel values using Coercer
pub fn coerce_text_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: String,
) -> Result<ScalarOrArray<String>, AvengerChartError> {
    let default_ref = default.clone();
    coerce_channel(
        data,
        scalars,
        channel,
        move |c, a| c.to_string(a, Some(&default_ref)),
        default,
    )
}

/// Get opacity channel value using Coercer
/// Ensures values are clamped to [0.0, 1.0] range
pub fn coerce_opacity_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: f32,
) -> Result<ScalarOrArray<f32>, AvengerChartError> {
    let clamped_default = default.clamp(0.0, 1.0);
    coerce_channel(
        data,
        scalars,
        channel,
        |c, a| {
            c.to_numeric(a, Some(clamped_default))
                .map(|values| values.map(|v| v.clamp(0.0, 1.0)))
        },
        clamped_default,
    )
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
