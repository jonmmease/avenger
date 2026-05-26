//! State for marks

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};

use datafusion::dataframe::DataFrame;

use crate::{Axis, CompiledDataContext, DataContext, FacetDataScope};

/// State shared by all mark types (uncompiled version)
/// Used during mark construction - stores live DataFrames that can be transformed
#[derive(Clone)]
pub struct MarkState {
    pub data: DataContext,

    // Faceting behavior for this mark
    pub facet_data_scope: FacetDataScope,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

/// State shared by all mark types (compiled version)
/// Used after compilation - stores serialized LogicalPlanNodes
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledMarkState {
    pub data: CompiledDataContext,

    /// Stable index of this mark in its compiled plot's mark list.
    pub mark_index: usize,

    // Faceting behavior for this mark
    pub facet_data_scope: FacetDataScope,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

impl CompiledMarkState {
    /// Convert MarkState to CompiledMarkState with an optional serialized DataFrame.
    pub fn from_mark_state(state: &MarkState, transformed_df: Option<DataFrame>) -> Self {
        Self {
            data: CompiledDataContext::new(transformed_df, state.data.channels().clone()),
            mark_index: 0,
            facet_data_scope: state.facet_data_scope,
            details: state.details.clone(),
            zindex: state.zindex,
            axis_configs: state.axis_configs.clone(),
        }
    }

    pub fn mark_index(&self) -> usize {
        self.mark_index
    }

    #[doc(hidden)]
    pub fn with_mark_index(mut self, mark_index: usize) -> Self {
        self.mark_index = mark_index;
        self
    }
}
