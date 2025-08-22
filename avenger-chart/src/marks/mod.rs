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

pub use channel::{ChannelExpr, ChannelValue};
pub use channel_descriptor::{ChannelDefault, ChannelDescriptor, ChannelType};
pub use data_context::DataContext;
pub use data_source::{DataSource, FacetStrategy};
pub use state::MarkState;

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::Expr;
use datafusion::scalar::ScalarValue;

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
}
