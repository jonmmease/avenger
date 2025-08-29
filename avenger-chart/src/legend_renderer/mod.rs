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
use std::sync::Arc;

/// Trait for implementing custom legend renderers
#[async_trait::async_trait]
pub trait LegendRenderer: Send + Sync + 'static {
    /// Check if this renderer can handle the given channels
    fn can_render(&self, channels: &[LegendChannel]) -> bool;

    /// Render the legend to scene marks
    async fn render(
        &self,
        channels: &[LegendChannel],
        config: &Legend,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<Option<SceneGroup>, AvengerChartError>;
}

/// Information about a channel that may contribute to a legend
#[derive(Clone, Debug)]
pub struct LegendChannel {
    pub name: String,
    pub expression: Option<Expr>,
    pub scale: ConfiguredScale,
    pub channel_type: String, // "fill", "stroke", "size", etc.
    pub mark_type: String,    // "point", "line", "rect", etc.
    pub related_channels: std::collections::HashMap<String, (Option<Expr>, ConfiguredScale)>, // Other channels from same mark
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
    pub expression: String, // Normalized expression string
    pub domain_type: DomainType,
    pub range_hash: u64, // Hash of the range values for comparison
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
    use crate::marks::channel::ChannelValue;
    use crate::utils::ScalarValueHelpers;
    use avenger_common::types::ColorOrGradient;
    use datafusion::logical_expr::Expr;
    use datafusion_common::ScalarValue;
    use std::collections::HashMap;

    /// Extract a constant scalar value from related_channels or mark_encodings
    /// Returns None if the expression is not constant (references columns)
    pub async fn get_constant_scalar(
        channel_name: &str,
        related_channels: &HashMap<String, (Option<Expr>, super::ConfiguredScale)>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<ScalarValue> {
        // First check related_channels
        if let Some((Some(expr), _)) = related_channels.get(channel_name) {
            if expr.column_refs().is_empty() {
                // Scalar expression - evaluate it
                if let Ok(scalars) =
                    crate::utils::eval_to_scalars(vec![expr.clone()], None, None).await
                {
                    return scalars.into_iter().next();
                }
            }
        }

        // Fallback to mark_encodings
        if let Some(channel_value) = mark_encodings.get(channel_name) {
            if let Some(expr) = channel_value.expr() {
                if expr.column_refs().is_empty() {
                    // Scalar expression - evaluate it
                    if let Ok(scalars) =
                        crate::utils::eval_to_scalars(vec![expr.clone()], None, None).await
                    {
                        return scalars.into_iter().next();
                    }
                }
            }
        }

        None
    }

    /// Extract a constant color value from related_channels or mark_encodings
    pub async fn get_constant_color(
        channel_name: &str,
        related_channels: &HashMap<String, (Option<Expr>, super::ConfiguredScale)>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<ColorOrGradient> {
        if let Some(scalar) =
            get_constant_scalar(channel_name, related_channels, mark_encodings).await
        {
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
    pub async fn get_constant_f32(
        channel_name: &str,
        related_channels: &HashMap<String, (Option<Expr>, super::ConfiguredScale)>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<f32> {
        if let Some(scalar) =
            get_constant_scalar(channel_name, related_channels, mark_encodings).await
        {
            scalar.as_f32().ok()
        } else {
            None
        }
    }

    /// Extract a constant string value from related_channels or mark_encodings
    pub async fn get_constant_string(
        channel_name: &str,
        related_channels: &HashMap<String, (Option<Expr>, super::ConfiguredScale)>,
        mark_encodings: &HashMap<String, ChannelValue>,
    ) -> Option<String> {
        if let Some(ScalarValue::Utf8(Some(s))) =
            get_constant_scalar(channel_name, related_channels, mark_encodings).await
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
