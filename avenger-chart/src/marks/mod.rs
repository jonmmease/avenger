pub mod channel;
pub mod line;
pub mod rect;
pub mod symbol;
pub mod util;
#[macro_use]
pub mod macros;
#[macro_use]
pub mod channel_macros;

pub use channel::{ChannelExpr, ChannelValue};

use crate::adjust::Adjust;
use crate::coords::CoordinateSystem;
use crate::derive::Derive;
use crate::error::AvengerChartError;
use crate::transforms::DataContext;
use avenger_common::types::SymbolShape;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::scalar::ScalarValue;
use std::collections::HashMap;

/// Types of channels that marks can support
#[derive(Debug, Clone)]
pub enum ChannelType {
    Position, // x, y, x2, y2
    Color,    // fill, stroke
    Size,     // stroke_width, size
    Text,     // text labels
    Numeric,  // opacity, angle
    Enum {
        // discrete choices with allowed values
        values: &'static [&'static str],
    },
}

/// Default value for a channel
#[derive(Debug, Clone)]
pub enum ChannelDefault {
    Scalar(ScalarValue),
    // Could extend with other types later
}

/// Descriptor for a channel that a mark supports
#[derive(Debug, Clone)]
pub struct ChannelDescriptor {
    pub name: &'static str,
    pub required: bool,
    pub channel_type: ChannelType,
    pub default_value: Option<ChannelDefault>,
    pub allow_column_ref: bool, // Can this channel vary per mark instance?
}

pub trait Mark<C: CoordinateSystem>: Send + Sync + 'static {
    fn into_config(self) -> MarkConfig<C>;

    /// Declare channels this mark supports
    fn supported_channels(&self) -> Vec<ChannelDescriptor>;

    /// Build scene marks from processed data
    fn render_from_data(
        &self,
        batch: Option<&RecordBatch>, // Per-row data (None if all scalar)
        scalars: &HashMap<String, ScalarValue>, // Scalar encodings
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

/// Data source strategy for marks in faceted plots
#[derive(Debug, Clone)]
pub enum DataSource {
    /// Inherit data from plot level (default for new marks)
    Inherited,
    /// Use explicit mark-level data
    Explicit,
}

/// Strategy for handling mark data in faceted plots
#[derive(Debug, Clone)]
pub enum FacetStrategy {
    /// Filter mark data by facet values (default)
    Filter,
    /// Show mark data in all facets (for reference marks)
    Broadcast,
    /// Skip this mark if facet variable not present in data
    Skip,
}

pub struct MarkConfig<C: CoordinateSystem> {
    pub mark_type: String,
    pub data: DataContext,

    // NEW: Data inheritance control
    pub data_source: DataSource,

    // NEW: Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
    pub shapes: Option<Vec<SymbolShape>>,
    pub adjustments: Vec<Box<dyn Adjust>>,
    pub derived_marks: Vec<Box<dyn Derive<C>>>,
}
