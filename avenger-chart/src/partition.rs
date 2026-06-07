//! Generic data-partition tree primitives.
//!
//! Facets are the first consumer of this tree, but the structure itself is a
//! reusable description of nested data subsets. Future repeat, coordinate-
//! positioned subplots, and other child-frame containers can build on the same
//! value/domain/observed-value model.

use std::collections::HashMap;

use datafusion::{
    arrow::record_batch::RecordBatch,
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, LogicalPlan, lit},
};
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};
use indexmap::IndexMap;

use avenger_chart_core::{
    AxisGuideVisibilityConfig, contains_aggregate, params_to_datafusion, scalar_total_cmp,
};

use crate::{error::AvengerChartError, facet::FacetDirection};

/// Format a partition value for display in container guides.
pub(crate) fn format_partition_value(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(s))
        | ScalarValue::LargeUtf8(Some(s))
        | ScalarValue::Utf8View(Some(s)) => s.to_string(),
        ScalarValue::Int8(Some(n)) => n.to_string(),
        ScalarValue::Int16(Some(n)) => n.to_string(),
        ScalarValue::Int32(Some(n)) => n.to_string(),
        ScalarValue::Int64(Some(n)) => n.to_string(),
        ScalarValue::UInt8(Some(n)) => n.to_string(),
        ScalarValue::UInt16(Some(n)) => n.to_string(),
        ScalarValue::UInt32(Some(n)) => n.to_string(),
        ScalarValue::UInt64(Some(n)) => n.to_string(),
        ScalarValue::Float32(Some(n)) => format!("{n:.2}"),
        ScalarValue::Float64(Some(n)) => format!("{n:.2}"),
        ScalarValue::Boolean(Some(b)) => b.to_string(),
        _ => format!("{value:?}"),
    }
}

/// Cache statistics for semantic partition slot queries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PartitionSlotCacheStats {
    pub(crate) hits: usize,
    pub(crate) misses: usize,
}

/// Cache for semantic partition slot values.
///
/// Facet tree construction uses this for both displayed domain slots and
/// observed slots under concrete parent filters. Responsive wrap can then
/// rebuild physical rows/columns without repeating the semantic partition
/// queries.
#[derive(Clone, Debug, Default)]
pub(crate) struct PartitionSlotCache {
    values: HashMap<String, Vec<ScalarValue>>,
    stats: PartitionSlotCacheStats,
}

impl PartitionSlotCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get(&mut self, key: &str) -> Option<Vec<ScalarValue>> {
        if let Some(values) = self.values.get(key) {
            self.stats.hits += 1;
            Some(values.clone())
        } else {
            self.stats.misses += 1;
            None
        }
    }

    pub(crate) fn insert(&mut self, key: String, values: Vec<ScalarValue>) {
        self.values.insert(key, values);
    }

    pub(crate) fn merge_from(&mut self, other: PartitionSlotCache) {
        self.values.extend(other.values);
        self.stats = other.stats;
    }

    pub(crate) fn stats(&self) -> PartitionSlotCacheStats {
        self.stats
    }
}

/// One dimension in a nested data-partition tree.
#[derive(Debug, Clone)]
pub(crate) struct PartitionDimensionSpec {
    /// Placement direction used by the first built-in partition consumer.
    pub(crate) direction: FacetDirection,
    /// Slot-sharing level for this partition dimension.
    pub(crate) sharing: u8,
    /// Field name for this partition dimension.
    pub(crate) field: String,
    /// Field expression used for grouping and filtering.
    pub(crate) field_expr: Expr,
    /// Optional aggregate/constant/partition expression used to order slots.
    pub(crate) order_expr: Option<Expr>,
    /// Sort order for `order_expr`; ties always use partition value ascending.
    pub(crate) order_descending: bool,
    /// Axis guide visibility policy for this partition dimension.
    pub(crate) axis_guide_visibility: AxisGuideVisibilityConfig,
}

/// A node in a nested data-partition tree.
///
/// Each node represents one partition level. `values` stores the domain slots
/// that the container should enumerate, while `observed_values` records the
/// values observed under the concrete parent filter.
#[derive(Debug, Clone)]
pub struct PartitionNode {
    /// Placement direction used by the first built-in partition consumer.
    pub direction: FacetDirection,
    /// Slot-sharing level for this partition.
    pub sharing: u8,
    /// Field name for this partition.
    pub field: String,
    /// Field expression for filtering.
    pub field_expr: Option<Expr>,
    /// Axis guide visibility policy for this partition level.
    pub axis_guide_visibility: AxisGuideVisibilityConfig,
    /// Domain slot values for this node.
    pub values: Vec<ScalarValue>,
    /// Values observed under the concrete parent-path filter for this node.
    pub observed_values: Vec<ScalarValue>,
    /// Minimum physical slots to reserve for this node.
    ///
    /// This is layout metadata rather than a semantic domain value list. Facet
    /// wrap uses it to preserve trailing holes in ragged physical rows without
    /// inventing fake data values or predicates.
    pub(crate) min_slot_count: Option<usize>,
    /// Partition content: either leaf values or branches keyed by values.
    pub content: PartitionContent,
}

/// Content of a partition node.
#[derive(Debug, Clone)]
pub enum PartitionContent {
    /// Leaf node: contains the innermost partition slot values.
    Leaf { values: Vec<ScalarValue> },
    /// Branch node: maps each slot value to a child partition node.
    ///
    /// Children are boxed to reduce async future state size and avoid stack
    /// overflow in deeply nested partition trees.
    Branch {
        children: IndexMap<ScalarValue, Box<PartitionNode>>,
    },
}

/// Classification for partition cells that do not contain rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartitionCellEmptyKind {
    /// The cell is part of the displayed domain but has no matching data rows.
    DataEmpty,
    /// The cell is present only because shared or explicit domain enumeration
    /// needs a placeholder for a value not present under this parent path.
    DomainPlaceholder,
}

/// Canonical metadata for one enumerated partition cell.
#[derive(Clone, Debug)]
pub(crate) struct PartitionCellPlan {
    pub(crate) value: ScalarValue,
    pub(crate) full_path: Vec<ScalarValue>,
    pub(crate) in_domain_slot: bool,
    pub(crate) has_data_rows: bool,
    pub(crate) empty_kind: PartitionCellEmptyKind,
    pub(crate) is_empty: bool,
    pub(crate) filter_predicate: Option<Expr>,
}

impl From<&PartitionCellPlan> for PartitionCellPlan {
    fn from(cell: &PartitionCellPlan) -> Self {
        cell.clone()
    }
}

/// Extract distinct partition keys from a DataFusion `DataFrame`.
pub struct PartitionKeyExtractor;

impl PartitionDimensionSpec {
    pub(crate) fn new(direction: FacetDirection, sharing: u8, field_expr: Expr) -> Self {
        let field = partition_field_name(&field_expr);
        Self {
            direction,
            sharing,
            field,
            field_expr,
            order_expr: None,
            order_descending: false,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
        }
    }

    pub(crate) fn with_ordering(
        mut self,
        order_expr: Option<Expr>,
        order_descending: bool,
    ) -> Self {
        self.order_expr = order_expr;
        self.order_descending = order_descending;
        self
    }

    pub(crate) fn with_axis_guide_visibility(
        mut self,
        visibility: AxisGuideVisibilityConfig,
    ) -> Self {
        self.axis_guide_visibility = visibility;
        self
    }

    #[cfg(test)]
    pub(crate) fn uses_shared_slots_at_depth(&self, current_depth: u8) -> bool {
        self.sharing >= current_depth
    }

    pub(crate) fn value_filter(&self, value: &ScalarValue) -> Expr {
        self.field_expr.clone().eq(lit(value.clone()))
    }

    pub(crate) async fn observed_values(
        &self,
        df: &DataFrame,
        parent_filter: Option<Expr>,
        params: &IndexMap<String, ScalarValue>,
        slot_cache: &mut PartitionSlotCache,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let cache_key = self.slot_cache_key(
            "observed",
            df.logical_plan(),
            parent_filter.as_ref(),
            params,
        );
        if let Some(cached) = slot_cache.get(&cache_key) {
            return Ok(cached);
        }
        if let Some(parent_filter) = parent_filter {
            let df_filtered = df.clone().filter(parent_filter)?;
            let values =
                PartitionKeyExtractor::extract_keys(&df_filtered, &self.field_expr, params).await?;
            slot_cache.insert(cache_key, values.clone());
            Ok(values)
        } else {
            let values = PartitionKeyExtractor::extract_keys(df, &self.field_expr, params).await?;
            slot_cache.insert(cache_key, values.clone());
            Ok(values)
        }
    }

    pub(crate) async fn domain_values(
        &self,
        df: &DataFrame,
        domain_filter: Option<Expr>,
        params: &IndexMap<String, ScalarValue>,
        slot_cache: &mut PartitionSlotCache,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let cache_key =
            self.slot_cache_key("domain", df.logical_plan(), domain_filter.as_ref(), params);
        if let Some(cached) = slot_cache.get(&cache_key) {
            return Ok(cached);
        }

        let scoped_df = if let Some(domain_filter) = domain_filter {
            df.clone().filter(domain_filter)?
        } else {
            df.clone()
        };
        let values = PartitionKeyExtractor::extract_ordered_keys(
            &scoped_df,
            &self.field_expr,
            self.order_expr.as_ref(),
            self.order_descending,
            params,
        )
        .await?;
        slot_cache.insert(cache_key, values.clone());
        Ok(values)
    }

    fn slot_cache_key(
        &self,
        kind: &str,
        data_plan: &LogicalPlan,
        filter: Option<&Expr>,
        params: &IndexMap<String, ScalarValue>,
    ) -> String {
        format!(
            "kind={}|data={:?}|direction={:?}|sharing={}|field={}|expr={}|order={}|desc={}|filter={}|params={}",
            kind,
            data_plan,
            self.direction,
            self.sharing,
            self.field,
            self.field_expr,
            self.order_expr
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "<default>".to_string()),
            self.order_descending,
            filter
                .map(ToString::to_string)
                .unwrap_or_else(|| "<global>".to_string()),
            partition_param_fingerprint(
                data_plan,
                &self.field_expr,
                self.order_expr.as_ref(),
                filter,
                params
            )
        )
    }
}

fn partition_param_fingerprint(
    data_plan: &LogicalPlan,
    field_expr: &Expr,
    order_expr: Option<&Expr>,
    filter: Option<&Expr>,
    params: &IndexMap<String, ScalarValue>,
) -> String {
    let mut names = Vec::<String>::new();
    collect_logical_plan_placeholders(data_plan, &mut names);
    collect_expr_placeholders(field_expr, &mut names);
    if let Some(order_expr) = order_expr {
        collect_expr_placeholders(order_expr, &mut names);
    }
    if let Some(filter) = filter {
        collect_expr_placeholders(filter, &mut names);
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let value = params
                .get(&name)
                .or_else(|| params.get(&format!("${name}")))
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "<missing>".to_string());
            format!("{name}={value}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn collect_logical_plan_placeholders(plan: &LogicalPlan, names: &mut Vec<String>) {
    let _ = plan.apply(|node| {
        for expr in node.expressions() {
            collect_expr_placeholders(&expr, names);
        }
        Ok(TreeNodeRecursion::Continue)
    });
}

fn collect_expr_placeholders(expr: &Expr, names: &mut Vec<String>) {
    let _ = expr.apply(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate {
            names.push(placeholder.id.trim_start_matches('$').to_string());
        }
        Ok(TreeNodeRecursion::Continue)
    });
}

impl PartitionKeyExtractor {
    /// Extract all distinct values for a single partition dimension.
    ///
    /// The supplied expression should resolve to the column used by the
    /// partition channel. Values are sorted to ensure deterministic placement.
    pub async fn extract_keys(
        df: &DataFrame,
        expr: &Expr,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let distinct_df = df.clone().select(vec![expr.clone()])?.distinct()?;

        let batches = Self::collect_with_params(distinct_df, params).await?;
        let mut values = Self::scalar_column_to_vec(&batches, 0)?;
        values.sort_by(scalar_total_cmp);

        Ok(values)
    }

    pub async fn extract_ordered_keys(
        df: &DataFrame,
        key_expr: &Expr,
        order_expr: Option<&Expr>,
        order_descending: bool,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let Some(order_expr) = order_expr else {
            return Self::extract_keys(df, key_expr, params).await;
        };

        validate_order_expr(key_expr, order_expr)?;

        if !contains_aggregate(order_expr) {
            let mut values = Self::extract_keys(df, key_expr, params).await?;
            if order_expr_matches_partition(key_expr, order_expr) && order_descending {
                values.sort_by(|a, b| scalar_total_cmp(b, a));
            }
            return Ok(values);
        }

        let ordered_df = df.clone().aggregate(
            vec![key_expr.clone()],
            vec![order_expr.clone().alias("__facet_order")],
        )?;
        let batches = Self::collect_with_params(ordered_df, params).await?;
        let mut keyed_values = Self::scalar_columns_to_pairs(&batches, 0, 1)?;

        keyed_values.sort_by(|(lhs_key, lhs_order), (rhs_key, rhs_order)| {
            let primary = scalar_total_cmp(lhs_order, rhs_order);
            let primary = if order_descending {
                primary.reverse()
            } else {
                primary
            };
            primary.then_with(|| scalar_total_cmp(lhs_key, rhs_key))
        });

        let mut values = Vec::with_capacity(keyed_values.len());
        for (key, _) in keyed_values {
            if !values
                .iter()
                .any(|existing| scalar_values_equivalent(existing, &key))
            {
                values.push(key);
            }
        }

        Ok(values)
    }

    async fn collect_with_params(
        df: DataFrame,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<RecordBatch>, AvengerChartError> {
        let batches = if let Some(param_values) = params_to_datafusion(params) {
            df.with_param_values(param_values)?.collect().await?
        } else {
            df.collect().await?
        };
        Ok(batches)
    }

    fn scalar_column_to_vec(
        batches: &[RecordBatch],
        column_index: usize,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let mut values = Vec::new();
        for batch in batches {
            let column = batch.column(column_index);
            for row in 0..batch.num_rows() {
                values.push(ScalarValue::try_from_array(column, row)?);
            }
        }
        Ok(values)
    }

    fn scalar_columns_to_pairs(
        batches: &[RecordBatch],
        key_column_index: usize,
        order_column_index: usize,
    ) -> Result<Vec<(ScalarValue, ScalarValue)>, AvengerChartError> {
        let mut values = Vec::new();
        for batch in batches {
            let key_column = batch.column(key_column_index);
            let order_column = batch.column(order_column_index);
            for row in 0..batch.num_rows() {
                values.push((
                    ScalarValue::try_from_array(key_column, row)?,
                    ScalarValue::try_from_array(order_column, row)?,
                ));
            }
        }
        Ok(values)
    }
}

fn validate_order_expr(key_expr: &Expr, order_expr: &Expr) -> Result<(), AvengerChartError> {
    if contains_aggregate(order_expr)
        || !order_expr.any_column_refs()
        || order_expr_matches_partition(key_expr, order_expr)
    {
        return Ok(());
    }

    Err(AvengerChartError::InvalidArgument(
        "Facet order_by expression must be an aggregate, literal/constant, or the partition expression"
            .to_string(),
    ))
}

fn order_expr_matches_partition(key_expr: &Expr, order_expr: &Expr) -> bool {
    order_expr == key_expr || order_expr.to_string() == key_expr.to_string()
}

fn partition_field_name(expr: &Expr) -> String {
    match expr {
        Expr::Column(col) => col.name.clone(),
        Expr::Alias(alias) => alias.name.clone(),
        _ => expr.to_string(),
    }
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
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
            values: values.clone(),
            observed_values: values.clone(),
            min_slot_count: None,
            content: PartitionContent::Leaf { values },
        }
    }

    /// Create a new leaf partition node with explicit observed values.
    pub fn leaf_with_observed(
        direction: FacetDirection,
        sharing: u8,
        field: String,
        field_expr: Option<Expr>,
        values: Vec<ScalarValue>,
        observed_values: Vec<ScalarValue>,
    ) -> Self {
        Self {
            direction,
            sharing,
            field,
            field_expr,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
            values: values.clone(),
            observed_values,
            min_slot_count: None,
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
        let values = children.keys().cloned().collect::<Vec<_>>();
        Self {
            direction,
            sharing,
            field,
            field_expr,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
            values: values.clone(),
            observed_values: values,
            min_slot_count: None,
            content: PartitionContent::Branch { children },
        }
    }

    /// Create a new branch partition node with explicit observed values.
    pub fn branch_with_observed(
        direction: FacetDirection,
        sharing: u8,
        field: String,
        field_expr: Option<Expr>,
        observed_values: Vec<ScalarValue>,
        children: IndexMap<ScalarValue, Box<PartitionNode>>,
    ) -> Self {
        let values = children.keys().cloned().collect::<Vec<_>>();
        Self {
            direction,
            sharing,
            field,
            field_expr,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
            values,
            observed_values,
            min_slot_count: None,
            content: PartitionContent::Branch { children },
        }
    }

    /// Create a new branch partition node with explicit domain and observed values.
    pub fn branch_with_values_and_observed(
        direction: FacetDirection,
        sharing: u8,
        field: String,
        field_expr: Option<Expr>,
        values: Vec<ScalarValue>,
        observed_values: Vec<ScalarValue>,
        children: IndexMap<ScalarValue, Box<PartitionNode>>,
    ) -> Self {
        Self {
            direction,
            sharing,
            field,
            field_expr,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
            values,
            observed_values,
            min_slot_count: None,
            content: PartitionContent::Branch { children },
        }
    }

    pub(crate) fn with_min_slot_count(mut self, min_slot_count: usize) -> Self {
        self.min_slot_count = Some(min_slot_count);
        self
    }

    pub(crate) fn min_slot_count(&self) -> Option<usize> {
        self.min_slot_count
    }

    /// Check if this is a leaf node.
    pub fn is_leaf(&self) -> bool {
        matches!(self.content, PartitionContent::Leaf { .. })
    }

    /// Get the partition slot values at this level.
    pub fn values(&self) -> Box<dyn Iterator<Item = &ScalarValue> + '_> {
        Box::new(self.values.iter())
    }

    pub fn observed_values(&self) -> impl Iterator<Item = &ScalarValue> {
        self.observed_values.iter()
    }

    pub(crate) fn with_axis_guide_visibility(
        mut self,
        visibility: AxisGuideVisibilityConfig,
    ) -> Self {
        self.axis_guide_visibility = visibility;
        self
    }

    /// Get the number of partition slot values at this level.
    pub fn domain_count(&self) -> usize {
        self.values.len()
    }

    /// Get child node for a specific value.
    pub fn child(&self, value: &ScalarValue) -> Option<&PartitionNode> {
        match &self.content {
            PartitionContent::Leaf { .. } => None,
            PartitionContent::Branch { children } => children
                .get(value)
                .or_else(|| {
                    children.iter().find_map(|(child_value, child_node)| {
                        scalar_values_equivalent(child_value, value).then_some(child_node)
                    })
                })
                .map(|b| b.as_ref()),
        }
    }

    /// Collect all reachable value paths below this node.
    pub(crate) fn reachable_paths(&self) -> Vec<Vec<ScalarValue>> {
        let mut paths = Vec::new();
        let mut prefix = Vec::new();
        self.collect_reachable_paths_recursive(&mut prefix, &mut paths);
        paths
    }

    fn collect_reachable_paths_recursive(
        &self,
        prefix: &mut Vec<ScalarValue>,
        out: &mut Vec<Vec<ScalarValue>>,
    ) {
        let values: Vec<ScalarValue> = self.values().cloned().collect();
        for value in &values {
            prefix.push(value.clone());
            out.push(prefix.clone());
            if let Some(child) = self.child(value) {
                child.collect_reachable_paths_recursive(prefix, out);
            }
            prefix.pop();
        }
    }

    /// Collect all node paths below this node, including this node's empty path.
    pub(crate) fn node_paths(&self) -> Vec<Vec<ScalarValue>> {
        let mut paths = Vec::new();
        let mut prefix = Vec::new();
        self.collect_node_paths_recursive(&mut prefix, &mut paths);
        paths
    }

    fn collect_node_paths_recursive(
        &self,
        prefix: &mut Vec<ScalarValue>,
        out: &mut Vec<Vec<ScalarValue>>,
    ) {
        out.push(prefix.clone());
        if let PartitionContent::Branch { children } = &self.content {
            for (value, child) in children {
                prefix.push(value.clone());
                child.collect_node_paths_recursive(prefix, out);
                prefix.pop();
            }
        }
    }

    /// Navigate to a partition node at a path of branch values.
    pub(crate) fn node_at_path(&self, path: &[ScalarValue]) -> Option<&PartitionNode> {
        if path.is_empty() {
            return Some(self);
        }

        let mut current = self;
        for value in path {
            current = current.child(value)?;
        }
        Some(current)
    }

    /// Check whether a full cell path exists in this partition tree.
    pub(crate) fn cell_exists(&self, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return true;
        }

        let parent_path = &path[..path.len() - 1];
        let target_value = &path[path.len() - 1];
        self.node_at_path(parent_path).is_some_and(|parent| {
            parent
                .values()
                .any(|value| scalar_values_equivalent(value, target_value))
        })
    }

    /// Check whether a full cell path has observed rows.
    pub(crate) fn cell_has_data(&self, path: &[ScalarValue]) -> bool {
        if path.is_empty() {
            return true;
        }

        let parent_path = &path[..path.len() - 1];
        let target_value = &path[path.len() - 1];
        self.node_at_path(parent_path).is_some_and(|parent| {
            parent
                .observed_values()
                .any(|value| scalar_values_equivalent(value, target_value))
        })
    }

    /// Collect unique slot values at a descendant depth.
    pub(crate) fn values_at_depth(&self, levels_to_descend: usize) -> Vec<ScalarValue> {
        if levels_to_descend == 0 {
            return self.values().cloned().collect();
        }

        let mut values = Vec::new();
        for value in self.values() {
            if let Some(child) = self.child(value) {
                values.extend(child.values_at_depth(levels_to_descend - 1));
            }
        }
        let mut ordered_unique = Vec::with_capacity(values.len());
        for value in values {
            if !ordered_unique
                .iter()
                .any(|existing| scalar_values_equivalent(existing, &value))
            {
                ordered_unique.push(value);
            }
        }
        ordered_unique
    }

    /// Build a filter predicate for a concrete path through this partition tree.
    ///
    /// Returns `None` for an empty path or when the path cannot be resolved.
    pub(crate) fn path_predicate(&self, path: &[ScalarValue]) -> Option<Expr> {
        if path.is_empty() {
            return None;
        }

        let mut current_node = self;
        let mut result: Option<Expr> = None;

        for (level_idx, value) in path.iter().enumerate() {
            if let Some(field_expr) = current_node.field_expr.clone() {
                let eq_expr = field_expr.eq(lit(value.clone()));
                result = Some(match result {
                    Some(existing) => existing.and(eq_expr),
                    None => eq_expr,
                });
            }

            if level_idx + 1 < path.len() {
                current_node = current_node.child(value)?;
            }
        }

        result
    }
}

pub(crate) fn scalar_values_equivalent(a: &ScalarValue, b: &ScalarValue) -> bool {
    if a == b {
        return true;
    }

    match (a, b) {
        (
            ScalarValue::Utf8(Some(lhs))
            | ScalarValue::LargeUtf8(Some(lhs))
            | ScalarValue::Utf8View(Some(lhs)),
            ScalarValue::Utf8(Some(rhs))
            | ScalarValue::LargeUtf8(Some(rhs))
            | ScalarValue::Utf8View(Some(rhs)),
        ) => lhs == rhs,
        (
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Utf8View(None),
            ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None) | ScalarValue::Utf8View(None),
        ) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use datafusion::prelude::col;
    use datafusion::{
        arrow::{
            array::{Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::min_max::max,
        prelude::SessionContext,
    };

    #[test]
    fn scalar_values_equivalent_matches_utf8_storage_variants() {
        assert!(scalar_values_equivalent(
            &ScalarValue::Utf8(Some("A".to_string())),
            &ScalarValue::Utf8View(Some("A".to_string()))
        ));
        assert!(scalar_values_equivalent(
            &ScalarValue::LargeUtf8(None),
            &ScalarValue::Utf8View(None)
        ));
        assert!(!scalar_values_equivalent(
            &ScalarValue::Utf8(Some("A".to_string())),
            &ScalarValue::Utf8(Some("B".to_string()))
        ));
    }

    fn string_scalar(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn ordered_key_df(ctx: &SessionContext) -> DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
            Field::new("other", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["B", "A", "C", "B", "A", "C"])),
                Arc::new(Float64Array::from(vec![8.0, 2.0, 5.0, 9.0, 3.0, 6.0])),
                Arc::new(StringArray::from(vec!["x", "y", "z", "x", "y", "z"])),
            ],
        )
        .expect("test batch");
        ctx.read_batch(batch).expect("test dataframe")
    }

    #[tokio::test]
    async fn partition_key_extractor_default_ordering_sorts_by_partition_value() {
        let ctx = SessionContext::new();
        let df = ordered_key_df(&ctx);
        let values = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            None,
            false,
            &IndexMap::new(),
        )
        .await
        .expect("default ordered keys");

        assert_eq!(
            values,
            vec![string_scalar("A"), string_scalar("B"), string_scalar("C")]
        );
    }

    #[tokio::test]
    async fn partition_key_extractor_orders_by_aggregate_ascending_and_descending() {
        let ctx = SessionContext::new();
        let df = ordered_key_df(&ctx);

        let ascending = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&max(col("value"))),
            false,
            &IndexMap::new(),
        )
        .await
        .expect("ascending aggregate order");
        assert_eq!(
            ascending,
            vec![string_scalar("A"), string_scalar("C"), string_scalar("B")]
        );

        let descending = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&max(col("value"))),
            true,
            &IndexMap::new(),
        )
        .await
        .expect("descending aggregate order");
        assert_eq!(
            descending,
            vec![string_scalar("B"), string_scalar("C"), string_scalar("A")]
        );
    }

    #[tokio::test]
    async fn partition_key_extractor_ties_break_by_partition_value() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["B", "A", "C", "B", "A", "C"])),
                Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 5.0, 5.0, 5.0])),
            ],
        )
        .expect("test batch");
        let df = ctx.read_batch(batch).expect("test dataframe");

        let values = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&max(col("value"))),
            true,
            &IndexMap::new(),
        )
        .await
        .expect("aggregate order with ties");

        assert_eq!(
            values,
            vec![string_scalar("A"), string_scalar("B"), string_scalar("C")]
        );
    }

    #[tokio::test]
    async fn partition_key_extractor_accepts_literal_and_partition_ordering() {
        let ctx = SessionContext::new();
        let df = ordered_key_df(&ctx);

        let literal_order = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&lit("constant")),
            true,
            &IndexMap::new(),
        )
        .await
        .expect("literal order");
        assert_eq!(
            literal_order,
            vec![string_scalar("A"), string_scalar("B"), string_scalar("C")]
        );

        let partition_desc = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&col("category")),
            true,
            &IndexMap::new(),
        )
        .await
        .expect("partition expression order");
        assert_eq!(
            partition_desc,
            vec![string_scalar("C"), string_scalar("B"), string_scalar("A")]
        );
    }

    #[tokio::test]
    async fn partition_key_extractor_rejects_non_aggregate_order_columns() {
        let ctx = SessionContext::new();
        let df = ordered_key_df(&ctx);

        let err = PartitionKeyExtractor::extract_ordered_keys(
            &df,
            &col("category"),
            Some(&col("other")),
            false,
            &IndexMap::new(),
        )
        .await
        .expect_err("invalid non-aggregate order expression");

        assert!(matches!(
            err,
            AvengerChartError::InvalidArgument(message)
                if message.contains("Facet order_by expression")
        ));
    }

    #[test]
    fn child_lookup_accepts_equivalent_utf8_storage_variants() {
        let mut children = IndexMap::new();
        children.insert(
            ScalarValue::Utf8(Some("A".to_string())),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "team".to_string(),
                None,
                vec![ScalarValue::Utf8(Some("x".to_string()))],
            )),
        );
        let node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "department".to_string(),
            None,
            children,
        );

        assert!(
            node.child(&ScalarValue::Utf8View(Some("A".to_string())))
                .is_some()
        );
    }

    #[test]
    fn partition_node_collects_reachable_paths_and_node_paths() {
        let mut children = IndexMap::new();
        children.insert(
            ScalarValue::Utf8(Some("Eng".to_string())),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "team".to_string(),
                None,
                vec![
                    ScalarValue::Utf8(Some("A".to_string())),
                    ScalarValue::Utf8(Some("B".to_string())),
                ],
            )),
        );
        let node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "department".to_string(),
            None,
            children,
        );

        assert_eq!(
            node.reachable_paths(),
            vec![
                vec![ScalarValue::Utf8(Some("Eng".to_string()))],
                vec![
                    ScalarValue::Utf8(Some("Eng".to_string())),
                    ScalarValue::Utf8(Some("A".to_string()))
                ],
                vec![
                    ScalarValue::Utf8(Some("Eng".to_string())),
                    ScalarValue::Utf8(Some("B".to_string()))
                ],
            ]
        );
        assert_eq!(
            node.node_paths(),
            vec![
                Vec::<ScalarValue>::new(),
                vec![ScalarValue::Utf8(Some("Eng".to_string()))]
            ]
        );
    }

    #[test]
    fn partition_node_cell_membership_uses_observed_values() {
        let node = PartitionNode::leaf_with_observed(
            FacetDirection::Row,
            0,
            "team".to_string(),
            None,
            vec![
                ScalarValue::Utf8(Some("A".to_string())),
                ScalarValue::Utf8(Some("B".to_string())),
            ],
            vec![ScalarValue::Utf8(Some("A".to_string()))],
        );

        assert!(node.cell_exists(&[ScalarValue::Utf8(Some("B".to_string()))]));
        assert!(!node.cell_has_data(&[ScalarValue::Utf8(Some("B".to_string()))]));
    }

    #[test]
    fn branch_node_preserves_domain_values_separate_from_children() {
        let shared_domain = vec![
            string_scalar("Alpha"),
            string_scalar("Beta"),
            string_scalar("Delta"),
            string_scalar("Gamma"),
        ];

        let backend_teams = PartitionNode::leaf_with_observed(
            FacetDirection::Column,
            1,
            "team".to_string(),
            None,
            shared_domain.clone(),
            vec![string_scalar("Delta"), string_scalar("Gamma")],
        );
        let frontend_teams = PartitionNode::leaf_with_observed(
            FacetDirection::Column,
            1,
            "team".to_string(),
            None,
            shared_domain.clone(),
            vec![string_scalar("Alpha"), string_scalar("Beta")],
        );

        let mut children = IndexMap::new();
        children.insert(string_scalar("Backend"), Box::new(backend_teams));
        children.insert(string_scalar("Frontend"), Box::new(frontend_teams));
        let dept_node = PartitionNode::branch_with_values_and_observed(
            FacetDirection::Column,
            0,
            "department".to_string(),
            None,
            vec![string_scalar("Backend"), string_scalar("Frontend")],
            vec![string_scalar("Backend"), string_scalar("Frontend")],
            children,
        );

        assert_eq!(dept_node.values_at_depth(1), shared_domain);
        let backend = dept_node
            .child(&string_scalar("Backend"))
            .expect("backend team node");
        assert!(backend.cell_exists(&[string_scalar("Alpha")]));
        assert!(!backend.cell_has_data(&[string_scalar("Alpha")]));
    }

    #[test]
    fn partition_node_builds_path_predicate_for_resolved_path() {
        let mut children = IndexMap::new();
        children.insert(
            ScalarValue::Utf8(Some("Eng".to_string())),
            Box::new(PartitionNode::leaf(
                FacetDirection::Row,
                0,
                "team".to_string(),
                Some(col("team")),
                vec![ScalarValue::Utf8(Some("A".to_string()))],
            )),
        );
        let node = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "department".to_string(),
            Some(col("department")),
            children,
        );

        let predicate = node
            .path_predicate(&[
                ScalarValue::Utf8(Some("Eng".to_string())),
                ScalarValue::Utf8(Some("A".to_string())),
            ])
            .expect("path predicate");

        assert_eq!(
            predicate.to_string(),
            "department = Utf8(\"Eng\") AND team = Utf8(\"A\")"
        );
    }

    #[test]
    fn partition_dimension_derives_field_name_from_expression() {
        let column_spec = PartitionDimensionSpec::new(FacetDirection::Row, 0, col("team"));
        assert_eq!(column_spec.field, "team");

        let alias_spec =
            PartitionDimensionSpec::new(FacetDirection::Column, 0, col("dept").alias("department"));
        assert_eq!(alias_spec.field, "department");
    }

    #[test]
    fn partition_dimension_shared_slots_depend_on_depth() {
        let spec = PartitionDimensionSpec::new(FacetDirection::Column, 2, col("team"));
        assert!(spec.uses_shared_slots_at_depth(1));
        assert!(spec.uses_shared_slots_at_depth(2));
        assert!(!spec.uses_shared_slots_at_depth(3));
    }

    #[test]
    fn format_partition_value_matches_expected_display_values() {
        assert_eq!(
            format_partition_value(&ScalarValue::Utf8View(Some("Team A".to_string()))),
            "Team A"
        );
        assert_eq!(format_partition_value(&ScalarValue::Int32(Some(12))), "12");
        assert_eq!(
            format_partition_value(&ScalarValue::Float64(Some(1.234))),
            "1.23"
        );
    }
}
