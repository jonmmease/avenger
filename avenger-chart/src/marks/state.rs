//! State for marks

use crate::coords::CoordinateSystem;
use crate::marks::{DataContext, DataSource, FacetStrategy};
use std::collections::HashMap;
use std::sync::Arc;

/// State shared by all mark types
pub struct MarkState<C: CoordinateSystem> {
    pub _phantom: std::marker::PhantomData<C>,
    pub data: DataContext,

    // Data inheritance control
    pub data_source: DataSource,

    // Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    #[doc(hidden)]
    pub axis_configs: HashMap<String, Arc<dyn Fn(C::Axis) -> C::Axis + Send + Sync>>,
}
