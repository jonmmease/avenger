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

pub use crate::partition::{PartitionContent, PartitionNode};

use crate::{
    error::AvengerChartError,
    facet::FacetDirection,
    facet::{
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        sharing_policy,
    },
    partition::{PartitionDimensionSpec, PartitionSlotCache, scalar_values_equivalent},
    plot::{
        CompiledPlot,
        compiled::{SharingGroupEdge, enumeration_ancestor_path},
    },
};
pub(crate) use avenger_chart_core::AxisOwnershipMode;
pub use avenger_chart_core::AxisVisibility;
use avenger_chart_core::{AxisPosition, CompiledMark, LogicalPlanNodeExt, SharingLevel};

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
    /// Channel-domain sharing levels extracted from innermost marks.
    /// Maps channel name (e.g., "x", "y") to sharing level (0=Free, N=Level(N), 255=Shared).
    /// Used for axis visibility decisions when CoordMeasurement is not available.
    channel_domain_sharing_levels: HashMap<String, SharingLevel>,
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
    /// Sharing levels observed in channels/nodes plus canonical levels `{0, 255}`.
    used_sharing_levels: Vec<SharingLevel>,
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
        channel_domain_sharing_levels: HashMap<String, SharingLevel>,
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
            channel_domain_sharing_levels,
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
        Self::init_with_caches(root, HashMap::new())
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
                .map(|(channel, level)| (channel, SharingLevel::from_raw(level)))
                .collect(),
        )
    }

    /// Create an empty tree (no faceting).
    pub fn empty() -> Self {
        Self::init_with_caches(None, HashMap::new())
    }

    /// Get the partition tree root, if any.
    pub fn root(&self) -> Option<&PartitionNode> {
        self.root.as_ref()
    }

    /// Get the depth of the partition hierarchy.
    pub fn depth(&self) -> usize {
        self.depth_cache
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
        // Get the DataFrame from plot-level data or first mark with data
        let df = get_dataframe_from_plot(plot, ctx);

        let df = match df {
            Some(df) => df,
            None => {
                // No data available; return an empty facet tree.
                // This can happen for plots without faceting or without data
                return Ok(Self::empty());
            }
        };

        // Cache for shared facet slot values to avoid redundant queries.
        let mut slot_cache = PartitionSlotCache::new();

        // Build partition tree by walking marks
        // Start at depth 1 (outermost facet level)
        let root = build_partition_tree(&plot.marks, &df, ctx, None, 1, &mut slot_cache).await?;

        // Extract channel-domain sharing levels from the innermost marks.
        let channel_domain_sharing_levels = extract_channel_domain_sharing_levels(&plot.marks);

        Ok(Self::new_with_channel_domain_sharing_levels(
            root,
            channel_domain_sharing_levels,
        ))
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
        for sharing_level in self.channel_domain_sharing_levels.values() {
            levels.insert(*sharing_level);
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
                direction: node.direction,
            });
        }

        let mut indices = Vec::with_capacity(path.len());
        let mut local_level_counts = Vec::with_capacity(path.len());
        let mut level_directions = Vec::with_capacity(path.len());

        for (level, value) in path.iter().enumerate() {
            local_level_counts.push(node.domain_count());
            level_directions.push(node.direction);

            let idx = match &node.content {
                PartitionContent::Leaf { values } => values
                    .iter()
                    .position(|v| scalar_values_equivalent(v, value))?,
                PartitionContent::Branch { children } => {
                    children.get_index_of(value).or_else(|| {
                        children
                            .keys()
                            .position(|child_value| scalar_values_equivalent(child_value, value))
                    })?
                }
            };
            indices.push(idx);

            if level + 1 < path.len() {
                node = node.child(value)?;
            }
        }

        Some(ResolvedFacetPathInfo {
            indices,
            local_level_counts,
            level_directions,
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

        let facet_depth = facet_path.len() as u8 + 1;
        let enumeration_path = enumeration_ancestor_path(facet_path, sharing_level, facet_depth);

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

        // Compute how many levels to include
        let levels_to_include = path.len().saturating_sub(sharing_level.raw() as usize);
        if levels_to_include == 0 {
            return None;
        }

        self.path_predicate(&path[..levels_to_include])
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

    /// Get the sharing level for a channel.
    ///
    /// Returns the sharing level stored during tree construction, or 255 (Shared)
    /// if the channel was not found. This is used for axis visibility decisions
    /// when the innermost subplot doesn't have access to CoordMeasurement.
    pub fn channel_domain_sharing_level(&self, channel: &str) -> u8 {
        self.channel_domain_sharing_level_typed(channel).raw()
    }

    pub(crate) fn channel_domain_sharing_level_typed(&self, channel: &str) -> SharingLevel {
        self.channel_domain_sharing_levels
            .get(channel)
            .copied()
            .unwrap_or(SharingLevel::GLOBAL)
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

        if matches!(ownership_mode, AxisOwnershipMode::DomainSlots) {
            return self.channel_axis_visibility_from_resolved(
                &resolved.indices,
                &resolved.local_level_counts,
                &resolved.level_directions,
                axis_position,
                sharing_level,
            );
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

        let labels_sharing = sharing_level.clamp_to_depth(relevant_depth as u8);
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

        AxisVisibility {
            show_labels: self.non_empty_owner_visible_for_cartesian_axis(
                path,
                resolved,
                axis_position,
                labels_sharing,
                labels_fallback,
            ),
            // In hole mode, title ownership follows the same non-empty edge
            // owner as labels. Otherwise a ragged row/column can lose its axis
            // title entirely when its geometric edge owner is a hole.
            show_title: self.non_empty_owner_visible_for_cartesian_axis(
                path,
                resolved,
                axis_position,
                title_sharing,
                title_fallback,
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
    /// # Sharing Groups
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
                PartitionContent::Branch { children } => {
                    if let Some((_, child)) = children.get_index(pos_idx) {
                        child.as_ref()
                    } else {
                        return AxisVisibility::visible();
                    }
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
                    PartitionContent::Branch { children } => {
                        if let Some((key, child)) = children.get_index(idx) {
                            values.push(key.clone());
                            current_node = Some(child.as_ref());
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

/// Extract channel-domain sharing levels from compiled marks by recursing through facet subplots.
///
/// This walks the mark tree to find the innermost (non-facet) marks and extracts
/// their channel-domain sharing levels. Returns a map from channel name to sharing level.
fn extract_channel_domain_sharing_levels(marks: &[Arc<dyn CompiledMark>]) -> HashMap<String, u8> {
    let mut result = HashMap::new();

    for mark in marks {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            // Recurse into subplot to find innermost marks
            let inner = extract_channel_domain_sharing_levels(&facet_mark.compiled_subplot().marks);
            result.extend(inner);
        } else {
            // Non-facet mark - extract channel-domain sharing levels.
            let data_context = mark.data_context();
            for (channel, channel_value) in data_context.channels() {
                if let Some(sharing) = channel_value.get_share_mode() {
                    result.insert(channel.clone(), sharing.to_level());
                }
            }
        }
    }

    result
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
    dimension: PartitionDimensionSpec,
    subplot: &'a CompiledPlot,
}

impl<'a> FacetPartitionMarkSpec<'a> {
    fn from_facet_mark(facet_mark: FacetSubplotRef<'a>, ctx: &SessionContext) -> Option<Self> {
        let (channels, channel_name, direction, subplot, slot_sharing) = match facet_mark {
            FacetSubplotRef::Row(facet_row) => (
                facet_row.compiled_state().data.channels(),
                "row",
                FacetDirection::Row,
                facet_row.compiled_subplot(),
                facet_row.facet_slot_sharing(),
            ),
            FacetSubplotRef::Col(facet_col) => (
                facet_col.compiled_state().data.channels(),
                "column",
                FacetDirection::Column,
                facet_col.compiled_subplot(),
                facet_col.facet_slot_sharing(),
            ),
        };

        let channel_value = channels.get(channel_name)?;
        let field_expr = channel_value.expr(ctx)?;
        let sharing = slot_sharing
            .or_else(|| channel_value.get_share_mode())
            .map(|s| s.to_level())
            .unwrap_or(0);

        Some(Self {
            dimension: PartitionDimensionSpec::new(direction, sharing, field_expr),
            subplot,
        })
    }
}

/// Recursively build a partition tree from compiled marks.
///
/// # Arguments
/// * `marks` - The marks to search for facets
/// * `df` - The DataFrame to query for distinct values
/// * `ctx` - Session context for expression evaluation
/// * `parent_filter` - Optional filter predicate from parent partitions (for non-shared slots)
/// * `current_depth` - Current depth in the facet hierarchy (1 = outermost)
/// * `slot_cache` - Cache for shared facet slot values to avoid redundant queries
async fn build_partition_tree(
    marks: &[Arc<dyn CompiledMark>],
    df: &DataFrame,
    ctx: &SessionContext,
    parent_filter: Option<Expr>,
    current_depth: u8,
    slot_cache: &mut PartitionSlotCache,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    for mark in marks {
        if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
            let Some(spec) = FacetPartitionMarkSpec::from_facet_mark(facet_mark, ctx) else {
                return Ok(None);
            };
            return Box::pin(build_partition_node(
                &spec,
                df,
                ctx,
                parent_filter,
                current_depth,
                slot_cache,
            ))
            .await;
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
    parent_filter: Option<Expr>,
    current_depth: u8,
    slot_cache: &mut PartitionSlotCache,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    let dimension = &spec.dimension;
    let observed_values = dimension.observed_values(df, parent_filter.clone()).await?;
    let values = dimension
        .domain_values(df, parent_filter.clone(), current_depth, slot_cache)
        .await?;

    if values.is_empty() {
        return Ok(None); // No values
    }

    // Check for nested facets in subplot
    let nested_facet = Box::pin(build_partition_tree(
        &spec.subplot.marks,
        df,
        ctx,
        None,
        current_depth + 1,
        slot_cache,
    ))
    .await?;

    if nested_facet.is_some() {
        // Build branch node with children for each value
        let mut children = IndexMap::new();

        for value in &values {
            // Build filter for this value to pass to child
            let value_filter = dimension.value_filter(value);
            let combined_filter = if let Some(pf) = &parent_filter {
                pf.clone().and(value_filter)
            } else {
                value_filter
            };

            // Recursively build child partition using the combined filter
            if let Some(child) = Box::pin(build_partition_tree(
                &spec.subplot.marks,
                df,
                ctx,
                Some(combined_filter),
                current_depth + 1,
                slot_cache,
            ))
            .await?
            {
                children.insert(value.clone(), Box::new(child));
            }
        }

        if children.is_empty() {
            // No valid children - make leaf
            Ok(Some(PartitionNode::leaf_with_observed(
                dimension.direction,
                dimension.sharing,
                dimension.field.clone(),
                Some(dimension.field_expr.clone()),
                values,
                observed_values,
            )))
        } else {
            Ok(Some(PartitionNode::branch_with_observed(
                dimension.direction,
                dimension.sharing,
                dimension.field.clone(),
                Some(dimension.field_expr.clone()),
                observed_values,
                children,
            )))
        }
    } else {
        // No nested facets - leaf node
        Ok(Some(PartitionNode::leaf_with_observed(
            dimension.direction,
            dimension.sharing,
            dimension.field.clone(),
            Some(dimension.field_expr.clone()),
            values,
            observed_values,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(s: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(s.to_string()))
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
