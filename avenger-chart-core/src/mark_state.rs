//! State for marks

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use datafusion::dataframe::DataFrame;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{Axis, CompiledDataContext, DataContext, FacetDataScope, SerializableExpr};

/// How a mark obtains rows when it has no explicit mark-level data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkDataMode {
    /// Use explicit mark data when provided, otherwise inherit plot/facet data.
    #[default]
    Inherit,
    /// Render a single scalar item and never inherit plot/facet data.
    Unit,
}

/// State shared by all mark types (uncompiled version)
/// Used during mark construction - stores live DataFrames that can be transformed
#[derive(Clone)]
pub struct MarkState {
    pub data: DataContext,
    pub data_mode: MarkDataMode,

    // Faceting behavior for this mark
    pub facet_data_scope: FacetDataScope,

    pub exclude_from_scale_domains: bool,
    pub visible: Option<LogicalExprNode>,
    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

/// State shared by all mark types (compiled version)
/// Used after compilation - stores serialized LogicalPlanNodes
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledMarkState {
    pub data: CompiledDataContext,
    #[serde(default)]
    pub data_mode: MarkDataMode,

    /// Stable index of this mark in its compiled plot's mark list.
    pub mark_index: usize,

    // Faceting behavior for this mark
    pub facet_data_scope: FacetDataScope,

    #[serde(default)]
    pub exclude_from_scale_domains: bool,
    #[serde(default)]
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub visible: Option<LogicalExprNode>,
    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

impl CompiledMarkState {
    /// Convert MarkState to CompiledMarkState with an optional serialized DataFrame.
    pub fn from_mark_state(state: &MarkState, transformed_df: Option<DataFrame>) -> Self {
        let data = if let Some(selection_clauses) = state.data.selection_clause_data() {
            CompiledDataContext::new_selection_clauses(
                selection_clauses.clone(),
                state.data.channels().clone(),
            )
        } else {
            CompiledDataContext::new(transformed_df, state.data.channels().clone())
        };
        Self {
            data,
            data_mode: state.data_mode,
            mark_index: 0,
            facet_data_scope: state.facet_data_scope,
            exclude_from_scale_domains: state.exclude_from_scale_domains,
            visible: state.visible.clone(),
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
