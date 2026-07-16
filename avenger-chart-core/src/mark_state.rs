//! State for marks

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use datafusion::dataframe::DataFrame;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{
    AvengerChartError, Axis, CompiledDataContext, DataContext, DefaultLogicalExprNodeExt,
    FacetDataScope, GeometrySpace, MarkId, RepeatContext, SerializableExpr,
    resolve_repeat_placeholders,
    view::{CompiledViewScope, ViewScopeState},
};

pub const DETAIL_ARRAY_COLUMN_PREFIX: &str = "__avenger_detail_";

pub fn detail_array_column_name(index: usize) -> String {
    format!("{DETAIL_ARRAY_COLUMN_PREFIX}{index}")
}

pub fn validate_structural_id(kind: &str, id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty()
        || id.contains('.')
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid {kind} id '{id}'; ids must be non-empty ASCII identifiers without periods"
        )));
    }
    Ok(())
}

pub fn validate_mark_target_path(kind: &str, path: &str) -> Result<(), AvengerChartError> {
    let mut segment_count = 0usize;
    for segment in path.split('.') {
        segment_count += 1;
        validate_structural_id(kind, segment).map_err(|_| {
            AvengerChartError::InvalidArgument(format!(
                "Invalid {kind} target '{path}'; targets must be non-empty dot-separated ASCII ids"
            ))
        })?;
    }
    if segment_count == 0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid {kind} target '{path}'; targets must be non-empty dot-separated ASCII ids"
        )));
    }
    Ok(())
}

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
    pub id: Option<String>,
    /// Additional public interaction/export aliases for this mark.
    pub public_aliases: Vec<String>,
    pub data: DataContext,
    pub view: Option<ViewScopeState>,
    pub data_mode: MarkDataMode,

    // Faceting behavior for this mark
    pub facet_data_scope: FacetDataScope,

    pub exclude_from_scale_domains: bool,
    pub visible: Option<LogicalExprNode>,
    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
    pub geometry_space: Option<GeometrySpace>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

/// Component/part provenance retained independently of public target aliases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledComponentProvenance {
    pub component_kind: String,
    pub component_id: Option<String>,
    pub part_alias: String,
}

/// Canonical identity and source metadata for one compiled primitive mark.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledMarkIdentity {
    pub runtime_id: MarkId,
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub public_aliases: Vec<String>,
    /// Private authoring group ordinals. These are diagnostic structure, not a
    /// public lookup path and not part of runtime identity.
    #[serde(default)]
    pub private_ancestry: Vec<usize>,
    #[serde(default)]
    pub component: Option<CompiledComponentProvenance>,
}

/// State shared by all mark types (compiled version)
/// Used after compilation - stores serialized LogicalPlanNodes
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledMarkState {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub identity: CompiledMarkIdentity,
    pub data: CompiledDataContext,
    #[serde(default)]
    pub view: Option<CompiledViewScope>,
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
    #[serde(default)]
    pub geometry_space: Option<GeometrySpace>,

    // Store axis configurations from channels
    pub axis_configs: HashMap<String, Arc<dyn Axis>>,
}

impl CompiledMarkState {
    /// Convert MarkState to CompiledMarkState with an optional serialized DataFrame.
    pub fn from_mark_state(state: &MarkState, transformed_df: Option<DataFrame>) -> Self {
        let data = if let Some(store_data) = state.data.store_data_ref() {
            CompiledDataContext::new_store_data_with_pattern_channels(
                store_data.clone(),
                state.data.transforms().to_vec(),
                state.data.channels().clone(),
                state.data.pattern_channels().clone(),
            )
        } else {
            CompiledDataContext::new_with_pattern_channels(
                transformed_df,
                state.data.transforms().to_vec(),
                state.data.channels().clone(),
                state.data.pattern_channels().clone(),
            )
        };
        Self {
            id: state.id.clone(),
            identity: CompiledMarkIdentity {
                source_name: state.id.clone(),
                public_aliases: state.public_aliases.clone(),
                ..Default::default()
            },
            data,
            view: state
                .view
                .as_ref()
                .map(CompiledViewScope::from_view_scope_state),
            data_mode: state.data_mode,
            mark_index: 0,
            facet_data_scope: state.facet_data_scope,
            exclude_from_scale_domains: state.exclude_from_scale_domains,
            visible: state.visible.clone(),
            details: state.details.clone(),
            zindex: state.zindex,
            geometry_space: state.geometry_space,
            axis_configs: state.axis_configs.clone(),
        }
    }

    pub fn mark_index(&self) -> usize {
        self.mark_index
    }

    pub fn geometry_space_or(&self, default: GeometrySpace) -> GeometrySpace {
        self.geometry_space.unwrap_or(default)
    }

    #[doc(hidden)]
    pub fn with_mark_index(mut self, mark_index: usize) -> Self {
        self.mark_index = mark_index;
        self
    }

    #[doc(hidden)]
    pub fn with_identity(mut self, identity: CompiledMarkIdentity) -> Self {
        debug_assert!(!identity.runtime_id.is_unresolved());
        self.id = identity.source_name.clone();
        self.identity = identity;
        self
    }

    pub fn with_component_provenance(mut self, component: CompiledComponentProvenance) -> Self {
        self.identity.component = Some(component);
        self
    }
}

impl MarkState {
    pub fn has_explicit_data_source(&self) -> bool {
        self.data.has_explicit_data_source()
    }

    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        let mut resolved = self.clone();
        resolved.data = self.data.resolve_repeat(ctx)?;
        resolved.view = self
            .view
            .as_ref()
            .map(|view| view.resolve_repeat(ctx))
            .transpose()?;
        resolved.visible = self
            .visible
            .clone()
            .map(|node| {
                LogicalExprNode::from_default_expr(resolve_repeat_placeholders(
                    node.to_default_expr(&datafusion::prelude::SessionContext::new())?,
                    ctx,
                )?)
            })
            .transpose()?;
        resolved.axis_configs = self
            .axis_configs
            .iter()
            .map(|(channel, axis)| {
                let mapped = axis
                    .as_ref()
                    .map_exprs(&mut |expr| resolve_repeat_placeholders(expr, ctx))?;
                Ok((channel.clone(), Arc::from(mapped)))
            })
            .collect::<Result<_, AvengerChartError>>()?;
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{CompiledDataContext, FacetDataScope};

    use super::*;

    #[test]
    fn compiled_mark_state_deserializes_without_geometry_space_field() {
        let state = CompiledMarkState {
            id: Some("line".to_string()),
            identity: CompiledMarkIdentity::default(),
            data: CompiledDataContext::default(),
            view: None,
            data_mode: MarkDataMode::Inherit,
            mark_index: 0,
            facet_data_scope: FacetDataScope::default(),
            exclude_from_scale_domains: false,
            visible: None,
            details: None,
            zindex: None,
            geometry_space: Some(GeometrySpace::Display),
            axis_configs: HashMap::new(),
        };

        let mut json = serde_json::to_value(&state).expect("serialize compiled mark state");
        json.as_object_mut()
            .expect("compiled mark state json object")
            .remove("geometry_space");
        json.as_object_mut()
            .expect("compiled mark state json object")
            .remove("view");

        let decoded: CompiledMarkState =
            serde_json::from_value(json).expect("deserialize without geometry_space");
        assert_eq!(decoded.geometry_space, None);
        assert!(decoded.view.is_none());
        assert_eq!(
            decoded.geometry_space_or(GeometrySpace::Coordinate),
            GeometrySpace::Coordinate
        );
    }
}
