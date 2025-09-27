//! Serializable scale configuration information
//!
//! This module provides structures to store scale configurations separately
//! from DataFrames, allowing DataFrames to be serialized without embedded UDFs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Information about a scale configuration used in a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleInfo {
    /// The name of the scale (e.g., "x", "color", etc.)
    pub scale_name: String,

    /// The type of scale (e.g., "linear", "ordinal", "band")
    pub scale_type: String,

    /// Band parameter for band/point scales (0.0 = start, 0.5 = center, 1.0 = end)
    pub band: Option<f64>,

    /// Additional scale configuration options
    pub options: HashMap<String, serde_json::Value>,
}

/// Channel value with scale information stored separately
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializableChannelValue {
    /// Expression that should be scaled (scale applied at render time)
    Scaled {
        /// The expression to scale (e.g., col("price"))
        expr_string: String,

        /// Reference to the scale configuration
        scale_info: ScaleInfo,
    },

    /// Literal value (no scaling)
    Value {
        /// The literal expression
        expr_string: String,
    },

    /// Conditional branches
    Conditional {
        /// List of (condition, value) pairs
        conditions: Vec<(String, SerializableConditionalValue)>,

        /// Default value if no conditions match
        otherwise: SerializableConditionalValue,

        /// Optional scale information for the overall conditional
        scale_info: Option<ScaleInfo>,
    },
}

/// Conditional value variant for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializableConditionalValue {
    /// Value that should be scaled
    Scaled { expr_string: String },

    /// Literal value
    Value { expr_string: String },
}

impl SerializableChannelValue {
    /// Check if this channel value requires scaling
    pub fn needs_scale(&self) -> bool {
        matches!(self, SerializableChannelValue::Scaled { .. })
            || matches!(
                self,
                SerializableChannelValue::Conditional {
                    scale_info: Some(_),
                    ..
                }
            )
    }

    /// Get the scale info if this channel uses scaling
    pub fn scale_info(&self) -> Option<&ScaleInfo> {
        match self {
            SerializableChannelValue::Scaled { scale_info, .. } => Some(scale_info),
            SerializableChannelValue::Conditional { scale_info, .. } => scale_info.as_ref(),
            SerializableChannelValue::Value { .. } => None,
        }
    }
}
