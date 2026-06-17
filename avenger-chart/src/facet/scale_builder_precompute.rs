use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use datafusion::logical_expr::{LogicalPlan, lit};
use datafusion::{common::ScalarValue, dataframe::DataFrame};
use tracing::{debug, trace};

use avenger_chart_core::{
    DefaultLogicalExprNodeExt, DomainCoordination, PositionedSubplotMarkCore, SharingLevel,
};

use crate::{
    concat::compiled_subplot,
    coords::CellDomainInfo,
    error::AvengerChartError,
    facet::{
        evaluated_facet_tree::EvaluatedFacetTree,
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        sharing_policy,
    },
    marks::CompiledMark,
    partition::PartitionKeyExtractor,
    plot::compiled::{
        ChildFrameDomainRequest, CompiledPlot, ContainerPathSegment, CoordinationKind,
        CoordinationScopeKey, aggregate_domain_requests, apply_domain_group_to_key,
        child_frame_domain_sharing_levels_for_plot, compiled_subplot_payload_child_plot,
        container_path_without_facet_segments,
        scales::build_scale_builder_from_compiled_plot_with_facet_scope,
    },
    render::EvaluationContext,
    scales::{DomainExtent, PlotScaleSpec, ScaleBuilder},
    serialization::LogicalPlanNodeExt,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FacetScaleBuilderNodeKey {
    subplot_ptr: usize,
    canonical_parent_path: Vec<ScalarValue>,
}

impl FacetScaleBuilderNodeKey {
    pub(crate) fn new(subplot: &Arc<CompiledPlot>, parent_path: &[ScalarValue]) -> Self {
        Self {
            subplot_ptr: Arc::as_ptr(subplot) as usize,
            canonical_parent_path: canonicalize_path(parent_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FacetScaleBuilderSubtreeKey {
    marks_scope_ptr: usize,
    canonical_parent_path: Vec<ScalarValue>,
}

impl FacetScaleBuilderSubtreeKey {
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
pub(crate) struct FacetScaleBuilderNodeArtifacts {
    pub(crate) child_facet_slot_sharing: Option<SharingLevel>,
    pub(crate) child_facet_depth: u8,
    pub(crate) requires_per_cell_channel_domain_sharing: bool,
    pub(crate) shared_scale_builder: ScaleBuilder,
    pub(crate) ancestor_scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder>,
    pub(crate) per_cell_scale_builder_cache: HashMap<Vec<ScalarValue>, ScaleBuilder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FacetDomainInfoKey {
    subplot_ptr: usize,
    canonical_full_cell_path: Vec<ScalarValue>,
    channel: String,
    facet_depth: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FacetChildFrameDomainInfoKey {
    relative_child_frame_path: Vec<ContainerPathSegment>,
    canonical_full_cell_path: Vec<ScalarValue>,
    channel: String,
    domain_coordination: DomainCoordination,
    facet_depth: u8,
    preserve_child_frame_path: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FacetChildFrameDomainInfo {
    pub(crate) relative_child_frame_path: Vec<ContainerPathSegment>,
    pub(crate) full_cell_path: Vec<ScalarValue>,
    pub(crate) channel: String,
    pub(crate) domain_sharing_level: SharingLevel,
    pub(crate) domain_coordination: DomainCoordination,
    pub(crate) facet_depth: u8,
    pub(crate) preserve_child_frame_path: bool,
    pub(crate) extent: DomainExtent,
}

#[derive(Default)]
struct FacetScaleBuilderPrecomputeState {
    precomputed_subtrees: HashSet<FacetScaleBuilderSubtreeKey>,
    node_artifacts: HashMap<FacetScaleBuilderNodeKey, Arc<FacetScaleBuilderNodeArtifacts>>,
    domain_infos: HashMap<FacetDomainInfoKey, CellDomainInfo>,
    child_frame_domain_infos: HashMap<FacetChildFrameDomainInfoKey, FacetChildFrameDomainInfo>,
}

#[derive(Default)]
pub(crate) struct FacetScaleBuilderPrecomputeStore {
    state: Mutex<FacetScaleBuilderPrecomputeState>,
}

impl FacetScaleBuilderPrecomputeStore {
    pub(crate) fn is_subtree_precomputed(&self, key: &FacetScaleBuilderSubtreeKey) -> bool {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .precomputed_subtrees
            .contains(key)
    }

    pub(crate) fn mark_subtree_precomputed(&self, key: FacetScaleBuilderSubtreeKey) {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .precomputed_subtrees
            .insert(key);
    }

    pub(crate) fn get_node_artifacts(
        &self,
        key: &FacetScaleBuilderNodeKey,
    ) -> Option<Arc<FacetScaleBuilderNodeArtifacts>> {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .node_artifacts
            .get(key)
            .cloned()
    }

    pub(crate) fn insert_node_artifacts(
        &self,
        key: FacetScaleBuilderNodeKey,
        artifacts: Arc<FacetScaleBuilderNodeArtifacts>,
    ) {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .node_artifacts
            .insert(key, artifacts);
    }

    pub(crate) fn insert_domain_infos(
        &self,
        compiled_subplot: &Arc<CompiledPlot>,
        infos: Vec<CellDomainInfo>,
    ) {
        let subplot_ptr = Arc::as_ptr(compiled_subplot) as usize;
        let mut state = self
            .state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned");
        for info in infos {
            let key = FacetDomainInfoKey {
                subplot_ptr,
                canonical_full_cell_path: canonicalize_path(&info.full_cell_path),
                channel: info.channel.clone(),
                facet_depth: info.facet_depth,
            };
            state.domain_infos.insert(key, info);
        }
    }

    pub(crate) fn domain_infos(&self) -> Vec<CellDomainInfo> {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .domain_infos
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn insert_child_frame_domain_infos(&self, infos: Vec<FacetChildFrameDomainInfo>) {
        let mut state = self
            .state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned");
        for info in infos {
            let key = FacetChildFrameDomainInfoKey {
                relative_child_frame_path: info.relative_child_frame_path.clone(),
                canonical_full_cell_path: canonicalize_path(&info.full_cell_path),
                channel: info.channel.clone(),
                domain_coordination: info.domain_coordination.clone(),
                facet_depth: info.facet_depth,
                preserve_child_frame_path: info.preserve_child_frame_path,
            };
            state.child_frame_domain_infos.insert(key, info);
        }
    }

    pub(crate) fn child_frame_domain_infos(&self) -> Vec<FacetChildFrameDomainInfo> {
        self.state
            .lock()
            .expect("FacetScaleBuilderPrecomputeStore lock poisoned")
            .child_frame_domain_infos
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn coordinated_child_frame_domain_extents(
        &self,
        relative_child_frame_path: &[ContainerPathSegment],
        full_cell_path: &[ScalarValue],
    ) -> HashMap<String, DomainExtent> {
        let infos = self.child_frame_domain_infos();
        if infos.is_empty() {
            return HashMap::new();
        }

        let unified =
            aggregate_domain_requests(infos.iter().filter_map(child_frame_domain_request_for_info));
        let mut sharing_levels = HashMap::new();
        for info in infos
            .iter()
            .filter(|info| info.relative_child_frame_path == relative_child_frame_path)
        {
            if info.domain_sharing_level.is_free() {
                continue;
            }
            sharing_levels
                .entry(info.channel.clone())
                .and_modify(|existing: &mut (SharingLevel, DomainCoordination, bool)| {
                    if info.domain_sharing_level > existing.0 {
                        *existing = (
                            info.domain_sharing_level,
                            info.domain_coordination.clone(),
                            info.preserve_child_frame_path,
                        );
                    }
                })
                .or_insert((
                    info.domain_sharing_level,
                    info.domain_coordination.clone(),
                    info.preserve_child_frame_path,
                ));
        }

        let facet_depth = full_cell_path.len() as u8;
        sharing_levels
            .into_iter()
            .filter_map(
                |(channel, (sharing_level, coordination, preserve_child_frame_path))| {
                    let key = child_frame_domain_coordination_scope_key(
                        relative_child_frame_path,
                        &channel,
                        full_cell_path,
                        sharing_level,
                        &coordination,
                        facet_depth,
                        preserve_child_frame_path,
                    );
                    unified.get(&key).cloned().map(|extent| (channel, extent))
                },
            )
            .collect()
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

pub(crate) fn child_frame_domain_coordination_scope_key(
    relative_child_frame_path: &[ContainerPathSegment],
    channel: &str,
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    coordination: &DomainCoordination,
    facet_depth: u8,
    preserve_child_frame_path: bool,
) -> CoordinationScopeKey {
    if preserve_child_frame_path {
        let ancestor_key =
            sharing_policy::domain_group_key(full_cell_path, sharing_level, facet_depth);
        let key = CoordinationScopeKey::partition_path_in_container(
            CoordinationKind::ScaleDomain,
            relative_child_frame_path.to_vec(),
            ancestor_key,
        );
        return apply_domain_group_to_key(key, channel, coordination);
    }

    let facet_depth = facet_depth as usize;
    let total_depth = facet_depth.saturating_add(relative_child_frame_path.len());
    let keep_count = if sharing_level.raw() as usize >= total_depth {
        0
    } else {
        total_depth.saturating_sub(sharing_level.raw() as usize)
    };
    let keep_facet_count = keep_count.min(facet_depth);
    let keep_child_frame_count = keep_count.saturating_sub(facet_depth);
    let ancestor_key = full_cell_path
        .get(..keep_facet_count.min(full_cell_path.len()))
        .unwrap_or(full_cell_path)
        .to_vec();
    let container_path = relative_child_frame_path
        .get(..keep_child_frame_count.min(relative_child_frame_path.len()))
        .unwrap_or(relative_child_frame_path)
        .to_vec();
    let key = CoordinationScopeKey::partition_path_in_container(
        CoordinationKind::ScaleDomain,
        container_path,
        ancestor_key,
    );
    apply_domain_group_to_key(key, channel, coordination)
}

fn child_frame_domain_request_for_info(
    info: &FacetChildFrameDomainInfo,
) -> Option<ChildFrameDomainRequest> {
    (!info.domain_sharing_level.is_free()).then(|| {
        ChildFrameDomainRequest::new(
            child_frame_domain_coordination_scope_key(
                &info.relative_child_frame_path,
                &info.channel,
                &info.full_cell_path,
                info.domain_sharing_level,
                &info.domain_coordination,
                info.facet_depth,
                info.preserve_child_frame_path,
            ),
            info.extent.clone(),
        )
    })
}

fn resolve_current_facet_node(
    compiled_marks: &[Arc<dyn CompiledMark>],
) -> Option<(Arc<CompiledPlot>, SharingLevel)> {
    for mark in compiled_marks {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            match facet_mark {
                FacetSubplotRef::Row(facet_row) => {
                    return Some((
                        facet_row.compiled_subplot_arc(),
                        facet_row
                            .facet_slot_sharing()
                            .map(SharingLevel::from)
                            .unwrap_or(SharingLevel::FREE),
                    ));
                }
                FacetSubplotRef::Col(facet_col) => {
                    return Some((
                        facet_col.compiled_subplot_arc(),
                        facet_col
                            .facet_slot_sharing()
                            .map(SharingLevel::from)
                            .unwrap_or(SharingLevel::FREE),
                    ));
                }
                FacetSubplotRef::Wrap(facet_wrap) => {
                    return Some((facet_wrap.physical_subplot_arc(), SharingLevel::FREE));
                }
            }
        }
    }

    None
}

fn resolve_child_facet_slot_sharing(compiled_subplot: &Arc<CompiledPlot>) -> Option<SharingLevel> {
    compiled_subplot.marks.iter().find_map(|mark| {
        facet_subplot_ref(mark.as_ref()).and_then(|facet_mark| match facet_mark {
            FacetSubplotRef::Row(facet_row) => {
                facet_row.facet_slot_sharing().map(SharingLevel::from)
            }
            FacetSubplotRef::Col(facet_col) => {
                facet_col.facet_slot_sharing().map(SharingLevel::from)
            }
            FacetSubplotRef::Wrap(facet_wrap) => {
                facet_wrap.facet_slot_sharing().map(SharingLevel::from)
            }
        })
    })
}

fn enumerate_cell_values_for_node(
    facet_tree: &EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    current_facet_slot_sharing: SharingLevel,
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
        .enumerate_values_for_facet(facet_path, current_facet_slot_sharing.raw())
        .unwrap_or_else(fallback)
}

async fn build_ancestor_group_scale_builders(
    cell_values: &[ScalarValue],
    sharing_level: SharingLevel,
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
        let ancestor_key = facet_tree.sharing_owner_path(&full_path, sharing_level.raw());
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

        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
            compiled_subplot,
            Some(filtered_df),
            eval_ctx,
            &ancestor_key,
            compiled_subplot.get_theme().as_ref(),
        ))
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

        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
            compiled_subplot,
            Some(data_override),
            eval_ctx,
            &full_path,
            compiled_subplot.get_theme().as_ref(),
        ))
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
) -> Result<FacetScaleBuilderNodeArtifacts, AvengerChartError> {
    let shared_scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
        compiled_subplot,
        Some(inherited_data_df.clone()),
        eval_ctx,
        facet_path,
        compiled_subplot.get_theme().as_ref(),
    ))
    .await?;

    let child_facet_depth = (facet_tree.logical_depth_for_path(facet_path) + 2) as u8;
    let child_facet_slot_sharing = resolve_child_facet_slot_sharing(compiled_subplot);
    let has_free_ordered_channel_domain_sharing =
        plot_has_free_ordered_channel_domain_sharing(compiled_subplot);
    let requires_per_cell_channel_domain_sharing =
        has_free_ordered_channel_domain_sharing || facet_tree.has_free_channel_domain_sharing();

    let ancestor_scale_builder_cache = if let Some(sharing_level) = child_facet_slot_sharing {
        if sharing_level > 0 && sharing_level < child_facet_depth {
            Box::pin(build_ancestor_group_scale_builders(
                cell_values,
                sharing_level,
                facet_path,
                facet_tree,
                inherited_data_df,
                compiled_subplot,
                eval_ctx,
            ))
            .await?
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    let needs_per_cell_scale_builder_cache = matches!(child_facet_slot_sharing, Some(level) if level.is_free())
        || requires_per_cell_channel_domain_sharing;

    let per_cell_scale_builder_cache = if needs_per_cell_scale_builder_cache {
        Box::pin(build_per_cell_scale_builders(
            cell_values,
            facet_path,
            facet_tree,
            inherited_data_df,
            compiled_subplot,
            eval_ctx,
        ))
        .await?
    } else {
        HashMap::new()
    };

    Ok(FacetScaleBuilderNodeArtifacts {
        child_facet_slot_sharing,
        child_facet_depth,
        requires_per_cell_channel_domain_sharing,
        shared_scale_builder,
        ancestor_scale_builder_cache,
        per_cell_scale_builder_cache,
    })
}

fn plot_has_free_ordered_channel_domain_sharing(plot: &CompiledPlot) -> bool {
    marks_have_free_ordered_channel_domain_sharing(&plot.marks, &plot.scale_specs)
}

fn marks_have_free_ordered_channel_domain_sharing(
    marks: &[Arc<dyn CompiledMark>],
    scale_specs: &HashMap<String, PlotScaleSpec>,
) -> bool {
    marks.iter().any(|mark| {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            return plot_has_free_ordered_channel_domain_sharing(facet_mark.compiled_subplot());
        }

        mark.data_context()
            .channels()
            .iter()
            .any(|(channel_name, channel_value)| {
                let Some(sharing) = channel_value.get_domain_scope() else {
                    return false;
                };
                if !SharingLevel::from(sharing).is_free() {
                    return false;
                }

                if channel_value
                    .get_scale_config()
                    .and_then(|config| config.ordering.as_option())
                    .is_some_and(|ordering| ordering.has_order_expr())
                {
                    return true;
                }

                let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                    return false;
                };
                let Some(PlotScaleSpec::Local(config)) = scale_specs.get(&scale_name) else {
                    return false;
                };
                config
                    .ordering
                    .as_option()
                    .is_some_and(|ordering| ordering.has_order_expr())
            })
    })
}

async fn collect_node_domain_infos(
    cell_values: &[ScalarValue],
    facet_path: &[ScalarValue],
    inherited_data_df: &DataFrame,
    compiled_subplot: &Arc<CompiledPlot>,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
    artifacts: &FacetScaleBuilderNodeArtifacts,
) -> Result<Vec<CellDomainInfo>, AvengerChartError> {
    let mut infos = Vec::new();
    let mut ordered_owner_extent_cache: HashMap<(Vec<ScalarValue>, String), DomainExtent> =
        HashMap::new();

    for value in cell_values {
        let mut full_path = facet_path.to_vec();
        full_path.push(value.clone());
        if !facet_tree.cell_exists(&full_path) || !facet_tree.cell_has_data(&full_path) {
            continue;
        }

        let canonical_full_path = canonicalize_path(&full_path);
        let cached_builder = artifacts
            .per_cell_scale_builder_cache
            .get(&canonical_full_path)
            .or_else(|| artifacts.per_cell_scale_builder_cache.get(&full_path));
        let scale_builder;
        let builder = if let Some(cached_builder) = cached_builder {
            cached_builder
        } else {
            let data_override = if let Some(predicate) = facet_tree.cell_predicate(&full_path, 0) {
                inherited_data_df.clone().filter(predicate).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to filter data for domain precompute cell {:?}: {}",
                        full_path, e
                    ))
                })?
            } else {
                inherited_data_df.clone()
            };

            scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
                compiled_subplot,
                Some(data_override),
                eval_ctx,
                &full_path,
                compiled_subplot.get_theme().as_ref(),
            ))
            .await?;
            &scale_builder
        };

        let facet_depth = facet_tree.logical_depth_for_path(&full_path) as u8;
        let domain_channels = facet_tree.domain_extent_channels();
        let domain_channel_refs = domain_channels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        for (channel, mut extent) in builder.extract_domain_extents(&domain_channel_refs) {
            let domain_coordination = facet_tree.channel_domain_coordination(&channel);
            let sharing_level = SharingLevel::from(domain_coordination.scope);
            let domain_sharing_level = sharing_level.raw();
            if extent.ordered_discrete && !sharing_level.is_free() {
                let owner_path = facet_tree.sharing_owner_path(&full_path, sharing_level.raw());
                let cache_key = (canonicalize_path(&owner_path), channel.clone());
                if let Some(cached_extent) = ordered_owner_extent_cache.get(&cache_key) {
                    extent = cached_extent.clone();
                } else {
                    let owner_data = if let Some(predicate) = facet_tree.path_predicate(&owner_path)
                    {
                        inherited_data_df.clone().filter(predicate).map_err(|e| {
                            AvengerChartError::InternalError(format!(
                                "Failed to filter data for ordered scale-domain owner {:?}: {}",
                                owner_path, e
                            ))
                        })?
                    } else {
                        inherited_data_df.clone()
                    };
                    let owner_builder =
                        Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
                            compiled_subplot,
                            Some(owner_data),
                            eval_ctx,
                            &owner_path,
                            compiled_subplot.get_theme().as_ref(),
                        ))
                        .await?;
                    if let Some(owner_extent) = owner_builder
                        .extract_domain_extents(&[channel.as_str()])
                        .remove(&channel)
                    {
                        extent = owner_extent.clone();
                        ordered_owner_extent_cache.insert(cache_key, owner_extent);
                    }
                }
            }
            infos.push(CellDomainInfo {
                full_cell_path: full_path.clone(),
                channel,
                domain_sharing_level,
                domain_coordination,
                facet_depth,
                owner_path: Some(facet_tree.sharing_owner_path(
                    &full_path,
                    SharingLevel::from_raw(domain_sharing_level).raw(),
                )),
                extent,
            });
        }
    }

    Ok(infos)
}

fn explicit_dataframe_for_plot(
    plot: &CompiledPlot,
    eval_ctx: &EvaluationContext,
) -> Option<DataFrame> {
    for mark in &plot.marks {
        if let Some(df) = mark
            .data_context()
            .dataframe_with_context(eval_ctx.session_context.as_ref())
            && !matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_))
        {
            return Some(df);
        }
    }

    plot.data.as_ref().and_then(|data_node| {
        data_node
            .to_logical_plan(eval_ctx.session_context.as_ref())
            .ok()
            .map(|logical_plan| {
                DataFrame::new(eval_ctx.session_context.state().clone(), logical_plan)
            })
    })
}

#[derive(Debug, Clone)]
struct EffectiveChildFrameDomainCoordination {
    coordination: DomainCoordination,
    level: SharingLevel,
    preserve_child_frame_path: bool,
}

fn effective_child_frame_domain_sharing(
    channel: &str,
    explicit_sharing: &HashMap<String, DomainCoordination>,
    facet_tree: &EvaluatedFacetTree,
) -> EffectiveChildFrameDomainCoordination {
    if let Some(coordination) = explicit_sharing.get(channel).cloned() {
        EffectiveChildFrameDomainCoordination {
            level: SharingLevel::from(coordination.scope),
            coordination,
            preserve_child_frame_path: false,
        }
    } else {
        let level = facet_tree.channel_domain_sharing_level_typed(channel);
        EffectiveChildFrameDomainCoordination {
            level,
            coordination: DomainCoordination::scale_name(level.into()),
            preserve_child_frame_path: true,
        }
    }
}

async fn collect_positioned_child_frame_domain_infos_for_mark(
    subplot: &dyn PositionedSubplotMarkCore,
    relative_child_frame_path: &[ContainerPathSegment],
    full_cell_path: &[ScalarValue],
    inherited_data_df: Option<&DataFrame>,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<Vec<FacetChildFrameDomainInfo>, AvengerChartError> {
    if !subplot.is_partitioned() {
        return Ok(Vec::new());
    }

    let Some(parent_data) = inherited_data_df else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Partitioned {} subplots require inherited parent data for facet scale-builder precompute",
            subplot.spec().outer_label
        )));
    };

    let ctx = eval_ctx.session_context.as_ref();
    let partition_expr = subplot
        .partition_expr()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Partitioned positioned subplot is missing its partition expression".to_string(),
            )
        })?
        .to_expr(ctx)?;
    let partition_values = Box::pin(PartitionKeyExtractor::extract_keys(
        parent_data,
        &partition_expr,
        &eval_ctx.params,
    ))
    .await?;
    let child_plot = compiled_subplot_payload_child_plot(subplot.payload());
    let explicit_sharing = child_frame_domain_sharing_levels_for_plot(child_plot);

    let mut infos = Vec::new();
    for value in partition_values {
        let filtered_data = parent_data
            .clone()
            .filter(partition_expr.clone().eq(lit(value.clone())))?;
        let mut child_relative_path = relative_child_frame_path.to_vec();
        child_relative_path.push(ContainerPathSegment::positioned_partition(
            subplot.mark_index(),
            value,
            subplot.key(),
        ));

        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
            child_plot,
            Some(filtered_data.clone()),
            eval_ctx,
            full_cell_path,
            child_plot.get_theme().as_ref(),
        ))
        .await?;

        let channels = scale_builder
            .channel_builders()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let facet_depth = full_cell_path.len() as u8;
        for (channel, extent) in scale_builder.extract_domain_extents(&channels) {
            let sharing =
                effective_child_frame_domain_sharing(&channel, &explicit_sharing, facet_tree);
            if sharing.level.is_free() {
                continue;
            }

            infos.push(FacetChildFrameDomainInfo {
                relative_child_frame_path: child_relative_path.clone(),
                full_cell_path: full_cell_path.to_vec(),
                channel,
                domain_sharing_level: sharing.level,
                domain_coordination: sharing.coordination.clone(),
                facet_depth,
                preserve_child_frame_path: sharing.preserve_child_frame_path,
                extent,
            });
        }

        infos.extend(
            Box::pin(collect_child_frame_domain_infos_for_marks(
                &child_plot.marks,
                &child_relative_path,
                full_cell_path,
                Some(&filtered_data),
                facet_tree,
                eval_ctx,
            ))
            .await?,
        );
    }

    Ok(infos)
}

async fn collect_child_frame_domain_infos_for_marks(
    compiled_marks: &[Arc<dyn CompiledMark>],
    relative_child_frame_path: &[ContainerPathSegment],
    full_cell_path: &[ScalarValue],
    inherited_data_df: Option<&DataFrame>,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<Vec<FacetChildFrameDomainInfo>, AvengerChartError> {
    let mut infos = Vec::new();

    for mark in compiled_marks {
        if let Some(subplot) = mark.as_positioned_subplot() {
            infos.extend(
                Box::pin(collect_positioned_child_frame_domain_infos_for_mark(
                    subplot,
                    relative_child_frame_path,
                    full_cell_path,
                    inherited_data_df,
                    facet_tree,
                    eval_ctx,
                ))
                .await?,
            );
        }

        if mark.mark_type() != "subplot" {
            continue;
        }
        let Some(subplot) = compiled_subplot(mark.as_ref()) else {
            continue;
        };

        let child_plot = subplot.compiled_subplot();
        let child_data_override = if subplot.inherits_parent_data() {
            inherited_data_df.cloned()
        } else {
            None
        };
        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot_with_facet_scope(
            child_plot,
            child_data_override.clone(),
            eval_ctx,
            full_cell_path,
            child_plot.get_theme().as_ref(),
        ))
        .await?;

        let mut child_relative_path = relative_child_frame_path.to_vec();
        child_relative_path.push(ContainerPathSegment::concat_child(
            subplot.child_index(),
            subplot.key(),
        ));

        let channels = scale_builder
            .channel_builders()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let explicit_sharing = child_frame_domain_sharing_levels_for_plot(child_plot);
        let facet_depth = full_cell_path.len() as u8;
        for (channel, extent) in scale_builder.extract_domain_extents(&channels) {
            let sharing =
                effective_child_frame_domain_sharing(&channel, &explicit_sharing, facet_tree);
            if sharing.level.is_free() {
                continue;
            }

            infos.push(FacetChildFrameDomainInfo {
                relative_child_frame_path: child_relative_path.clone(),
                full_cell_path: full_cell_path.to_vec(),
                channel,
                domain_sharing_level: sharing.level,
                domain_coordination: sharing.coordination.clone(),
                facet_depth,
                preserve_child_frame_path: sharing.preserve_child_frame_path,
                extent,
            });
        }

        let nested_explicit_data = if subplot.inherits_parent_data() {
            None
        } else {
            explicit_dataframe_for_plot(child_plot, eval_ctx)
        };
        let nested_inherited_data = if subplot.inherits_parent_data() {
            child_data_override.as_ref().or(inherited_data_df)
        } else {
            nested_explicit_data.as_ref()
        };
        infos.extend(
            Box::pin(collect_child_frame_domain_infos_for_marks(
                &child_plot.marks,
                &child_relative_path,
                full_cell_path,
                nested_inherited_data,
                facet_tree,
                eval_ctx,
            ))
            .await?,
        );
    }

    Ok(infos)
}

async fn ensure_subtree_precomputed_internal(
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
    inherited_data_df: &DataFrame,
    facet_tree: &EvaluatedFacetTree,
    eval_ctx: &EvaluationContext,
) -> Result<(), AvengerChartError> {
    let Some((compiled_subplot, current_facet_slot_sharing)) =
        resolve_current_facet_node(compiled_marks)
    else {
        return Ok(());
    };

    let store = eval_ctx.facet_scale_builder_precompute_store();
    let node_key = FacetScaleBuilderNodeKey::new(&compiled_subplot, facet_path);

    let cell_values =
        enumerate_cell_values_for_node(facet_tree, facet_path, current_facet_slot_sharing);
    let artifacts = if let Some(artifacts) = store.get_node_artifacts(&node_key) {
        artifacts
    } else {
        let artifacts = Box::pin(build_node_artifacts(
            &cell_values,
            facet_path,
            inherited_data_df,
            &compiled_subplot,
            facet_tree,
            eval_ctx,
        ))
        .await?;
        let artifacts = Arc::new(artifacts);
        store.insert_node_artifacts(node_key.clone(), artifacts.clone());
        trace!(facet_path = ?facet_path, "facet scale-builder precompute built node artifacts");
        artifacts
    };

    let domain_infos = Box::pin(collect_node_domain_infos(
        &cell_values,
        facet_path,
        inherited_data_df,
        &compiled_subplot,
        facet_tree,
        eval_ctx,
        &artifacts,
    ))
    .await?;
    store.insert_domain_infos(&compiled_subplot, domain_infos);

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

        let relative_child_frame_path =
            container_path_without_facet_segments(eval_ctx.child_frame_container_path());
        let child_frame_domain_infos = Box::pin(collect_child_frame_domain_infos_for_marks(
            &compiled_subplot.marks,
            &relative_child_frame_path,
            &full_path,
            Some(&child_data_df),
            facet_tree,
            eval_ctx,
        ))
        .await?;
        store.insert_child_frame_domain_infos(child_frame_domain_infos);

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
    let subtree_key = FacetScaleBuilderSubtreeKey::new(compiled_marks, facet_path);
    let store = eval_ctx.facet_scale_builder_precompute_store();
    if store.is_subtree_precomputed(&subtree_key) {
        return Ok(());
    }

    Box::pin(ensure_subtree_precomputed_internal(
        compiled_marks,
        facet_path,
        inherited_data_df,
        facet_tree,
        eval_ctx,
    ))
    .await?;
    store.mark_subtree_precomputed(subtree_key);
    debug!(
        facet_path = ?facet_path,
        "facet scale-builder precompute completed for subtree"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_scales::domain_extent::{DomainExtent, SerializableDomainValue};

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
        let store = FacetScaleBuilderPrecomputeStore::default();
        let key = FacetScaleBuilderNodeKey {
            subplot_ptr: 42,
            canonical_parent_path: canonicalize_path(&[s_utf8_view("x"), s_large_utf8("y")]),
        };
        let artifacts = Arc::new(FacetScaleBuilderNodeArtifacts::default());
        store.insert_node_artifacts(key, artifacts.clone());

        let lookup = FacetScaleBuilderNodeKey {
            subplot_ptr: 42,
            canonical_parent_path: canonicalize_path(&[s_utf8("x"), s_utf8("y")]),
        };
        let found = store.get_node_artifacts(&lookup);
        assert!(found.is_some());
        assert!(Arc::ptr_eq(&found.unwrap(), &artifacts));
    }

    #[test]
    fn subtree_key_dedupes_by_scope_and_path() {
        let store = FacetScaleBuilderPrecomputeStore::default();
        let key = FacetScaleBuilderSubtreeKey {
            marks_scope_ptr: 77,
            canonical_parent_path: canonicalize_path(&[s_utf8_view("r")]),
        };
        assert!(!store.is_subtree_precomputed(&key));
        store.mark_subtree_precomputed(key.clone());
        assert!(store.is_subtree_precomputed(&key));
    }

    #[test]
    fn child_frame_domain_lookup_coordinates_by_sharing_path_and_facet_group() {
        let store = FacetScaleBuilderPrecomputeStore::default();
        let sepal = vec![ContainerPathSegment::concat_child(0, Some("sepal"))];
        let petal = vec![ContainerPathSegment::concat_child(1, Some("petal"))];
        store.insert_child_frame_domain_infos(vec![
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: false,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "setosa".to_string(),
                )]),
            },
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("virginica")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: false,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "virginica".to_string(),
                )]),
            },
            FacetChildFrameDomainInfo {
                relative_child_frame_path: petal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: false,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "narrow".to_string(),
                )]),
            },
        ]);

        let sepal_extents =
            store.coordinated_child_frame_domain_extents(&sepal, &[s_utf8("setosa")]);
        let petal_extents =
            store.coordinated_child_frame_domain_extents(&petal, &[s_utf8("setosa")]);

        assert_eq!(
            sepal_extents
                .get("fill")
                .and_then(DomainExtent::discrete_values)
                .map(|values| values.len()),
            Some(3)
        );
        assert_eq!(
            petal_extents
                .get("fill")
                .and_then(DomainExtent::discrete_values)
                .map(|values| values.len()),
            Some(3)
        );
    }

    #[test]
    fn child_frame_domain_lookup_coordinates_named_groups_across_channels() {
        let store = FacetScaleBuilderPrecomputeStore::default();
        let sepal = vec![ContainerPathSegment::concat_child(0, Some("sepal"))];
        store.insert_child_frame_domain_infos(vec![
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "x".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::named(
                    SharingLevel::GLOBAL.into(),
                    "height",
                )
                .unwrap(),
                facet_depth: 1,
                preserve_child_frame_path: false,
                extent: DomainExtent::numeric(0.0, 2.0),
            },
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "y".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::named(
                    SharingLevel::GLOBAL.into(),
                    "height",
                )
                .unwrap(),
                facet_depth: 1,
                preserve_child_frame_path: false,
                extent: DomainExtent::numeric(0.0, 101.0),
            },
        ]);

        let extents = store.coordinated_child_frame_domain_extents(&sepal, &[s_utf8("setosa")]);

        assert_eq!(extents.get("x"), Some(&DomainExtent::numeric(0.0, 101.0)));
        assert_eq!(extents.get("y"), Some(&DomainExtent::numeric(0.0, 101.0)));
    }

    #[test]
    fn default_facet_scoped_child_frame_domains_preserve_child_path() {
        let store = FacetScaleBuilderPrecomputeStore::default();
        let sepal = vec![ContainerPathSegment::concat_child(0, Some("sepal"))];
        let petal = vec![ContainerPathSegment::concat_child(1, Some("petal"))];
        store.insert_child_frame_domain_infos(vec![
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: true,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "setosa".to_string(),
                )]),
            },
            FacetChildFrameDomainInfo {
                relative_child_frame_path: sepal.clone(),
                full_cell_path: vec![s_utf8("virginica")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: true,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "virginica".to_string(),
                )]),
            },
            FacetChildFrameDomainInfo {
                relative_child_frame_path: petal.clone(),
                full_cell_path: vec![s_utf8("setosa")],
                channel: "fill".to_string(),
                domain_sharing_level: SharingLevel::GLOBAL,
                domain_coordination: DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
                facet_depth: 1,
                preserve_child_frame_path: true,
                extent: DomainExtent::discrete(vec![SerializableDomainValue::String(
                    "narrow".to_string(),
                )]),
            },
        ]);

        let sepal_extents =
            store.coordinated_child_frame_domain_extents(&sepal, &[s_utf8("setosa")]);
        let petal_extents =
            store.coordinated_child_frame_domain_extents(&petal, &[s_utf8("setosa")]);

        assert_eq!(
            sepal_extents
                .get("fill")
                .and_then(DomainExtent::discrete_values)
                .map(|values| values.len()),
            Some(2)
        );
        assert_eq!(
            petal_extents
                .get("fill")
                .and_then(DomainExtent::discrete_values)
                .map(|values| values.len()),
            Some(1)
        );
    }
}
