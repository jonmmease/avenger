//! Internal state for marks

use crate::coords::CoordinateSystem;
use crate::marks::{DataContext, DataSource, FacetStrategy};
use avenger_common::types::SymbolShape;

/// Internal state shared by all mark types
pub(crate) struct MarkState<C: CoordinateSystem> {
    pub(crate) _phantom: std::marker::PhantomData<C>,
    pub data: DataContext,

    // Data inheritance control
    pub data_source: DataSource,

    // Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
}