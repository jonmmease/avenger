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
use std::sync::Arc;

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
pub struct EvaluatedFacetSpec {
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

impl EvaluatedFacetSpec {
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

        // Build partition tree by walking marks
        // Start at depth 1 (outermost facet level)
        let root = build_partition_tree(&plot.marks, &df, ctx, None, 1).await?;

        Ok(Self { root })
    }

    // ========================================================================
    // Query methods - commented out until needed
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
async fn build_partition_tree(
    marks: &[Arc<dyn CompiledMark>],
    df: &DataFrame,
    ctx: &SessionContext,
    parent_filter: Option<Expr>,
    current_depth: u8,
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

    let df_for_domain = if use_shared_domain || parent_filter.is_none() {
        df.clone()
    } else {
        df.clone().filter(parent_filter.clone().unwrap())?
    };

    // Get distinct values
    let values = FacetKeyExtractor::extract_keys(&df_for_domain, &field_expr).await?;

    if values.is_empty() {
        return Ok(None); // No values
    }

    // Check for nested facets in subplot
    let nested_facet =
        Box::pin(build_partition_tree(&subplot.marks, df, ctx, None, current_depth + 1)).await?;

    if nested_facet.is_some() {
        // Build branch node with children for each value
        let mut children = IndexMap::new();

        for value in &values {
            // Build filter for this value
            let value_filter = field_expr.clone().eq(lit(value.clone()));
            let combined_filter = if let Some(pf) = &parent_filter {
                pf.clone().and(value_filter)
            } else {
                value_filter
            };

            // Recursively build child partition
            if let Some(child) = Box::pin(build_partition_tree(
                &subplot.marks,
                df,
                ctx,
                Some(combined_filter),
                current_depth + 1,
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
        let spec = EvaluatedFacetSpec::empty();
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

        let spec = EvaluatedFacetSpec::new(Some(root));

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

        let spec = EvaluatedFacetSpec::new(Some(col_node));

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

        let spec = EvaluatedFacetSpec::new(Some(root));

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
