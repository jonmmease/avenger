pub mod compiled_data_context;
pub mod data_context;
pub mod facet_strategy;
pub mod line;
#[macro_use]
pub mod macros;
pub mod rect;
pub mod state;
pub mod subplot;
pub mod symbol;
pub mod util;

use std::{any::Any, collections::HashMap, sync::Arc};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::mark::SceneMark;

use datafusion::{
    arrow::{datatypes::DataType, record_batch::RecordBatch},
    logical_expr::{Expr, lit},
    scalar::ScalarValue,
};
use datafusion_common::ScalarValue as DatafusionScalarValue;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{
    coords::{CoordinateSystem, CoordinateSystemTransform},
    error::AvengerChartError,
    legend::LegendRenderer,
    render::RenderContext,
    scales::{ScaleRange, ScaleSpec},
    serialization::SerializableExpr,
    theme::Theme,
};

pub use crate::channel::{ChannelDefault, ChannelDescriptor, ChannelValue, ConditionalValue};
pub use compiled_data_context::CompiledDataContext;
pub use data_context::DataContext;
pub use facet_strategy::FacetStrategy;
pub use state::{CompiledMarkState, MarkState};
pub use subplot::{CompiledCartesianSubplot, CompiledConcatSubplot, Subplot, SubplotDataSource};
pub use util::default_scale_for_data_type;

/// Expression for computing radius/padding requirements for marks
///
/// Used to determine how much space a mark needs beyond its base position,
/// accounting for visual properties like size, stroke width, etc.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RadiusExpression {
    /// Same radius in all directions (e.g., circular symbols)
    Symmetric(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
    /// Different radius for negative and positive directions (e.g., bars extending from baseline)
    Asymmetric {
        /// Radius in the negative direction
        #[serde_as(as = "FromInto<SerializableExpr>")]
        lower: LogicalExprNode,
        /// Radius in the positive direction
        #[serde_as(as = "FromInto<SerializableExpr>")]
        upper: LogicalExprNode,
    },
}

/// Core trait for all mark types (uncompiled)
#[async_trait::async_trait]
pub trait Mark<C: CoordinateSystem>: Send + Sync + 'static {
    /// Get the mark's state (uncompiled version with DataContext)
    fn state(&self) -> &MarkState;

    /// Get mutable reference to the mark's state
    fn state_mut(&mut self) -> &mut MarkState;

    /// Get the data context for this mark (for accessing encodings and data during construction)
    fn data_context(&self) -> &DataContext;

    /// Build a CompiledMark from this Mark with the provided compiled state
    /// This enables type-erased rendering without the coordinate system generic
    /// The compiled_state contains the transformed DataFrame (e.g., after aggregation)
    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;

    /// Convenience method to compile a mark without transforming its DataFrame
    /// This is useful for tests and simple cases where no aggregation is needed
    async fn compile_untransformed(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let mark_state = self.state();
        let df = mark_state.data.dataframe().cloned();
        let compiled_state = CompiledMarkState::from_mark_state(mark_state, df);
        self.compile(compiled_state, ctx).await
    }
}

#[typetag::serde(tag = "type")]
#[async_trait::async_trait]
pub trait CompiledMark: Any + Send + Sync {
    /// Get the mark's state (compiled version with CompiledDataContext)
    fn state(&self) -> &CompiledMarkState;

    /// Get mutable reference to the mark's state
    fn state_mut(&mut self) -> &mut CompiledMarkState;

    /// Get the data context for this mark (for accessing encodings and serialized data)
    fn data_context(&self) -> &CompiledDataContext;

    /// Get the mark type name (e.g., "rect", "line", "symbol")
    fn mark_type(&self) -> &str;

    /// Downcast support (override where needed)
    fn as_any(&self) -> &dyn Any {
        panic!("as_any not implemented for this mark type")
    }

    /// Declare channels this mark supports
    fn supported_channels(&self) -> Vec<ChannelDescriptor>;

    /// Measure pass: cache intermediate data for the render pass
    ///
    /// This method is called during the measurement phase to cache intermediate
    /// computation results in a MarkMeasurement for the render pass.
    ///
    /// The plot area dimensions are provided via `context.plot_width` and `context.plot_height`.
    ///
    /// # Arguments
    /// Render the mark from prepared data batches
    ///
    /// # Arguments
    /// * `data` - RecordBatch with array data (multiple rows), or None if all channels are scalar
    /// * `scalars` - RecordBatch with scalar data (single row) for channels that don't vary per mark
    /// * `context` - RenderContext containing theme, dimensions, and other rendering state
    /// * `coord` - Coordinate system for position transformations
    ///
    /// # Returns
    /// A vector of scene marks ready for rendering
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    /// Whether this mark type supports the order encoding channel
    fn supports_order(&self) -> bool {
        false
    }

    /// Whether this mark needs the full DataFrame as RecordBatch
    ///
    /// Most marks only need columns for their specific channels (default: false).
    /// Container marks like facets need all columns to pass to nested marks (return: true).
    ///
    /// When true, the rendering system will convert the entire DataFrame to
    /// RecordBatch instead of selecting only the mark's channel columns.
    fn wants_full_data_batch(&self) -> bool {
        false // Default: only select needed channels
    }

    /// Returns the default value for a channel if not explicitly mapped
    /// First checks theme defaults, then falls back to mark-specific defaults
    fn default_channel_value(
        &self,
        channel: &str,
        context: &RenderContext<'_>,
    ) -> Option<ScalarValue> {
        // Check theme defaults first
        let mark_type = self.mark_type();

        if let Some(default) = context
            .theme()
            .mark_default(mark_type, channel, context.params())
        {
            return Some(default);
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
    ) -> Option<Box<dyn ScaleSpec>> {
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
    /// * `params` - Runtime parameters for theme resolution
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
        _theme: &Theme,
        _params: &IndexMap<String, DatafusionScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }
}
