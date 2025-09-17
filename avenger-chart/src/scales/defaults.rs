//! Default scale creation using theme and trait-based type detection

use crate::error::AvengerChartError;
use crate::render_context::RenderContext;
use crate::scales::{Auto, Scale};
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::arrow::datatypes::DataType;
use std::sync::Arc;

/// Channel characteristics for scale selection
#[derive(Debug, Clone, Copy)]
pub struct ChannelCharacteristics {
    pub expected_domain: DomainKind,
    pub expected_range: RangeKind,
}

/// Get the expected characteristics of a channel
pub fn get_channel_characteristics(channel: &str) -> ChannelCharacteristics {
    match channel {
        // Position channels - typically continuous
        "x" | "y" | "x2" | "y2" | "r" | "theta" | "radius" | "angle" => ChannelCharacteristics {
            expected_domain: DomainKind::Numeric,
            expected_range: RangeKind::Continuous,
        },

        // Color channels - can be categorical or continuous
        "fill" | "stroke" | "color" => ChannelCharacteristics {
            expected_domain: DomainKind::Categorical,
            expected_range: RangeKind::Discrete,
        },

        // Opacity channels - continuous 0-1
        "opacity" | "fill_opacity" | "stroke_opacity" => ChannelCharacteristics {
            expected_domain: DomainKind::Numeric,
            expected_range: RangeKind::Continuous,
        },

        // Shape channel - categorical
        "shape" => ChannelCharacteristics {
            expected_domain: DomainKind::Categorical,
            expected_range: RangeKind::Discrete,
        },

        // Size channels - continuous positive
        "size" | "stroke_width" | "width" | "height" => ChannelCharacteristics {
            expected_domain: DomainKind::Numeric,
            expected_range: RangeKind::Continuous,
        },

        // Dash channel - categorical
        "stroke_dash" => ChannelCharacteristics {
            expected_domain: DomainKind::Categorical,
            expected_range: RangeKind::Discrete,
        },

        // Default to continuous numeric
        _ => ChannelCharacteristics {
            expected_domain: DomainKind::Numeric,
            expected_range: RangeKind::Continuous,
        },
    }
}

/// Create a default scale for a channel using theme and scale traits
pub fn create_default_scale_for_channel(
    channel: &str,
    scale_impl: Arc<dyn ScaleImpl>,
    context: &RenderContext,
) -> Result<Scale<Auto>, AvengerChartError> {
    let _characteristics = get_channel_characteristics(channel); // TODO: Use for validation
    let range_kind = scale_impl.range_kind();

    let mut scale = Scale::<Auto>::from_impl(scale_impl);

    // Use generic "mark" type for global scales
    // Individual marks will override with their specific type in default_channel_range
    let mark_type = "mark";

    // Use the new get_range_for_channel method which supports CSS discrete/continuous properties
    let range = context
        .theme
        .get_range_for_channel(mark_type, channel, range_kind, None);
    scale = scale.range(range);

    Ok(scale)
}

/// Determine if a data type represents categorical data
pub fn is_categorical_data_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean
    )
}

/// Determine if a data type represents temporal data
pub fn is_temporal_data_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
    )
}

/// Determine if a data type represents numeric data
pub fn is_numeric_data_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Float32
            | DataType::Float64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}
