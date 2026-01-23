//! Partition model for faceted visualizations.
//!
//! This module implements the grammar-based partition model for nested faceting.
//! It provides explicit types for:
//! - `CompiledPartition`: A single partition variable with field, direction, and sharing config
//! - `FacetPartitionList`: An ordered list of partitions (outermost first)
//! - `SubplotIndex`: Complete assignment of field values for a subplot
//! - `SharingGroup`: A prefix of partition bindings that defines a sharing group
//!
//! The key insight from the grammar is that visibility and sharing decisions can be
//! derived from the materialized structure rather than computed incrementally.
//!
//! # Grammar Reference
//!
//! The grammar defines:
//! ```text
//! partitions: [Partition]  // ordered, outermost first
//!
//! Partition = {
//!   field: str,
//!   direction: Row | Col,
//!   domain_sharing: int,  // 0 = nest, 255 = cross (global)
//! }
//!
//! depth = len(partitions)
//! partition_depth(k) = max(0, depth - k)
//! group_of(I, k) = prefix of I up to partition_depth(k)
//! ```

use crate::channel::config_traits::ScaleSharing;
use crate::facet::context::AxisPosition;
use crate::guide::FacetDirection;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Serializable representation of partition domain values.
///
/// Similar to `SerializableDomainValue` in coordination.rs but focused on
/// the subset of types used for facet partitioning (typically categorical).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionValue {
    String(String),
    Int(i64),
    Bool(bool),
    Null,
}

impl PartitionValue {
    /// Convert from ScalarValue
    pub fn from_scalar(value: &ScalarValue) -> Self {
        match value {
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                PartitionValue::String(s.clone())
            }
            ScalarValue::Utf8View(Some(s)) => PartitionValue::String(s.clone()),
            ScalarValue::Int8(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::Int16(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::Int32(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::Int64(Some(n)) => PartitionValue::Int(*n),
            ScalarValue::UInt8(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::UInt16(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::UInt32(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::UInt64(Some(n)) => PartitionValue::Int(*n as i64),
            ScalarValue::Boolean(Some(b)) => PartitionValue::Bool(*b),
            ScalarValue::Dictionary(_, inner) => Self::from_scalar(inner.as_ref()),
            _ => PartitionValue::Null,
        }
    }

    /// Convert to ScalarValue
    pub fn to_scalar(&self) -> ScalarValue {
        match self {
            PartitionValue::String(s) => ScalarValue::Utf8(Some(s.clone())),
            PartitionValue::Int(n) => ScalarValue::Int64(Some(*n)),
            PartitionValue::Bool(b) => ScalarValue::Boolean(Some(*b)),
            PartitionValue::Null => ScalarValue::Null,
        }
    }
}

/// Represents a single partition variable in the facet hierarchy.
///
/// This maps directly to the grammar's `Partition` type:
/// ```text
/// Partition = {
///   field: str,
///   direction: Row | Col,
///   domain_sharing: int,  // 0 = nest, 255 = cross
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPartition {
    /// Field name (e.g., "region", "year")
    pub field: String,

    /// Row or Column direction
    pub direction: FacetDirection,

    /// Domain sharing level for this partition variable.
    /// - 0: Free/nest (compute domain from filtered data per parent cell)
    /// - 255: Shared/cross (use global domain, may create empty cells)
    /// - 1-254: Intermediate sharing levels
    pub domain_sharing: u8,

    /// Ordered domain values computed from data.
    /// These are the unique values of `field` in the appropriate scope.
    pub domain_values: Vec<PartitionValue>,
}

impl CompiledPartition {
    /// Create a new compiled partition
    pub fn new(
        field: impl Into<String>,
        direction: FacetDirection,
        domain_sharing: ScaleSharing,
        domain_values: Vec<ScalarValue>,
    ) -> Self {
        Self {
            field: field.into(),
            direction,
            domain_sharing: domain_sharing.to_level(),
            domain_values: domain_values
                .iter()
                .map(PartitionValue::from_scalar)
                .collect(),
        }
    }

    /// Number of cells in this partition dimension
    pub fn cell_count(&self) -> usize {
        self.domain_values.len()
    }
}

/// Complete partition list for a faceted visualization.
///
/// This is the grammar's explicit `partitions: [Partition]` list, ordered
/// from outermost to innermost.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FacetPartitionList {
    /// Ordered partitions, outermost first
    pub partitions: Vec<CompiledPartition>,
}

impl FacetPartitionList {
    /// Create an empty partition list
    pub fn new() -> Self {
        Self {
            partitions: Vec::new(),
        }
    }

    /// Grammar's depth = len(partitions)
    pub fn depth(&self) -> usize {
        self.partitions.len()
    }

    /// Add a partition (appends to innermost position)
    pub fn push(&mut self, partition: CompiledPartition) {
        self.partitions.push(partition);
    }

    /// Get partition at index (0 = outermost)
    pub fn get(&self, index: usize) -> Option<&CompiledPartition> {
        self.partitions.get(index)
    }

    /// Row partitions only
    pub fn row_partitions(&self) -> impl Iterator<Item = (usize, &CompiledPartition)> {
        self.partitions
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.direction, FacetDirection::Row))
    }

    /// Column partitions only
    pub fn col_partitions(&self) -> impl Iterator<Item = (usize, &CompiledPartition)> {
        self.partitions
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.direction, FacetDirection::Column))
    }

    /// Grammar's partition_depth(k) = max(0, depth - k)
    ///
    /// This determines how many leading partitions define a sharing group.
    /// Higher k = fewer partitions = more global sharing.
    pub fn partition_depth(&self, sharing_level: u8) -> usize {
        let k = sharing_level as usize;
        self.depth().saturating_sub(k)
    }

    /// Grammar's group_of(I, k): map subplot to its sharing group.
    ///
    /// Returns the prefix of field-value pairs that defines which sharing group
    /// the subplot belongs to for the given sharing level.
    pub fn group_of(&self, subplot: &SubplotIndex, sharing_level: u8) -> SharingGroup {
        let prefix_len = self.partition_depth(sharing_level);
        let prefix = self
            .partitions
            .iter()
            .take(prefix_len)
            .map(|p| {
                let value = subplot
                    .bindings
                    .get(&p.field)
                    .cloned()
                    .unwrap_or(PartitionValue::Null);
                (p.field.clone(), value)
            })
            .collect();
        SharingGroup { prefix }
    }

    /// Compute row index vector for a subplot.
    ///
    /// Returns a vector of indices, one per row partition, indicating the
    /// subplot's position in each row dimension.
    pub fn row_index(&self, subplot: &SubplotIndex) -> Vec<usize> {
        self.row_partitions()
            .map(|(_, p)| {
                let value = subplot.bindings.get(&p.field);
                value
                    .and_then(|v| p.domain_values.iter().position(|dv| dv == v))
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Compute column index vector for a subplot.
    ///
    /// Returns a vector of indices, one per column partition, indicating the
    /// subplot's position in each column dimension.
    pub fn col_index(&self, subplot: &SubplotIndex) -> Vec<usize> {
        self.col_partitions()
            .map(|(_, p)| {
                let value = subplot.bindings.get(&p.field);
                value
                    .and_then(|v| p.domain_values.iter().position(|dv| dv == v))
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Grammar's materialize_subplots(spec).
    ///
    /// Produces the complete set of subplot indices by taking the Cartesian product
    /// of all partition domain values.
    ///
    /// Note: This assumes cross semantics (shared domains). For nest semantics,
    /// filtering would be needed at each level.
    pub fn materialize_subplots(&self) -> Vec<SubplotIndex> {
        let mut subplots = vec![SubplotIndex::new()];

        for partition in &self.partitions {
            let mut next = Vec::new();
            for ctx in subplots {
                for val in &partition.domain_values {
                    let mut bindings = ctx.bindings.clone();
                    bindings.insert(partition.field.clone(), val.clone());
                    next.push(SubplotIndex { bindings });
                }
            }
            subplots = next;
        }

        subplots
    }

    /// Determine if a subplot should show x-axis tick labels.
    ///
    /// Grammar rule:
    /// ```text
    /// show_x_ticks(I) ↔
    ///   let g = group_of(I, x.sharing) in
    ///   row_index(I) == boundary_row(S_g, x_position)
    /// ```
    pub fn show_x_ticks(
        &self,
        subplot: &SubplotIndex,
        x_sharing: u8,
        x_position: AxisPosition,
        all_subplots: &[SubplotIndex],
    ) -> bool {
        // If no row partitions, x-axis ticks are always shown
        if self.row_partitions().count() == 0 {
            return true;
        }

        let group = self.group_of(subplot, x_sharing);
        let group_members: Vec<_> = all_subplots
            .iter()
            .filter(|s| self.group_of(s, x_sharing) == group)
            .collect();

        let boundary_row = self.boundary_row(&group_members, x_position);
        let my_row = self.row_index(subplot);

        my_row == boundary_row
    }

    /// Determine if a subplot should show y-axis tick labels.
    ///
    /// Grammar rule:
    /// ```text
    /// show_y_ticks(I) ↔
    ///   let g = group_of(I, y.sharing) in
    ///   col_index(I) == boundary_col(S_g, y_position)
    /// ```
    pub fn show_y_ticks(
        &self,
        subplot: &SubplotIndex,
        y_sharing: u8,
        y_position: AxisPosition,
        all_subplots: &[SubplotIndex],
    ) -> bool {
        // If no column partitions, y-axis ticks are always shown
        if self.col_partitions().count() == 0 {
            return true;
        }

        let group = self.group_of(subplot, y_sharing);
        let group_members: Vec<_> = all_subplots
            .iter()
            .filter(|s| self.group_of(s, y_sharing) == group)
            .collect();

        let boundary_col = self.boundary_col(&group_members, y_position);
        let my_col = self.col_index(subplot);

        my_col == boundary_col
    }

    /// Determine if a subplot should show x-axis title.
    ///
    /// Title appears once at the global boundary across ALL subplots.
    pub fn show_x_title(&self, subplot: &SubplotIndex, x_position: AxisPosition) -> bool {
        // If no row partitions, always show title
        if self.row_partitions().count() == 0 {
            return true;
        }

        // Title shown at global boundary
        let all_subplots = self.materialize_subplots();
        let all_refs: Vec<_> = all_subplots.iter().collect();
        let boundary_row = self.boundary_row(&all_refs, x_position);
        let my_row = self.row_index(subplot);

        my_row == boundary_row
    }

    /// Determine if a subplot should show y-axis title.
    ///
    /// Title appears once at the global boundary across ALL subplots.
    pub fn show_y_title(&self, subplot: &SubplotIndex, y_position: AxisPosition) -> bool {
        // If no column partitions, always show title
        if self.col_partitions().count() == 0 {
            return true;
        }

        // Title shown at global boundary
        let all_subplots = self.materialize_subplots();
        let all_refs: Vec<_> = all_subplots.iter().collect();
        let boundary_col = self.boundary_col(&all_refs, y_position);
        let my_col = self.col_index(subplot);

        my_col == boundary_col
    }

    /// Compute boundary row index for a group of subplots.
    ///
    /// Grammar's boundary_row(S_g, position):
    /// - Bottom: max row_index in group
    /// - Top: min row_index in group
    fn boundary_row(&self, group: &[&SubplotIndex], position: AxisPosition) -> Vec<usize> {
        match position {
            AxisPosition::Bottom => group
                .iter()
                .map(|s| self.row_index(s))
                .max()
                .unwrap_or_default(),
            AxisPosition::Top => group
                .iter()
                .map(|s| self.row_index(s))
                .min()
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    /// Compute boundary column index for a group of subplots.
    ///
    /// Grammar's boundary_col(S_g, position):
    /// - Left: min col_index in group
    /// - Right: max col_index in group
    fn boundary_col(&self, group: &[&SubplotIndex], position: AxisPosition) -> Vec<usize> {
        match position {
            AxisPosition::Left => group
                .iter()
                .map(|s| self.col_index(s))
                .min()
                .unwrap_or_default(),
            AxisPosition::Right => group
                .iter()
                .map(|s| self.col_index(s))
                .max()
                .unwrap_or_default(),
            _ => vec![],
        }
    }
}

/// Grammar's SubplotIndex: complete context binding all partition fields.
///
/// This represents a unique subplot in the materialized grid, with a value
/// assigned for each partition variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubplotIndex {
    /// Field -> Value bindings for all partitions
    pub bindings: IndexMap<String, PartitionValue>,
}

impl std::hash::Hash for SubplotIndex {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the bindings in order (IndexMap preserves insertion order)
        for (key, value) in &self.bindings {
            key.hash(state);
            value.hash(state);
        }
    }
}

impl SubplotIndex {
    /// Create an empty subplot index
    pub fn new() -> Self {
        Self {
            bindings: IndexMap::new(),
        }
    }

    /// Add a binding
    pub fn with_binding(mut self, field: impl Into<String>, value: PartitionValue) -> Self {
        self.bindings.insert(field.into(), value);
        self
    }

    /// Get the value for a field
    pub fn get(&self, field: &str) -> Option<&PartitionValue> {
        self.bindings.get(field)
    }
}

impl Default for SubplotIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Grammar's SharingGroup: prefix of partition bindings.
///
/// A sharing group is defined by a prefix of field-value pairs from the partition
/// list. All subplots with the same prefix belong to the same sharing group and
/// share scales/domains at that level.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SharingGroup {
    /// Prefix of field->value pairs defining this group
    pub prefix: Vec<(String, PartitionValue)>,
}

impl SharingGroup {
    /// Create an empty sharing group (global scope)
    pub fn global() -> Self {
        Self { prefix: Vec::new() }
    }

    /// Number of partition levels in this group's prefix
    pub fn depth(&self) -> usize {
        self.prefix.len()
    }

    /// Check if this is the global sharing group (empty prefix)
    pub fn is_global(&self) -> bool {
        self.prefix.is_empty()
    }
}

/// Pre-computed visibility decisions for all subplots.
///
/// This cache computes visibility once for all subplots based on the grammar's
/// rules, rather than computing incrementally during iteration.
#[derive(Debug, Clone)]
pub struct VisibilityCache {
    /// subplot bindings -> visibility decisions
    visibility: IndexMap<SubplotIndex, SubplotVisibility>,
}

/// Visibility decisions for a single subplot.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubplotVisibility {
    /// Whether to show x-axis tick labels
    pub show_x_ticks: bool,
    /// Whether to show y-axis tick labels
    pub show_y_ticks: bool,
    /// Whether to show x-axis title
    pub show_x_title: bool,
    /// Whether to show y-axis title
    pub show_y_title: bool,
}

impl VisibilityCache {
    /// Compute visibility for all subplots.
    ///
    /// This implements the grammar's visibility rules in a single pass over the
    /// materialized subplot set, rather than computing incrementally.
    pub fn compute(
        partition_list: &FacetPartitionList,
        x_sharing: u8,
        y_sharing: u8,
        x_position: AxisPosition,
        y_position: AxisPosition,
    ) -> Self {
        let subplots = partition_list.materialize_subplots();
        let mut visibility = IndexMap::new();

        for subplot in &subplots {
            let vis = SubplotVisibility {
                show_x_ticks: partition_list.show_x_ticks(subplot, x_sharing, x_position, &subplots),
                show_y_ticks: partition_list.show_y_ticks(subplot, y_sharing, y_position, &subplots),
                show_x_title: partition_list.show_x_title(subplot, x_position),
                show_y_title: partition_list.show_y_title(subplot, y_position),
            };
            visibility.insert(subplot.clone(), vis);
        }

        Self { visibility }
    }

    /// Get visibility for a subplot
    pub fn get(&self, subplot: &SubplotIndex) -> Option<&SubplotVisibility> {
        self.visibility.get(subplot)
    }

    /// Get visibility for a subplot by its bindings
    pub fn get_by_bindings(
        &self,
        bindings: &IndexMap<String, PartitionValue>,
    ) -> Option<&SubplotVisibility> {
        self.visibility.get(&SubplotIndex {
            bindings: bindings.clone(),
        })
    }

    /// Number of subplots in the cache
    pub fn len(&self) -> usize {
        self.visibility.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.visibility.is_empty()
    }
}

/// Build a SubplotIndex from params by extracting values for each partition field.
///
/// This function looks up each partition field in the params and builds a SubplotIndex
/// with the corresponding values. Fields not found in params get PartitionValue::Null.
pub fn build_subplot_index_from_params(
    partition_list: &FacetPartitionList,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> SubplotIndex {
    let mut bindings = IndexMap::new();

    for partition in &partition_list.partitions {
        // Look up the field value in params
        // The field name in params should match the partition field name
        if let Some(value) = params.get(&partition.field) {
            bindings.insert(partition.field.clone(), PartitionValue::from_scalar(value));
        } else {
            // Try uppercase param name convention
            let upper_field = partition.field.to_uppercase();
            if let Some(value) = params.get(&upper_field) {
                bindings.insert(partition.field.clone(), PartitionValue::from_scalar(value));
            } else {
                // Field not found, use Null
                bindings.insert(partition.field.clone(), PartitionValue::Null);
            }
        }
    }

    SubplotIndex { bindings }
}

/// Compute visibility for a single subplot directly from the partition list.
///
/// This is a convenience function that computes visibility without building a full
/// VisibilityCache. It's useful when you only need visibility for one subplot.
///
/// # Arguments
/// * `partition_list` - The partition list defining the facet structure
/// * `subplot` - The subplot index to compute visibility for
/// * `x_sharing` - Sharing level for x-axis (0=Free, 255=Shared, 1-254=Level)
/// * `y_sharing` - Sharing level for y-axis
/// * `x_position` - X-axis position (typically Bottom)
/// * `y_position` - Y-axis position (typically Left)
pub fn compute_subplot_visibility(
    partition_list: &FacetPartitionList,
    subplot: &SubplotIndex,
    x_sharing: u8,
    y_sharing: u8,
    x_position: AxisPosition,
    y_position: AxisPosition,
) -> SubplotVisibility {
    let all_subplots = partition_list.materialize_subplots();

    SubplotVisibility {
        show_x_ticks: partition_list.show_x_ticks(subplot, x_sharing, x_position, &all_subplots),
        show_y_ticks: partition_list.show_y_ticks(subplot, y_sharing, y_position, &all_subplots),
        show_x_title: partition_list.show_x_title(subplot, x_position),
        show_y_title: partition_list.show_y_title(subplot, y_position),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_list_depth() {
        let mut list = FacetPartitionList::new();
        assert_eq!(list.depth(), 0);

        list.push(CompiledPartition::new(
            "region",
            FacetDirection::Row,
            ScaleSharing::Shared,
            vec![ScalarValue::Utf8(Some("East".to_string()))],
        ));
        assert_eq!(list.depth(), 1);

        list.push(CompiledPartition::new(
            "year",
            FacetDirection::Column,
            ScaleSharing::Free,
            vec![ScalarValue::Int64(Some(2023))],
        ));
        assert_eq!(list.depth(), 2);
    }

    #[test]
    fn test_partition_depth_formula() {
        let mut list = FacetPartitionList::new();
        for i in 0..4 {
            list.push(CompiledPartition::new(
                format!("field{}", i),
                FacetDirection::Row,
                ScaleSharing::Shared,
                vec![],
            ));
        }

        // depth = 4
        // partition_depth(k) = max(0, 4 - k)
        assert_eq!(list.partition_depth(0), 4); // Free: include all
        assert_eq!(list.partition_depth(1), 3); // Share 1 level
        assert_eq!(list.partition_depth(2), 2); // Share 2 levels
        assert_eq!(list.partition_depth(3), 1); // Share 3 levels
        assert_eq!(list.partition_depth(4), 0); // Share 4 levels (global)
        assert_eq!(list.partition_depth(255), 0); // Shared (global)
    }

    #[test]
    fn test_materialize_subplots() {
        let mut list = FacetPartitionList::new();

        list.push(CompiledPartition::new(
            "region",
            FacetDirection::Row,
            ScaleSharing::Shared,
            vec![
                ScalarValue::Utf8(Some("East".to_string())),
                ScalarValue::Utf8(Some("West".to_string())),
            ],
        ));

        list.push(CompiledPartition::new(
            "year",
            FacetDirection::Column,
            ScaleSharing::Shared,
            vec![ScalarValue::Int64(Some(2022)), ScalarValue::Int64(Some(2023))],
        ));

        let subplots = list.materialize_subplots();

        // 2 regions x 2 years = 4 subplots
        assert_eq!(subplots.len(), 4);

        // Check that all combinations exist
        let has_east_2022 = subplots.iter().any(|s| {
            s.get("region") == Some(&PartitionValue::String("East".to_string()))
                && s.get("year") == Some(&PartitionValue::Int(2022))
        });
        assert!(has_east_2022);
    }

    #[test]
    fn test_group_of() {
        let mut list = FacetPartitionList::new();

        list.push(CompiledPartition::new(
            "region",
            FacetDirection::Row,
            ScaleSharing::Shared,
            vec![
                ScalarValue::Utf8(Some("East".to_string())),
                ScalarValue::Utf8(Some("West".to_string())),
            ],
        ));

        list.push(CompiledPartition::new(
            "year",
            FacetDirection::Column,
            ScaleSharing::Shared,
            vec![ScalarValue::Int64(Some(2022)), ScalarValue::Int64(Some(2023))],
        ));

        let subplot = SubplotIndex::new()
            .with_binding("region", PartitionValue::String("East".to_string()))
            .with_binding("year", PartitionValue::Int(2022));

        // Level 0 (Free): group includes all bindings
        let group0 = list.group_of(&subplot, 0);
        assert_eq!(group0.depth(), 2);

        // Level 1: group includes first partition only
        let group1 = list.group_of(&subplot, 1);
        assert_eq!(group1.depth(), 1);
        assert_eq!(
            group1.prefix[0],
            ("region".to_string(), PartitionValue::String("East".to_string()))
        );

        // Level 2+ (Shared): empty prefix (global)
        let group2 = list.group_of(&subplot, 2);
        assert!(group2.is_global());
    }

    #[test]
    fn test_row_col_index() {
        let mut list = FacetPartitionList::new();

        list.push(CompiledPartition::new(
            "region",
            FacetDirection::Row,
            ScaleSharing::Shared,
            vec![
                ScalarValue::Utf8(Some("East".to_string())),
                ScalarValue::Utf8(Some("West".to_string())),
            ],
        ));

        list.push(CompiledPartition::new(
            "year",
            FacetDirection::Column,
            ScaleSharing::Shared,
            vec![ScalarValue::Int64(Some(2022)), ScalarValue::Int64(Some(2023))],
        ));

        let subplot = SubplotIndex::new()
            .with_binding("region", PartitionValue::String("West".to_string()))
            .with_binding("year", PartitionValue::Int(2023));

        assert_eq!(list.row_index(&subplot), vec![1]); // West is index 1
        assert_eq!(list.col_index(&subplot), vec![1]); // 2023 is index 1
    }

    #[test]
    fn test_visibility_cache() {
        let mut list = FacetPartitionList::new();

        // 2 rows x 2 columns
        list.push(CompiledPartition::new(
            "region",
            FacetDirection::Row,
            ScaleSharing::Shared,
            vec![
                ScalarValue::Utf8(Some("North".to_string())),
                ScalarValue::Utf8(Some("South".to_string())),
            ],
        ));

        list.push(CompiledPartition::new(
            "year",
            FacetDirection::Column,
            ScaleSharing::Shared,
            vec![ScalarValue::Int64(Some(2022)), ScalarValue::Int64(Some(2023))],
        ));

        // x shared globally, y shared globally, x at bottom, y at left
        let cache = VisibilityCache::compute(
            &list,
            255, // x globally shared
            255, // y globally shared
            AxisPosition::Bottom,
            AxisPosition::Left,
        );

        assert_eq!(cache.len(), 4);

        // Bottom-left: South/2022 - should show both ticks
        let south_2022 = SubplotIndex::new()
            .with_binding("region", PartitionValue::String("South".to_string()))
            .with_binding("year", PartitionValue::Int(2022));
        let vis = cache.get(&south_2022).unwrap();
        assert!(vis.show_x_ticks, "Bottom row should show x ticks");
        assert!(vis.show_y_ticks, "Left column should show y ticks");

        // Top-right: North/2023 - should show neither
        let north_2023 = SubplotIndex::new()
            .with_binding("region", PartitionValue::String("North".to_string()))
            .with_binding("year", PartitionValue::Int(2023));
        let vis = cache.get(&north_2023).unwrap();
        assert!(!vis.show_x_ticks, "Top row should not show x ticks");
        assert!(!vis.show_y_ticks, "Right column should not show y ticks");
    }
}
