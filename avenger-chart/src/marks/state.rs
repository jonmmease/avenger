//! State for marks

use crate::coords::CoordinateSystem;
use crate::marks::{DataContext, FacetStrategy};
use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::axis::Axis;
use crate::guide::CoordinateGuideBuilder;

/// State shared by all mark types
#[derive(Clone, Serialize, Deserialize)]
pub struct MarkState {
    #[serde(skip)]
    pub data: DataContext,

    // Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}
