//! Evaluated facet structure for visibility, filtering, and layout.
//!
//! This module provides a unified data structure that holds everything about
//! the evaluated facet hierarchy. It's built once from data at the start of
//! `CompiledPlot::evaluate()`, then queried throughout measurement and rendering for:
//! - Visibility decisions (axis ticks, titles, facet labels)
//! - Filter predicates for data slicing
//! - Domain values for iteration
//! - Position and count information for layout

use crate::error::AvengerChartError;
use crate::facet::keys::FacetKeyExtractor;
use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};
use crate::guide::FacetDirection;
use crate::marks::CompiledMark;
use crate::plot::CompiledPlot;
use crate::serialization::LogicalPlanNodeExt;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Cache for shared domain values to avoid redundant queries.
/// Key is the field name; value is the ordered list of distinct values.
/// Only used for shared domains (sharing >= current_depth) where we query unfiltered data.
type SharedDomainCache = HashMap<String, Vec<ScalarValue>>;

// Re-export AxisPosition for use in visibility queries
pub use crate::facet::context::AxisPosition;

/// Evaluated facet structure - built once from data at evaluate() time, queried throughout.
///
/// This is the single source of truth for all facet-related operations.
/// It captures the tree structure of partition values (handling non-shared domains),
/// and provides methods for visibility, filtering, and layout queries.
///
/// Note: This struct contains ONLY the partition tree structure. All configuration
/// (scale sharing levels, axis positions) is passed as parameters to query methods.
/// This allows the same facet spec to be used with different configurations.
#[derive(Debug, Clone)]
pub struct EvaluatedFacetTree {
    /// Tree of partition values (handles non-shared domains)
    root: Option<PartitionNode>,
}

/// A node in the partition tree.
///
/// Each node represents one partition level in the facet hierarchy.
/// For non-shared domains, children vary per parent value.
#[derive(Debug, Clone)]
pub struct PartitionNode {
    /// Direction of this partition (Row or Column)
    pub direction: FacetDirection,
    /// Domain sharing level for this partition
    pub sharing: u8,
    /// Field name for this partition
    pub field: String,
    /// Field expression for filtering (e.g., col("department"))
    pub field_expr: Option<Expr>,
    /// Content: either leaf values or branch with children
    pub content: PartitionContent,
}

/// Content of a partition node - either leaf values or branch with children.
#[derive(Debug, Clone)]
pub enum PartitionContent {
    /// Leaf node: contains the domain values at the innermost level
    Leaf { values: Vec<ScalarValue> },
    /// Branch node: maps each value to a child partition node
    /// Children are boxed to reduce async future state size and avoid stack overflow.
    Branch {
        children: IndexMap<ScalarValue, Box<PartitionNode>>,
    },
}

impl EvaluatedFacetTree {
    /// Create a new EvaluatedFacetSpec with the given partition tree.
    ///
    /// Note: All configuration (scale sharing levels, axis positions) is passed
    /// as parameters to query methods like `subplot_visibility`.
    pub fn new(root: Option<PartitionNode>) -> Self {
        Self { root }
    }

    /// Create an empty spec (no faceting).
    pub fn empty() -> Self {
        Self { root: None }
    }

    /// Get the partition tree root, if any.
    pub fn root(&self) -> Option<&PartitionNode> {
        self.root.as_ref()
    }

    /// Get the depth of the partition hierarchy.
    pub fn depth(&self) -> usize {
        fn count_depth(node: &PartitionNode) -> usize {
            match &node.content {
                PartitionContent::Leaf { .. } => 1,
                PartitionContent::Branch { children } => {
                    // All children should have same depth, take first
                    1 + children
                        .values()
                        .next()
                        .map(|b| count_depth(b.as_ref()))
                        .unwrap_or(0)
                }
            }
        }
        self.root.as_ref().map(count_depth).unwrap_or(0)
    }

    // ========================================================================
    // Building
    // ========================================================================

    /// Build from a compiled plot by discovering facet structure and querying data.
    ///
    /// This performs the pre-pass: walks the mark tree to find facet marks,
    /// queries distinct values for each partition, and builds the tree structure.
    /// Uses a cache to avoid redundant queries for shared domains.
    pub async fn from_compiled_plot(
        plot: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<Self, AvengerChartError> {
        // Get the DataFrame from plot-level data or first mark with data
        let df = get_dataframe_from_plot(plot, ctx);

        let df = match df {
            Some(df) => df,
            None => {
                // No data available - return empty spec
                // This can happen for plots without faceting or without data
                return Ok(Self::empty());
            }
        };

        // Cache for shared domain values to avoid redundant queries
        let mut domain_cache = SharedDomainCache::new();

        // Build partition tree by walking marks
        // Start at depth 1 (outermost facet level)
        let root = build_partition_tree(&plot.marks, &df, ctx, None, 1, &mut domain_cache).await?;

        Ok(Self { root })
    }

    // ========================================================================
    // Query methods
    // ========================================================================

    /// Get filter predicate for a cell at the given path, respecting sharing level.
    ///
    /// # Arguments
    /// * `path` - Sequence of values identifying the cell, from outermost to innermost level.
    ///            Each value corresponds to one level of the partition tree.
    /// * `sharing_level` - Channel's sharing level:
    ///   - 0 (Free): include all levels in the filter
    ///   - 1+ (Level(N)): include only levels deeper than (depth - N)
    ///   - 255 (Shared): return None (use full data)
    ///
    /// # Returns
    /// - `Some(Expr)` with the filter predicate for the specified cell and sharing level
    /// - `None` if the path is invalid, or if sharing_level is Shared (255)
    ///
    /// # Example
    /// For a 3-level hierarchy (Region > Department > Team) with path ["East", "Eng", "A"]:
    /// - sharing_level=0 (Free): `region="East" AND dept="Eng" AND team="A"`
    /// - sharing_level=1 (Level(1)): `region="East" AND dept="Eng"` (skip last 1 level)
    /// - sharing_level=2 (Level(2)): `region="East"` (skip last 2 levels)
    /// - sharing_level=3+ (Level(3+) or Shared): `None` (use full data)
    pub fn cell_predicate(&self, path: &[ScalarValue], sharing_level: u8) -> Option<Expr> {
        // Shared (255) means use full data - no filter needed
        if sharing_level == 255 {
            return None;
        }

        let Some(root) = &self.root else {
            return None;
        };

        // Collect (field_expr, value) pairs by walking the tree
        let mut filters: Vec<(Expr, ScalarValue)> = Vec::new();
        let mut current_node = root;

        for (level_idx, value) in path.iter().enumerate() {
            // Get the field expression for this level
            let field_expr = current_node.field_expr.clone()?;
            filters.push((field_expr, value.clone()));

            // Navigate to next level if not at the end of path
            if level_idx + 1 < path.len() {
                current_node = current_node.child(value)?;
            }
        }

        if filters.is_empty() {
            return None;
        }

        // Determine how many levels to include based on sharing_level
        // partition_depth = max(0, depth - sharing_level)
        // We include the first partition_depth levels
        let total_depth = filters.len();
        let levels_to_skip = sharing_level as usize;
        let levels_to_include = total_depth.saturating_sub(levels_to_skip);

        if levels_to_include == 0 {
            // Sharing level encompasses all levels - use full data
            return None;
        }

        // Build AND expression from the first `levels_to_include` filters
        let mut result: Option<Expr> = None;
        for (field_expr, value) in filters.into_iter().take(levels_to_include) {
            let eq_expr = field_expr.eq(lit(value));
            result = Some(match result {
                Some(existing) => existing.and(eq_expr),
                None => eq_expr,
            });
        }

        result
    }

    /// Navigate to a partition node at a given path.
    ///
    /// # Arguments
    /// * `path` - Sequence of values identifying the position, from outermost to target level.
    ///
    /// # Returns
    /// The partition node at the specified path, or None if the path is invalid.
    pub fn node_at_path(&self, path: &[ScalarValue]) -> Option<&PartitionNode> {
        let root = self.root.as_ref()?;

        if path.is_empty() {
            return Some(root);
        }

        let mut current = root;
        for value in path {
            current = current.child(value)?;
        }
        Some(current)
    }

    /// Get the domain values of a nested (inner) facet given the outer path.
    ///
    /// This navigates to the node at `outer_path`, then returns the domain values
    /// of its child partition (the inner facet). Useful for getting the inner
    /// facet's domain without re-querying.
    ///
    /// # Arguments
    /// * `outer_path` - Path to the outer facet node (empty for root level)
    ///
    /// # Returns
    /// The domain values of the inner facet, or None if there's no inner facet.
    pub fn inner_domain_at_path(&self, outer_path: &[ScalarValue]) -> Option<Vec<ScalarValue>> {
        let outer_node = if outer_path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(outer_path)?
        };

        // For a branch node, all children should have the same structure
        // Get the first child to access the inner domain
        match &outer_node.content {
            PartitionContent::Leaf { .. } => None, // No inner facet
            PartitionContent::Branch { children } => {
                // The inner domain values are the values of the first child node
                // (for shared domains, all children have the same domain)
                children
                    .values()
                    .next()
                    .map(|child| child.values().cloned().collect())
            }
        }
    }

    /// Compute the maximum inner cell count across all outer values.
    ///
    /// This is used for uniform Free scaling - when the inner facet uses Free
    /// scaling mode, we need to know the maximum number of inner cells across
    /// all outer cells to ensure uniform sizing.
    ///
    /// # Arguments
    /// * `outer_path` - Path to the outer facet node (empty for root level)
    ///
    /// # Returns
    /// The maximum number of inner cells across all outer values, or None if
    /// there's no inner facet or the path is invalid.
    pub fn max_inner_cell_count(&self, outer_path: &[ScalarValue]) -> Option<usize> {
        let outer_node = if outer_path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(outer_path)?
        };

        match &outer_node.content {
            PartitionContent::Leaf { .. } => None,
            PartitionContent::Branch { children } => {
                let max_count = children
                    .values()
                    .map(|child| child.domain_count())
                    .max()
                    .unwrap_or(1);
                Some(max_count.max(1)) // Floor at 1 to prevent divide-by-zero
            }
        }
    }

    /// Get all inner domain values for each outer cell (for Free scaling).
    ///
    /// Returns a map from outer value to the inner domain for that cell.
    /// Useful when inner domains vary per outer cell (Free scaling).
    ///
    /// # Arguments
    /// * `outer_path` - Path to the outer facet node (empty for root level)
    ///
    /// # Returns
    /// Map of outer value -> inner domain values, or None if invalid.
    pub fn inner_domains_per_cell(
        &self,
        outer_path: &[ScalarValue],
    ) -> Option<IndexMap<ScalarValue, Vec<ScalarValue>>> {
        let outer_node = if outer_path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(outer_path)?
        };

        match &outer_node.content {
            PartitionContent::Leaf { .. } => None,
            PartitionContent::Branch { children } => {
                let result: IndexMap<ScalarValue, Vec<ScalarValue>> = children
                    .iter()
                    .map(|(outer_val, child)| {
                        let inner_vals: Vec<ScalarValue> = child.values().cloned().collect();
                        (outer_val.clone(), inner_vals)
                    })
                    .collect();
                Some(result)
            }
        }
    }

    /// Get the union of all inner domain values across all outer cells.
    ///
    /// This returns the sorted set of all distinct inner domain values, which is
    /// equivalent to querying `DISTINCT inner_field` from the full dataset.
    /// Used for inner facet domain computation in `detect_nested_facet_and_compute_coordination`.
    ///
    /// # Arguments
    /// * `outer_path` - Path to the outer facet node (empty for root level)
    ///
    /// # Returns
    /// Sorted vector of all distinct inner domain values, or None if invalid.
    pub fn inner_domain_union(&self, outer_path: &[ScalarValue]) -> Option<Vec<ScalarValue>> {
        use crate::facet::scalar_cmp::scalar_total_cmp;

        let outer_node = if outer_path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(outer_path)?
        };

        match &outer_node.content {
            PartitionContent::Leaf { .. } => None,
            PartitionContent::Branch { children } => {
                // Collect all unique values across all children
                let mut all_values: Vec<ScalarValue> = children
                    .values()
                    .flat_map(|child| child.values().cloned())
                    .collect();

                // Sort and deduplicate
                all_values.sort_by(scalar_total_cmp);
                all_values.dedup();

                Some(all_values)
            }
        }
    }

    /// Get the outer domain values (values at the current node level).
    ///
    /// # Arguments
    /// * `path` - Path to the node (empty for root level)
    ///
    /// # Returns
    /// Vector of domain values at this level, or None if path is invalid.
    pub fn domain_values_at(&self, path: &[ScalarValue]) -> Option<Vec<ScalarValue>> {
        let node = if path.is_empty() {
            self.root.as_ref()?
        } else {
            self.node_at_path(path)?
        };

        Some(node.values().cloned().collect())
    }

    /// Check if this spec has any facet structure (non-empty).
    pub fn has_facets(&self) -> bool {
        self.root.is_some()
    }

    /// Check if the spec has a nested facet (depth >= 2).
    pub fn has_nested_facets(&self) -> bool {
        self.depth() >= 2
    }

    // ========================================================================
    // Additional query methods - commented out until needed
    // ========================================================================

    /*
    // ----- Visibility queries -----

    /// Path to a specific subplot in the facet hierarchy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SubplotPath {
        pub values: Vec<ScalarValue>,
    }

    /// Visibility decisions for an innermost subplot (Cartesian).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct SubplotVisibility {
        pub show_x_ticks: bool,
        pub show_y_ticks: bool,
        pub show_x_title: bool,
        pub show_y_title: bool,
    }

    /// Visibility decisions for an intermediate facet guide level.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FacetGuideVisibility {
        pub direction: FacetDirection,
        pub show_facet_labels: bool,
    }

    /// Position and count information for layout calculations.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PositionInfo {
        pub position_path: Vec<usize>,
        pub level_counts: Vec<usize>,
    }

    /// Get visibility decisions for an innermost subplot.
    pub fn subplot_visibility(
        &self,
        facet_values: &[ScalarValue],
        x_sharing: u8,
        y_sharing: u8,
        x_position: AxisPosition,
        y_position: AxisPosition,
    ) -> SubplotVisibility {
        // ... implementation ...
    }

    /// Get visibility decisions for a facet guide at an intermediate level.
    pub fn facet_guide_visibility(
        &self,
        facet_values: &[ScalarValue],
    ) -> Option<FacetGuideVisibility> {
        // ... implementation ...
    }

    // ----- Filter predicates -----

    /// Get the filter predicate for a subplot.
    pub fn subplot_predicate(&self, facet_values: &[ScalarValue]) -> Option<Expr> {
        // ... implementation ...
    }

    /// Get the filter predicate for domain inference at a specific sharing level.
    pub fn domain_inference_predicate(
        &self,
        facet_values: &[ScalarValue],
        sharing_level: u8,
    ) -> Option<Expr> {
        // ... implementation ...
    }

    // ----- Domain values -----

    /// Get domain values at a level given the parent path.
    pub fn domain_values(&self, parent_values: &[ScalarValue]) -> Option<&[ScalarValue]> {
        // ... implementation ...
    }

    /// Get the partition node at a given path.
    pub fn partition_at(&self, parent_values: &[ScalarValue]) -> Option<&PartitionNode> {
        // ... implementation ...
    }

    // ----- Position and layout -----

    /// Get position and count information for a subplot.
    pub fn position_info(&self, facet_values: &[ScalarValue]) -> Option<PositionInfo> {
        // ... implementation ...
    }

    // ----- Iteration -----

    /// Iterate over all subplot paths in the facet structure.
    pub fn iter_subplots(&self) -> SubplotIterator {
        // ... implementation ...
    }

    /// Get the total number of subplots.
    pub fn subplot_count(&self) -> usize {
        // ... implementation ...
    }
    */
}

impl PartitionNode {
    /// Create a new leaf partition node.
    pub fn leaf(
        direction: FacetDirection,
        sharing: u8,
        field: String,
        field_expr: Option<Expr>,
        values: Vec<ScalarValue>,
    ) -> Self {
        Self {
            direction,
            sharing,
            field,
            field_expr,
            content: PartitionContent::Leaf { values },
        }
    }

    /// Create a new branch partition node.
    pub fn branch(
        direction: FacetDirection,
        sharing: u8,
        field: String,
        field_expr: Option<Expr>,
        children: IndexMap<ScalarValue, Box<PartitionNode>>,
    ) -> Self {
        Self {
            direction,
            sharing,
            field,
            field_expr,
            content: PartitionContent::Branch { children },
        }
    }

    /// Check if this is a leaf node (innermost partition).
    pub fn is_leaf(&self) -> bool {
        matches!(self.content, PartitionContent::Leaf { .. })
    }

    /// Get the domain values at this level.
    pub fn values(&self) -> Box<dyn Iterator<Item = &ScalarValue> + '_> {
        match &self.content {
            PartitionContent::Leaf { values } => Box::new(values.iter()),
            PartitionContent::Branch { children } => Box::new(children.keys()),
        }
    }

    /// Get the number of domain values at this level.
    pub fn domain_count(&self) -> usize {
        match &self.content {
            PartitionContent::Leaf { values } => values.len(),
            PartitionContent::Branch { children } => children.len(),
        }
    }

    /// Get child node for a specific value.
    pub fn child(&self, value: &ScalarValue) -> Option<&PartitionNode> {
        match &self.content {
            PartitionContent::Leaf { .. } => None,
            PartitionContent::Branch { children } => children.get(value).map(|b| b.as_ref()),
        }
    }
}

// ============================================================================
// Helper functions for building the partition tree
// ============================================================================

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
    if let Some(data_node) = &plot.data {
        if let Ok(logical_plan) = data_node.to_logical_plan(ctx) {
            return Some(DataFrame::new(ctx.state().clone(), logical_plan));
        }
    }

    None
}

/// Check if a DataFrame is an empty relation placeholder.
fn is_empty_relation(df: &DataFrame) -> bool {
    use datafusion::logical_expr::LogicalPlan;
    matches!(df.logical_plan(), LogicalPlan::EmptyRelation(_))
}

/// Extract a human-readable field name from an expression.
fn extract_field_name(expr: &Expr) -> String {
    match expr {
        Expr::Column(col) => col.name.clone(),
        Expr::Alias(alias) => alias.name.clone(),
        _ => expr.to_string(),
    }
}

/// Recursively build a partition tree from compiled marks.
///
/// # Arguments
/// * `marks` - The marks to search for facets
/// * `df` - The DataFrame to query for distinct values
/// * `ctx` - Session context for expression evaluation
/// * `parent_filter` - Optional filter predicate from parent partitions (for non-shared domains)
/// * `current_depth` - Current depth in the facet hierarchy (1 = outermost)
/// * `domain_cache` - Cache for shared domain values to avoid redundant queries
async fn build_partition_tree(
    marks: &[Arc<dyn CompiledMark>],
    df: &DataFrame,
    ctx: &SessionContext,
    parent_filter: Option<Expr>,
    current_depth: u8,
    domain_cache: &mut SharedDomainCache,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    for mark in marks {
        let mark_type = mark.mark_type();

        if mark_type == "facet_row" {
            if let Some(facet_row) = mark.as_any().downcast_ref::<CompiledFacetRow>() {
                return Box::pin(build_partition_node(
                    facet_row.state.data.channels(),
                    "row",
                    FacetDirection::Row,
                    &facet_row.compiled_subplot,
                    facet_row.facet_scale_sharing,
                    df,
                    ctx,
                    parent_filter,
                    current_depth,
                    domain_cache,
                ))
                .await;
            }
        } else if mark_type == "facet_col" {
            if let Some(facet_col) = mark.as_any().downcast_ref::<CompiledFacetCol>() {
                return Box::pin(build_partition_node(
                    facet_col.state.data.channels(),
                    "column",
                    FacetDirection::Column,
                    &facet_col.compiled_subplot,
                    facet_col.facet_scale_sharing,
                    df,
                    ctx,
                    parent_filter,
                    current_depth,
                    domain_cache,
                ))
                .await;
            }
        }
    }

    // No facet found
    Ok(None)
}

/// Build a partition node for a specific facet.
#[allow(clippy::too_many_arguments)]
async fn build_partition_node(
    channels: &IndexMap<String, crate::marks::ChannelValue>,
    channel_name: &str,
    direction: FacetDirection,
    subplot: &Arc<CompiledPlot>,
    scale_sharing: Option<crate::channel::config_traits::ScaleSharing>,
    df: &DataFrame,
    ctx: &SessionContext,
    parent_filter: Option<Expr>,
    current_depth: u8,
    domain_cache: &mut SharedDomainCache,
) -> Result<Option<PartitionNode>, AvengerChartError> {
    // Get channel value
    let channel_value = match channels.get(channel_name) {
        Some(cv) => cv,
        None => return Ok(None), // No facet channel
    };

    // Get field expression
    let field_expr = match channel_value.expr(ctx) {
        Some(expr) => expr,
        None => return Ok(None), // No expression
    };

    // Get sharing level
    let sharing = scale_sharing
        .or_else(|| channel_value.get_share_mode())
        .map(|s| s.to_level())
        .unwrap_or(0);

    // Extract field name
    let field = extract_field_name(&field_expr);

    // Determine whether to use filtered or unfiltered data for domain values
    // If sharing level >= current depth, domain is shared (use unfiltered data)
    // If sharing level < current depth (including Free/0), domain varies per parent (use filtered)
    let use_shared_domain = sharing >= current_depth;

    // Get distinct values, using cache for shared domains
    let values = if use_shared_domain {
        // For shared domains, check cache first (keyed by field name since we use unfiltered data)
        if let Some(cached) = domain_cache.get(&field) {
            cached.clone()
        } else {
            let vals = FacetKeyExtractor::extract_keys(df, &field_expr).await?;
            domain_cache.insert(field.clone(), vals.clone());
            vals
        }
    } else if parent_filter.is_some() {
        // For non-shared domains with a parent filter, query filtered data (no caching)
        let df_filtered = df.clone().filter(parent_filter.clone().unwrap())?;
        FacetKeyExtractor::extract_keys(&df_filtered, &field_expr).await?
    } else {
        // No parent filter - query unfiltered data
        FacetKeyExtractor::extract_keys(df, &field_expr).await?
    };

    if values.is_empty() {
        return Ok(None); // No values
    }

    // Check for nested facets in subplot
    let nested_facet = Box::pin(build_partition_tree(
        &subplot.marks,
        df,
        ctx,
        None,
        current_depth + 1,
        domain_cache,
    ))
    .await?;

    if nested_facet.is_some() {
        // Build branch node with children for each value
        let mut children = IndexMap::new();

        for value in &values {
            // Build filter for this value to pass to child
            let value_filter = field_expr.clone().eq(lit(value.clone()));
            let combined_filter = if let Some(pf) = &parent_filter {
                pf.clone().and(value_filter)
            } else {
                value_filter
            };

            // Recursively build child partition using the combined filter
            if let Some(child) = Box::pin(build_partition_tree(
                &subplot.marks,
                df,
                ctx,
                Some(combined_filter),
                current_depth + 1,
                domain_cache,
            ))
            .await?
            {
                children.insert(value.clone(), Box::new(child));
            }
        }

        if children.is_empty() {
            // No valid children - make leaf
            Ok(Some(PartitionNode::leaf(
                direction,
                sharing,
                field,
                Some(field_expr),
                values,
            )))
        } else {
            Ok(Some(PartitionNode::branch(
                direction,
                sharing,
                field,
                Some(field_expr),
                children,
            )))
        }
    } else {
        // No nested facets - leaf node
        Ok(Some(PartitionNode::leaf(
            direction,
            sharing,
            field,
            Some(field_expr),
            values,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(s: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(s.to_string()))
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
        // Col > Row with shared domains (same values regardless of parent)
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

        // Empty spec should return None
        let empty_spec = EvaluatedFacetTree::empty();
        let pred = empty_spec.cell_predicate(&path, 0);
        assert!(pred.is_none());
    }

    /*
    // Additional tests commented out until query methods are added back

    #[test]
    fn test_visibility_single_row_level1() {
        // ... implementation ...
    }

    #[test]
    fn test_visibility_level4_sharing_four_level_row() {
        // ... implementation ...
    }

    #[test]
    fn test_visibility_level2_sharing_four_level_row() {
        // ... implementation ...
    }

    #[test]
    fn test_visibility_col_row_nesting() {
        // ... implementation ...
    }

    #[test]
    fn test_iterator() {
        // ... implementation ...
    }

    #[test]
    fn test_position_info() {
        // ... implementation ...
    }

    #[test]
    fn test_domain_inference_predicate_four_level_row() {
        // ... implementation ...
    }

    #[test]
    fn test_domain_inference_predicate_col_row() {
        // ... implementation ...
    }
    */
}
