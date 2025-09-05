//! Default scale creation using theme and trait-based type detection

use crate::error::AvengerChartError;
use crate::render_context::RenderContext;
use crate::scales::{Scale, Auto};
use crate::utils::ScalarValueHelpers;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::lit;
use datafusion_common::ScalarValue;
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
        "x" | "y" | "x2" | "y2" | "r" | "theta" | "radius" | "angle" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Numeric,
                expected_range: RangeKind::Continuous,
            }
        }
        
        // Color channels - can be categorical or continuous
        "fill" | "stroke" | "color" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Categorical,
                expected_range: RangeKind::Discrete,
            }
        }
        
        // Opacity channels - continuous 0-1
        "opacity" | "fill_opacity" | "stroke_opacity" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Numeric,
                expected_range: RangeKind::Continuous,
            }
        }
        
        // Shape channel - categorical
        "shape" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Categorical,
                expected_range: RangeKind::Discrete,
            }
        }
        
        // Size channels - continuous positive
        "size" | "stroke_width" | "width" | "height" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Numeric,
                expected_range: RangeKind::Continuous,
            }
        }
        
        // Dash channel - categorical
        "stroke_dash" => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Categorical,
                expected_range: RangeKind::Discrete,
            }
        }
        
        // Default to continuous numeric
        _ => {
            ChannelCharacteristics {
                expected_domain: DomainKind::Numeric,
                expected_range: RangeKind::Continuous,
            }
        }
    }
}

/// Create a default scale for a channel using theme and scale traits
pub fn create_default_scale_for_channel(
    channel: &str,
    scale_impl: Arc<dyn ScaleImpl>,
    context: &RenderContext,
) -> Result<Scale<Auto>, AvengerChartError> {
    let _characteristics = get_channel_characteristics(channel); // TODO: Use for validation
    let domain_kind = scale_impl.domain_kind();
    let range_kind = scale_impl.range_kind();
    
    let mut scale = Scale::<Auto>::from_impl(scale_impl);
    
    // Set range based on channel type and theme
    match (channel, domain_kind, range_kind) {
        // Color channels with discrete range
        (ch, DomainKind::Categorical, RangeKind::Discrete) 
            if ch == "fill" || ch == "stroke" || ch == "color" => {
            let colors: Vec<ScalarValue> = context.theme.colors.categorical
                .iter()
                .map(|c| ScalarValue::Utf8(Some(c.clone())))
                .collect();
            scale = scale.range_discrete(colors);
        }
        
        // Shape channel
        ("shape", DomainKind::Categorical, RangeKind::Discrete) => {
            let shapes: Vec<ScalarValue> = context.theme.shapes.get_shape_names()
                .into_iter()
                .map(|s| ScalarValue::Utf8(Some(s)))
                .collect();
            scale = scale.range_discrete(shapes);
        }
        
        // Stroke dash channel
        ("stroke_dash", DomainKind::Categorical, RangeKind::Discrete) => {
            let dashes: Vec<ScalarValue> = context.theme.dashes.get_dash_names()
                .into_iter()
                .map(|d| ScalarValue::Utf8(Some(d)))
                .collect();
            scale = scale.range_discrete(dashes);
        }
        
        // Size channels with continuous range
        (ch, _, RangeKind::Continuous) if ch == "size" => {
            // Get default size from theme
            let default_size = context.theme.mark_defaults
                .get("symbol", "size")
                .and_then(|v| v.as_f32().ok())
                .unwrap_or(64.0);
            scale = scale.range_interval(lit(default_size * 0.5), lit(default_size * 2.0));
        }
        
        // Stroke width
        ("stroke_width", _, RangeKind::Continuous) => {
            scale = scale.range_interval(lit(0.5), lit(5.0));
        }
        
        // Opacity channels
        (ch, _, RangeKind::Continuous) 
            if ch == "opacity" || ch == "fill_opacity" || ch == "stroke_opacity" => {
            scale = scale.range_interval(lit(0.0), lit(1.0));
        }
        
        _ => {
            // Use default range already set by scale implementation
        }
    }
    
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