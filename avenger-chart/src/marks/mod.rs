pub mod channel;
pub mod channel_descriptor;
pub mod data_context;
pub mod facet_strategy;
pub mod line;
pub mod rect;
pub mod state;
pub mod symbol;
pub mod util;
#[macro_use]
pub mod macros;
#[macro_use]
pub mod channel_macros;
#[macro_use]
pub mod position_channel_macros;

pub use channel::{ChannelValue, ConditionalValue};
pub use channel_descriptor::{ChannelDefault, ChannelDescriptor};
pub use data_context::DataContext;
pub use facet_strategy::FacetStrategy;
pub use state::MarkState;
pub use util::default_scale_for_data_type;

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::legend::LegendRenderer;
use crate::render_context::RenderContext;
use crate::scales::ScaleRange;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::scalar::ScalarValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Expression for computing radius/padding requirements for marks
///
/// Used to determine how much space a mark needs beyond its base position,
/// accounting for visual properties like size, stroke width, etc.
#[derive(Debug, Clone)]
pub enum RadiusExpression {
    /// Same radius in all directions (e.g., circular symbols)
    Symmetric(Expr),
    /// Different radius for negative and positive directions (e.g., bars extending from baseline)
    Asymmetric {
        /// Radius in the negative direction
        lower: Expr,
        /// Radius in the positive direction
        upper: Expr,
    },
}

/// Core trait for all mark types
pub trait Mark<C: CoordinateSystem>: Send + Sync + 'static {
    /// Get the mark's state
    fn state(&self) -> &MarkState<C>;

    /// Get mutable reference to the mark's state
    fn state_mut(&mut self) -> &mut MarkState<C>;

    /// Get the data context for this mark (for accessing encodings and data)
    fn data_context(&self) -> &DataContext;

    /// Get the mark type name (e.g., "rect", "line", "symbol")
    fn mark_type(&self) -> &str;

    /// Declare channels this mark supports
    fn supported_channels(&self) -> Vec<ChannelDescriptor>;

    /// Build scene marks from processed data
    ///
    /// # Arguments
    /// * `data` - RecordBatch with array data (multiple rows), or None if all channels are scalar
    /// * `scalars` - RecordBatch with scalar data (single row) for channels that don't vary per mark
    /// * `context` - RenderContext containing theme, dimensions, and other rendering state
    /// * `coord` - Coordinate system for position transformations
    ///
    /// # Returns
    /// A vector of scene marks ready for rendering
    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &C,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Whether this mark type supports the order encoding channel
    fn supports_order(&self) -> bool {
        false
    }

    /// Returns the default value for a channel if not explicitly mapped
    /// First checks theme defaults, then falls back to mark-specific defaults
    fn default_channel_value(&self, channel: &str, context: &RenderContext) -> Option<ScalarValue> {
        // Check theme defaults first
        if let Some(default) = context.theme.mark_defaults.get(self.mark_type(), channel) {
            return Some(default.clone());
        }
        self.mark_specific_default(channel)
    }

    /// Mark-specific default values (to be overridden by marks)
    fn mark_specific_default(&self, _channel: &str) -> Option<ScalarValue> {
        None
    }

    /// Returns expressions for computing the radius/padding needed for this mark
    /// along the specified dimension.
    ///
    /// The `resolve_channel` function returns an expression for any channel,
    /// including defaults if the channel is not explicitly mapped.
    ///
    /// # Example
    /// For a symbol mark, this might return an expression like:
    /// `sqrt(size) * 0.5 + stroke_width / 2`
    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    /// Get the name of the channel used for sorting this mark's data
    ///
    /// # Returns
    /// * `Some(channel_name)` - The name of the channel used for sorting
    /// * `None` - If the mark doesn't support sorting or uses default order
    ///
    /// # Default Implementation
    /// Returns `Some("order")` if `supports_order()` returns true, otherwise `None`
    fn sorting_channel(&self) -> Option<&str> {
        // Default: use "order" channel if mark supports ordering
        if self.supports_order() {
            Some("order")
        } else {
            None
        }
    }

    /// Get the preferred legend renderer for a channel
    ///
    /// # Arguments
    /// * `channel` - The channel name (e.g., "fill", "size")
    /// * `scale` - The configured scale for this channel
    ///
    /// # Returns
    /// * `Some(renderer)` - The preferred legend renderer for this channel
    /// * `None` - If this mark doesn't want a legend for the channel
    ///
    /// # Note
    /// Returning `None` means the channel does not get a legend. This is called
    /// for individual channels before considering merged legends.
    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<Arc<dyn LegendRenderer>> {
        None
    }

    /// Get the preferred legend renderer for merged channels
    ///
    /// Called when multiple channels from the same mark have matching MergeKeys,
    /// allowing them to be combined into a single legend (e.g., size and color
    /// both mapped to the same data field).
    ///
    /// # Arguments
    /// * `channels` - Array of channels that could be merged
    /// * `scales` - Map of channel names to their configured scales
    ///
    /// # Returns
    /// * `Some(renderer)` - A renderer capable of handling all the merged channels
    /// * `None` - If these channels cannot be merged with a single renderer
    ///
    /// # Default Implementation
    /// Uses the renderer from the first channel if it supports merging all channels
    fn preferred_merged_legend_renderer(
        &self,
        channels: &[crate::legend::LegendChannel],
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Option<Arc<dyn LegendRenderer>> {
        // Default implementation: try to find a renderer that supports all channels
        // Marks can override this for custom behavior

        // Get the renderer from the first channel
        let first_channel = &channels[0];
        let scale = scales.get(&first_channel.name)?;
        let renderer = self.preferred_legend_renderer(&first_channel.channel_type, scale)?;

        // Check if it supports merging all the channels
        if renderer.supports_merge(channels) {
            Some(renderer)
        } else {
            None // Can't merge these channels
        }
    }

    /// Get the preferred scale type for a channel based on data type
    /// Returns None to use system defaults
    fn preferred_scale_type(
        &self,
        _channel: &str,
        data_type: &DataType,
    ) -> Option<Arc<dyn ScaleImpl>> {
        default_scale_for_data_type(data_type)
    }

    /// Get default scale options for a channel and scale type
    ///
    /// These are mark-specific preferences that override system defaults.
    ///
    /// # Arguments
    /// * `channel` - The channel name
    /// * `scale_impl` - The scale implementation
    /// * `_data_type` - The data type of the channel
    ///
    /// # Returns
    /// A map of option names to their values
    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> HashMap<String, Expr> {
        use datafusion::logical_expr::lit;
        let mut options = HashMap::new();

        // Default: Color scales with continuous numeric output should use nice for better legend labels
        if matches!(channel, "fill" | "stroke" | "color") && util::is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }

        options
    }

    /// Get default range for a channel after domain has been determined
    ///
    /// This method is called after the scale's domain has been inferred and normalized.
    /// Marks can use this to provide channel-specific default ranges.
    ///
    /// # Arguments
    /// * `channel` - The channel name
    /// * `scale_impl` - The scale implementation being used
    /// * `domain` - The resolved domain type (Discrete with count or Interval)
    /// * `data_type` - The data type of the channel
    /// * `theme` - The current theme for accessing default values
    ///
    /// # Returns
    /// * `Some(range)` - A specific range to use for this channel
    /// * `None` - Use system defaults
    fn default_channel_range(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &crate::scales::ResolvedDomain,
        _data_type: &DataType,
        _theme: &crate::theme::Theme,
    ) -> Option<ScaleRange> {
        None
    }
}
