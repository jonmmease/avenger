use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use datafusion::{common::ScalarValue, dataframe::DataFrame};
use tracing::{debug, trace};

use crate::{
    error::AvengerChartError,
    facet::{
        evaluated_facet_tree::EvaluatedFacetTree,
        marks::facet::{FacetMarkRef, facet_mark_ref},
        path_math,
    },
    marks::CompiledMark,
    plot::compiled::{CompiledPlot, scales::build_scale_builder_from_marks},
    render::EvaluationContext,
    scales::ScaleBuilder,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FacetScaleNodeKey {
    subplot_ptr: usize,
    canonical_parent_path: Vec<ScalarValue>,
}

impl FacetScaleNodeKey {
    pub(crate) fn new(subplot: &Arc<CompiledPlot>, parent_path: &[ScalarValue]) -> Self {
        Self {
            subplot_ptr: Arc::as_ptr(subplot) as usize,
            canonical_parent_path: canonicalize_path(parent_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FacetScaleSubtreeKey {
    marks_scope_ptr: usize,
    canonical_parent_path: Vec<ScalarValue>,
}

impl FacetScaleSubtreeKey {
    pub(crate) fn new(
        compiled_marks: &[Arc<dyn CompiledMark>],
        parent_path: &[ScalarValue],
    ) -> Self {
        Self {
            marks_scope_ptr: compiled_marks.as_ptr() as usize,
            canonical_parent_path: canonicalize_path(parent_path),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FacetScaleNodeArtifacts {
    pub(crate) nested_col_sharing: Option<u8>,
    pub(crate) nested_depth: u8,
    pub(crate) shared_scale_builder: ScaleBuilder,
    pub(crate) ancestor_scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder>,
    pub(crate) per_cell_scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder>,
}

#[derive(Default)]
struct FacetScalePrecomputeState {
    precomputed_subtrees: HashSet<FacetScaleSubtreeKey>,
    node_artifacts: HashMap<FacetScaleNodeKey, Arc<FacetScaleNodeArtifacts>>,
}

#[derive(Default)]
pub(crate) struct FacetScalePrecomputeStore {
    state: Mutex<FacetScalePrecomputeState>,
}

impl FacetScalePrecomputeStore {
    pub(crate) fn is_subtree_precomputed(&self, key: &FacetScaleSubtreeKey) -> bool {
        self.state
            .lock()
            .expect("FacetScalePrecomputeStore lock poisoned")
            .precomputed_subtrees
            .contains(key)
    }

    pub(crate) fn mark_subtree_precomputed(&self, key: FacetScaleSubtreeKey) {
        self.state
            .lock()
            .expect("FacetScalePrecomputeStore lock poisoned")
            .precomputed_subtrees
            .insert(key);
    }

    pub(crate) fn get_node_artifacts(
        &self,
        key: &FacetScaleNodeKey,
    ) -> Option<Arc<FacetScaleNodeArtifacts>> {
        self.state
            .lock()
            .expect("FacetScalePrecomputeStore lock poisoned")
            .node_artifacts
            .get(key)
            .cloned()
    }

    pub(crate) fn insert_node_artifacts(
        &self,
        key: FacetScaleNodeKey,
        artifacts: Arc<FacetScaleNodeArtifacts>,
    ) {
        self.state
            .lock()
            .expect("FacetScalePrecomputeStore lock poisoned")
            .node_artifacts
            .insert(key, artifacts);
    }
}

pub(crate) fn canonicalize_scalar(value: &ScalarValue) -> ScalarValue {
    match value {
        ScalarValue::Utf8(Some(v))
        | ScalarValue::LargeUtf8(Some(v))
        | ScalarValue::Utf8View(Some(v)) => ScalarValue::Utf8(Some(v.clone())),
        ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Utf8View(None) => {
            ScalarValue::Utf8(None)
        }
        _ => value.clone(),
    }
}

pub(crate) fn canonicalize_path(path: &[ScalarValue]) -> Vec<ScalarValue> {
    path.iter().map(canonicalize_scalar).collect()
}

fn resolve_current_facet_node<'a>(
    compiled_marks: &'a [Arc<dyn CompiledMark>],
) -> Option<(&'a Arc<CompiledPlot>, u8)> {
    for mark in compiled_marks {
        if let Some(facet_mark) = facet_mark_ref(mark.as_ref()) {
            match facet_mark {
                FacetMarkRef::Row(facet_row) => {
                    return Some((
                        facet_row.compiled_subplot(),
                        facet_row
                            .facet_scale_sharing()
                            .map(|sharing| sharing.to_level())
                            .unwrap_or(0),
                    ));
                }
                FacetMarkRef::Col(facet_col) => {
                    return Some((
                        facet_col.compiled_subplot(),
                        facet_col
                            .facet_scale_sharing()
                            .map(|sharing| sharing.to_level())
                            .unwrap_or(0),
                    ));
                }
            }
        }
    }

    None
}

fn nested_facet_sharing_level(compiled_subplot: &Arc<CompiledPlot>) -> Option<u8> {
    compiled_subplot.marks.iter().find_map(|mark| {
        facet_mark_ref(mark.as_ref()).and_then(|facet_mark| match facet_mark {
            FacetMarkRef::Row(facet_row) => facet_row.facet_scale_sharing().map(|s| s.to_level()),
            FacetMarkRef::Col(facet_col) => facet_col.facet_scale_sharing().map(|s| s.to_level()),
        })
    })
}

fn enumerate_cell_values_for_node(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    current_sharing_level: u8,
) -> Vec<ScalarValue> {
    let current_node = if facet_path.is_empty() {
        facet_tree.root()
    } else {
        facet_tree.node_at_path(facet_path)
    };

    let fallback = || {
        current_node
            .map(|node| node.values().cloned().collect())
            .unwrap_or_default()
    };

    facet_tree
        .enumerate_values_for_facet(facet_path, current_sharing_level)
        .unwrap_or_else(fallback)
}

async fn build_ancestor_group_scale_builders(
    cell_values: &[ScalarValue],
    sharing_level: u8,
    parent_path: &[ScalarValue],
    facet_tree: &EvaluatedFacetTree,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    eval_ctx: &EvaluationContext,
) -> Result<HashMap<Vec<ScalarValue>, ScaleBuilder>, AvengerChartError> {
    let mut cache = HashMap::new();
    let mut groups: HashMap<Vec<ScalarValue>, Vec<ScalarValue>> = HashMap::new();

    for value in cell_values {
        let mut full_path = parent_path.to_vec();
        full_path.push(value.clone());
        let ancestor_key = path_math::nested_measurement_ancestor_key(
            &full_path,
            sharing_level,
            full_path.len() as u8 + 1,
        );
        groups.entry(ancestor_key).or_default().push(value.clone());
    }

    for (ancestor_key, _group_values) in groups {
        let group_predicate = facet_tree.path_predicate(&ancestor_key);
        let filtered_df = if let Some(pred) = group_predicate {
            data_df.clone().filter(pred).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to filter data for ancestor key {:?}: {}",
                    ancestor_key, e
                ))
            })?
        } else {
            data_df.clone()
        };

        let scale_builder = build_scale_builder_from_marks(
            &compiled_subplot.marks,
            &compiled_subplot.scale_specs,
            &compiled_subplot.coord_transform,
            &compiled_subplot.data,
            Some(filtered_df),
            &eval_ctx.session_context,
            &eval_ctx.params,
            compiled_subplot.get_theme().as_ref(),
        )
        .await?;

        cache.insert(canonicalize_path(&ancestor_key), scale_builder);
    }

    Ok(cache)
}

async fn build_per_cell_scale_builders(
    cell_values: &[ScalarValue],
    parent_path: &[ScalarValue],
    facet_tree: &EvaluatedFacetTree,
    data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    eval_ctx: &EvaluationContext,
) -> Result<HashMap<Vec<ScalarValue>, ScaleBuilder>, AvengerChartError> {
    let mut cache = HashMap::new();

    for value in cell_values {
        let mut full_path = parent_path.to_vec();
        full_path.push(value.clone());
        if !facet_tree.cell_exists(&full_path) {
            continue;
        }

        let data_override = if let Some(predicate) = facet_tree.cell_predicate(&full_path, 0) {
            data_df.clone().filter(predicate).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to filter data for facet cell {:?}: {}",
                    full_path, e
                ))
            })?
        } else {
            data_df.clone()
        };

        let scale_builder = build_scale_builder_from_marks(
            &compiled_subplot.marks,
            &compiled_subplot.scale_specs,
            &compiled_subplot.coord_transform,
            &compiled_subplot.data,
            Some(data_override),
            &eval_ctx.session_context,
            &eval_ctx.params,
            compiled_subplot.get_theme().as_ref(),
        )
        .await?;

        cache.insert(canonicalize_path(&full_path), scale_builder);
    }

    Ok(cache)
}

pub(crate) async fn build_node_artifacts(
    cell_values: &[ScalarValue],
    facet_path: &[ScalarValue],
    inherited_data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<FacetScaleNodeArtifacts, AvengerChartError> {
    let shared_scale_builder = build_scale_builder_from_marks(
        &compiled_subplot.marks,
        &compiled_subplot.scale_specs,
        &compiled_subplot.coord_transform,
        &compiled_subplot.data,
        Some(inherited_data_df.clone()),
        &eval_ctx.session_context,
        &eval_ctx.params,
        compiled_subplot.get_theme().as_ref(),
    )
    .await?;

    let nested_depth = (facet_path.len() + 2) as u8;
    let nested_col_sharing = nested_facet_sharing_level(compiled_subplot);

    let ancestor_scale_builder_cache = if let Some(sharing_level) = nested_col_sharing {
        if sharing_level > 0 && sharing_level < nested_depth {
            build_ancestor_group_scale_builders(
                cell_values,
                sharing_level,
                facet_path,
                facet_tree,
                inherited_data_df,
                compiled_subplot,
                eval_ctx,
            )
            .await?
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    let per_cell_scale_builder_cache = if matches!(nested_col_sharing, Some(0)) {
        build_per_cell_scale_builders(
            cell_values,
            facet_path,
            facet_tree,
            inherited_data_df,
            compiled_subplot,
            eval_ctx,
        )
        .await?
    } else {
        HashMap::new()
    };

    Ok(FacetScaleNodeArtifacts {
        nested_col_sharing,
        nested_depth,
        shared_scale_builder,
        ancestor_scale_builder_cache,
        per_cell_scale_builder_cache,
    })
}

async fn ensure_subtree_precomputed_internal(
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
    inherited_data_df: &DataFrame,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let Some((compiled_subplot, current_sharing_level)) =
        resolve_current_facet_node(compiled_marks)
    else {
        return Ok(());
    };

    let store = eval_ctx.facet_scale_precompute_store();
    let node_key = FacetScaleNodeKey::new(compiled_subplot, facet_path);

    let cell_values = enumerate_cell_values_for_node(facet_tree, facet_path, current_sharing_level);
    if store.get_node_artifacts(&node_key).is_none() {
        let artifacts = build_node_artifacts(
            &cell_values,
            facet_path,
            inherited_data_df,
            compiled_subplot,
            facet_tree,
            eval_ctx,
        )
        .await?;
        store.insert_node_artifacts(node_key.clone(), Arc::new(artifacts));
        trace!(facet_path = ?facet_path, "facet scale precompute built node artifacts");
    }

    for value in &cell_values {
        let mut full_path = facet_path.to_vec();
        full_path.push(value.clone());
        if !facet_tree.cell_exists(&full_path) {
            continue;
        }

        let child_data_df = if let Some(predicate) = facet_tree.cell_predicate(&full_path, 0) {
            inherited_data_df.clone().filter(predicate).map_err(|e| {
                AvengerChartError::InternalError(format!(
                    "Failed to filter data for precompute cell {:?}: {}",
                    full_path, e
                ))
            })?
        } else {
            inherited_data_df.clone()
        };

        Box::pin(ensure_subtree_precomputed_internal(
            &compiled_subplot.marks,
            &full_path,
            &child_data_df,
            facet_tree,
            eval_ctx,
        ))
        .await?;
    }

    Ok(())
}

pub(crate) async fn ensure_subtree_precomputed(
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
    inherited_data_df: &DataFrame,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let subtree_key = FacetScaleSubtreeKey::new(compiled_marks, facet_path);
    let store = eval_ctx.facet_scale_precompute_store();
    if store.is_subtree_precomputed(&subtree_key) {
        return Ok(());
    }

    ensure_subtree_precomputed_internal(
        compiled_marks,
        facet_path,
        inherited_data_df,
        facet_tree,
        eval_ctx,
    )
    .await?;
    store.mark_subtree_precomputed(subtree_key);
    debug!(
        facet_path = ?facet_path,
        "facet scale precompute completed for subtree"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s_utf8(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn s_utf8_view(value: &str) -> ScalarValue {
        ScalarValue::Utf8View(Some(value.to_string()))
    }

    fn s_large_utf8(value: &str) -> ScalarValue {
        ScalarValue::LargeUtf8(Some(value.to_string()))
    }

    #[test]
    fn canonicalize_path_normalizes_utf8_variants() {
        let input = vec![s_utf8_view("a"), s_large_utf8("b"), s_utf8("c")];
        let canonical = canonicalize_path(&input);
        assert_eq!(canonical, vec![s_utf8("a"), s_utf8("b"), s_utf8("c")]);
    }

    #[test]
    fn store_uses_canonicalized_paths_for_lookup() {
        let store = FacetScalePrecomputeStore::default();
        let key = FacetScaleNodeKey {
            subplot_ptr: 42,
            canonical_parent_path: canonicalize_path(&[s_utf8_view("x"), s_large_utf8("y")]),
        };
        let artifacts = Arc::new(FacetScaleNodeArtifacts::default());
        store.insert_node_artifacts(key, artifacts.clone());

        let lookup = FacetScaleNodeKey {
            subplot_ptr: 42,
            canonical_parent_path: canonicalize_path(&[s_utf8("x"), s_utf8("y")]),
        };
        let found = store.get_node_artifacts(&lookup);
        assert!(found.is_some());
        assert!(Arc::ptr_eq(&found.unwrap(), &artifacts));
    }

    #[test]
    fn subtree_key_dedupes_by_scope_and_path() {
        let store = FacetScalePrecomputeStore::default();
        let key = FacetScaleSubtreeKey {
            marks_scope_ptr: 77,
            canonical_parent_path: canonicalize_path(&[s_utf8_view("r")]),
        };
        assert!(!store.is_subtree_precomputed(&key));
        store.mark_subtree_precomputed(key.clone());
        assert!(store.is_subtree_precomputed(&key));
    }
}
