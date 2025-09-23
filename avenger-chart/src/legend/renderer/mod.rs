//! Extensible legend rendering system

pub mod colorbar;
pub mod line;
pub mod rect;
pub mod symbol;

pub use colorbar::ColorbarRenderer;
pub use line::LineLegendRenderer;
pub use rect::RectLegendRenderer;
pub use symbol::SymbolLegendRenderer;

use crate::error::AvengerChartError;
use crate::legend::Legend;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::SceneGroup;
use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Trait for implementing custom legend renderers
#[typetag::serde(tag = "type")]
pub trait LegendRenderer: Send + Sync + 'static {
    /// Get the name of this renderer for debugging
    fn name(&self) -> &'static str {
        "UnnamedRenderer"
    }

    /// Check if this renderer can handle the given channels
    fn can_render(&self, channels: &[LegendChannel]) -> bool;

    /// Check if this renderer supports merging these specific channels
    /// Only called when channels have matching MergeKeys (same expression, same domain)
    fn supports_merge(&self, channels: &[LegendChannel]) -> bool {
        // Only called for channels that are already verified to be mergeable
        // (same expression, same discrete domain, same mark)

        if channels.len() <= 1 {
            return true; // Single channel is always "mergeable"
        }

        // Check if this renderer can vary all the channel types
        let channel_types: std::collections::HashSet<_> =
            channels.iter().map(|c| c.channel_type.as_str()).collect();

        // All channel types must be in the supported set
        channel_types.is_subset(&self.supported_merge_channels())
    }

    /// Return set of channel types this renderer can merge
    fn supported_merge_channels(&self) -> std::collections::HashSet<&'static str> {
        std::collections::HashSet::new() // Default: no merging support
    }

    /// Whether this legend prefers flexible layout (can stretch to fill space)
    /// Used by layout system to determine if legend should grow/shrink
    fn prefers_flexible_layout(&self) -> bool {
        false // Default: fixed size
    }

    /// Render the legend to scene marks
    fn render(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<Option<SceneGroup>, AvengerChartError>;

    /// Measure the size this legend will require by rendering it
    fn measure(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        available_space: taffy::Size<f32>,
    ) -> Result<taffy::Size<f32>, AvengerChartError> {
        // Default implementation: render at origin and measure bounds
        use avenger_geometry::marks::MarkGeometryUtils;
        use avenger_geometry::rtree::EnvelopeUtils;
        use taffy::Size;

        if let Some(group) = self.render(
            channels,
            config,
            0.0,
            0.0,
            available_space.width,
            available_space.height,
        )? {
            let bounds = group.bounding_box();

            // Account for stroke width on background if present
            // Background strokes extend 0.5 pixels outside on each side (total 1.0 pixel)
            let stroke_adjustment = if config.background_stroke.is_set() {
                1.0 // Total stroke width that extends beyond the fill
            } else {
                0.0
            };

            Ok(Size {
                width: bounds.width() - stroke_adjustment,
                height: bounds.height() - stroke_adjustment,
            })
        } else {
            Ok(Size {
                width: 0.0,
                height: 0.0,
            })
        }
    }
}

/// Information about a related channel that may affect legend rendering
#[derive(Clone, Debug)]
pub enum ChannelInfo {
    /// Channel that varies based on a scale
    Scaled {
        expr: Option<Expr>,
        scale: ConfiguredScale,
    },
    /// Channel with a constant value (no scale needed)
    Constant { expr: Expr },
}

/// Information about a channel that may contribute to a legend
#[derive(Clone, Debug)]
pub struct LegendChannel {
    pub name: String,
    pub expression: Option<Expr>,
    pub scale: ConfiguredScale,
    pub channel_type: String, // "fill", "stroke", "size", etc.
    pub mark_type: String,    // "point", "line", "rect", etc.
    pub mark_index: usize,    // Index of the mark in the plot's marks array
    pub related_channels: std::collections::HashMap<String, ChannelInfo>, // Other channels from same mark
}

/// Type of domain for merging purposes
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum DomainType {
    Discrete,
    Continuous,
}

/// Key for identifying mergeable channels
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MergeKey {
    /// Normalized expression (e.g., "col(category)")
    pub expression: String,
    /// The actual domain values for exact matching
    pub domain_values: Vec<ScalarValue>,
    /// Mark index for same-mark merging
    pub mark_index: usize,
}

impl MergeKey {
    /// Create a merge key from a legend channel
    pub fn from_channel(channel: &LegendChannel) -> Option<Self> {
        use crate::scales::{ConfiguredScaleLegendExt, DomainValues};

        // Only discrete scales can be merged
        let domain_values = match channel.scale.domain_values().ok()? {
            DomainValues::Discrete(values) => values,
            DomainValues::Interval(_, _) => return None, // No merging for continuous
        };

        // Must have an expression to merge
        let expression = channel.expression.as_ref().map(normalize_expression)?;

        Some(MergeKey {
            expression,
            domain_values,
            mark_index: channel.mark_index,
        })
    }

    /// Check if two channels are mergeable
    pub fn is_mergeable(a: &LegendChannel, b: &LegendChannel) -> bool {
        // Generate keys for both
        match (Self::from_channel(a), Self::from_channel(b)) {
            (Some(key_a), Some(key_b)) => key_a == key_b,
            _ => false,
        }
    }
}

/// Trait for channel configs to declare legend capabilities
pub trait ChannelLegendCapability {
    /// Get the legend renderer for this channel
    fn legend_renderer(&self) -> Arc<dyn LegendRenderer>;

    /// Check if this channel can merge with another
    fn can_merge_with(&self, other: &dyn ChannelLegendCapability) -> bool;

    /// Get the merge key for grouping compatible channels
    fn merge_key(&self) -> Option<MergeKey>;

    /// Get the expression for this channel
    fn expression(&self) -> Option<Expr>;

    /// Get the channel type (fill, stroke, size, etc.)
    fn channel_type(&self) -> &str;
}

/// Group of channels that will be rendered together in one legend
pub struct LegendGroup {
    pub channels: Vec<LegendChannel>,
    pub renderer: Arc<dyn LegendRenderer>,
    pub merge_key: Option<MergeKey>,
    pub config: Legend,
}

/// Utility to normalize expressions for comparison
pub fn normalize_expression(expr: &Expr) -> String {
    // Simple normalization - in practice would want more sophisticated comparison
    format!("{:?}", expr)
}

/// Helper functions for extracting constant values from channels
pub mod helpers {
    use crate::channel::ChannelValue;
    use crate::utils::ScalarValueHelpers;
    use avenger_common::types::ColorOrGradient;
    use datafusion_common::ScalarValue;
    use std::collections::HashMap;

    /// Extract a constant scalar value from related_channels or mark_encodings
    /// Returns None if the expression is not constant (references columns)
    pub fn get_constant_scalar(
        channel_name: &str,
        related_channels: &HashMap<String, super::ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<ScalarValue> {
        // First check related_channels
        match related_channels.get(channel_name) {
            Some(super::ChannelInfo::Scaled {
                expr: Some(expr), ..
            })
            | Some(super::ChannelInfo::Constant { expr }) => {
                if expr.column_refs().is_empty() {
                    // Try to simplify - this handles literals and simple expressions
                    if let Ok(scalar) = crate::utils::simplify_to_scalar_sync(expr.clone()) {
                        return Some(scalar);
                    }
                }
            }
            _ => {}
        }

        // Fallback to mark_encodings
        if let Some(channel_value) = mark_encodings.get(channel_name) {
            if let Some(expr) = channel_value.expr() {
                if expr.column_refs().is_empty() {
                    // Try to simplify - this handles literals and simple expressions
                    if let Ok(scalar) = crate::utils::simplify_to_scalar_sync(expr.clone()) {
                        return Some(scalar);
                    }
                }
            }
        }

        None
    }

    /// Extract a constant color value from related_channels or mark_encodings
    pub fn get_constant_color(
        channel_name: &str,
        related_channels: &HashMap<String, super::ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<ColorOrGradient> {
        if let Some(scalar) = get_constant_scalar(channel_name, related_channels, mark_encodings) {
            // Try to convert to color
            if let Ok(color_array) = ScalarValue::iter_to_array(std::iter::once(scalar)) {
                use avenger_scales::scales::coerce::Coercer;
                let coercer = Coercer::default();
                if let Ok(colors) = coercer.to_color(&color_array, None) {
                    if let Some(color) = colors.as_vec(1, None).first() {
                        return Some(color.clone());
                    }
                }
            }
        }
        None
    }

    /// Extract a constant f32 value from related_channels or mark_encodings
    pub fn get_constant_f32(
        channel_name: &str,
        related_channels: &HashMap<String, super::ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<f32> {
        if let Some(scalar) = get_constant_scalar(channel_name, related_channels, mark_encodings) {
            scalar.as_f32().ok()
        } else {
            None
        }
    }

    /// Extract a constant string value from related_channels or mark_encodings
    pub fn get_constant_string(
        channel_name: &str,
        related_channels: &HashMap<String, super::ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<String> {
        if let Some(ScalarValue::Utf8(Some(s))) =
            get_constant_scalar(channel_name, related_channels, mark_encodings)
        {
            return Some(s);
        }
        None
    }
}

/// Compute hash of range values for merging comparison
pub fn compute_range_hash(values: &[ScalarValue]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for value in values {
        format!("{:?}", value).hash(&mut hasher);
    }
    hasher.finish()
}
