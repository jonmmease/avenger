//! Utility functions for extracting channel values from batches
//!
//! This module provides functions to coerce channel values from DataFusion RecordBatches
//! into typed values using the avenger-scales Coercer system.

use avenger_common::{
    types::{ColorOrGradient, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_scales::scales::coerce::Coercer;

use datafusion::arrow::record_batch::RecordBatch;
use datafusion_common::ScalarValue;

use crate::{
    chart_core::{MarkRenderContext, ScalarValueHelpers},
    error::AvengerChartError,
};

pub use avenger_chart_core::mark_channel_coercion::{
    coerce_bool_channel, coerce_channel, coerce_color_channel, coerce_numeric_channel,
    coerce_opacity_channel, coerce_stroke_cap_channel, coerce_stroke_dash_channel,
    coerce_stroke_join_channel, coerce_text_channel,
};

/// Helper to convert a ScalarValue to a ColorOrGradient using the Coercer
fn scalar_to_color(
    scalar: &ScalarValue,
    fallback: [f32; 4],
) -> Result<ColorOrGradient, AvengerChartError> {
    let array_ref = scalar.to_array()?;
    let coercer = Coercer::default();
    Ok(coercer
        .to_color(&array_ref, Some(ColorOrGradient::Color(fallback)))
        .ok()
        .and_then(|result| result.first().cloned())
        .unwrap_or(ColorOrGradient::Color(fallback)))
}

// ===== CompiledMark versions of utility functions =====

/// Get numeric channel values using Coercer with CompiledMark defaults
pub fn coerce_numeric_channel_with_renderer(
    mark: &dyn crate::marks::CompiledMark,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    context: &MarkRenderContext<'_>,
    fallback_default: f32,
) -> Result<ScalarOrArray<f32>, AvengerChartError> {
    // Get default from mark, falling back to provided default
    let default = mark
        .default_channel_value(channel, context)
        .and_then(|scalar| scalar.as_f32().ok())
        .unwrap_or(fallback_default);

    coerce_channel(
        data,
        scalars,
        channel,
        |c, a| c.to_numeric(a, Some(default)),
        default,
    )
}

/// Get color channel values using Coercer with CompiledMark defaults
pub fn coerce_color_channel_with_renderer(
    mark: &dyn crate::marks::CompiledMark,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    context: &MarkRenderContext<'_>,
    fallback_default: [f32; 4],
) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
    // Get default from mark - the mark's default_channel_value returns a ScalarValue
    // which for colors is typically a string like "#4682b4"
    let default = if let Some(default_scalar) = mark.default_channel_value(channel, context) {
        scalar_to_color(&default_scalar, fallback_default)?
    } else {
        ColorOrGradient::Color(fallback_default)
    };

    let default_for_closure = default.clone();
    coerce_channel(
        data,
        scalars,
        channel,
        move |c, a| c.to_color(a, Some(default_for_closure.clone())),
        default,
    )
}

/// Get boolean channel values using Coercer with CompiledMark defaults
pub fn coerce_bool_channel_with_renderer(
    mark: &dyn crate::marks::CompiledMark,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    context: &MarkRenderContext<'_>,
    fallback_default: bool,
) -> Result<ScalarOrArray<bool>, AvengerChartError> {
    // Get default from mark, falling back to provided default
    let default = mark
        .default_channel_value(channel, context)
        .and_then(|scalar| match scalar {
            ScalarValue::Boolean(Some(b)) => Some(b),
            _ => None,
        })
        .unwrap_or(fallback_default);

    coerce_channel(data, scalars, channel, |c, a| c.to_boolean(a), default)
}

/// Get stroke cap channel value using Coercer with CompiledMark defaults
pub fn coerce_stroke_cap_channel_with_renderer(
    mark: &dyn crate::marks::CompiledMark,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    context: &MarkRenderContext<'_>,
    fallback_default: StrokeCap,
) -> Result<StrokeCap, AvengerChartError> {
    // Get default from mark
    let default = mark
        .default_channel_value(channel, context)
        .and_then(|scalar| match scalar {
            ScalarValue::Utf8(Some(s)) => match s.as_str() {
                "butt" => Some(StrokeCap::Butt),
                "round" => Some(StrokeCap::Round),
                "square" => Some(StrokeCap::Square),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(fallback_default);

    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_cap(a), default)
        .map(|v| v.first().cloned().unwrap_or(default))
}

/// Get stroke join channel value using Coercer with CompiledMark defaults
pub fn coerce_stroke_join_channel_with_renderer(
    mark: &dyn crate::marks::CompiledMark,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    context: &MarkRenderContext<'_>,
    fallback_default: StrokeJoin,
) -> Result<StrokeJoin, AvengerChartError> {
    // Get default from mark
    let default = mark
        .default_channel_value(channel, context)
        .and_then(|scalar| match scalar {
            ScalarValue::Utf8(Some(s)) => match s.as_str() {
                "miter" => Some(StrokeJoin::Miter),
                "round" => Some(StrokeJoin::Round),
                "bevel" => Some(StrokeJoin::Bevel),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(fallback_default);

    coerce_channel(data, scalars, channel, |c, a| c.to_stroke_join(a), default)
        .map(|v| v.first().cloned().unwrap_or(default))
}
