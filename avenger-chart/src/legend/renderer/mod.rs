//! Extensible legend rendering system

pub mod colorbar;
pub mod line;
pub mod rect;
pub mod symbol;

pub use colorbar::CompiledColorbar;
pub use line::CompiledLineLegend;
pub use rect::CompiledRectLegend;
pub use symbol::CompiledSymbolLegend;

use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::SceneGroup;
use datafusion::{common::ScalarValue, logical_expr::Expr, prelude::SessionContext};
use indexmap::IndexMap;
use taffy::Size;

use crate::{
    error::AvengerChartError,
    legend::Legend,
    scales::{ConfiguredScaleLegendExt, DomainValues},
    theme::Theme,
};

/// Trait for implementing custom legend renderers
#[async_trait::async_trait]
#[typetag::serde(tag = "type")]
pub trait LegendRenderer: Send + Sync + 'static {
    /// Get the name of this renderer for debugging
    fn name(&self) -> &'static str {
        "UnnamedRenderer"
    }

    /// Check if this renderer can handle the given channels
    fn can_evaluate(&self, channels: &[LegendChannel]) -> bool;

    /// Check if this renderer supports merging these specific channels
    /// Only called when channels have matching MergeKeys (same expression, same domain)
    fn supports_merge(&self, channels: &[LegendChannel]) -> bool {
        // Only called for channels that are already verified to be mergeable
        // (same expression, same discrete domain, same mark)

        if channels.len() <= 1 {
            return true; // Single channel is always "mergeable"
        }

        // Check if this renderer can vary all the channel types
        let channel_types: HashSet<_> = channels.iter().map(|c| c.channel_type.as_str()).collect();

        // All channel types must be in the supported set
        channel_types.is_subset(&self.supported_merge_channels())
    }

    /// Return set of channel types this renderer can merge
    fn supported_merge_channels(&self) -> HashSet<&'static str> {
        HashSet::new() // Default: no merging support
    }

    /// Whether this legend prefers flexible layout (can stretch to fill space)
    /// Used by layout system to determine if legend should grow/shrink
    fn prefers_flexible_layout(&self) -> bool {
        false // Default: fixed size
    }

    /// Evaluate the legend to scene marks
    async fn evaluate(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Option<SceneGroup>, AvengerChartError>;

    /// Measure the size this legend will require by rendering it
    async fn measure(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        available_space: Size<f32>,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Size<f32>, AvengerChartError> {
        // Default implementation: render at origin and measure bounds

        if let Some(group) = self
            .evaluate(
                channels,
                config,
                0.0,
                0.0,
                available_space.width,
                available_space.height,
                theme,
                params,
                ctx,
            )
            .await?
        {
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
    pub channel_type: String,      // "fill", "stroke", "size", etc.
    pub sharing_level: Option<u8>, // Scale sharing level (0=Free, N=Level(N), 255=Shared)
    pub mark_type: String,         // "point", "line", "rect", etc.
    pub mark_index: usize,         // Index of the mark in the plot's marks array
    pub related_channels: HashMap<String, ChannelInfo>, // Other channels from same mark
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
///
/// Uses Debug format for exact matching - legends only merge if expressions are identical.
/// This conservative approach ensures that only truly identical user-specified expressions
/// result in merged legends.
pub fn normalize_expression(expr: &Expr) -> String {
    format!("{:?}", expr)
}

/// Helper functions for extracting constant values from channels
pub mod helpers {
    use std::collections::HashMap;

    use avenger_common::types::ColorOrGradient;
    use datafusion::{common::ScalarValue, prelude::SessionContext};

    use super::ChannelInfo;
    use crate::{
        channel::ChannelValue,
        utils::{ScalarValueHelpers, simplify_to_scalar_sync},
    };

    /// Extract a constant scalar value from related_channels or mark_encodings
    /// Returns None if the expression is not constant (references columns)
    pub fn get_constant_scalar(
        channel_name: &str,
        related_channels: &HashMap<String, ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
        session_context: &SessionContext,
    ) -> Option<ScalarValue> {
        // First check related_channels
        match related_channels.get(channel_name) {
            Some(ChannelInfo::Scaled {
                expr: Some(expr), ..
            })
            | Some(ChannelInfo::Constant { expr }) => {
                if expr.column_refs().is_empty() {
                    // Try to simplify - this handles literals and simple expressions
                    if let Ok(scalar) = simplify_to_scalar_sync(expr.clone()) {
                        return Some(scalar);
                    }
                }
            }
            _ => {}
        }

        // Fallback to mark_encodings
        if let Some(channel_value) = mark_encodings.get(channel_name) {
            if let Some(expr) = channel_value.expr(session_context) {
                if expr.column_refs().is_empty() {
                    // Try to simplify - this handles literals and simple expressions
                    if let Ok(scalar) = simplify_to_scalar_sync(expr.clone()) {
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
        related_channels: &HashMap<String, ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
        session_context: &SessionContext,
    ) -> Option<ColorOrGradient> {
        use avenger_scales::scales::coerce::Coercer;

        if let Some(scalar) = get_constant_scalar(
            channel_name,
            related_channels,
            mark_encodings,
            session_context,
        ) {
            // Try to convert to color
            if let Ok(color_array) = ScalarValue::iter_to_array(std::iter::once(scalar)) {
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
        related_channels: &HashMap<String, ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
        session_context: &SessionContext,
    ) -> Option<f32> {
        if let Some(scalar) = get_constant_scalar(
            channel_name,
            related_channels,
            mark_encodings,
            session_context,
        ) {
            scalar.as_f32().ok()
        } else {
            None
        }
    }

    /// Extract a constant string value from related_channels or mark_encodings
    pub fn get_constant_string(
        channel_name: &str,
        related_channels: &HashMap<String, ChannelInfo>,
        mark_encodings: &HashMap<String, ChannelValue>,
        session_context: &SessionContext,
    ) -> Option<String> {
        if let Some(scalar) = get_constant_scalar(
            channel_name,
            related_channels,
            mark_encodings,
            session_context,
        ) {
            return scalar.as_scalar_string().ok();
        };
        None
    }
}

/// Compute hash of range values for merging comparison
pub fn compute_range_hash(values: &[ScalarValue]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for value in values {
        format!("{:?}", value).hash(&mut hasher);
    }
    hasher.finish()
}
