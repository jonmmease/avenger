//! State for marks

use std::{collections::HashMap, sync::Arc};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use datafusion::dataframe::DataFrame;

use crate::{
    axis::Axis,
    marks::{ChannelValue, CompiledDataContext, DataContext, FacetStrategy},
};

/// State shared by all mark types (uncompiled version)
/// Used during mark construction - stores live DataFrames that can be transformed
#[derive(Clone)]
pub struct MarkState {
    pub data: DataContext,

    // Faceting behavior for this mark
    pub facet_strategy: FacetStrategy,

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
    pub facet_strategy: FacetStrategy,

    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

impl CompiledMarkState {
    /// Convert MarkState to CompiledMarkState with a transformed DataFrame
    ///
    /// This is used during plot compilation to apply transformations (like aggregation)
    /// to the mark's data before serialization.
    pub fn from_mark_state(state: &MarkState, transformed_df: Option<DataFrame>) -> Self {
        Self {
            data: CompiledDataContext::new(transformed_df, state.data.channels().clone()),
            mark_index: 0,
            facet_strategy: state.facet_strategy.clone(),
            details: state.details.clone(),
            zindex: state.zindex,
            axis_configs: state.axis_configs.clone(),
        }
    }

    /// Convert MarkState to CompiledMarkState with a transformed DataFrame and updated channels
    ///
    /// This is used when transformations (like aggregation) modify the channel expressions
    /// to reference the output columns of the transformation.
    pub fn from_mark_state_with_channels(
        state: &MarkState,
        transformed_df: DataFrame,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        Self {
            data: CompiledDataContext::new(Some(transformed_df), channels),
            mark_index: 0,
            facet_strategy: state.facet_strategy.clone(),
            details: state.details.clone(),
            zindex: state.zindex,
            axis_configs: state.axis_configs.clone(),
        }
    }

    pub fn mark_index(&self) -> usize {
        self.mark_index
    }

    pub(crate) fn with_mark_index(mut self, mark_index: usize) -> Self {
        self.mark_index = mark_index;
        self
    }
}
