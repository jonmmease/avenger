pub mod channel;
pub mod channel_descriptor;
pub mod data_context;
pub mod data_source;
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

pub use channel::{ChannelValue, ConditionalBuilder, ConditionalValue};
pub use channel_descriptor::{ChannelDefault, ChannelDescriptor, ChannelType};
pub use data_context::DataContext;
pub use data_source::{DataSource, FacetStrategy};
pub use state::MarkState;

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::legend_renderer::LegendRenderer;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

/// Expression for computing radius/padding requirements
#[derive(Debug, Clone)]
pub enum RadiusExpression {
    /// Same radius in all directions
    Symmetric(Expr),
    /// Different radius for negative and positive directions
    Asymmetric { lower: Expr, upper: Expr },
}

/// Core trait for all mark types
pub trait Mark<C: CoordinateSystem>: Send + Sync + 'static {
    /// Get the mark's state
    fn state(&self) -> &MarkState<C>;

    /// Get mutable reference to the mark's state
    fn state_mut(&mut self) -> &mut MarkState<C>;

    /// Get the data context for this mark (for accessing encodings and data)
    fn data_context(&self) -> &DataContext;

    /// Get the data source type
    fn data_source(&self) -> DataSource;

    /// Get the mark type name (e.g., "rect", "line", "symbol")
    fn mark_type(&self) -> &str;

    /// Declare channels this mark supports
    fn supported_channels(&self) -> Vec<ChannelDescriptor>;

    /// Build scene marks from processed data
    /// data: RecordBatch with array data (multiple rows), or None if all channels are scalar
    /// scalars: RecordBatch with scalar data (single row) for channels that don't vary per mark
    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Whether this mark type supports the order encoding channel
    fn supports_order(&self) -> bool {
        false // Default to false, marks opt-in
    }

    /// Returns the default value for a channel if not explicitly mapped
    fn default_channel_value(&self, _channel: &str) -> Option<ScalarValue> {
        None
    }

    /// Returns expressions for computing the radius/padding needed for this mark
    /// along the specified dimension.
    ///
    /// The `resolve_channel` function returns an expression for any channel,
    /// including defaults if the channel is not explicitly mapped.
    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    /// Get the preferred legend renderer for a channel
    /// Returns None if this mark doesn't want a legend for the channel
    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<Arc<dyn LegendRenderer>> {
        None // Default: no legend preference
    }

    /// Get the preferred legend renderer for merged channels
    /// Only called when channels have matching MergeKeys
    fn preferred_merged_legend_renderer(
        &self,
        channels: &[crate::legend_renderer::LegendChannel],
        scales: &std::collections::HashMap<String, ConfiguredScale>,
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

    /// Get a unique identifier for this mark instance
    fn mark_id(&self) -> String {
        // Default: use pointer address as unique ID
        format!("{:p}", self as *const _)
    }
}
