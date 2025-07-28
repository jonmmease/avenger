pub mod channel;
pub mod line;
pub mod rect;
pub mod symbol;
#[macro_use]
pub mod macros;

pub use channel::{ChannelExpr, ChannelValue};

use crate::adjust::Adjust;
use crate::coords::CoordinateSystem;
use crate::derive::Derive;
use crate::transforms::DataContext;
use avenger_common::types::SymbolShape;

pub trait Mark<C: CoordinateSystem>: Send + Sync + 'static {
    fn into_config(self) -> MarkConfig<C>;
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
