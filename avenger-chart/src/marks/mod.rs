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

/// Padding requirements computed by a mark
#[derive(Debug, Default, Clone)]
pub struct MarkPadding {
    /// Padding needed on lower x-axis bound in pixels
    pub x_lower: Option<f64>,
    /// Padding needed on upper x-axis bound in pixels
    pub x_upper: Option<f64>,
    /// Padding needed on lower y-axis bound in pixels
    pub y_lower: Option<f64>,
    /// Padding needed on upper y-axis bound in pixels
    pub y_upper: Option<f64>,
}

impl MarkPadding {
    /// Create padding with symmetric values
    pub fn symmetric(x: f64, y: f64) -> Self {
        Self {
            x_lower: Some(x),
            x_upper: Some(x),
            y_lower: Some(y),
            y_upper: Some(y),
        }
    }
    
    /// Create padding with no padding
    pub fn none() -> Self {
        Self::default()
    }
}

/// Current clip bounds of the plot area
#[derive(Debug, Clone)]
pub struct ClipBounds {
    /// Minimum x value of the clip area
    pub x_min: f64,
    /// Maximum x value of the clip area
    pub x_max: f64,
    /// Minimum y value of the clip area
    pub y_min: f64,
    /// Maximum y value of the clip area
    pub y_max: f64,
}

/// Types of channels that marks can support
#[derive(Debug, Clone)]
pub enum ChannelType {
    Position, // x, y, x2, y2
    Color,    // fill, stroke
    Size,     // stroke_width, size
    Text,     // text labels
    Numeric,  // opacity, angle
    Boolean,  // defined, visible
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

    /// Declare which channels contribute to padding calculation (including positional)
    fn padding_channels(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Compute padding requirements based on channel values
    /// data: RecordBatch containing the padding-relevant channels (including positional)
    /// scalars: Scalar values for padding-relevant channels
    /// clip_bounds: Current clip bounds of the plot area
    /// plot_area_width: Width of the plot area in pixels
    /// plot_area_height: Height of the plot area in pixels
    fn compute_padding(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _clip_bounds: &ClipBounds,
        _plot_area_width: f32,
        _plot_area_height: f32,
    ) -> Result<MarkPadding, AvengerChartError> {
        Ok(MarkPadding::default())
    }
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

/// Internal state shared by all mark types
pub(crate) struct MarkState<C: CoordinateSystem> {
    pub data: DataContext,

    // NEW: Data inheritance control
    pub data_source: DataSource,

    // NEW: Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
    #[allow(dead_code)] // Reserved for future use
    pub shapes: Option<Vec<SymbolShape>>,
    pub adjustments: Vec<Box<dyn Adjust>>,
    pub derived_marks: Vec<Box<dyn Derive<C>>>,
}
