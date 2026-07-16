//! Evaluated facet structure for visibility, filtering, and layout.
//!
//! This module provides a unified data structure that holds everything about
//! the evaluated facet hierarchy. It's built once from data at the start of
//! `CompiledPlot::evaluate()`, then queried throughout measurement and rendering for:
//! - Visibility decisions (axis ticks, titles, facet labels)
//! - Filter predicates for data slicing
//! - Facet slot values for iteration
//! - Position and count information for layout

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};
use indexmap::IndexMap;
use tracing::debug;

use avenger_chart_core::{ChannelValue, DomainCoordination, NestedBandSpec};

pub use crate::partition::{PartitionContent, PartitionNode};

use crate::{
    error::AvengerChartError,
    facet::FacetDirection,
    facet::{
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        sharing_policy,
    },
    partition::{PartitionDimensionSpec, PartitionSlotCache, scalar_values_equivalent},
    plot::{CompiledPlot, compiled::SharingGroupEdge},
    render::context::{FacetDimensionSizing, FacetRuntimeSizingPolicy},
};
pub(crate) use avenger_chart_core::AxisOwnershipMode;
pub use avenger_chart_core::AxisVisibility;
use avenger_chart_core::{
    AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, AxisPosition, CompiledMark,
    DefaultLogicalExprNodeExt, ExprHelpers, FacetWrapColumnMode, LogicalPlanNodeExt, SharingLevel,
    contains_aggregate, params_to_datafusion,
};

/// Evaluated facet structure - built once from data at evaluate() time, queried throughout.
///
/// This is the single source of truth for all facet-related operations.
/// It captures the tree structure of partition values (handling non-shared facet slots),
/// and provides methods for visibility, filtering, and layout queries.
///
/// Note: This struct contains ONLY the partition tree structure. All configuration
/// (slot sharing levels, axis positions) is passed as parameters to query methods.
/// This allows the same evaluated tree to be used with different configurations.
#[derive(Debug, Clone)]
pub struct EvaluatedFacetTree {
    /// Tree of partition values (handles non-shared facet slots).
    root: Option<PartitionNode>,
    /// Cached depth of the partition hierarchy.
    depth_cache: usize,
    /// Cached facet slot counts per nesting level.
    level_counts_cache: Vec<usize>,
    /// Channel-domain coordination targets extracted from innermost marks.
    ///
    /// Used for scale-domain coordination and for deriving sharing levels when
    /// guide visibility decisions do not have access to CoordMeasurement.
    channel_domain_coordinations: HashMap<String, DomainCoordination>,
    /// Nested-band scale metadata extracted from innermost marks.
    channel_nested_band_configs: HashMap<String, NestedBandSpec>,
    /// Cached path metadata for resolved concrete paths.
    path_info_cache: HashMap<Vec<ScalarValue>, ResolvedFacetPathInfo>,
    /// Cached predicates for valid, non-empty paths.
    path_predicate_cache: HashMap<Vec<ScalarValue>, Expr>,
    /// Cached slot membership by parent path.
    slot_membership_cache: HashMap<Vec<ScalarValue>, SlotMembership>,
    /// Cached facet value enumeration keyed by `(facet_path, sharing_level)`.
    enumeration_cache: HashMap<(Vec<ScalarValue>, SharingLevel), Vec<ScalarValue>>,
    /// Cached jagged-tree checks by axis position.
    jagged_axis_cache: HashMap<AxisPosition, bool>,
    /// CoordinationScope levels observed in channels/nodes plus canonical levels `{0, 255}`.
    used_sharing_levels: Vec<SharingLevel>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FacetWrapLayoutContext {
    current_available_width: Option<f32>,
    width_is_canvas_constrained: bool,
    height_is_leaf_plot_area_sized: bool,
}

impl FacetWrapLayoutContext {
    pub(crate) fn from_policy(policy: FacetRuntimeSizingPolicy, root_available_width: f32) -> Self {
        Self {
            current_available_width: match policy.width {
                FacetDimensionSizing::CanvasConstrained { .. } => Some(root_available_width),
                FacetDimensionSizing::LeafPlotAreaSized { .. } => None,
            },
            width_is_canvas_constrained: policy.width.is_canvas_constrained(),
            height_is_leaf_plot_area_sized: policy.height.is_leaf_plot_area_sized(),
        }
    }

    fn with_column_slots(self, columns: usize) -> Self {
        Self {
            current_available_width: self
                .current_available_width
                .map(|width| width / columns.max(1) as f32),
            ..self
        }
    }
}

/// Resolved geometry metadata for a concrete facet path.
///
/// This captures branch-local counts so ownership rules remain correct for
/// ragged trees where sibling branches have different cardinalities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFacetPathInfo {
    /// Position index at each depth for the provided path.
    pub indices: Vec<usize>,
    /// Facet slot count at each depth on the same concrete branch as `indices`.
    pub local_level_counts: Vec<usize>,
    /// Facet direction at each depth for the provided path.
    pub level_directions: Vec<FacetDirection>,
    /// Axis guide visibility config at each depth for the provided path.
    pub axis_guide_visibility: Vec<AxisGuideVisibilityConfig>,
    /// Facet direction at the resolved depth.
    pub direction: FacetDirection,
}

#[derive(Debug, Clone, Default)]
struct SlotMembership {
    domain_values: HashSet<ScalarValue>,
    observed_values: HashSet<ScalarValue>,
}

impl EvaluatedFacetTree {
    fn canonical_scalar(value: &ScalarValue) -> ScalarValue {
        match value {
            ScalarValue::Utf8(Some(v))
            | ScalarValue::LargeUtf8(Some(v))
            | ScalarValue::Utf8View(Some(v)) => ScalarValue::Utf8(Some(v.clone())),
            ScalarValue::Utf8(None)
            | ScalarValue::LargeUtf8(None)
            | ScalarValue::Utf8View(None) => ScalarValue::Utf8(None),
            _ => value.clone(),
        }
    }

    fn canonical_path(path: &[ScalarValue]) -> Vec<ScalarValue> {
        path.iter().map(Self::canonical_scalar).collect()
    }

    fn count_depth(node: &PartitionNode) -> usize {
        match &node.content {
            PartitionContent::Leaf { .. } => 1,
            PartitionContent::Branch { children } => {
                1 + children
                    .values()
                    .next()
                    .map(|b| Self::count_depth(b.as_ref()))
                    .unwrap_or(0)
            }
        }
    }

    fn init_with_caches(
        root: Option<PartitionNode>,
        channel_domain_coordinations: HashMap<String, DomainCoordination>,
        channel_nested_band_configs: HashMap<String, NestedBandSpec>,
    ) -> Self {
        let depth_cache = root.as_ref().map(Self::count_depth).unwrap_or(0);
        let level_counts_cache = root
            .as_ref()
            .map(Self::collect_level_counts_first_branch_for_root)
            .unwrap_or_default();

        let mut tree = Self {
            root,
            depth_cache,
            level_counts_cache,
            channel_domain_coordinations,
            channel_nested_band_configs,
            path_info_cache: HashMap::new(),
            path_predicate_cache: HashMap::new(),
            slot_membership_cache: HashMap::new(),
            enumeration_cache: HashMap::new(),
            jagged_axis_cache: HashMap::new(),
            used_sharing_levels: Vec::new(),
        };
        tree.rebuild_precalculated_caches();
        tree
    }

    /// Create a new EvaluatedFacetTree with the given partition tree.
    ///
    /// Note: All configuration (slot sharing levels, axis positions) is passed
    /// as parameters to query methods like `subplot_visibility`.
    pub fn new(root: Option<PartitionNode>) -> Self {
        Self::init_with_caches(root, HashMap::new(), HashMap::new())
    }

    /// Create a new EvaluatedFacetTree with partition tree and channel-domain sharing levels.
    pub fn new_with_channel_domain_sharing_levels(
        root: Option<PartitionNode>,
        channel_domain_sharing_levels: HashMap<String, u8>,
    ) -> Self {
        Self::init_with_caches(
            root,
            channel_domain_sharing_levels
                .into_iter()
                .map(|(channel, level)| {
                    (
                        channel,
                        DomainCoordination::scale_name(SharingLevel::from_raw(level).into()),
                    )
                })
                .collect(),
            HashMap::new(),
        )
    }

    /// Create an empty tree (no faceting).
    pub fn empty() -> Self {
        Self::init_with_caches(None, HashMap::new(), HashMap::new())
    }

    /// Get the partition tree root, if any.
    pub fn root(&self) -> Option<&PartitionNode> {
        self.root.as_ref()
    }

    /// Get the depth of the partition hierarchy.
    pub fn depth(&self) -> usize {
        self.depth_cache
    }

    pub(crate) fn structure_cache_key(&self) -> Vec<String> {
        let mut key = Vec::new();
        key.push(format!("depth:{:?}", self.depth_cache));
        let Some(root) = self.root.as_ref() else {
            key.push("empty".to_string());
            return key;
        };
        Self::push_node_structure_cache_key(root, &mut Vec::new(), &mut key);
        key
    }

    pub(crate) fn logical_structure_cache_key(&self) -> Vec<String> {
        let mut key = Vec::new();
        key.push(format!("logical_depth:{:?}", self.logical_depth()));
        let Some(root) = self.root.as_ref() else {
            key.push("empty".to_string());
            return key;
        };
        Self::push_logical_node_structure_cache_key(root, &mut Vec::new(), &mut key);
        key
    }

    fn push_node_structure_cache_key(
        node: &PartitionNode,
        path: &mut Vec<ScalarValue>,
        key: &mut Vec<String>,
    ) {
        let values = node.values().cloned().collect::<Vec<_>>();
        let observed_values = node.observed_values().cloned().collect::<Vec<_>>();
        key.push(format!(
            "node:path={:?};direction={:?};sharing={};field={};axis_policy={:?};min_slots={:?};values={:?};observed={:?}",
            Self::canonical_path(path),
            node.direction,
            node.sharing,
            node.field,
            node.axis_guide_visibility,
            node.min_slot_count(),
            values,
            observed_values
        ));
        if let PartitionContent::Branch { children } = &node.content {
            for (value, child) in children {
                path.push(value.clone());
                Self::push_node_structure_cache_key(child, path, key);
                path.pop();
            }
        }
    }

    fn push_logical_node_structure_cache_key(
        node: &PartitionNode,
        logical_path: &mut Vec<ScalarValue>,
        key: &mut Vec<String>,
    ) {
        if is_wrap_row_field(&node.field) {
            Self::push_logical_wrap_node_structure_cache_key(node, logical_path, key);
            return;
        }

        let values = node.values().cloned().collect::<Vec<_>>();
        let observed_values = node.observed_values().cloned().collect::<Vec<_>>();
        key.push(format!(
            "node:path={:?};direction={:?};sharing={};field={};axis_policy={:?};values={:?};observed={:?}",
            Self::canonical_path(logical_path),
            node.direction,
            node.sharing,
            node.field,
            node.axis_guide_visibility,
            values,
            observed_values
        ));
        if let PartitionContent::Branch { children } = &node.content {
            for (value, child) in children {
                logical_path.push(value.clone());
                Self::push_logical_node_structure_cache_key(child, logical_path, key);
                logical_path.pop();
            }
        }
    }

    fn push_logical_wrap_node_structure_cache_key(
        node: &PartitionNode,
        logical_path: &mut Vec<ScalarValue>,
        key: &mut Vec<String>,
    ) {
        let mut field = wrap_value_field_name(&node.field["__avenger_wrap_row:".len()..]);
        let mut values = Vec::new();
        let mut observed_values = Vec::new();
        if let PartitionContent::Branch { children } = &node.content {
            for child in children.values() {
                field = child.field.clone();
                values.extend(child.values().cloned());
                observed_values.extend(child.observed_values().cloned());
            }
        }

        key.push(format!(
            "wrap:path={:?};direction={:?};sharing={};field={};axis_policy={:?};values={:?};observed={:?}",
            Self::canonical_path(logical_path),
            FacetDirection::Column,
            node.sharing,
            field,
            node.axis_guide_visibility,
            values,
            observed_values
        ));

        if let PartitionContent::Branch { children } = &node.content {
            for row_child in children.values() {
                if let PartitionContent::Branch {
                    children: value_children,
                } = &row_child.content
                {
                    for (value, value_child) in value_children {
                        logical_path.push(value.clone());
                        Self::push_logical_node_structure_cache_key(value_child, logical_path, key);
                        logical_path.pop();
                    }
                }
            }
        }
    }

    pub(crate) fn logical_cell_key_for_path(&self, path: &[ScalarValue]) -> Option<Vec<String>> {
        let mut current = self.root.as_ref()?;
        let mut key = Vec::new();
        for (idx, value) in path.iter().enumerate() {
            if !is_wrap_row_field(&current.field) {
                key.push(format!(
                    "{}={:?}",
                    current.field,
                    Self::canonical_scalar(value)
                ));
            }
            if idx + 1 < path.len() {
                current = current.child(value)?;
            }
        }
        Some(key)
    }

    pub(crate) fn logical_depth(&self) -> usize {
        self.root
            .as_ref()
            .map(Self::count_logical_depth)
            .unwrap_or(0)
    }

    fn count_logical_depth(node: &PartitionNode) -> usize {
        let own_weight = usize::from(!is_wrap_row_field(&node.field));
        let child_depth = match &node.content {
            PartitionContent::Leaf { .. } => 0,
            PartitionContent::Branch { children } => children
                .values()
                .next()
                .map(|b| Self::count_logical_depth(b.as_ref()))
                .unwrap_or(0),
        };
        own_weight + child_depth
    }

    // ========================================================================
    // Building
    // ========================================================================

    /// Build from a compiled plot by discovering facet structure and querying data.
    ///
    /// This performs the pre-pass: walks the mark tree to find facet subplot marks,
    /// queries distinct values for each partition, and builds the tree structure.
    /// Uses a cache to avoid redundant queries for shared facet slots.
    pub async fn from_compiled_plot(
        plot: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<Self, AvengerChartError> {
        Self::from_compiled_plot_with_params(plot, ctx, &IndexMap::new()).await
    }

    pub async fn from_compiled_plot_with_params(
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError> {
        Self::from_compiled_plot_with_params_and_wrap_layout_context(
            plot,
            ctx,
            params,
            FacetWrapLayoutContext::default(),
        )
        .await
    }

    pub(crate) async fn from_compiled_plot_with_params_and_wrap_layout_context(
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        wrap_layout_context: FacetWrapLayoutContext,
    ) -> Result<Self, AvengerChartError> {
        let mut slot_cache = PartitionSlotCache::new();
        Self::from_compiled_plot_with_params_wrap_layout_context_and_slot_cache(
            plot,
            ctx,
            params,
            wrap_layout_context,
            &mut slot_cache,
        )
        .await
    }

    pub(crate) async fn from_compiled_plot_with_params_wrap_layout_context_and_slot_cache(
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        wrap_layout_context: FacetWrapLayoutContext,
        slot_cache: &mut PartitionSlotCache,
    ) -> Result<Self, AvengerChartError> {
        Self::from_compiled_plot_with_params_data_override_wrap_layout_context_and_slot_cache(
            plot,
            ctx,
            params,
            None,
            wrap_layout_context,
            slot_cache,
        )
        .await
    }

    pub(crate) async fn from_compiled_plot_with_params_data_override_wrap_layout_context_and_slot_cache(
        plot: &CompiledPlot,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        wrap_layout_context: FacetWrapLayoutContext,
        slot_cache: &mut PartitionSlotCache,
    ) -> Result<Self, AvengerChartError> {
        let df = data_override
            .cloned()
            .or_else(|| get_dataframe_from_plot(plot, ctx));

        let df = match df {
            Some(df) => df,
            None => {
                // No data available; return an empty facet tree.
                // This can happen for plots without faceting or without data
                return Ok(Self::empty());
            }
        };

        // Build partition tree by walking marks
        // Start at depth 1 (outermost facet level)
        let root = build_partition_tree(
            &plot.marks,
            &df,
            ctx,
            params,
            &[],
            &[],
            1,
            slot_cache,
            wrap_layout_context,
        )
        .await?;

        // Extract channel-domain coordination targets from the innermost marks.
        let (channel_domain_coordinations, channel_nested_band_configs) =
            extract_plot_channel_domain_metadata(plot);

        Ok(Self::init_with_caches(
            root,
            channel_domain_coordinations,
            channel_nested_band_configs,
        ))
    }

    pub(crate) fn plot_contains_facet_mark(plot: &CompiledPlot) -> bool {
        contains_facet_mark(&plot.marks)
    }

    pub(crate) fn data_root_for_plot(
        plot: &CompiledPlot,
        ctx: &SessionContext,
        data_override: Option<&DataFrame>,
    ) -> Option<DataFrame> {
        data_override
            .cloned()
            .or_else(|| get_dataframe_from_plot(plot, ctx))
    }

    fn collect_reachable_paths(&self) -> Vec<Vec<ScalarValue>> {
        let mut paths = vec![Vec::new()];
        let Some(root) = self.root.as_ref() else {
            return paths;
        };
        paths.extend(root.reachable_paths());
        paths
    }

    fn collect_node_paths(&self) -> Vec<Vec<ScalarValue>> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };
        root.node_paths()
    }

    fn collect_node_sharing_levels_recursive(
        node: &PartitionNode,
        levels: &mut HashSet<SharingLevel>,
    ) {
        levels.insert(SharingLevel::from_raw(node.sharing));
        if let PartitionContent::Branch { children } = &node.content {
            for child in children.values() {
                Self::collect_node_sharing_levels_recursive(child, levels);
            }
        }
    }

    fn collect_used_sharing_levels(&self) -> Vec<SharingLevel> {
        let mut levels: HashSet<SharingLevel> = HashSet::new();
        levels.insert(SharingLevel::FREE);
        levels.insert(SharingLevel::GLOBAL);
        for coordination in self.channel_domain_coordinations.values() {
            levels.insert(SharingLevel::from(coordination.scope));
        }
        for config in self.channel_nested_band_configs.values() {
            for level in config.levels.values() {
                if let Some(coordination) = &level.domain_coordination {
                    levels.insert(SharingLevel::from(coordination.scope));
                }
            }
        }
        if let Some(root) = self.root.as_ref() {
            Self::collect_node_sharing_levels_recursive(root, &mut levels);
        }
        let mut levels: Vec<SharingLevel> = levels.into_iter().collect();
        levels.sort_by_key(|level| level.raw());
        levels
    }

    fn rebuild_precalculated_caches(&mut self) {
        self.path_info_cache.clear();
        self.path_predicate_cache.clear();
        self.slot_membership_cache.clear();
        self.enumeration_cache.clear();
        self.jagged_axis_cache.clear();
        self.used_sharing_levels.clear();

        self.used_sharing_levels = self.collect_used_sharing_levels();

        let reachable_paths = self.collect_reachable_paths();
        for path in &reachable_paths {
            if let Some(info) = self.resolve_path_info_uncached(path) {
                self.path_info_cache
                    .entry(Self::canonical_path(path))
                    .or_insert(info);
            }
        }

        for path in &reachable_paths {
            if path.is_empty() {
                continue;
            }
            if let Some(predicate) = self.path_predicate_uncached(path) {
                self.path_predicate_cache
                    .entry(Self::canonical_path(path))
                    .or_insert(predicate);
            }
        }

        for node_path in self.collect_node_paths() {
            let Some(node) = self.node_at_path(&node_path) else {
                continue;
            };

            let domain_values: HashSet<ScalarValue> =
                node.values().map(Self::canonical_scalar).collect();
            let observed_values: HashSet<ScalarValue> =
                node.observed_values().map(Self::canonical_scalar).collect();

            self.slot_membership_cache.insert(
                Self::canonical_path(&node_path),
                SlotMembership {
                    domain_values,
                    observed_values,
                },
            );
        }

        let node_paths = self.collect_node_paths();
        for node_path in &node_paths {
            for sharing_level in &self.used_sharing_levels {
                if let Some(values) =
                    self.enumerate_values_for_facet_uncached(node_path, *sharing_level)
                {
                    self.enumeration_cache
                        .insert((Self::canonical_path(node_path), *sharing_level), values);
                }
            }
        }

        for axis_position in [
            AxisPosition::Left,
            AxisPosition::Right,
            AxisPosition::Top,
            AxisPosition::Bottom,
        ] {
            self.jagged_axis_cache.insert(
                axis_position,
                self.is_jagged_for_axis_uncached(axis_position),
            );
        }
    }

    fn path_predicate_uncached(&self, path: &[ScalarValue]) -> Option<Expr> {
        self.root.as_ref()?.path_predicate(path)
    }

    fn resolve_path_info_uncached(&self, path: &[ScalarValue]) -> Option<ResolvedFacetPathInfo> {
        let mut node = self.root.as_ref()?;

        if path.is_empty() {
            return Some(ResolvedFacetPathInfo {
                indices: Vec::new(),
                local_level_counts: Vec::new(),
                level_directions: Vec::new(),
                axis_guide_visibility: Vec::new(),
                direction: node.direction,
            });
        }

        let mut indices = Vec::with_capacity(path.len());
        let mut local_level_counts = Vec::with_capacity(path.len());
        let mut level_directions = Vec::with_capacity(path.len());
        let mut axis_guide_visibility = Vec::with_capacity(path.len());

        for (level, value) in path.iter().enumerate() {
            local_level_counts.push(node.domain_count());
            level_directions.push(node.direction);
            axis_guide_visibility.push(node.axis_guide_visibility);

            let idx = node
                .values
                .iter()
                .position(|v| scalar_values_equivalent(v, value))?;
            indices.push(idx);

            if level + 1 < path.len() {
                node = node.child(value)?;
            }
        }

        Some(ResolvedFacetPathInfo {
            indices,
            local_level_counts,
            level_directions,
            axis_guide_visibility,
            direction: node.direction,
        })
    }

    fn enumerate_values_for_facet_uncached(
        &self,
        facet_path: &[ScalarValue],
        sharing_level: SharingLevel,
    ) -> Option<Vec<ScalarValue>> {
        let current_node = if facet_path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(facet_path)?
        };

        if sharing_level.is_free() {
            return Some(current_node.values().cloned().collect());
        }

        let enumeration_path = self.sharing_owner_path(facet_path, sharing_level.raw());

        let ancestor_node = if enumeration_path.is_empty() {
            self.root.as_ref()
        } else {
            self.node_at_path(&enumeration_path)
        };

        if let Some(ancestor) = ancestor_node {
            let levels_to_descend = facet_path.len().saturating_sub(enumeration_path.len());
            Some(ancestor.values_at_depth(levels_to_descend))
        } else {
            Some(current_node.values().cloned().collect())
        }
    }

    fn path_component_logical_weights(&self, path: &[ScalarValue]) -> Option<Vec<usize>> {
        if path.is_empty() {
            return Some(Vec::new());
        }

        let mut weights = Vec::with_capacity(path.len());
        let mut current = self.root.as_ref()?;
        for (idx, value) in path.iter().enumerate() {
            let weight = if is_wrap_row_field(&current.field) {
                0
            } else {
                1
            };
            weights.push(weight);
            if idx + 1 < path.len() {
                current = current.child(value)?;
            }
        }
        Some(weights)
    }

    pub(crate) fn logical_depth_for_path(&self, path: &[ScalarValue]) -> usize {
        self.path_component_logical_weights(path)
            .map(|weights| weights.into_iter().sum())
            .unwrap_or(path.len())
    }

    pub(crate) fn logical_values_for_path(&self, path: &[ScalarValue]) -> Vec<ScalarValue> {
        let Some(weights) = self.path_component_logical_weights(path) else {
            return path.to_vec();
        };
        path.iter()
            .zip(weights)
            .filter_map(|(value, weight)| (weight > 0).then_some(value.clone()))
            .collect()
    }

    pub(crate) fn sharing_owner_path(
        &self,
        full_path: &[ScalarValue],
        sharing_level: u8,
    ) -> Vec<ScalarValue> {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        if full_path.is_empty() || sharing_level.is_global() {
            return Vec::new();
        }
        if sharing_level.is_free() {
            return full_path.to_vec();
        }

        let Some(weights) = self.path_component_logical_weights(full_path) else {
            let keep = full_path.len().saturating_sub(sharing_level.raw() as usize);
            return full_path.iter().take(keep).cloned().collect();
        };
        let total_logical_depth: usize = weights.iter().sum();
        if sharing_level.raw() as usize >= total_logical_depth {
            return Vec::new();
        }
        let target_logical_depth = total_logical_depth.saturating_sub(sharing_level.raw() as usize);
        let mut logical_depth = 0usize;
        let mut keep_physical = 0usize;
        let mut pending_zero_count = 0usize;
        for weight in weights {
            if weight == 0 {
                pending_zero_count += 1;
                continue;
            }
            if logical_depth + weight <= target_logical_depth {
                keep_physical += pending_zero_count + 1;
                pending_zero_count = 0;
                logical_depth += weight;
            } else {
                break;
            }
        }
        full_path.iter().take(keep_physical).cloned().collect()
    }

    pub(crate) fn partition_exprs_between_sharing_levels(
        &self,
        full_path: &[ScalarValue],
        from_level: SharingLevel,
        to_level: SharingLevel,
    ) -> Vec<Expr> {
        if full_path.is_empty() {
            return Vec::new();
        }

        let from_path = self.sharing_owner_path(full_path, from_level.raw());
        let to_path = self.sharing_owner_path(full_path, to_level.raw());
        if to_path.len() <= from_path.len() || !to_path.starts_with(&from_path) {
            return Vec::new();
        }

        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };

        let mut exprs = Vec::new();
        let mut current_node = root;
        for (level_idx, value) in to_path.iter().enumerate() {
            if level_idx >= from_path.len()
                && let Some(field_expr) = current_node.field_expr.clone()
            {
                exprs.push(field_expr);
            }

            if level_idx + 1 < to_path.len() {
                let Some(child) = current_node.child(value) else {
                    return Vec::new();
                };
                current_node = child;
            }
        }

        exprs
    }

    fn is_jagged_for_axis_uncached(&self, axis_position: AxisPosition) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        let (varying_direction, branching_direction) = match axis_position {
            AxisPosition::Left | AxisPosition::Right => {
                (FacetDirection::Row, FacetDirection::Column)
            }
            AxisPosition::Top | AxisPosition::Bottom => {
                (FacetDirection::Column, FacetDirection::Row)
            }
        };
        Self::check_jagged_in_tree(root, branching_direction, varying_direction)
    }

    // ========================================================================
    // Query methods
    // ========================================================================

    /// Build filter predicate for a path through the facet tree.
    ///
    /// This is the core predicate-building method. It walks the tree and builds
    /// an AND expression for each level in the path.
    ///
    /// # Arguments
    /// * `path` - Sequence of values identifying position in the tree, from outermost
    ///   to the desired depth. Can be a full cell path or a partial ancestor path.
    ///
    /// # Returns
    /// - `Some(Expr)` with filter like `field1 = value1 AND field2 = value2 AND ...`
    /// - `None` if path is empty or invalid
    ///
    /// # Example
    /// For path `["Eng", "Backend"]` in a Division > Dept > Team hierarchy:
    /// Returns: `division = "Eng" AND department = "Backend"`
    pub fn path_predicate(&self, path: &[ScalarValue]) -> Option<Expr> {
        let canonical_path = Self::canonical_path(path);
        if let Some(predicate) = self.path_predicate_cache.get(&canonical_path) {
            return Some(predicate.clone());
        }
        self.path_predicate_uncached(path)
    }

    /// Get filter predicate for a cell at the given path, respecting sharing level.
    ///
    /// Delegates to `path_predicate` after truncating the path based on sharing level.
    ///
    /// # Arguments
    /// * `path` - Full cell path from outermost to innermost level.
    /// * `sharing_level` - How many levels to exclude from the filter:
    ///   - 0 (Free): include all levels
    ///   - N (Level(N)): exclude last N levels
    ///   - 255 (Shared): return None (use full data)
    ///
    /// # Returns
    /// - `Some(Expr)` with filter predicate for the truncated path
    /// - `None` if path is invalid, empty after truncation, or sharing_level is 255
    ///
    /// # Example
    /// For path ["East", "Eng", "A"] (Region > Department > Team):
    /// - sharing_level=0: `region="East" AND dept="Eng" AND team="A"`
    /// - sharing_level=1: `region="East" AND dept="Eng"` (skip last 1)
    /// - sharing_level=2: `region="East"` (skip last 2)
    /// - sharing_level=3+: `None` (use full data)
    pub fn cell_predicate(&self, path: &[ScalarValue], sharing_level: u8) -> Option<Expr> {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        // Shared (255) means use full data - no filter needed
        if sharing_level.is_global() {
            return None;
        }

        let owner_path = self.sharing_owner_path(path, sharing_level.raw());
        if owner_path.is_empty() {
            return None;
        }

        self.path_predicate(&owner_path)
    }

    /// Navigate to a partition node at a given path.
    ///
    /// # Arguments
    /// * `path` - Sequence of values identifying the position, from outermost to target level.
    ///
    /// # Returns
    /// The partition node at the specified path, or None if the path is invalid.
    pub fn node_at_path(&self, path: &[ScalarValue]) -> Option<&PartitionNode> {
        self.root.as_ref()?.node_at_path(path)
    }

    /// Check whether a full cell path exists in the evaluated facet tree.
    ///
    /// This treats `path` as a full value path (including the leaf cell value),
    /// and checks that the parent node contains the final value.
    pub fn cell_exists(&self, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return self.root().is_some();
        }

        let parent_path = Self::canonical_path(&path[..path.len() - 1]);
        let target_value = Self::canonical_scalar(&path[path.len() - 1]);

        if let Some(membership) = self.slot_membership_cache.get(&parent_path) {
            return membership.domain_values.contains(&target_value);
        }

        self.root
            .as_ref()
            .is_some_and(|root| root.cell_exists(path))
    }

    /// Check whether a full cell path has any observed rows after full facet filtering.
    pub fn cell_has_data(&self, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return self.root().is_some();
        }

        let parent_path = Self::canonical_path(&path[..path.len() - 1]);
        let target_value = Self::canonical_scalar(&path[path.len() - 1]);

        if let Some(membership) = self.slot_membership_cache.get(&parent_path) {
            return membership.observed_values.contains(&target_value);
        }

        self.root
            .as_ref()
            .is_some_and(|root| root.cell_has_data(path))
    }

    /// Enumerate facet cell values for a facet at `facet_path` using Level(N) sharing semantics.
    ///
    /// `facet_path` is the path to the parent groups of the current facet level
    /// (its length is `facet_depth - 1`).
    ///
    /// Returns `None` when `facet_path` does not exist in the tree.
    pub fn enumerate_values_for_facet(
        &self,
        facet_path: &[ScalarValue],
        sharing_level: u8,
    ) -> Option<Vec<ScalarValue>> {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        let canonical_path = Self::canonical_path(facet_path);
        if let Some(values) = self
            .enumeration_cache
            .get(&(canonical_path.clone(), sharing_level))
        {
            return Some(values.clone());
        }

        self.enumerate_values_for_facet_uncached(facet_path, sharing_level)
    }

    pub(crate) fn min_slot_count_for_facet(&self, facet_path: &[ScalarValue]) -> Option<usize> {
        let node = if facet_path.is_empty() {
            self.root.as_ref()
        } else {
            self.node_at_path(facet_path)
        }?;
        node.min_slot_count()
    }

    pub(crate) fn ragged_orthogonal_slot_count_for_facet(
        &self,
        facet_path: &[ScalarValue],
    ) -> Option<usize> {
        if facet_path.is_empty() {
            return None;
        }

        let current = self.node_at_path(facet_path)?;
        let parent_path = &facet_path[..facet_path.len() - 1];
        let parent = if parent_path.is_empty() {
            self.root.as_ref()
        } else {
            self.node_at_path(parent_path)
        }?;

        if current.direction == parent.direction {
            return None;
        }

        let PartitionContent::Branch { children } = &parent.content else {
            return None;
        };
        if children.len() < 2 {
            return None;
        }

        let max_sibling_slots = children
            .values()
            .filter(|child| child.direction == current.direction)
            .map(|child| child.domain_count())
            .max()?;

        (max_sibling_slots > current.domain_count()).then_some(max_sibling_slots)
    }

    /// Get the sharing level for a channel.
    ///
    /// Returns the sharing level stored during tree construction, or 255 (Shared)
    /// if the channel was not found. This is used for axis visibility decisions
    /// when the innermost subplot doesn't have access to CoordMeasurement.
    pub fn channel_domain_sharing_level(&self, channel: &str) -> u8 {
        self.channel_domain_sharing_level_typed(channel).raw()
    }

    pub(crate) fn channel_domain_sharing_level_typed(&self, channel: &str) -> SharingLevel {
        SharingLevel::from(self.channel_domain_coordination(channel).scope)
    }

    pub(crate) fn channel_domain_coordination(&self, channel: &str) -> DomainCoordination {
        self.channel_domain_coordinations
            .get(channel)
            .cloned()
            .unwrap_or_else(|| DomainCoordination::scale_name(SharingLevel::GLOBAL.into()))
    }

    pub(crate) fn channel_nested_band_configs(&self) -> &HashMap<String, NestedBandSpec> {
        &self.channel_nested_band_configs
    }

    pub(crate) fn has_free_channel_domain_sharing(&self) -> bool {
        self.channel_domain_coordinations
            .values()
            .any(|coordination| SharingLevel::from(coordination.scope).is_free())
    }

    pub(crate) fn has_wrap_levels(&self) -> bool {
        self.root
            .as_ref()
            .is_some_and(partition_node_has_wrap_level)
    }

    pub(crate) fn domain_extent_channels(&self) -> Vec<String> {
        let mut channels = ["x", "y", "x2", "y2"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        for channel in self.channel_domain_coordinations.keys() {
            if !channels.iter().any(|existing| existing == channel) {
                channels.push(channel.clone());
            }
        }
        channels
    }

    /// Check if the tree is actually jagged for a given axis position.
    ///
    /// Returns true when sibling branches in the branching direction have
    /// different child counts at a varying-direction level. This means some
    /// subplots cannot be covered by edge-only label display.
    ///
    /// For y-axis (Left/Right): branching=Column, varying=Row — jagged when
    /// different columns have different numbers of rows.
    /// For x-axis (Top/Bottom): branching=Row, varying=Column — jagged when
    /// different rows have different numbers of columns.
    pub fn is_jagged_for_axis(&self, axis_position: AxisPosition) -> bool {
        self.jagged_axis_cache
            .get(&axis_position)
            .copied()
            .unwrap_or_else(|| self.is_jagged_for_axis_uncached(axis_position))
    }

    /// Recursively check if any branching-direction node has children whose
    /// varying-direction descendant counts differ.
    fn check_jagged_in_tree(
        node: &PartitionNode,
        branching_direction: FacetDirection,
        varying_direction: FacetDirection,
    ) -> bool {
        if let PartitionContent::Branch { children } = &node.content {
            if node.direction == branching_direction {
                // This node branches in the direction we care about.
                // Check if children's varying-direction counts differ.
                let counts: Vec<usize> = children
                    .values()
                    .map(|child| Self::count_at_direction(child, varying_direction))
                    .collect();
                if counts.windows(2).any(|w| w[0] != w[1]) {
                    return true;
                }
            }
            // Continue checking deeper levels
            children.values().any(|child| {
                Self::check_jagged_in_tree(child, branching_direction, varying_direction)
            })
        } else {
            false
        }
    }

    /// Count the slot set size at the first level matching the target direction.
    fn count_at_direction(node: &PartitionNode, target_direction: FacetDirection) -> usize {
        if node.direction == target_direction {
            return node.domain_count();
        }
        // Recurse into first child to find the target direction level
        if let PartitionContent::Branch { children } = &node.content
            && let Some(child) = children.values().next()
        {
            return Self::count_at_direction(child, target_direction);
        }
        0
    }

    /// Get level counts (slot count at each nesting level).
    ///
    /// Returns vec where index is nesting level and value is slot count.
    ///
    /// Semantics: counts are derived by following the first branch at each level.
    /// This is intentional for asymmetric trees and matches existing behavior.
    pub fn level_counts(&self) -> Vec<usize> {
        self.level_counts_cache.clone()
    }

    fn level_counts_ref(&self) -> &[usize] {
        &self.level_counts_cache
    }

    fn collect_level_counts_first_branch(
        node: &PartitionNode,
        counts: &mut Vec<usize>,
        level: usize,
    ) {
        // Ensure vector is large enough
        if counts.len() <= level {
            counts.resize(level + 1, 0);
        }
        // Record count at this level
        counts[level] = node.domain_count();

        // Recurse to children (use first child to get next level structure).
        // This preserves existing first-branch semantics for potentially
        // asymmetric trees without panicking in debug builds.
        if let PartitionContent::Branch { ref children } = node.content
            && let Some(first_child) = children.values().next()
        {
            Self::collect_level_counts_first_branch(first_child.as_ref(), counts, level + 1);
        }
    }

    fn collect_level_counts_first_branch_for_root(root: &PartitionNode) -> Vec<usize> {
        let mut counts = Vec::new();
        Self::collect_level_counts_first_branch(root, &mut counts, 0);
        counts
    }

    fn axis_visibility_from_resolved(
        &self,
        position_indices: &[usize],
        level_counts: &[usize],
        axis_position: AxisPosition,
        sharing_level: SharingLevel,
        direction: FacetDirection,
    ) -> AxisVisibility {
        if level_counts.is_empty() {
            return AxisVisibility::visible();
        }

        let facet_depth = position_indices.len() as u8;
        AxisVisibility {
            show_labels: sharing_policy::show_axis_labels(
                position_indices,
                level_counts,
                facet_depth,
                sharing_level,
                direction,
                axis_position,
            ),
            show_title: sharing_policy::show_axis_title(
                position_indices,
                level_counts,
                facet_depth,
                direction,
                axis_position,
            ),
        }
    }

    fn channel_axis_visibility_from_resolved(
        &self,
        position_indices: &[usize],
        level_counts: &[usize],
        level_directions: &[FacetDirection],
        axis_position: AxisPosition,
        sharing_level: SharingLevel,
    ) -> AxisVisibility {
        if level_counts.is_empty() {
            return AxisVisibility::visible();
        }

        let title_level_counts =
            self.cartesian_title_level_counts(position_indices.len(), level_counts);

        AxisVisibility {
            show_labels: sharing_policy::show_cartesian_axis_labels(
                position_indices,
                level_counts,
                level_directions,
                axis_position,
                sharing_level,
            ),
            show_title: sharing_policy::show_cartesian_axis_title(
                position_indices,
                &title_level_counts,
                level_directions,
                axis_position,
            ),
        }
    }

    fn cartesian_title_level_counts(
        &self,
        depth: usize,
        local_level_counts: &[usize],
    ) -> Vec<usize> {
        let mut title_level_counts = local_level_counts.to_vec();
        if title_level_counts.len() < depth {
            title_level_counts.resize(depth, 0);
        } else if title_level_counts.len() > depth {
            title_level_counts.truncate(depth);
        }

        title_level_counts
    }

    fn cartesian_relevant_direction_for_axis(axis_position: AxisPosition) -> FacetDirection {
        match axis_position {
            AxisPosition::Top | AxisPosition::Bottom => FacetDirection::Row,
            AxisPosition::Left | AxisPosition::Right => FacetDirection::Column,
        }
    }

    fn axis_guide_visibility_config_for_cartesian_axis(
        resolved: &ResolvedFacetPathInfo,
        axis_position: AxisPosition,
    ) -> AxisGuideVisibilityConfig {
        let relevant_direction = Self::cartesian_relevant_direction_for_axis(axis_position);
        let mut config = AxisGuideVisibilityConfig::auto();

        for (direction, level_config) in resolved
            .level_directions
            .iter()
            .zip(resolved.axis_guide_visibility.iter())
        {
            if *direction != relevant_direction {
                continue;
            }
            if level_config.labels != AxisGuideVisibilityPolicy::Auto {
                config.labels = level_config.labels;
            }
            if level_config.title != AxisGuideVisibilityPolicy::Auto {
                config.title = level_config.title;
            }
        }

        config
    }

    fn resolve_axis_policy_visibility(
        &self,
        path: &[ScalarValue],
        resolved: &ResolvedFacetPathInfo,
        axis_position: AxisPosition,
        policy: AxisGuideVisibilityPolicy,
        auto_visible: bool,
    ) -> bool {
        match policy {
            AxisGuideVisibilityPolicy::Auto => auto_visible,
            AxisGuideVisibilityPolicy::All => true,
            AxisGuideVisibilityPolicy::OuterEdges => {
                let relevant_depth = resolved
                    .level_directions
                    .iter()
                    .filter(|&&direction| {
                        direction == Self::cartesian_relevant_direction_for_axis(axis_position)
                    })
                    .count();
                if relevant_depth == 0 || resolved.indices.len() != resolved.level_directions.len()
                {
                    return true;
                }

                self.non_empty_owner_visible_for_cartesian_axis(
                    path,
                    resolved,
                    axis_position,
                    SharingLevel::from_raw(relevant_depth as u8),
                    auto_visible,
                )
            }
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups => auto_visible,
        }
    }

    pub(crate) fn axis_guide_visibility_config_for_path(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
    ) -> Option<AxisGuideVisibilityConfig> {
        let resolved = self.resolve_path_info(path)?;
        Some(Self::axis_guide_visibility_config_for_cartesian_axis(
            &resolved,
            axis_position,
        ))
    }

    fn collect_non_empty_paths_for_axis_strip(
        &self,
        path: &[ScalarValue],
        level_directions: &[FacetDirection],
        axis_position: AxisPosition,
    ) -> Vec<Vec<ScalarValue>> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };

        if path.len() != level_directions.len() {
            return Vec::new();
        }

        let relevant_direction = Self::cartesian_relevant_direction_for_axis(axis_position);
        let mut candidates = Vec::new();
        let mut prefix = Vec::with_capacity(path.len());
        Self::collect_non_empty_paths_for_axis_strip_recursive(
            root,
            path,
            level_directions,
            relevant_direction,
            0,
            &mut prefix,
            &mut candidates,
        );
        candidates
    }

    fn collect_non_empty_paths_for_axis_strip_recursive(
        node: &PartitionNode,
        path: &[ScalarValue],
        level_directions: &[FacetDirection],
        relevant_direction: FacetDirection,
        depth: usize,
        prefix: &mut Vec<ScalarValue>,
        out: &mut Vec<Vec<ScalarValue>>,
    ) {
        if depth >= path.len() || depth >= level_directions.len() {
            return;
        }

        let direction = level_directions[depth];
        if direction != relevant_direction {
            let value = &path[depth];
            let is_observed = node
                .observed_values()
                .any(|observed| scalar_values_equivalent(observed, value));
            if !is_observed {
                return;
            }

            prefix.push(value.clone());
            if depth + 1 == path.len() {
                out.push(prefix.clone());
                prefix.pop();
                return;
            }

            if let Some(child) = node.child(value) {
                Self::collect_non_empty_paths_for_axis_strip_recursive(
                    child,
                    path,
                    level_directions,
                    relevant_direction,
                    depth + 1,
                    prefix,
                    out,
                );
            }
            prefix.pop();
            return;
        }

        let observed_values: Vec<ScalarValue> = node.observed_values().cloned().collect();
        for observed_value in &observed_values {
            prefix.push(observed_value.clone());
            if depth + 1 == path.len() {
                out.push(prefix.clone());
                prefix.pop();
                continue;
            }

            if let Some(child) = node.child(observed_value) {
                Self::collect_non_empty_paths_for_axis_strip_recursive(
                    child,
                    path,
                    level_directions,
                    relevant_direction,
                    depth + 1,
                    prefix,
                    out,
                );
            }
            prefix.pop();
        }
    }

    fn non_empty_owner_visible_for_cartesian_axis(
        &self,
        path: &[ScalarValue],
        resolved: &ResolvedFacetPathInfo,
        axis_position: AxisPosition,
        sharing_level: SharingLevel,
        fallback_visible: bool,
    ) -> bool {
        let Some(scope) = sharing_policy::cartesian_axis_ownership_scope_for_sharing(
            sharing_policy::GuideOwnershipRole::CartesianAxisLabels,
            &resolved.indices,
            &resolved.local_level_counts,
            &resolved.level_directions,
            axis_position,
            sharing_level,
        ) else {
            return fallback_visible;
        };

        let current_projected = &scope.position_indices;
        let boundary = scope.boundary;
        let edge = scope.edge;
        let current_prefix = &current_projected[..boundary];
        let current_suffix = &current_projected[boundary..];

        let candidate_paths = self.collect_non_empty_paths_for_axis_strip(
            path,
            &resolved.level_directions,
            axis_position,
        );
        if candidate_paths.is_empty() {
            return fallback_visible;
        }

        let mut best_suffix: Option<Vec<usize>> = None;
        for candidate_path in &candidate_paths {
            let Some(candidate_resolved) = self.resolve_path_info(candidate_path) else {
                continue;
            };
            let Some(candidate_scope) = sharing_policy::cartesian_axis_ownership_scope_for_sharing(
                sharing_policy::GuideOwnershipRole::CartesianAxisLabels,
                &candidate_resolved.indices,
                &candidate_resolved.local_level_counts,
                &candidate_resolved.level_directions,
                axis_position,
                sharing_level,
            ) else {
                continue;
            };
            let candidate_projected = candidate_scope.position_indices;

            if candidate_projected.len() != current_projected.len() {
                continue;
            }
            if &candidate_projected[..boundary] != current_prefix {
                continue;
            }

            let candidate_suffix = &candidate_projected[boundary..];
            match &mut best_suffix {
                None => best_suffix = Some(candidate_suffix.to_vec()),
                Some(best) => {
                    let replace = match edge {
                        SharingGroupEdge::Start => candidate_suffix < best.as_slice(),
                        SharingGroupEdge::End => candidate_suffix > best.as_slice(),
                    };
                    if replace {
                        *best = candidate_suffix.to_vec();
                    }
                }
            }
        }

        best_suffix
            .map(|best| current_suffix == best.as_slice())
            .unwrap_or(fallback_visible)
    }

    fn channel_axis_visibility_from_resolved_with_mode(
        &self,
        path: &[ScalarValue],
        resolved: &ResolvedFacetPathInfo,
        axis_position: AxisPosition,
        sharing_level: SharingLevel,
        ownership_mode: AxisOwnershipMode,
    ) -> AxisVisibility {
        if resolved.local_level_counts.is_empty() {
            return AxisVisibility::visible();
        }

        let owner_path = self.sharing_owner_path(path, sharing_level.raw());
        let physical_sharing_level =
            SharingLevel::from_raw(path.len().saturating_sub(owner_path.len()) as u8);

        if matches!(ownership_mode, AxisOwnershipMode::DomainSlots) {
            let auto = self.channel_axis_visibility_from_resolved(
                &resolved.indices,
                &resolved.local_level_counts,
                &resolved.level_directions,
                axis_position,
                physical_sharing_level,
            );
            let policy =
                Self::axis_guide_visibility_config_for_cartesian_axis(resolved, axis_position);
            return AxisVisibility {
                show_labels: self.resolve_axis_policy_visibility(
                    path,
                    resolved,
                    axis_position,
                    policy.labels,
                    auto.show_labels,
                ),
                show_title: self.resolve_axis_policy_visibility(
                    path,
                    resolved,
                    axis_position,
                    policy.title,
                    auto.show_title,
                ),
            };
        }

        let relevant_depth = resolved
            .level_directions
            .iter()
            .filter(|&&direction| {
                direction == Self::cartesian_relevant_direction_for_axis(axis_position)
            })
            .count();
        if relevant_depth == 0 || resolved.indices.len() != resolved.level_directions.len() {
            return AxisVisibility::visible();
        }

        let labels_sharing = physical_sharing_level.clamp_to_depth(relevant_depth as u8);
        let labels_fallback = sharing_policy::show_cartesian_axis_labels(
            &resolved.indices,
            &resolved.local_level_counts,
            &resolved.level_directions,
            axis_position,
            labels_sharing,
        );
        let title_level_counts =
            self.cartesian_title_level_counts(resolved.indices.len(), &resolved.local_level_counts);
        let title_fallback = sharing_policy::show_cartesian_axis_title(
            &resolved.indices,
            &title_level_counts,
            &resolved.level_directions,
            axis_position,
        );
        let title_sharing = SharingLevel::from_raw(relevant_depth as u8);

        let auto_labels = self.non_empty_owner_visible_for_cartesian_axis(
            path,
            resolved,
            axis_position,
            labels_sharing,
            labels_fallback,
        );
        let auto_title = self.non_empty_owner_visible_for_cartesian_axis(
            path,
            resolved,
            axis_position,
            title_sharing,
            title_fallback,
        );
        let policy = Self::axis_guide_visibility_config_for_cartesian_axis(resolved, axis_position);

        AxisVisibility {
            show_labels: self.resolve_axis_policy_visibility(
                path,
                resolved,
                axis_position,
                policy.labels,
                auto_labels,
            ),
            // In hole mode, title ownership follows the same non-empty edge
            // owner as labels. Otherwise a ragged row/column can lose its axis
            // title entirely when its geometric edge owner is a hole.
            show_title: self.resolve_axis_policy_visibility(
                path,
                resolved,
                axis_position,
                policy.title,
                auto_title,
            ),
        }
    }

    /// Resolve branch-local path metadata for a concrete cell path.
    ///
    /// Unlike `level_counts()`, this returns counts from the same branch as the
    /// provided path, which is required for ragged-tree owner checks.
    pub fn resolve_path_info(&self, path: &[ScalarValue]) -> Option<ResolvedFacetPathInfo> {
        let canonical_path = Self::canonical_path(path);
        if let Some(info) = self.path_info_cache.get(&canonical_path) {
            return Some(info.clone());
        }
        self.resolve_path_info_uncached(path)
    }

    /// Convert a path of facet slot values to position indices.
    ///
    /// This is the inverse of `path_values_from_indices`. Given a path like
    /// `["East", "Eng"]`, returns the indices `[0, 1]` if "East" is at index 0
    /// and "Eng" is at index 1 in their respective levels.
    ///
    /// Returns `None` if any value in the path is not found at its level.
    pub fn indices_from_path(&self, path: &[ScalarValue]) -> Option<Vec<usize>> {
        self.resolve_path_info(path)
            .map(|resolved| resolved.indices)
    }

    /// Determine axis visibility for a cell at given path in the facet grid.
    ///
    /// This is a convenience wrapper around `axis_visibility` that converts
    /// a value path to indices first. Use this when you have the cell's value
    /// path (e.g., `["East", "Eng"]`) rather than indices.
    ///
    /// # Arguments
    /// * `path` - Sequence of values identifying the cell (e.g., `["East", "Eng"]`)
    /// * `axis_position` - Which edge the axis is on (Top/Bottom/Left/Right)
    /// * `sharing_level` - Channel's sharing level: 0=Free, N=Level(N), 255=Shared
    ///
    /// Returns:
    /// - `Some(AxisVisibility)` for valid (or empty) paths
    /// - `None` when the path is invalid
    pub fn axis_visibility_for_path_checked(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> Option<AxisVisibility> {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        if path.is_empty() {
            return Some(AxisVisibility::visible());
        }

        let resolved = self.resolve_path_info(path)?;

        Some(self.axis_visibility_from_resolved(
            &resolved.indices,
            &resolved.local_level_counts,
            axis_position,
            sharing_level,
            resolved.direction,
        ))
    }

    /// Compatibility wrapper that defaults invalid paths to visible.
    pub fn axis_visibility_for_path(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> AxisVisibility {
        self.axis_visibility_for_path_checked(path, axis_position, sharing_level)
            .unwrap_or_else(AxisVisibility::visible)
    }

    /// Determine cartesian axis visibility for a concrete facet cell path.
    ///
    /// This applies axis ownership in a way that composes mixed row/column nesting:
    /// - x axes are controlled by row-facet levels
    /// - y axes are controlled by column-facet levels
    ///
    /// while preserving orthogonal strip grouping and branch-local ragged counts.
    pub fn channel_axis_visibility_for_path_checked(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> Option<AxisVisibility> {
        self.channel_axis_visibility_for_path_checked_with_mode(
            path,
            axis_position,
            sharing_level,
            AxisOwnershipMode::DomainSlots,
        )
    }

    pub(crate) fn channel_axis_visibility_for_path_checked_with_mode(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
        ownership_mode: AxisOwnershipMode,
    ) -> Option<AxisVisibility> {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        if path.is_empty() {
            return Some(AxisVisibility::visible());
        }

        let resolved = self.resolve_path_info(path)?;

        Some(self.channel_axis_visibility_from_resolved_with_mode(
            path,
            &resolved,
            axis_position,
            sharing_level,
            ownership_mode,
        ))
    }

    /// Compatibility wrapper that defaults invalid paths to visible.
    pub fn channel_axis_visibility_for_path(
        &self,
        path: &[ScalarValue],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> AxisVisibility {
        self.channel_axis_visibility_for_path_checked(path, axis_position, sharing_level)
            .unwrap_or_else(AxisVisibility::visible)
    }

    /// Determine axis visibility for a cell at given position in the facet grid.
    ///
    /// This implements sharing-level-aware visibility: axes show labels/titles only on cells
    /// that are first (or last) within their sharing group, based on axis position and facet direction.
    ///
    /// # Arguments
    /// * `position_indices` - Cell position indices at each nesting level (e.g., `[2]` for 3rd column,
    ///   `[1, 0]` for nested facets)
    /// * `axis_position` - Which edge the axis is on (Top/Bottom/Left/Right)
    /// * `sharing_level` - Channel's sharing level: 0=Free, N=Level(N), 255=Shared
    ///
    /// # Returns
    /// `AxisVisibility` indicating whether labels and title should be shown.
    ///
    /// # Visibility Rules
    ///
    /// For **Column facets** (horizontal layout):
    /// - Y axis (Left): show only on first cell within sharing group
    /// - Y axis (Right): show only on last cell within sharing group
    /// - X axis: always show (not affected by column layout)
    ///
    /// For **Row facets** (vertical layout):
    /// - X axis (Bottom): show only on last cell within sharing group
    /// - X axis (Top): show only on first cell within sharing group
    /// - Y axis: always show (not affected by row layout)
    ///
    /// # CoordinationScope Groups
    ///
    /// With `Level(N)` sharing and `facet_depth = D`:
    /// - Group is defined by the first `(D - N)` levels (the "prefix")
    /// - Cell is "first in group" if suffix `position_indices[D-N..]` are all 0
    /// - Cell is "last in group" if suffix are all at max
    pub fn axis_visibility(
        &self,
        position_indices: &[usize],
        axis_position: AxisPosition,
        sharing_level: u8,
    ) -> AxisVisibility {
        let sharing_level = SharingLevel::from_raw(sharing_level);
        // If no facets, always show
        let Some(root) = &self.root else {
            return AxisVisibility::visible();
        };

        debug!(
            position = ?position_indices,
            axis = ?axis_position,
            sharing_level = sharing_level.raw(),
            counts = ?self.level_counts_ref(),
            "axis_visibility"
        );

        let mut node = root;
        for (level, &pos_idx) in position_indices.iter().enumerate() {
            if pos_idx >= node.domain_count() {
                return AxisVisibility::visible();
            }

            if level + 1 == position_indices.len() {
                break;
            }

            node = match &node.content {
                PartitionContent::Branch { .. } => {
                    let Some(value) = node.values.get(pos_idx) else {
                        return AxisVisibility::visible();
                    };
                    let Some(child) = node.child(value) else {
                        return AxisVisibility::visible();
                    };
                    child
                }
                PartitionContent::Leaf { .. } => return AxisVisibility::visible(),
            };
        }

        self.axis_visibility_from_resolved(
            position_indices,
            self.level_counts_ref(),
            axis_position,
            sharing_level,
            node.direction,
        )
    }
}

#[cfg(test)]
impl EvaluatedFacetTree {
    fn path_values_from_indices(&self, indices: &[usize]) -> Vec<ScalarValue> {
        let mut values = Vec::with_capacity(indices.len());
        let mut current_node = self.root.as_ref();

        for &idx in indices {
            match current_node {
                Some(node) => match &node.content {
                    PartitionContent::Leaf {
                        values: domain_values,
                    } => {
                        if let Some(val) = domain_values.get(idx) {
                            values.push(val.clone());
                        }
                        break;
                    }
                    PartitionContent::Branch { .. } => {
                        if let Some(value) = node.values.get(idx) {
                            values.push(value.clone());
                            current_node = node.child(value);
                        } else {
                            break;
                        }
                    }
                },
                None => break,
            }
        }
        values
    }
}

// ============================================================================
// Helper functions for building the partition tree
// ============================================================================

/// Extract channel-domain coordination targets from compiled marks by recursing through facet subplots.
///
/// This walks the mark tree to find the innermost (non-facet) marks and extracts
/// scale-domain metadata.
fn extract_plot_channel_domain_metadata(
    plot: &CompiledPlot,
) -> (
    HashMap<String, DomainCoordination>,
    HashMap<String, NestedBandSpec>,
) {
    extract_channel_domain_metadata_inner(&plot.marks)
}

#[cfg(test)]
fn extract_channel_domain_metadata(
    marks: &[Arc<dyn CompiledMark>],
) -> (
    HashMap<String, DomainCoordination>,
    HashMap<String, NestedBandSpec>,
) {
    extract_channel_domain_metadata_inner(marks)
}

fn extract_channel_domain_metadata_inner(
    marks: &[Arc<dyn CompiledMark>],
) -> (
    HashMap<String, DomainCoordination>,
    HashMap<String, NestedBandSpec>,
) {
    let mut coordinations = HashMap::new();
    let mut nested_band_configs = HashMap::new();
    let mut implicit_scaled_channels = HashSet::new();
    let mut nested_channels_with_level_coordination = HashSet::new();

    for mark in marks {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            // Recurse into subplot to find innermost marks
            let (inner_coordinations, inner_nested_band_configs) =
                extract_plot_channel_domain_metadata(facet_mark.compiled_subplot());
            for (scale_name, coordination) in inner_coordinations {
                coordinations
                    .entry(scale_name)
                    .and_modify(|existing: &mut DomainCoordination| {
                        if coordination.scope.to_level() > existing.scope.to_level() {
                            *existing = coordination.clone();
                        }
                    })
                    .or_insert(coordination);
            }
            for (scale_name, config) in inner_nested_band_configs {
                if nested_config_has_level_coordination(&config) {
                    nested_channels_with_level_coordination.insert(scale_name.clone());
                }
                nested_band_configs.entry(scale_name).or_insert(config);
            }
        } else {
            // Non-facet mark - extract channel-domain coordination targets.
            merge_channel_domain_metadata_from_channels(
                mark.data_context().channels(),
                &mut coordinations,
                &mut nested_band_configs,
                &mut implicit_scaled_channels,
                &mut nested_channels_with_level_coordination,
            );
        }
    }

    for scale_name in &nested_channels_with_level_coordination {
        implicit_scaled_channels.remove(scale_name);
        coordinations
            .entry(scale_name.clone())
            .or_insert_with(|| DomainCoordination::scale_name(SharingLevel::FREE.into()));
    }

    for scale_name in implicit_scaled_channels {
        coordinations
            .entry(scale_name)
            .or_insert_with(|| DomainCoordination::scale_name(SharingLevel::GLOBAL.into()));
    }

    (coordinations, nested_band_configs)
}

fn merge_channel_domain_metadata_from_channels(
    channels: &IndexMap<String, ChannelValue>,
    coordinations: &mut HashMap<String, DomainCoordination>,
    nested_band_configs: &mut HashMap<String, NestedBandSpec>,
    implicit_scaled_channels: &mut HashSet<String>,
    nested_channels_with_level_coordination: &mut HashSet<String>,
) {
    for (channel, channel_value) in channels {
        let Some(scale_name) = channel_value.get_scale_name(channel) else {
            continue;
        };
        if let Some(config) = channel_value.get_nested_band_config().cloned() {
            if nested_config_has_level_coordination(&config) {
                nested_channels_with_level_coordination.insert(scale_name.clone());
            }
            nested_band_configs
                .entry(scale_name.clone())
                .or_insert(config);
        }
        if let Some(coordination) = channel_value.get_domain_coordination().cloned() {
            coordinations
                .entry(scale_name)
                .and_modify(|existing: &mut DomainCoordination| {
                    if coordination.scope.to_level() > existing.scope.to_level() {
                        *existing = coordination.clone();
                    }
                })
                .or_insert(coordination);
        } else {
            implicit_scaled_channels.insert(scale_name);
        }
    }
}

fn nested_config_has_level_coordination(config: &NestedBandSpec) -> bool {
    config
        .levels
        .values()
        .any(|level| level.domain_coordination.is_some())
}

#[cfg(test)]
fn extract_channel_domain_coordinations(
    marks: &[Arc<dyn CompiledMark>],
) -> HashMap<String, DomainCoordination> {
    extract_channel_domain_metadata(marks).0
}

/// Get a DataFrame from plot-level data or first mark with data.
fn get_dataframe_from_plot(plot: &CompiledPlot, ctx: &SessionContext) -> Option<DataFrame> {
    // Try marks first (they may have explicit data)
    for mark in &plot.marks {
        if let Some(df) = mark.data_context().dataframe_with_context(ctx) {
            // Skip empty relation placeholders
            if !is_empty_relation(&df) {
                return Some(df);
            }
        }
    }

    // Fall back to plot-level data
    if let Some(data_node) = &plot.data
        && let Ok(logical_plan) = data_node.to_logical_plan(ctx)
    {
        return Some(DataFrame::new(ctx.state().clone(), logical_plan));
    }

    None
}

/// Check if a DataFrame is an empty relation placeholder.
fn is_empty_relation(df: &DataFrame) -> bool {
    matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_))
}

/// Resolved metadata for building a partition node from a facet subplot mark.
struct FacetPartitionMarkSpec<'a> {
    kind: FacetPartitionKind,
    dimension: PartitionDimensionSpec,
    subplot: &'a CompiledPlot,
    column_mode: FacetWrapColumnModeExpr,
}

#[derive(Clone, Debug)]
enum FacetWrapColumnModeExpr {
    Auto,
    Fixed(Expr),
    ResponsiveWidth(Expr),
}

impl FacetWrapColumnModeExpr {
    fn from_mode(
        mode: FacetWrapColumnMode,
        ctx: &SessionContext,
    ) -> Result<Self, AvengerChartError> {
        match mode {
            FacetWrapColumnMode::Auto => Ok(Self::Auto),
            FacetWrapColumnMode::Fixed(expr) => Ok(Self::Fixed(expr.to_expr(ctx)?)),
            FacetWrapColumnMode::ResponsiveWidth(expr) => {
                Ok(Self::ResponsiveWidth(expr.to_expr(ctx)?))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FacetPartitionKind {
    Band,
    Wrap,
}

impl<'a> FacetPartitionMarkSpec<'a> {
    fn from_facet_mark(
        facet_mark: FacetSubplotRef<'a>,
        ctx: &SessionContext,
    ) -> Result<Option<Self>, AvengerChartError> {
        let (
            channels,
            channel_name,
            direction,
            subplot,
            slot_sharing,
            order_expr_node,
            order_descending,
            axis_guide_visibility,
            column_mode,
            kind,
        ) = match facet_mark {
            FacetSubplotRef::Row(facet_row) => (
                facet_row.compiled_state().data.channels(),
                "row",
                FacetDirection::Row,
                facet_row.compiled_subplot(),
                facet_row.facet_slot_sharing(),
                facet_row.facet_order_expr(),
                facet_row.facet_order_descending(),
                facet_row.axis_guide_visibility(),
                FacetWrapColumnMode::Auto,
                FacetPartitionKind::Band,
            ),
            FacetSubplotRef::Col(facet_col) => (
                facet_col.compiled_state().data.channels(),
                "column",
                FacetDirection::Column,
                facet_col.compiled_subplot(),
                facet_col.facet_slot_sharing(),
                facet_col.facet_order_expr(),
                facet_col.facet_order_descending(),
                facet_col.axis_guide_visibility(),
                FacetWrapColumnMode::Auto,
                FacetPartitionKind::Band,
            ),
            FacetSubplotRef::Wrap(facet_wrap) => (
                facet_wrap.compiled_state().data.channels(),
                "wrap",
                FacetDirection::Column,
                facet_wrap.compiled_subplot(),
                facet_wrap.facet_slot_sharing(),
                facet_wrap.facet_order_expr(),
                facet_wrap.facet_order_descending(),
                facet_wrap.axis_guide_visibility(),
                facet_wrap.facet_column_mode(),
                FacetPartitionKind::Wrap,
            ),
        };

        let Some(channel_value) = channels.get(channel_name) else {
            return Ok(None);
        };
        let Some(field_expr) = channel_value.expr(ctx) else {
            return Ok(None);
        };
        let order_expr = order_expr_node.map(|expr| expr.to_expr(ctx)).transpose()?;
        let sharing = slot_sharing
            .or_else(|| channel_value.get_domain_scope())
            .map(|s| s.to_level())
            .unwrap_or(0);

        Ok(Some(Self {
            kind,
            dimension: PartitionDimensionSpec::new(direction, sharing, field_expr)
                .with_ordering(order_expr, order_descending)
                .with_axis_guide_visibility(axis_guide_visibility.unwrap_or_default()),
            subplot,
            column_mode: FacetWrapColumnModeExpr::from_mode(column_mode, ctx)?,
        }))
    }
}

/// Recursively build a partition tree from compiled marks.
///
/// # Arguments
/// * `marks` - The marks to search for facets
/// * `df` - The DataFrame to query for distinct values
/// * `ctx` - Session context for expression evaluation
/// * `parent_path` - Values from outer facet levels.
/// * `ancestor_filters` - One filter expression for each value in `parent_path`.
/// * `current_depth` - Current depth in the facet hierarchy (1 = outermost)
/// * `slot_cache` - Cache for shared facet slot values to avoid redundant queries
async fn build_partition_tree(
    marks: &[Arc<dyn CompiledMark>],
    df: &DataFrame,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    parent_path: &[ScalarValue],
    ancestor_filters: &[Expr],
    current_depth: u8,
    slot_cache: &mut PartitionSlotCache,
    wrap_layout_context: FacetWrapLayoutContext,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    for mark in marks {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            let Some(spec) = FacetPartitionMarkSpec::from_facet_mark(facet_mark, ctx)? else {
                return Ok(None);
            };
            return match spec.kind {
                FacetPartitionKind::Band => {
                    Box::pin(build_partition_node(
                        &spec,
                        df,
                        ctx,
                        params,
                        parent_path,
                        ancestor_filters,
                        current_depth,
                        slot_cache,
                        wrap_layout_context,
                    ))
                    .await
                }
                FacetPartitionKind::Wrap => {
                    Box::pin(build_wrap_partition_node(
                        &spec,
                        df,
                        ctx,
                        params,
                        parent_path,
                        ancestor_filters,
                        current_depth,
                        slot_cache,
                        wrap_layout_context,
                    ))
                    .await
                }
            };
        }
    }

    // No facet found
    Ok(None)
}

/// Build a partition node for a specific facet.
async fn build_partition_node(
    spec: &FacetPartitionMarkSpec<'_>,
    df: &DataFrame,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    parent_path: &[ScalarValue],
    ancestor_filters: &[Expr],
    current_depth: u8,
    slot_cache: &mut PartitionSlotCache,
    wrap_layout_context: FacetWrapLayoutContext,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    let dimension = &spec.dimension;
    let parent_filter = combine_filters(ancestor_filters);
    let sharing_level = SharingLevel::from_raw(dimension.sharing);
    let domain_filter = domain_filter_for_sharing(ancestor_filters, sharing_level, current_depth);
    let observed_values = dimension
        .observed_values(df, parent_filter.clone(), params, slot_cache)
        .await?;
    let values = dimension
        .domain_values(df, domain_filter, params, slot_cache)
        .await?;

    if values.is_empty() {
        return Ok(None); // No values
    }

    // Check for nested facets in subplot
    if contains_facet_mark(&spec.subplot.marks) {
        // Build branch node with children for each value
        let mut children = IndexMap::new();
        let child_wrap_layout_context = match dimension.direction {
            FacetDirection::Column => wrap_layout_context.with_column_slots(values.len()),
            FacetDirection::Row => wrap_layout_context,
        };

        for value in &values {
            // Build filter for this value to pass to child
            let value_filter = dimension.value_filter(value);
            let mut child_path = parent_path.to_vec();
            child_path.push(value.clone());
            let mut child_filters = ancestor_filters.to_vec();
            child_filters.push(value_filter);

            // Recursively build child partition using the combined filter
            if let Some(child) = Box::pin(build_partition_tree(
                &spec.subplot.marks,
                df,
                ctx,
                params,
                &child_path,
                &child_filters,
                current_depth + 1,
                slot_cache,
                child_wrap_layout_context,
            ))
            .await?
            {
                children.insert(value.clone(), Box::new(child));
            }
        }

        if children.is_empty() {
            // No valid children - make leaf
            Ok(Some(
                PartitionNode::leaf_with_observed(
                    dimension.direction,
                    dimension.sharing,
                    dimension.field.clone(),
                    Some(dimension.field_expr.clone()),
                    values,
                    observed_values,
                )
                .with_axis_guide_visibility(dimension.axis_guide_visibility),
            ))
        } else {
            Ok(Some(
                PartitionNode::branch_with_values_and_observed(
                    dimension.direction,
                    dimension.sharing,
                    dimension.field.clone(),
                    Some(dimension.field_expr.clone()),
                    values,
                    observed_values,
                    children,
                )
                .with_axis_guide_visibility(dimension.axis_guide_visibility),
            ))
        }
    } else {
        // No nested facets - leaf node
        Ok(Some(
            PartitionNode::leaf_with_observed(
                dimension.direction,
                dimension.sharing,
                dimension.field.clone(),
                Some(dimension.field_expr.clone()),
                values,
                observed_values,
            )
            .with_axis_guide_visibility(dimension.axis_guide_visibility),
        ))
    }
}

fn domain_filter_for_sharing(
    ancestor_filters: &[Expr],
    sharing_level: SharingLevel,
    current_depth: u8,
) -> Option<Expr> {
    if sharing_level.is_free() {
        return combine_filters(ancestor_filters);
    }

    let parent_logical_depth = (current_depth as usize).saturating_sub(1);
    let keep_count = if sharing_level.raw() as usize >= parent_logical_depth {
        0
    } else {
        parent_logical_depth.saturating_sub(sharing_level.raw() as usize)
    };
    combine_filters(&ancestor_filters[..keep_count.min(ancestor_filters.len())])
}

fn wrap_row_field_name(field: &str) -> String {
    format!("__avenger_wrap_row:{field}")
}

fn wrap_value_field_name(field: &str) -> String {
    format!("__avenger_wrap_value:{field}")
}

pub(crate) fn is_wrap_row_field(field: &str) -> bool {
    field.starts_with("__avenger_wrap_row:")
}

fn partition_node_has_wrap_level(node: &PartitionNode) -> bool {
    if is_wrap_row_field(&node.field) {
        return true;
    }

    match &node.content {
        PartitionContent::Leaf { .. } => false,
        PartitionContent::Branch { children } => children
            .values()
            .any(|child| partition_node_has_wrap_level(child.as_ref())),
    }
}

fn observed_values_for_slice(
    values: &[ScalarValue],
    observed_values: &[ScalarValue],
) -> Vec<ScalarValue> {
    values
        .iter()
        .filter(|value| {
            observed_values
                .iter()
                .any(|observed| scalar_values_equivalent(value, observed))
        })
        .cloned()
        .collect()
}

fn wrap_row_values(row_count: usize) -> Vec<ScalarValue> {
    (0..row_count)
        .map(|row| ScalarValue::Int64(Some(row as i64)))
        .collect()
}

fn scalar_to_columns(value: ScalarValue, label: &str) -> Result<usize, AvengerChartError> {
    let columns = match value {
        ScalarValue::Int8(Some(v)) => v as i64,
        ScalarValue::Int16(Some(v)) => v as i64,
        ScalarValue::Int32(Some(v)) => v as i64,
        ScalarValue::Int64(Some(v)) => v,
        ScalarValue::UInt8(Some(v)) => v as i64,
        ScalarValue::UInt16(Some(v)) => v as i64,
        ScalarValue::UInt32(Some(v)) => v as i64,
        ScalarValue::UInt64(Some(v)) => i64::try_from(v).unwrap_or(i64::MAX),
        ScalarValue::Float32(Some(v)) => v.round() as i64,
        ScalarValue::Float64(Some(v)) => v.round() as i64,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must evaluate to a positive number, got {other:?}"
            )));
        }
    };
    if columns < 1 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must evaluate to a positive number, got {columns}"
        )));
    }
    Ok(columns as usize)
}

fn scalar_to_positive_f32(value: ScalarValue, label: &str) -> Result<f32, AvengerChartError> {
    let value = match value {
        ScalarValue::Int8(Some(v)) => v as f32,
        ScalarValue::Int16(Some(v)) => v as f32,
        ScalarValue::Int32(Some(v)) => v as f32,
        ScalarValue::Int64(Some(v)) => v as f32,
        ScalarValue::UInt8(Some(v)) => v as f32,
        ScalarValue::UInt16(Some(v)) => v as f32,
        ScalarValue::UInt32(Some(v)) => v as f32,
        ScalarValue::UInt64(Some(v)) => v as f32,
        ScalarValue::Float32(Some(v)) => v,
        ScalarValue::Float64(Some(v)) => v as f32,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must evaluate to a positive number, got {other:?}"
            )));
        }
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must evaluate to a positive number, got {value}"
        )));
    }
    Ok(value)
}

async fn resolve_fixed_wrap_columns(
    df: &DataFrame,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    domain_filter: Option<Expr>,
    columns_expr: &Expr,
) -> Result<usize, AvengerChartError> {
    if contains_aggregate(columns_expr) {
        let scoped_df = if let Some(domain_filter) = domain_filter {
            df.clone().filter(domain_filter)?
        } else {
            df.clone()
        };
        let aggregate_df = scoped_df.aggregate(
            vec![],
            vec![columns_expr.clone().alias("__facet_wrap_columns")],
        )?;
        let datafusion_params = params_to_datafusion(params);
        let batches = if let Some(param_values) = datafusion_params {
            aggregate_df
                .with_param_values(param_values)?
                .collect()
                .await?
        } else {
            aggregate_df.collect().await?
        };
        let batch = batches.first().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "FacetWrap aggregate columns expression returned no rows".to_string(),
            )
        })?;
        if batch.num_rows() == 0 {
            return Err(AvengerChartError::InvalidArgument(
                "FacetWrap aggregate columns expression returned no rows".to_string(),
            ));
        }
        return scalar_to_columns(
            ScalarValue::try_from_array(batch.column(0), 0)?,
            "FacetWrap columns expression",
        );
    }

    if columns_expr.any_column_refs() {
        return Err(AvengerChartError::InvalidArgument(
            "FacetWrap columns expression must be a constant, parameter, or aggregate expression"
                .to_string(),
        ));
    }

    let datafusion_params = params_to_datafusion(params);
    let scalar = columns_expr
        .eval_to_scalar(Some(ctx), datafusion_params.as_ref())
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    scalar_to_columns(scalar, "FacetWrap columns expression")
}

async fn resolve_responsive_wrap_columns(
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    target_width_expr: &Expr,
    slot_count: usize,
    layout_context: FacetWrapLayoutContext,
) -> Result<usize, AvengerChartError> {
    if !layout_context.width_is_canvas_constrained {
        return Err(AvengerChartError::InvalidArgument(
            "FacetWrap responsive_columns requires a canvas-constrained width; use canvas_constraint(width) with plot_constraint(height), or use columns(...) for an exact count".to_string(),
        ));
    }
    if !layout_context.height_is_leaf_plot_area_sized {
        return Err(AvengerChartError::InvalidArgument(
            "FacetWrap responsive_columns requires a leaf plot-area-sized height; height canvas-constrained wrapping is under-specified without an aspect target".to_string(),
        ));
    }
    if contains_aggregate(target_width_expr) || target_width_expr.any_column_refs() {
        return Err(AvengerChartError::InvalidArgument(
            "FacetWrap responsive_columns target width must be a constant or parameter expression"
                .to_string(),
        ));
    }

    let available_width = layout_context.current_available_width.ok_or_else(|| {
        AvengerChartError::InvalidArgument(
            "FacetWrap responsive_columns requires an estimated available width".to_string(),
        )
    })?;
    if !available_width.is_finite() || available_width <= 0.0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "FacetWrap responsive_columns available width must be positive, got {available_width}"
        )));
    }

    let datafusion_params = params_to_datafusion(params);
    let target_width = scalar_to_positive_f32(
        target_width_expr
            .eval_to_scalar(Some(ctx), datafusion_params.as_ref())
            .await
            .map_err(AvengerChartError::DataFusionError)?,
        "FacetWrap responsive_columns target width",
    )?;

    let mut best_columns = 1;
    let mut best_delta = f32::INFINITY;
    for columns in 1..=slot_count.max(1) {
        let estimated_width = available_width / columns as f32;
        let delta = (estimated_width - target_width).abs();
        if delta < best_delta {
            best_columns = columns;
            best_delta = delta;
        }
    }
    tracing::debug!(
        target: "avenger_chart::resize",
        slot_count,
        available_width,
        target_width,
        columns = best_columns,
        best_delta,
        "facet_wrap.responsive_columns"
    );
    Ok(best_columns)
}

async fn resolve_wrap_columns(
    df: &DataFrame,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    domain_filter: Option<Expr>,
    column_mode: &FacetWrapColumnModeExpr,
    slot_count: usize,
    layout_context: FacetWrapLayoutContext,
) -> Result<usize, AvengerChartError> {
    match column_mode {
        FacetWrapColumnModeExpr::Auto => Ok((slot_count as f64).sqrt().ceil().max(1.0) as usize),
        FacetWrapColumnModeExpr::Fixed(expr) => {
            resolve_fixed_wrap_columns(df, ctx, params, domain_filter, expr).await
        }
        FacetWrapColumnModeExpr::ResponsiveWidth(expr) => {
            resolve_responsive_wrap_columns(ctx, params, expr, slot_count, layout_context).await
        }
    }
}

async fn build_wrap_partition_node(
    spec: &FacetPartitionMarkSpec<'_>,
    df: &DataFrame,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    parent_path: &[ScalarValue],
    ancestor_filters: &[Expr],
    current_depth: u8,
    slot_cache: &mut PartitionSlotCache,
    wrap_layout_context: FacetWrapLayoutContext,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    let dimension = &spec.dimension;
    let parent_filter = combine_filters(ancestor_filters);
    let sharing_level = SharingLevel::from_raw(dimension.sharing);
    let domain_filter = domain_filter_for_sharing(ancestor_filters, sharing_level, current_depth);
    let observed_values = dimension
        .observed_values(df, parent_filter, params, slot_cache)
        .await?;
    let values = dimension
        .domain_values(df, domain_filter.clone(), params, slot_cache)
        .await?;

    if values.is_empty() {
        return Ok(None);
    }

    let columns = resolve_wrap_columns(
        df,
        ctx,
        params,
        domain_filter,
        &spec.column_mode,
        values.len(),
        wrap_layout_context,
    )
    .await?;
    let row_count = values.len().div_ceil(columns);
    let row_values = wrap_row_values(row_count);
    let observed_rows = row_values
        .iter()
        .enumerate()
        .filter_map(|(row_index, row_value)| {
            let start = row_index * columns;
            let end = (start + columns).min(values.len());
            let row_slice = &values[start..end];
            (!observed_values_for_slice(row_slice, &observed_values).is_empty())
                .then_some(row_value.clone())
        })
        .collect::<Vec<_>>();

    let mut row_children = IndexMap::new();
    let has_nested_facets = contains_facet_mark(&spec.subplot.marks);
    let child_wrap_layout_context = wrap_layout_context.with_column_slots(columns);
    for (row_index, row_value) in row_values.iter().enumerate() {
        let start = row_index * columns;
        let end = (start + columns).min(values.len());
        let row_slice = values[start..end].to_vec();
        let row_observed = observed_values_for_slice(&row_slice, &observed_values);

        let column_node = if has_nested_facets {
            let mut value_children = IndexMap::new();
            for value in &row_slice {
                let value_filter = dimension.value_filter(value);
                let mut child_path = parent_path.to_vec();
                child_path.push(row_value.clone());
                child_path.push(value.clone());
                let mut child_filters = ancestor_filters.to_vec();
                child_filters.push(value_filter);

                if let Some(child) = Box::pin(build_partition_tree(
                    &spec.subplot.marks,
                    df,
                    ctx,
                    params,
                    &child_path,
                    &child_filters,
                    current_depth + 1,
                    slot_cache,
                    child_wrap_layout_context,
                ))
                .await?
                {
                    value_children.insert(value.clone(), Box::new(child));
                }
            }

            if value_children.is_empty() {
                PartitionNode::leaf_with_observed(
                    FacetDirection::Column,
                    0,
                    wrap_value_field_name(&dimension.field),
                    Some(dimension.field_expr.clone()),
                    row_slice,
                    row_observed,
                )
                .with_axis_guide_visibility(dimension.axis_guide_visibility)
                .with_min_slot_count(columns)
            } else {
                PartitionNode::branch_with_values_and_observed(
                    FacetDirection::Column,
                    0,
                    wrap_value_field_name(&dimension.field),
                    Some(dimension.field_expr.clone()),
                    row_slice.clone(),
                    row_observed,
                    value_children,
                )
                .with_axis_guide_visibility(dimension.axis_guide_visibility)
                .with_min_slot_count(columns)
            }
        } else {
            PartitionNode::leaf_with_observed(
                FacetDirection::Column,
                0,
                wrap_value_field_name(&dimension.field),
                Some(dimension.field_expr.clone()),
                row_slice,
                row_observed,
            )
            .with_axis_guide_visibility(dimension.axis_guide_visibility)
            .with_min_slot_count(columns)
        };
        row_children.insert(row_value.clone(), Box::new(column_node));
    }

    Ok(Some(
        PartitionNode::branch_with_values_and_observed(
            FacetDirection::Row,
            dimension.sharing,
            wrap_row_field_name(&dimension.field),
            None,
            row_values,
            observed_rows,
            row_children,
        )
        .with_axis_guide_visibility(dimension.axis_guide_visibility),
    ))
}

fn combine_filters(filters: &[Expr]) -> Option<Expr> {
    filters
        .iter()
        .cloned()
        .reduce(|combined, filter| combined.and(filter))
}

fn contains_facet_mark(marks: &[Arc<dyn CompiledMark>]) -> bool {
    marks
        .iter()
        .any(|mark| facet_subplot_ref(mark.as_ref()).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use avenger_chart_core::DomainCoordinationGroup;

    fn scalar(s: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(s.to_string()))
    }

    async fn responsive_wrap_data(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES
                ('A', 0.0, 0.1),
                ('B', 1.0, 0.2),
                ('C', 2.0, 0.3),
                ('D', 3.0, 0.4),
                ('E', 4.0, 0.5),
                ('F', 5.0, 0.6)
            ) AS t(facet, x, y)",
        )
        .await
        .expect("responsive wrap data")
    }

    fn responsive_layout_context(width: f32) -> FacetWrapLayoutContext {
        FacetWrapLayoutContext {
            current_available_width: Some(width),
            width_is_canvas_constrained: true,
            height_is_leaf_plot_area_sized: true,
        }
    }

    async fn responsive_wrap_tree(
        ctx: &SessionContext,
        target_width: Expr,
        available_width: f32,
    ) -> Result<EvaluatedFacetTree, AvengerChartError> {
        let plot = crate::plot::Chart::<FacetWrap>::new()
            .data(responsive_wrap_data(ctx).await)
            .mark(
                Subplot::new(
                    crate::plot::Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("x")).y(col("y"))),
                )
                .wrap_with(col("facet"), move |c| c.responsive_columns(target_width)),
            );
        let compiled = plot.compile(ctx).await?;
        EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            ctx,
            compiled.get_default_params(),
            responsive_layout_context(available_width),
        )
        .await
    }

    async fn responsive_wrap_tree_from_sql(
        ctx: &SessionContext,
        sql: &str,
        available_width: f32,
    ) -> Result<EvaluatedFacetTree, AvengerChartError> {
        let df = ctx.sql(sql).await?;
        let plot = crate::plot::Chart::<FacetWrap>::new().data(df).mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );
        let compiled = plot.compile(ctx).await?;
        EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            ctx,
            compiled.get_default_params(),
            responsive_layout_context(available_width),
        )
        .await
    }

    async fn ordered_responsive_wrap_tree(
        ctx: &SessionContext,
        order_desc: bool,
        available_width: f32,
    ) -> Result<EvaluatedFacetTree, AvengerChartError> {
        use datafusion::functions_aggregate::min_max::max;

        let df = responsive_wrap_data(ctx).await;
        let plot = crate::plot::Chart::<FacetWrap>::new().data(df).mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .wrap_with(col("facet"), move |c| {
                let c = c.responsive_columns(180.0).order_by(max(col("y")));
                if order_desc {
                    c.order_desc()
                } else {
                    c.order_asc()
                }
            }),
        );
        let compiled = plot.compile(ctx).await?;
        EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            ctx,
            compiled.get_default_params(),
            responsive_layout_context(available_width),
        )
        .await
    }

    async fn row_nested_responsive_wrap_tree(
        ctx: &SessionContext,
        available_width: f32,
    ) -> Result<EvaluatedFacetTree, AvengerChartError> {
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 'A', 0.0, 0.1),
                    ('North', 'B', 1.0, 0.2),
                    ('North', 'C', 2.0, 0.3),
                    ('South', 'A', 10.0, 0.4),
                    ('South', 'B', 11.0, 0.5),
                    ('South', 'C', 12.0, 0.6)
                ) AS t(region, facet, x, y)",
            )
            .await?;
        let wrap = crate::plot::Plot::<FacetWrap>::new().mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );
        let plot = crate::plot::Chart::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(wrap).row(col("region")));
        let compiled = plot.compile(ctx).await?;
        EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            ctx,
            compiled.get_default_params(),
            responsive_layout_context(available_width),
        )
        .await
    }

    fn top_level_wrap_column_count(tree: &EvaluatedFacetTree) -> usize {
        let root = tree.root().expect("wrap root");
        let first_row = root.values().next().expect("first wrap row").clone();
        root.child(&first_row)
            .expect("wrap row child")
            .values()
            .count()
    }

    fn nested_column_wrap_column_count(tree: &EvaluatedFacetTree) -> usize {
        let root = tree.root().expect("outer column root");
        let first_column = root.values().next().expect("first outer column").clone();
        let wrap_root = root.child(&first_column).expect("nested wrap root");
        let first_row = wrap_root
            .values()
            .next()
            .expect("first nested wrap row")
            .clone();
        wrap_root
            .child(&first_row)
            .expect("nested wrap row child")
            .values()
            .count()
    }

    #[tokio::test]
    async fn fixed_wrap_rows_preserve_trailing_physical_slot_count() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.1),
                    ('B', 1.0, 0.2),
                    ('C', 2.0, 0.3)
                ) AS t(facet, x, y)",
            )
            .await?;
        let plot = crate::plot::Chart::<FacetWrap>::new().data(df).mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .wrap_with(col("facet"), |c| c.columns(2).empty_cells_as_holes()),
        );
        let compiled = plot.compile(&ctx).await?;
        let tree = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx).await?;
        let root = tree.root().expect("wrap root");
        let rows = root.values().cloned().collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);

        let first_row_child = root.child(&rows[0]).expect("first wrap row child");
        assert_eq!(first_row_child.values().count(), 2);
        assert_eq!(first_row_child.min_slot_count(), Some(2));

        let second_row_child = root.child(&rows[1]).expect("second wrap row child");
        assert_eq!(second_row_child.values().count(), 1);
        assert_eq!(second_row_child.min_slot_count(), Some(2));
        assert_eq!(tree.min_slot_count_for_facet(&[rows[1].clone()]), Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_columns_choose_from_available_width() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let narrow = responsive_wrap_tree(&ctx, lit(180.0), 300.0).await?;
        let wide = responsive_wrap_tree(&ctx, lit(180.0), 900.0).await?;

        assert_eq!(top_level_wrap_column_count(&narrow), 2);
        assert_eq!(top_level_wrap_column_count(&wide), 5);
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_logical_structure_ignores_physical_row_reflow()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let narrow = responsive_wrap_tree(&ctx, lit(180.0), 300.0).await?;
        let wide = responsive_wrap_tree(&ctx, lit(180.0), 900.0).await?;

        assert_ne!(narrow.structure_cache_key(), wide.structure_cache_key());
        assert_eq!(
            narrow.logical_structure_cache_key(),
            wide.logical_structure_cache_key()
        );
        assert_eq!(narrow.logical_depth(), 1);
        assert_eq!(wide.logical_depth(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_logical_cell_key_skips_synthetic_row() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let tree = responsive_wrap_tree(&ctx, lit(180.0), 300.0).await?;
        let root = tree.root().expect("wrap root");
        let first_row = root.values().next().expect("first row").clone();
        let first_value = root
            .child(&first_row)
            .expect("wrap value node")
            .values()
            .next()
            .expect("first wrap value")
            .clone();
        let key = tree
            .logical_cell_key_for_path(&[first_row, first_value])
            .expect("logical cell key");

        assert_eq!(key, vec!["__avenger_wrap_value:facet=Utf8(\"A\")"]);
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_logical_structure_changes_when_values_change()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let base = responsive_wrap_tree_from_sql(
            &ctx,
            "SELECT * FROM (VALUES
                ('A', 0.0, 0.1),
                ('B', 1.0, 0.2),
                ('C', 2.0, 0.3)
            ) AS t(facet, x, y)",
            500.0,
        )
        .await?;
        let changed = responsive_wrap_tree_from_sql(
            &ctx,
            "SELECT * FROM (VALUES
                ('A', 0.0, 0.1),
                ('B', 1.0, 0.2),
                ('D', 2.0, 0.3)
            ) AS t(facet, x, y)",
            500.0,
        )
        .await?;

        assert_ne!(
            base.logical_structure_cache_key(),
            changed.logical_structure_cache_key()
        );
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_logical_structure_changes_when_order_changes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let asc = ordered_responsive_wrap_tree(&ctx, false, 900.0).await?;
        let desc = ordered_responsive_wrap_tree(&ctx, true, 900.0).await?;

        assert_ne!(
            asc.logical_structure_cache_key(),
            desc.logical_structure_cache_key()
        );
        Ok(())
    }

    #[tokio::test]
    async fn nested_row_wrap_logical_cell_key_includes_row_and_wrap_value()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let tree = row_nested_responsive_wrap_tree(&ctx, 500.0).await?;
        let root = tree.root().expect("row root");
        let row_value = root.values().next().expect("first row").clone();
        let wrap_root = root.child(&row_value).expect("wrap root");
        let wrap_row = wrap_root.values().next().expect("first wrap row").clone();
        let wrap_value = wrap_root
            .child(&wrap_row)
            .expect("wrap row child")
            .values()
            .next()
            .expect("first wrap value")
            .clone();
        let key = tree
            .logical_cell_key_for_path(&[row_value, wrap_row, wrap_value])
            .expect("logical cell key");

        assert_eq!(
            key,
            vec![
                "region=Utf8(\"North\")",
                "__avenger_wrap_value:facet=Utf8(\"A\")"
            ]
        );
        assert_eq!(tree.logical_depth(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_columns_accept_params() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let target = {
            let __avenger_param_name = "target_width";
            let __avenger_param_default: datafusion::common::ScalarValue =
                ScalarValue::Float32(Some(160.0));
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        };
        let plot = crate::plot::Chart::<FacetWrap>::new()
            .data(responsive_wrap_data(&ctx).await)
            .param(target.clone())
            .mark(
                Subplot::new(
                    crate::plot::Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("x")).y(col("y"))),
                )
                .wrap_with(col("facet"), move |c| c.responsive_columns(target.expr())),
            );
        let compiled = plot.compile(&ctx).await?;
        let tree = EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            &ctx,
            compiled.get_default_params(),
            responsive_layout_context(480.0),
        )
        .await?;

        assert_eq!(top_level_wrap_column_count(&tree), 3);
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_columns_reject_invalid_targets() -> Result<(), AvengerChartError> {
        use datafusion::functions_aggregate::min_max::max;

        let ctx = SessionContext::new();
        let zero = responsive_wrap_tree(&ctx, lit(0.0), 400.0)
            .await
            .expect_err("zero target width should fail");
        assert!(zero.to_string().contains("positive number"), "{zero}");

        let column_ref = responsive_wrap_tree(&ctx, col("x"), 400.0)
            .await
            .expect_err("column target width should fail");
        assert!(
            column_ref.to_string().contains("constant or parameter"),
            "{column_ref}"
        );

        let aggregate = responsive_wrap_tree(&ctx, max(col("x")), 400.0)
            .await
            .expect_err("aggregate target width should fail");
        assert!(
            aggregate.to_string().contains("constant or parameter"),
            "{aggregate}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn responsive_wrap_columns_reject_unsupported_sizing() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let plot = crate::plot::Chart::<FacetWrap>::new()
            .data(responsive_wrap_data(&ctx).await)
            .mark(
                Subplot::new(
                    crate::plot::Plot::<Cartesian>::new()
                        .mark(Symbol::new().x(col("x")).y(col("y"))),
                )
                .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
            );
        let compiled = plot.compile(&ctx).await?;

        let plot_width_sized =
            EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
                &compiled,
                &ctx,
                compiled.get_default_params(),
                FacetWrapLayoutContext {
                    current_available_width: None,
                    width_is_canvas_constrained: false,
                    height_is_leaf_plot_area_sized: true,
                },
            )
            .await
            .expect_err("plot-width-sized responsive wrap should fail");
        assert!(
            plot_width_sized
                .to_string()
                .contains("canvas-constrained width"),
            "{plot_width_sized}"
        );

        let canvas_height_sized =
            EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
                &compiled,
                &ctx,
                compiled.get_default_params(),
                FacetWrapLayoutContext {
                    current_available_width: Some(480.0),
                    width_is_canvas_constrained: true,
                    height_is_leaf_plot_area_sized: false,
                },
            )
            .await
            .expect_err("canvas-height-sized responsive wrap should fail");
        assert!(
            canvas_height_sized
                .to_string()
                .contains("leaf plot-area-sized height"),
            "{canvas_height_sized}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn responsive_nested_column_wrap_uses_local_width() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('North', 'A', 0.0, 0.1),
                    ('North', 'B', 1.0, 0.2),
                    ('North', 'C', 2.0, 0.3),
                    ('North', 'D', 3.0, 0.4),
                    ('North', 'E', 4.0, 0.5),
                    ('North', 'F', 5.0, 0.6),
                    ('South', 'A', 10.0, 0.1),
                    ('South', 'B', 11.0, 0.2),
                    ('South', 'C', 12.0, 0.3),
                    ('South', 'D', 13.0, 0.4),
                    ('South', 'E', 14.0, 0.5),
                    ('South', 'F', 15.0, 0.6)
                ) AS t(region, facet, x, y)",
            )
            .await
            .expect("nested responsive wrap data");
        let wrap = crate::plot::Plot::<FacetWrap>::new().mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );
        let plot = crate::plot::Chart::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(wrap).column(col("region")));
        let compiled = plot.compile(&ctx).await?;
        let tree = EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
            &compiled,
            &ctx,
            compiled.get_default_params(),
            responsive_layout_context(720.0),
        )
        .await?;

        assert_eq!(nested_column_wrap_column_count(&tree), 2);
        Ok(())
    }

    fn scalar_view(s: &str) -> ScalarValue {
        ScalarValue::Utf8View(Some(s.to_string()))
    }

    fn scalar_large(s: &str) -> ScalarValue {
        ScalarValue::LargeUtf8(Some(s.to_string()))
    }

    #[test]
    fn test_empty_spec() {
        let spec = EvaluatedFacetTree::empty();
        assert_eq!(spec.depth(), 0);
    }

    #[test]
    fn test_single_level_row() {
        let root = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops")],
        );

        let spec = EvaluatedFacetTree::new(Some(root));

        assert_eq!(spec.depth(), 1);
    }

    #[test]
    fn test_nested_with_shared_domain() {
        // Col > Row with shared facet slots (same values regardless of parent)
        let row_node = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("East"), Box::new(row_node.clone()));
        children.insert(scalar("West"), Box::new(row_node));

        let col_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            children,
        );

        let spec = EvaluatedFacetTree::new(Some(col_node));

        assert_eq!(spec.depth(), 2);
    }

    #[test]
    fn test_four_level_row_nesting() {
        // Row > Row > Row > Row (same-type nesting)
        fn build_level(depth: usize, values: Vec<&str>) -> PartitionNode {
            if depth >= 4 {
                // Leaf level
                PartitionNode::leaf(
                    FacetDirection::Row,
                    0,
                    format!("level{}", depth),
                    None,
                    values.iter().map(|s| scalar(s)).collect(),
                )
            } else {
                // Branch level
                let mut children = IndexMap::new();
                for v in &values {
                    children.insert(scalar(v), Box::new(build_level(depth + 1, vec!["A", "B"])));
                }
                PartitionNode::branch(
                    FacetDirection::Row,
                    0,
                    format!("level{}", depth),
                    None,
                    children,
                )
            }
        }

        let root = build_level(1, vec!["X", "Y"]);

        let spec = EvaluatedFacetTree::new(Some(root));

        assert_eq!(spec.depth(), 4);
    }

    #[test]
    fn test_values_iterator() {
        // Test that values() works for both leaf and branch nodes
        let leaf = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            None,
            vec![scalar("A"), scalar("B")],
        );
        let leaf_values: Vec<_> = leaf.values().collect();
        assert_eq!(leaf_values, vec![&scalar("A"), &scalar("B")]);

        let mut children = IndexMap::new();
        children.insert(scalar("X"), Box::new(leaf.clone()));
        children.insert(scalar("Y"), Box::new(leaf));
        let branch = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            children,
        );
        let branch_values: Vec<_> = branch.values().collect();
        assert_eq!(branch_values, vec![&scalar("X"), &scalar("Y")]);

        // Test domain_count
        assert_eq!(branch.domain_count(), 2);
    }

    #[test]
    fn test_cell_exists() {
        let team_leaf = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![scalar("A"), scalar("B")],
        );

        let mut dept_children = IndexMap::new();
        dept_children.insert(scalar("Eng"), Box::new(team_leaf));
        let dept_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "dept".to_string(),
            None,
            dept_children,
        );

        let tree = EvaluatedFacetTree::new(Some(dept_node));

        assert!(tree.cell_exists(&[]));
        assert!(tree.cell_exists(&[scalar("Eng")]));
        assert!(tree.cell_exists(&[scalar("Eng"), scalar("A")]));
        assert!(!tree.cell_exists(&[scalar("Ops")]));
        assert!(!tree.cell_exists(&[scalar("Eng"), scalar("Z")]));
    }

    #[test]
    fn test_cell_has_data_can_differ_from_cell_exists_for_shared_domain() {
        let team_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            255,
            "team".to_string(),
            None,
            vec![scalar("A"), scalar("B")],
            vec![scalar("A")],
        );

        let mut dept_children = IndexMap::new();
        dept_children.insert(scalar("Eng"), Box::new(team_leaf));
        let dept_node = PartitionNode::branch(
            FacetDirection::Column,
            255,
            "dept".to_string(),
            None,
            dept_children,
        );

        let tree = EvaluatedFacetTree::new(Some(dept_node));
        assert!(tree.cell_exists(&[scalar("Eng"), scalar("B")]));
        assert!(!tree.cell_has_data(&[scalar("Eng"), scalar("B")]));
        assert!(tree.cell_has_data(&[scalar("Eng"), scalar("A")]));
    }

    fn build_enumeration_test_tree() -> EvaluatedFacetTree {
        // Region (Col) > Department (Row) > Team (Row)
        //
        // East:
        //   Eng -> A, B
        //   Ops -> B, C
        // West:
        //   Eng -> C, D
        //   Ops -> D, E
        //
        // Values intentionally overlap to validate dedup in Level(N) enumeration.
        let team_leaf_eng_east = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![scalar("A"), scalar("B")],
        );
        let team_leaf_ops_east = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![scalar("B"), scalar("C")],
        );
        let team_leaf_eng_west = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![scalar("C"), scalar("D")],
        );
        let team_leaf_ops_west = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![scalar("D"), scalar("E")],
        );

        let mut dept_children_east = IndexMap::new();
        dept_children_east.insert(scalar("Eng"), Box::new(team_leaf_eng_east));
        dept_children_east.insert(scalar("Ops"), Box::new(team_leaf_ops_east));
        let dept_node_east = PartitionNode::branch(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            None,
            dept_children_east,
        );

        let mut dept_children_west = IndexMap::new();
        dept_children_west.insert(scalar("Eng"), Box::new(team_leaf_eng_west));
        dept_children_west.insert(scalar("Ops"), Box::new(team_leaf_ops_west));
        let dept_node_west = PartitionNode::branch(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            None,
            dept_children_west,
        );

        let mut region_children = IndexMap::new();
        region_children.insert(scalar("East"), Box::new(dept_node_east));
        region_children.insert(scalar("West"), Box::new(dept_node_west));
        let region_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            region_children,
        );

        EvaluatedFacetTree::new(Some(region_node))
    }

    fn build_hole_visibility_test_tree() -> EvaluatedFacetTree {
        // Column (petal_width_bin) > Row (species)
        //
        // Shared row slot values exist in every column slot, but observed values
        // differ by column, creating hole cells when empty policy is Hole.
        let all_species = vec![
            scalar("Iris-setosa"),
            scalar("Iris-versicolor"),
            scalar("Iris-virginica"),
        ];

        let medium_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            255,
            "species".to_string(),
            None,
            all_species.clone(),
            vec![scalar("Iris-versicolor"), scalar("Iris-virginica")],
        );
        let narrow_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            255,
            "species".to_string(),
            None,
            all_species.clone(),
            vec![scalar("Iris-setosa")],
        );
        let wide_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            255,
            "species".to_string(),
            None,
            all_species,
            vec![scalar("Iris-virginica")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("medium"), Box::new(medium_leaf));
        children.insert(scalar("narrow"), Box::new(narrow_leaf));
        children.insert(scalar("wide"), Box::new(wide_leaf));

        let root = PartitionNode::branch_with_observed(
            FacetDirection::Column,
            255,
            "petal_width_bin".to_string(),
            None,
            vec![scalar("medium"), scalar("narrow"), scalar("wide")],
            children,
        );

        EvaluatedFacetTree::new(Some(root))
    }

    fn build_free_row_title_test_tree() -> EvaluatedFacetTree {
        // Column (petal_width_bin) > Row (species), non-shared row domains.
        //
        // Medium/Wide have two local row slots, Narrow has one. Title ownership
        // should still respect geometric strip edges and not relocate into
        // Narrow's single local row.
        let medium_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar("Iris-versicolor"), scalar("Iris-virginica")],
            vec![scalar("Iris-versicolor"), scalar("Iris-virginica")],
        );
        let narrow_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar("Iris-setosa")],
            vec![scalar("Iris-setosa")],
        );
        let wide_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar("Iris-versicolor"), scalar("Iris-virginica")],
            vec![scalar("Iris-versicolor"), scalar("Iris-virginica")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("medium"), Box::new(medium_leaf));
        children.insert(scalar("narrow"), Box::new(narrow_leaf));
        children.insert(scalar("wide"), Box::new(wide_leaf));

        let root = PartitionNode::branch_with_observed(
            FacetDirection::Column,
            255,
            "petal_width_bin".to_string(),
            None,
            vec![scalar("medium"), scalar("narrow"), scalar("wide")],
            children,
        );

        EvaluatedFacetTree::new(Some(root))
    }

    fn build_predicate_test_tree() -> EvaluatedFacetTree {
        use datafusion::logical_expr::col;

        let team_leaf = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            Some(col("team")),
            vec![scalar("A"), scalar("B")],
        );

        let mut dept_children = IndexMap::new();
        dept_children.insert(scalar("Eng"), Box::new(team_leaf.clone()));
        dept_children.insert(scalar("Ops"), Box::new(team_leaf));
        let dept_node = PartitionNode::branch(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            Some(col("dept")),
            dept_children,
        );

        let mut region_children = IndexMap::new();
        region_children.insert(scalar("East"), Box::new(dept_node.clone()));
        region_children.insert(scalar("West"), Box::new(dept_node));
        let region_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            Some(col("region")),
            region_children,
        );

        EvaluatedFacetTree::new(Some(region_node))
    }

    fn build_jagged_test_tree() -> EvaluatedFacetTree {
        // Column -> Row where row counts differ by column branch.
        let east_rows = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar("A"), scalar("B")],
        );
        let west_rows = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar("A")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("East"), Box::new(east_rows));
        children.insert(scalar("West"), Box::new(west_rows));

        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            children,
        );

        EvaluatedFacetTree::new(Some(root))
    }

    fn cell_exists_uncached(tree: &EvaluatedFacetTree, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return tree.root().is_some();
        }

        let parent_path = &path[..path.len() - 1];
        let target_value = &path[path.len() - 1];
        tree.node_at_path(parent_path).is_some_and(|parent| {
            parent
                .values()
                .any(|v| scalar_values_equivalent(v, target_value))
        })
    }

    fn cell_has_data_uncached(tree: &EvaluatedFacetTree, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return tree.root().is_some();
        }

        let parent_path = &path[..path.len() - 1];
        let target_value = &path[path.len() - 1];
        tree.node_at_path(parent_path).is_some_and(|parent| {
            parent
                .observed_values()
                .any(|v| scalar_values_equivalent(v, target_value))
        })
    }

    #[test]
    fn test_enumerate_values_for_facet_level0() {
        let tree = build_enumeration_test_tree();
        let path = vec![scalar("East"), scalar("Eng")];
        let values = tree.enumerate_values_for_facet(&path, 0).unwrap();
        assert_eq!(values, vec![scalar("A"), scalar("B")]);
    }

    #[test]
    fn test_enumerate_values_for_facet_level1() {
        let tree = build_enumeration_test_tree();
        let path = vec![scalar("East"), scalar("Eng")];
        let values = tree.enumerate_values_for_facet(&path, 1).unwrap();
        assert_eq!(values, vec![scalar("A"), scalar("B"), scalar("C")]);
    }

    #[test]
    fn test_enumerate_values_for_facet_global() {
        let tree = build_enumeration_test_tree();
        let path = vec![scalar("East"), scalar("Eng")];
        let values = tree.enumerate_values_for_facet(&path, 255).unwrap();
        assert_eq!(
            values,
            vec![
                scalar("A"),
                scalar("B"),
                scalar("C"),
                scalar("D"),
                scalar("E"),
            ]
        );
    }

    #[test]
    fn test_enumerate_values_for_facet_invalid_path() {
        let tree = build_enumeration_test_tree();
        let path = vec![scalar("North"), scalar("Eng")];
        assert!(tree.enumerate_values_for_facet(&path, 0).is_none());
    }

    #[test]
    fn test_cache_parity_resolve_path_info_for_valid_paths() {
        let tree = build_enumeration_test_tree();

        for path in tree.collect_reachable_paths() {
            assert_eq!(
                tree.resolve_path_info(&path),
                tree.resolve_path_info_uncached(&path),
                "resolve_path_info mismatch for path: {path:?}"
            );
        }
    }

    #[test]
    fn test_cache_parity_path_predicate_and_cell_predicate_truncation() {
        let tree = build_predicate_test_tree();

        for path in tree.collect_reachable_paths() {
            if path.is_empty() {
                continue;
            }
            let cached = tree.path_predicate(&path).map(|expr| expr.to_string());
            let uncached = tree
                .path_predicate_uncached(&path)
                .map(|expr| expr.to_string());
            assert_eq!(
                cached, uncached,
                "path_predicate mismatch for path: {path:?}"
            );
        }

        let full_path = vec![scalar("East"), scalar("Eng"), scalar("A")];
        for sharing_level in [0_u8, 1, 2, 3, 255] {
            let cached = tree
                .cell_predicate(&full_path, sharing_level)
                .map(|expr| expr.to_string());
            let uncached = if sharing_level == 255 {
                None
            } else {
                let levels_to_include = full_path.len().saturating_sub(sharing_level as usize);
                if levels_to_include == 0 {
                    None
                } else {
                    tree.path_predicate_uncached(&full_path[..levels_to_include])
                        .map(|expr| expr.to_string())
                }
            };
            assert_eq!(
                cached, uncached,
                "cell_predicate mismatch for sharing={sharing_level}"
            );
        }
    }

    #[test]
    fn test_cache_parity_cell_exists_and_cell_has_data() {
        let tree = build_hole_visibility_test_tree();
        let paths = vec![
            vec![],
            vec![scalar("medium")],
            vec![scalar("narrow")],
            vec![scalar("wide")],
            vec![scalar("missing")],
            vec![scalar("medium"), scalar("Iris-setosa")],
            vec![scalar("medium"), scalar("Iris-virginica")],
            vec![scalar("narrow"), scalar("Iris-setosa")],
            vec![scalar("narrow"), scalar("Iris-virginica")],
            vec![scalar("wide"), scalar("Iris-setosa")],
            vec![scalar("wide"), scalar("missing")],
            vec![scalar("missing"), scalar("Iris-setosa")],
            vec![scalar("narrow"), scalar("Iris-setosa"), scalar("extra")],
        ];

        for path in paths {
            assert_eq!(
                tree.cell_exists(&path),
                cell_exists_uncached(&tree, &path),
                "cell_exists mismatch for path: {path:?}"
            );
            assert_eq!(
                tree.cell_has_data(&path),
                cell_has_data_uncached(&tree, &path),
                "cell_has_data mismatch for path: {path:?}"
            );
        }
    }

    #[test]
    fn test_cache_parity_enumerate_values_for_facet_across_sharing_levels() {
        let tree = build_enumeration_test_tree();
        for facet_path in tree.collect_node_paths() {
            for sharing_level in [0_u8, 1, 255] {
                assert_eq!(
                    tree.enumerate_values_for_facet(&facet_path, sharing_level),
                    tree.enumerate_values_for_facet_uncached(
                        &facet_path,
                        SharingLevel::from_raw(sharing_level),
                    ),
                    "enumeration mismatch for path={facet_path:?}, sharing={sharing_level}"
                );
            }
        }
    }

    #[test]
    fn test_cache_lookup_utf8_variant_equivalence() {
        use datafusion::logical_expr::col;

        let row_leaf = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            0,
            "species".to_string(),
            Some(col("species")),
            vec![scalar_view("Iris-setosa"), scalar_view("Iris-virginica")],
            vec![scalar_view("Iris-setosa")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar_view("narrow"), Box::new(row_leaf));
        let root = PartitionNode::branch_with_observed(
            FacetDirection::Column,
            0,
            "petal_width_bin".to_string(),
            Some(col("petal_width_bin")),
            vec![scalar_view("narrow")],
            children,
        );
        let tree = EvaluatedFacetTree::new(Some(root));

        let utf8_path = vec![scalar("narrow"), scalar("Iris-setosa")];
        let large_utf8_path = vec![scalar_large("narrow"), scalar_large("Iris-setosa")];
        let utf8_facet_path = vec![scalar("narrow")];
        let large_utf8_facet_path = vec![scalar_large("narrow")];

        assert!(tree.resolve_path_info(&utf8_path).is_some());
        assert!(tree.resolve_path_info(&large_utf8_path).is_some());
        assert!(tree.path_predicate(&utf8_path).is_some());
        assert!(tree.path_predicate(&large_utf8_path).is_some());
        assert!(tree.cell_exists(&utf8_path));
        assert!(tree.cell_exists(&large_utf8_path));
        assert!(tree.cell_has_data(&utf8_path));
        assert!(tree.cell_has_data(&large_utf8_path));
        assert_eq!(
            tree.enumerate_values_for_facet(&utf8_facet_path, 0),
            tree.enumerate_values_for_facet(&large_utf8_facet_path, 0),
        );
    }

    #[test]
    fn test_cache_parity_is_jagged_for_axis() {
        use avenger_chart_core::AxisPosition;

        let tree = build_jagged_test_tree();
        for axis_position in [
            AxisPosition::Left,
            AxisPosition::Right,
            AxisPosition::Top,
            AxisPosition::Bottom,
        ] {
            assert_eq!(
                tree.is_jagged_for_axis(axis_position),
                tree.is_jagged_for_axis_uncached(axis_position),
                "jagged mismatch for axis: {axis_position:?}"
            );
        }
    }

    #[test]
    fn test_cache_precompute_constructor_coverage() {
        let root = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops")],
        );

        let with_new = EvaluatedFacetTree::new(Some(root.clone()));
        assert_eq!(with_new.depth(), 1);
        assert!(with_new.used_sharing_levels.contains(&SharingLevel::FREE));
        assert!(with_new.used_sharing_levels.contains(&SharingLevel::GLOBAL));
        assert!(
            with_new
                .path_info_cache
                .contains_key(&Vec::<ScalarValue>::new())
        );
        assert!(with_new.path_info_cache.contains_key(&vec![scalar("Eng")]));
        assert!(
            with_new
                .slot_membership_cache
                .contains_key(&Vec::<ScalarValue>::new())
        );
        assert!(!with_new.enumeration_cache.is_empty());
        assert_eq!(with_new.jagged_axis_cache.len(), 4);

        let mut channel_sharing = HashMap::new();
        channel_sharing.insert("x".to_string(), 2);
        let with_levels =
            EvaluatedFacetTree::new_with_channel_domain_sharing_levels(Some(root), channel_sharing);
        assert_eq!(with_levels.channel_domain_sharing_level("x"), 2);
        assert!(
            with_levels
                .used_sharing_levels
                .contains(&SharingLevel::FREE)
        );
        assert!(
            with_levels
                .used_sharing_levels
                .contains(&SharingLevel::from_raw(2))
        );
        assert!(
            with_levels
                .used_sharing_levels
                .contains(&SharingLevel::GLOBAL)
        );

        let empty = EvaluatedFacetTree::empty();
        assert_eq!(empty.depth(), 0);
        assert!(empty.path_info_cache.is_empty());
        assert!(empty.path_predicate_cache.is_empty());
        assert!(empty.slot_membership_cache.is_empty());
        assert!(empty.enumeration_cache.is_empty());
        assert_eq!(empty.jagged_axis_cache.len(), 4);
        assert!(empty.jagged_axis_cache.values().all(|value| !*value));
        assert!(empty.used_sharing_levels.contains(&SharingLevel::FREE));
        assert!(empty.used_sharing_levels.contains(&SharingLevel::GLOBAL));
    }

    #[tokio::test]
    async fn extract_channel_domain_coordinations_preserves_named_groups()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().x_with(lit(1.0), |c| c.with_domain_group("height").share_domain()))
            .compile(&ctx)
            .await?;

        let coordinations = extract_channel_domain_coordinations(&compiled.marks);
        let coordination = coordinations
            .get("x")
            .expect("expected x domain coordination");

        assert_eq!(coordination.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("height".to_string())
        );

        let tree = EvaluatedFacetTree::init_with_caches(None, coordinations, HashMap::new());
        assert_eq!(
            tree.channel_domain_coordination("x").group,
            DomainCoordinationGroup::Named("height".to_string())
        );
        assert_eq!(tree.channel_domain_sharing_level("x"), u8::MAX);

        Ok(())
    }

    #[tokio::test]
    async fn extract_channel_domain_coordinations_adds_default_shared_scaled_channels()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .fill(col("continuous_fill")),
            )
            .compile(&ctx)
            .await?;

        let coordinations = extract_channel_domain_coordinations(&compiled.marks);
        let fill = coordinations
            .get("fill")
            .expect("default scaled fill should be coordinated");

        assert_eq!(fill.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(fill.group, DomainCoordinationGroup::ScaleName);

        Ok(())
    }

    #[tokio::test]
    async fn extract_plot_channel_domain_metadata_includes_mark_owned_parallel_dimensions()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Parallel>::new()
            .mark(
                ParallelLine::new()
                    .dimension_with("mpg", col("mpg"), |dimension| dimension.free_domain())
                    .dimension("origin", col("origin")),
            )
            .compile(&ctx)
            .await?;

        let (coordinations, nested_configs) = extract_plot_channel_domain_metadata(&compiled);
        let mpg = coordinations
            .get("mpg")
            .expect("parallel dimension domain coordination");
        let origin = coordinations
            .get("origin")
            .expect("implicit parallel dimension domain coordination");

        assert!(SharingLevel::from(mpg.scope).is_free());
        assert_eq!(mpg.group, DomainCoordinationGroup::ScaleName);
        assert!(SharingLevel::from(origin.scope).is_global());
        assert!(!coordinations.contains_key("__avenger_parallel_dim_mpg"));
        assert!(nested_configs.is_empty());

        let tree = EvaluatedFacetTree::init_with_caches(None, coordinations, nested_configs);
        assert!(SharingLevel::from(tree.channel_domain_coordination("mpg").scope).is_free());
        assert!(tree.has_free_channel_domain_sharing());

        Ok(())
    }

    #[tokio::test]
    async fn nested_level_domain_coordination_suppresses_implicit_channel_sharing()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x_with(avenger_chart_core::nested(["outer", "inner"]), |x| {
                        x.level(0, |l| l.domain_scope(CoordinationScope::Shared))
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Level(1))
                                    .nest_scope(NestScope::Shared)
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y(lit(0.0))
                    .y2(lit(1.0)),
            )
            .compile(&ctx)
            .await?;

        let (coordinations, nested_configs) = extract_channel_domain_metadata(&compiled.marks);
        let x = coordinations
            .get("x")
            .expect("nested x should have coordination metadata");

        assert_eq!(x.scope, CoordinationScope::Level(0));
        assert!(nested_configs.contains_key("x"));
        let tree =
            EvaluatedFacetTree::init_with_caches(None, coordinations, nested_configs.clone());
        assert!(
            tree.used_sharing_levels
                .contains(&SharingLevel::from_raw(1))
        );

        let explicit = crate::plot::Chart::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x_with(avenger_chart_core::nested(["outer", "inner"]), |x| {
                        x.with_domain_scope(CoordinationScope::Shared)
                            .level(0, |l| l.domain_scope(CoordinationScope::Shared))
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y(lit(0.0))
                    .y2(lit(1.0)),
            )
            .compile(&ctx)
            .await?;
        let (explicit_coordinations, _) = extract_channel_domain_metadata(&explicit.marks);

        assert_eq!(
            explicit_coordinations
                .get("x")
                .expect("explicit x coordination")
                .scope,
            CoordinationScope::Level(u8::MAX)
        );

        Ok(())
    }

    #[tokio::test]
    async fn explicit_domain_coordination_wins_over_implicit_related_channel()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x_with(col("x0"), |c| c.free_domain())
                    .x2(col("x1"))
                    .y(lit(0.0))
                    .y2(col("height")),
            )
            .compile(&ctx)
            .await?;

        let coordinations = extract_channel_domain_coordinations(&compiled.marks);
        let x = coordinations
            .get("x")
            .expect("x/x2 shared scale should have coordination");

        assert_eq!(x.scope, CoordinationScope::Level(0));

        Ok(())
    }

    #[tokio::test]
    async fn facet_config_axis_guide_visibility_reaches_evaluated_tree()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('top', 1.0, 1.0),
                    ('bottom', 2.0, 2.0)
                ) AS t(facet, x, y)",
            )
            .await?;
        let plot = crate::plot::Chart::<FacetRow>::new().data(df).mark(
            Subplot::new(
                crate::plot::Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))),
            )
            .row_with(col("facet"), |c| {
                c.axis_guide_visibility(AxisGuideVisibilityPolicy::All)
            }),
        );
        let compiled = plot.compile(&ctx).await?;
        let tree = EvaluatedFacetTree::from_compiled_plot(&compiled, &ctx).await?;
        let top_path = vec![scalar("top")];
        let visibility = tree
            .channel_axis_visibility_for_path_checked(&top_path, AxisPosition::Bottom, 255)
            .expect("top facet path");

        assert!(visibility.show_labels);
        assert!(visibility.show_title);
        Ok(())
    }

    #[test]
    fn test_axis_visibility_for_path_checked_valid_path_returns_some() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("East"), scalar("Eng"), scalar("A")];
        let visibility = tree.axis_visibility_for_path_checked(&path, AxisPosition::Left, 0);
        assert!(visibility.is_some());
    }

    #[test]
    fn test_axis_visibility_for_path_checked_invalid_path_returns_none() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("North"), scalar("Eng"), scalar("A")];
        let visibility = tree.axis_visibility_for_path_checked(&path, AxisPosition::Left, 0);
        assert!(visibility.is_none());
    }

    #[test]
    fn test_axis_visibility_for_path_wrapper_defaults_invalid_to_visible() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("North"), scalar("Eng"), scalar("A")];
        let visibility = tree.axis_visibility_for_path(&path, AxisPosition::Left, 0);
        assert!(visibility.show_labels);
        assert!(visibility.show_title);
    }

    #[test]
    fn test_channel_axis_visibility_for_path_checked_valid_path_returns_some() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("East"), scalar("Eng"), scalar("A")];
        let visibility =
            tree.channel_axis_visibility_for_path_checked(&path, AxisPosition::Bottom, 1);
        assert!(visibility.is_some());
    }

    #[test]
    fn test_channel_axis_visibility_for_path_checked_invalid_path_returns_none() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("North"), scalar("Eng"), scalar("A")];
        let visibility =
            tree.channel_axis_visibility_for_path_checked(&path, AxisPosition::Bottom, 1);
        assert!(visibility.is_none());
    }

    #[test]
    fn test_channel_axis_visibility_for_path_wrapper_defaults_invalid_to_visible() {
        use avenger_chart_core::AxisPosition;

        let tree = build_enumeration_test_tree();
        let path = vec![scalar("North"), scalar("Eng"), scalar("A")];
        let visibility = tree.channel_axis_visibility_for_path(&path, AxisPosition::Bottom, 1);
        assert!(visibility.show_labels);
        assert!(visibility.show_title);
    }

    fn two_row_policy_tree(policy: AxisGuideVisibilityPolicy) -> EvaluatedFacetTree {
        let rows = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "row".to_string(),
            None,
            vec![scalar("top"), scalar("bottom")],
        )
        .with_axis_guide_visibility(AxisGuideVisibilityConfig::same(policy));
        let mut columns = IndexMap::new();
        columns.insert(scalar("only"), Box::new(rows));
        EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            0,
            "column".to_string(),
            None,
            columns,
        )))
    }

    #[test]
    fn axis_guide_visibility_all_shows_inner_axis_labels() {
        let tree = two_row_policy_tree(AxisGuideVisibilityPolicy::All);
        let top_path = vec![scalar("only"), scalar("top")];
        let visibility = tree
            .channel_axis_visibility_for_path_checked(&top_path, AxisPosition::Bottom, 255)
            .expect("valid path");

        assert!(visibility.show_labels);
        assert!(visibility.show_title);
    }

    #[test]
    fn axis_guide_visibility_outer_edges_hides_inner_axis_labels_even_when_free() {
        let tree = two_row_policy_tree(AxisGuideVisibilityPolicy::OuterEdges);
        let top_path = vec![scalar("only"), scalar("top")];
        let bottom_path = vec![scalar("only"), scalar("bottom")];

        let top = tree
            .channel_axis_visibility_for_path_checked(&top_path, AxisPosition::Bottom, 0)
            .expect("valid top path");
        let bottom = tree
            .channel_axis_visibility_for_path_checked(&bottom_path, AxisPosition::Bottom, 0)
            .expect("valid bottom path");

        assert!(!top.show_labels);
        assert!(!top.show_title);
        assert!(bottom.show_labels);
        assert!(bottom.show_title);
    }

    #[test]
    fn test_channel_axis_visibility_non_empty_mode_relocates_bottom_owner() {
        use avenger_chart_core::AxisPosition;

        let tree = build_hole_visibility_test_tree();
        let path = vec![scalar("narrow"), scalar("Iris-setosa")];

        let domain_visibility = tree
            .channel_axis_visibility_for_path_checked_with_mode(
                &path,
                AxisPosition::Bottom,
                255,
                AxisOwnershipMode::DomainSlots,
            )
            .unwrap();
        assert!(!domain_visibility.show_labels);
        assert!(!domain_visibility.show_title);

        let non_empty_visibility = tree
            .channel_axis_visibility_for_path_checked_with_mode(
                &path,
                AxisPosition::Bottom,
                255,
                AxisOwnershipMode::NonEmptySlots,
            )
            .unwrap();
        assert!(non_empty_visibility.show_labels);
        assert!(non_empty_visibility.show_title);
    }

    #[test]
    fn test_channel_axis_visibility_non_empty_mode_relocates_right_owner() {
        use avenger_chart_core::AxisPosition;

        let tree = build_hole_visibility_test_tree();
        let path = vec![scalar("narrow"), scalar("Iris-setosa")];

        let domain_visibility = tree
            .channel_axis_visibility_for_path_checked_with_mode(
                &path,
                AxisPosition::Right,
                255,
                AxisOwnershipMode::DomainSlots,
            )
            .unwrap();
        assert!(!domain_visibility.show_labels);
        assert!(!domain_visibility.show_title);

        let non_empty_visibility = tree
            .channel_axis_visibility_for_path_checked_with_mode(
                &path,
                AxisPosition::Right,
                255,
                AxisOwnershipMode::NonEmptySlots,
            )
            .unwrap();
        assert!(non_empty_visibility.show_labels);
        assert!(non_empty_visibility.show_title);
    }

    #[test]
    fn test_channel_axis_visibility_domain_mode_matches_default_path() {
        use avenger_chart_core::AxisPosition;

        let tree = build_hole_visibility_test_tree();
        let path = vec![scalar("wide"), scalar("Iris-virginica")];

        let default_visibility =
            tree.channel_axis_visibility_for_path_checked(&path, AxisPosition::Bottom, 255);
        let explicit_domain_visibility = tree.channel_axis_visibility_for_path_checked_with_mode(
            &path,
            AxisPosition::Bottom,
            255,
            AxisOwnershipMode::DomainSlots,
        );

        assert_eq!(default_visibility, explicit_domain_visibility);
    }

    #[test]
    fn test_channel_axis_title_uses_branch_local_edge_owner() {
        use avenger_chart_core::AxisPosition;

        let tree = build_free_row_title_test_tree();

        let narrow_top = vec![scalar("narrow"), scalar("Iris-setosa")];
        let medium_bottom = vec![scalar("medium"), scalar("Iris-virginica")];

        let narrow_visibility =
            tree.channel_axis_visibility_for_path_checked(&narrow_top, AxisPosition::Bottom, 0);
        let medium_visibility =
            tree.channel_axis_visibility_for_path_checked(&medium_bottom, AxisPosition::Bottom, 0);

        let narrow_visibility = narrow_visibility.unwrap();
        let medium_visibility = medium_visibility.unwrap();

        assert!(narrow_visibility.show_title);
        assert!(medium_visibility.show_title);
    }

    #[test]
    fn ragged_free_nested_rows_expose_orthogonal_sibling_physical_slot_count() {
        let tree = build_free_row_title_test_tree();
        let narrow_path = vec![scalar("narrow")];

        assert_eq!(tree.min_slot_count_for_facet(&narrow_path), None);
        assert_eq!(
            tree.ragged_orthogonal_slot_count_for_facet(&narrow_path),
            Some(2)
        );
        assert_eq!(
            tree.enumerate_values_for_facet(&narrow_path, SharingLevel::FREE.raw()),
            Some(vec![scalar("Iris-setosa")])
        );
    }

    #[test]
    fn test_channel_axis_title_non_empty_mode_relocates_to_visible_edge_owner() {
        use avenger_chart_core::AxisPosition;

        let tree = build_free_row_title_test_tree();
        let narrow_top = vec![scalar("narrow"), scalar("Iris-setosa")];

        let narrow_visibility = tree
            .channel_axis_visibility_for_path_checked_with_mode(
                &narrow_top,
                AxisPosition::Bottom,
                0,
                AxisOwnershipMode::NonEmptySlots,
            )
            .unwrap();

        assert!(narrow_visibility.show_title);
    }

    #[test]
    fn test_path_resolution_matches_utf8_and_utf8view_values() {
        use avenger_chart_core::AxisPosition;

        let row_leaf = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "species".to_string(),
            None,
            vec![scalar_view("Iris-setosa"), scalar_view("Iris-virginica")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar_view("narrow"), Box::new(row_leaf));
        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "petal_width_bin".to_string(),
            None,
            children,
        );

        let tree = EvaluatedFacetTree::new(Some(root));
        let utf8_path = vec![scalar("narrow"), scalar("Iris-setosa")];

        let resolved = tree
            .resolve_path_info(&utf8_path)
            .expect("Utf8 path should resolve against Utf8View tree values");
        assert_eq!(resolved.indices, vec![0, 0]);
        assert!(tree.cell_exists(&utf8_path));

        let visibility =
            tree.channel_axis_visibility_for_path_checked(&utf8_path, AxisPosition::Left, 255);
        assert!(
            visibility.is_some(),
            "Path-matched cartesian visibility should not fall back to unresolved"
        );
    }

    #[test]
    fn test_cell_predicate() {
        use datafusion::logical_expr::col;

        // Build a 3-level hierarchy: Region (Col) > Department (Row) > Team (Row)
        // With field expressions so we can test predicate generation
        let team_leaf_eng = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            Some(col("team")),
            vec![scalar("A"), scalar("B")],
        );
        let team_leaf_ops = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "team".to_string(),
            Some(col("team")),
            vec![scalar("X"), scalar("Y")],
        );

        let mut dept_children_east = IndexMap::new();
        dept_children_east.insert(scalar("Eng"), Box::new(team_leaf_eng.clone()));
        dept_children_east.insert(scalar("Ops"), Box::new(team_leaf_ops.clone()));
        let dept_node_east = PartitionNode::branch(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            Some(col("dept")),
            dept_children_east,
        );

        let mut dept_children_west = IndexMap::new();
        dept_children_west.insert(scalar("Eng"), Box::new(team_leaf_eng));
        dept_children_west.insert(scalar("Ops"), Box::new(team_leaf_ops));
        let dept_node_west = PartitionNode::branch(
            FacetDirection::Row,
            0,
            "dept".to_string(),
            Some(col("dept")),
            dept_children_west,
        );

        let mut region_children = IndexMap::new();
        region_children.insert(scalar("East"), Box::new(dept_node_east));
        region_children.insert(scalar("West"), Box::new(dept_node_west));
        let region_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            Some(col("region")),
            region_children,
        );

        let spec = EvaluatedFacetTree::new(Some(region_node));
        assert_eq!(spec.depth(), 3);

        // Test path: ["East", "Eng", "A"]
        let path = vec![scalar("East"), scalar("Eng"), scalar("A")];

        // sharing_level=0 (Free): include all 3 levels
        let pred = spec.cell_predicate(&path, 0);
        assert!(pred.is_some());
        let pred_str = format!("{}", pred.unwrap());
        assert!(pred_str.contains("region"));
        assert!(pred_str.contains("dept"));
        assert!(pred_str.contains("team"));

        let partition_exprs = spec.partition_exprs_between_sharing_levels(
            &path,
            SharingLevel::GLOBAL,
            SharingLevel::FREE,
        );
        assert_eq!(
            partition_exprs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["region", "dept", "team"]
        );

        let partition_exprs = spec.partition_exprs_between_sharing_levels(
            &path,
            SharingLevel::from_raw(1),
            SharingLevel::FREE,
        );
        assert_eq!(
            partition_exprs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["team"]
        );

        // sharing_level=1 (Level(1)): skip last 1 level, include 2
        let pred = spec.cell_predicate(&path, 1);
        assert!(pred.is_some());
        let pred_str = format!("{}", pred.unwrap());
        assert!(pred_str.contains("region"));
        assert!(pred_str.contains("dept"));
        assert!(!pred_str.contains("team"));

        // sharing_level=2 (Level(2)): skip last 2 levels, include 1
        let pred = spec.cell_predicate(&path, 2);
        assert!(pred.is_some());
        let pred_str = format!("{}", pred.unwrap());
        assert!(pred_str.contains("region"));
        assert!(!pred_str.contains("dept"));
        assert!(!pred_str.contains("team"));

        // sharing_level=3 (Level(3)): skip all 3 levels, return None
        let pred = spec.cell_predicate(&path, 3);
        assert!(pred.is_none());

        // sharing_level=255 (Shared): always return None
        let pred = spec.cell_predicate(&path, 255);
        assert!(pred.is_none());

        // Empty path should return None
        let empty_path: Vec<ScalarValue> = vec![];
        let pred = spec.cell_predicate(&empty_path, 0);
        assert!(pred.is_none());

        // Empty tree should return None
        let empty_spec = EvaluatedFacetTree::empty();
        let pred = empty_spec.cell_predicate(&path, 0);
        assert!(pred.is_none());
    }

    #[test]
    fn test_level_counts_empty() {
        let spec = EvaluatedFacetTree::empty();
        assert_eq!(spec.level_counts(), Vec::<usize>::new());
    }

    #[test]
    fn test_level_counts_single_level() {
        let root = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops"), scalar("Sales")],
        );
        let spec = EvaluatedFacetTree::new(Some(root));
        assert_eq!(spec.level_counts(), vec![3]);
    }

    #[test]
    fn test_level_counts_two_levels() {
        // Col (2 values) > Row (3 values each)
        let row_node = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops"), scalar("Sales")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("East"), Box::new(row_node.clone()));
        children.insert(scalar("West"), Box::new(row_node));

        let col_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            children,
        );

        let spec = EvaluatedFacetTree::new(Some(col_node));
        assert_eq!(spec.level_counts(), vec![2, 3]);
    }

    #[test]
    fn test_path_values_from_indices_empty() {
        let spec = EvaluatedFacetTree::empty();
        assert_eq!(
            spec.path_values_from_indices(&[]),
            Vec::<ScalarValue>::new()
        );
        assert_eq!(
            spec.path_values_from_indices(&[0]),
            Vec::<ScalarValue>::new()
        );
    }

    #[test]
    fn test_path_values_from_indices_single_level() {
        let root = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops"), scalar("Sales")],
        );
        let spec = EvaluatedFacetTree::new(Some(root));

        // Index 0 -> "Eng"
        assert_eq!(spec.path_values_from_indices(&[0]), vec![scalar("Eng")]);
        // Index 1 -> "Ops"
        assert_eq!(spec.path_values_from_indices(&[1]), vec![scalar("Ops")]);
        // Index 2 -> "Sales"
        assert_eq!(spec.path_values_from_indices(&[2]), vec![scalar("Sales")]);
        // Index 3 -> out of bounds, empty
        assert_eq!(
            spec.path_values_from_indices(&[3]),
            Vec::<ScalarValue>::new()
        );
    }

    #[test]
    fn test_path_values_from_indices_two_levels() {
        // Col (East, West) > Row (Eng, Ops)
        let row_node = PartitionNode::leaf(
            FacetDirection::Row,
            0,
            "department".to_string(),
            None,
            vec![scalar("Eng"), scalar("Ops")],
        );

        let mut children = IndexMap::new();
        children.insert(scalar("East"), Box::new(row_node.clone()));
        children.insert(scalar("West"), Box::new(row_node));

        let col_node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "region".to_string(),
            None,
            children,
        );

        let spec = EvaluatedFacetTree::new(Some(col_node));

        // [0] -> ["East"]
        assert_eq!(spec.path_values_from_indices(&[0]), vec![scalar("East")]);
        // [1] -> ["West"]
        assert_eq!(spec.path_values_from_indices(&[1]), vec![scalar("West")]);
        // [0, 0] -> ["East", "Eng"]
        assert_eq!(
            spec.path_values_from_indices(&[0, 0]),
            vec![scalar("East"), scalar("Eng")]
        );
        // [1, 1] -> ["West", "Ops"]
        assert_eq!(
            spec.path_values_from_indices(&[1, 1]),
            vec![scalar("West"), scalar("Ops")]
        );
        // [0, 2] -> ["East"] (second index out of bounds)
        assert_eq!(spec.path_values_from_indices(&[0, 2]), vec![scalar("East")]);
    }
}
