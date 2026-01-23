//! Shared types and helper functions for facet guides.
//!
//! This module contains types used by both FacetRowGuide and FacetColGuide:
//! - FacetSource: representation of a compiled facet mark and data context
//! - FacetTitles: unified title references for measurement helpers
//! - Overflow aggregation and coordination helpers
//! - Scale sharing computation functions

use crate::channel::config_traits::ScaleSharing;
use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};
use crate::facet::partition::{CompiledPartition, FacetPartitionList, PartitionValue};
use crate::guide::{FacetDirection, OverflowSpaceRequirement};
use crate::marks::CompiledMark;
use crate::plot::CompiledPlot;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Default overflow fallback values (left, right, top, bottom) used when
/// measurement cannot be performed (e.g., in data_override path without facet expression).
///
/// These values provide reasonable padding for typical axis labels and titles:
/// - Left: 30.0 pixels for y-axis labels
/// - Right: 30.0 pixels for right-side elements
/// - Top: 40.0 pixels for title/header space
/// - Bottom: 20.0 pixels for x-axis labels
pub const DEFAULT_OVERFLOW_FALLBACK: (f32, f32, f32, f32) = (30.0, 30.0, 40.0, 20.0);

/// Returns the default overflow fallback values as a tuple.
/// Used when overflow cannot be measured (data_override path without facet expression).
#[inline]
pub fn default_overflow_fallback() -> (f32, f32, f32, f32) {
    DEFAULT_OVERFLOW_FALLBACK
}

/// Unified title references for measure_overflow and evaluate generic helpers.
///
/// This struct encapsulates the different title fields between Row and Column facets:
/// - `FacetRowGuide` has only `unified_y_title`
/// - `FacetColGuide` has both `unified_x_title` and `unified_y_title`
///
/// The `primary_unified` field corresponds to the dimension's own unified title:
/// - Row facets: `unified_y_title` (unifies y-axis across rows)
/// - Column facets: `unified_x_title` (unifies x-axis across columns)
///
/// The `nested_unified` field is only used by FacetColGuide for the nested case
/// where a FacetRow is inside a FacetCol and needs to render the unified y-title.
#[derive(Clone, Debug, Default)]
pub struct FacetTitles<'a> {
    /// The primary unified title for this dimension
    /// Row: unified_y_title, Column: unified_x_title
    pub primary_unified: Option<&'a String>,

    /// Optional nested unified title (only used by FacetColGuide)
    /// When FacetCol contains FacetRow, this holds unified_y_title
    pub nested_unified: Option<&'a String>,
}

/// A facet source represents a compiled facet mark and its associated data context.
/// Used during overflow measurement to determine how many cells should be measured.
#[derive(Clone, Serialize, Deserialize)]
pub struct FacetSource {
    pub subplot: Arc<CompiledPlot>,
    pub data: crate::marks::CompiledDataContext,
    pub user_title: Option<String>,
    /// Scale sharing mode for this facet dimension.
    /// When `Shared`, measurement should use the full domain (including empty cells).
    /// When `Free` or None, measurement should only use values present in the current data slice.
    pub facet_scale_sharing: Option<ScaleSharing>,
}

/// Apply shared overflow from coordination context for uniform padding.
///
/// This ensures plot areas align even when scales are not shared, by applying
/// the maximum overflow from any sibling facet cell.
///
/// # Arguments
/// * `overflow` - Mutable tuple (top, bottom, left, right) to apply shared values to
/// * `params` - Parameters containing coordination context
pub fn apply_shared_overflow_coordination(
    overflow: &mut (f32, f32, f32, f32),
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) {
    use crate::facet::coordination::{
        FacetCoordinationContext, SHARED_OVERFLOW_BOTTOM, SHARED_OVERFLOW_LEFT,
        SHARED_OVERFLOW_RIGHT, SHARED_OVERFLOW_TOP,
    };
    let coord_ctx = FacetCoordinationContext::from_params(params).unwrap_or_default();
    if let Some(shared_top) = coord_ctx.get_coordinated_spacing(SHARED_OVERFLOW_TOP) {
        overflow.0 = overflow.0.max(shared_top);
    }
    if let Some(shared_bottom) = coord_ctx.get_coordinated_spacing(SHARED_OVERFLOW_BOTTOM) {
        overflow.1 = overflow.1.max(shared_bottom);
    }
    if let Some(shared_left) = coord_ctx.get_coordinated_spacing(SHARED_OVERFLOW_LEFT) {
        overflow.2 = overflow.2.max(shared_left);
    }
    if let Some(shared_right) = coord_ctx.get_coordinated_spacing(SHARED_OVERFLOW_RIGHT) {
        overflow.3 = overflow.3.max(shared_right);
    }
}

/// Aggregate overflow from cached per-facet overflow values.
///
/// This aggregates overflow for unified title positioning when cached overflow
/// values are available (from the measurement phase). The aggregation strategy is:
/// - Top/bottom: MAX across all subplots (for proper unified title positioning)
/// - Left: from first subplot (leftmost/topmost edge)
/// - Right: from last subplot (rightmost/bottommost edge)
///
/// Returns (top_max, bottom_max, left_max, right_max).
pub fn aggregate_cached_overflow(
    per_facet_overflow: &[OverflowSpaceRequirement],
    mut top_max: f32,
    mut bottom_max: f32,
    mut left_max: f32,
    mut right_max: f32,
) -> (f32, f32, f32, f32) {
    // Aggregate top/bottom across all subplots for unified title positioning
    for overflow_item in per_facet_overflow {
        top_max = top_max.max(overflow_item.top);
        bottom_max = bottom_max.max(overflow_item.bottom);
    }

    // Use first subplot's left overflow (leftmost/topmost edge)
    if let Some(first) = per_facet_overflow.first() {
        left_max = left_max.max(first.left);
    }

    // Use last subplot's right overflow (rightmost/bottommost edge)
    if let Some(last) = per_facet_overflow.last() {
        right_max = right_max.max(last.right);
    }

    (top_max, bottom_max, left_max, right_max)
}

/// Compute scale sharing mode for each channel from marks.
/// This extracts the sharing configuration from channel definitions.
pub fn compute_scale_sharing_from_marks(
    marks: &[Arc<dyn CompiledMark>],
) -> HashMap<String, ScaleSharing> {
    let mut scale_sharing_by_channel = HashMap::new();

    // Collect all unique channel names from all marks
    let mut all_channels = std::collections::HashSet::new();
    for m in marks {
        for ch in m.data_context().channels().keys() {
            all_channels.insert(ch.clone());
        }
    }

    // Determine sharing mode for each channel using max level
    // (higher level = more global sharing)
    for ch in all_channels {
        let mut max_level: u8 = 0; // Start with Free/Level(0)
        for m in marks {
            if let Some(cv) = m.data_context().channels().get(&ch) {
                if let Some(share_mode) = cv.get_share_mode() {
                    max_level = max_level.max(share_mode.to_level());
                }
            }
        }
        scale_sharing_by_channel.insert(ch, ScaleSharing::from_level(max_level));
    }

    scale_sharing_by_channel
}

/// Compute scale sharing mode for nested facet scenarios.
/// When the marks contain a nested facet (e.g., FacetRow inside FacetCol),
/// we need to look at the INNERMOST subplot's marks to get the x/y channel
/// scale sharing from the actual channel configurations (e.g., `.x_with(..., |c| c.with_scale_sharing(...))`).
///
/// This function recursively searches through all nesting levels to find x/y channel
/// scale sharing from the deepest marks (e.g., Symbol, Line, etc.).
pub fn compute_scale_sharing_for_nested_facet(
    marks: &[Arc<dyn CompiledMark>],
) -> HashMap<String, ScaleSharing> {
    let mut scale_sharing = HashMap::new();

    // Recursively extract scale sharing from marks
    extract_scale_sharing_recursive(marks, &mut scale_sharing);

    scale_sharing
}

/// Recursively extract x/y scale sharing from marks, traversing through nested facets.
fn extract_scale_sharing_recursive(
    marks: &[Arc<dyn CompiledMark>],
    scale_sharing: &mut HashMap<String, ScaleSharing>,
) {
    for m in marks {
        let mark_type = m.mark_type();

        // Check for nested FacetRow - recurse into its subplot
        if mark_type == "facet_row" {
            if let Some(facet_row) = m.as_any().downcast_ref::<CompiledFacetRow>() {
                extract_scale_sharing_recursive(
                    &facet_row.compiled_subplot.marks,
                    scale_sharing,
                );
                continue;
            }
        }

        // Check for nested FacetCol - recurse into its subplot
        if mark_type == "facet_col" {
            if let Some(facet_col) = m.as_any().downcast_ref::<CompiledFacetCol>() {
                extract_scale_sharing_recursive(
                    &facet_col.compiled_subplot.marks,
                    scale_sharing,
                );
                continue;
            }
        }

        // For non-facet marks, extract x/y channel scale sharing
        let channels = m.data_context().channels();
        for channel_name in ["x", "y"] {
            if let Some(channel_value) = channels.get(channel_name) {
                let share_mode = channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                // Only update if not Free (keep the most specific non-Free sharing mode)
                if !share_mode.is_free() {
                    scale_sharing.insert(channel_name.to_string(), share_mode);
                } else if !scale_sharing.contains_key(channel_name) {
                    // Set Free as default if no value set yet
                    scale_sharing.insert(channel_name.to_string(), share_mode);
                }
            }
        }
    }
}

/// Recursively check if marks contain a nested facet of the specified type.
///
/// This is used to determine whether cached overflow values need re-computation.
/// When a guide's subplot (or any descendant) contains a same-type facet, the cached
/// overflow includes that nested facet's labels in the same dimension the current
/// guide uses, causing inflation. In this case, we need to re-compute intrinsic overflow.
///
/// The search is recursive because overflow values propagate up through the entire
/// subtree. For example, in FacetCol > FacetRow > FacetCol > Cartesian:
/// - The outermost FacetColGuide receives overflow from FacetRow
/// - But FacetRow's overflow includes FacetCol (Team)'s TOP/BOTTOM labels
/// - So we need to search the entire subtree, not just immediate children
///
/// # Arguments
/// * `marks` - The marks to search through
/// * `facet_type` - The facet type to look for ("facet_row" or "facet_col")
///
/// # Returns
/// `true` if a nested facet of the specified type is found anywhere in the subtree
pub fn marks_contain_nested_facet_type(marks: &[Arc<dyn CompiledMark>], facet_type: &str) -> bool {
    for m in marks {
        let mark_type = m.mark_type();

        // Check if this mark is the type we're looking for
        if mark_type == facet_type {
            return true;
        }

        // Recursively search into nested facets
        if mark_type == "facet_row" {
            if let Some(facet_row) = m.as_any().downcast_ref::<CompiledFacetRow>() {
                if marks_contain_nested_facet_type(&facet_row.compiled_subplot.marks, facet_type) {
                    return true;
                }
            }
        } else if mark_type == "facet_col" {
            if let Some(facet_col) = m.as_any().downcast_ref::<CompiledFacetCol>() {
                if marks_contain_nested_facet_type(&facet_col.compiled_subplot.marks, facet_type) {
                    return true;
                }
            }
        }
    }
    false
}

/// Build a CompiledPartition for a facet from available information.
///
/// This helper creates a partition entry for the grammar-based partition list,
/// enabling structure-derived visibility decisions for nested facets.
///
/// # Arguments
/// * `channel_name` - The channel name (e.g., "row" or "column")
/// * `direction` - The facet direction (Row or Column)
/// * `domain_values` - The ordered domain values for this partition
/// * `facet_scale_sharing` - The scale sharing mode for this facet dimension
///
/// # Returns
/// A CompiledPartition representing this facet's partition in the hierarchy
pub fn build_partition_for_facet(
    channel_name: &str,
    direction: FacetDirection,
    domain_values: &[ScalarValue],
    facet_scale_sharing: Option<ScaleSharing>,
) -> CompiledPartition {
    let partition_values: Vec<PartitionValue> = domain_values
        .iter()
        .map(PartitionValue::from_scalar)
        .collect();

    CompiledPartition {
        field: channel_name.to_string(),
        direction,
        domain_sharing: facet_scale_sharing
            .map(|s| s.to_level())
            .unwrap_or(0),
        domain_values: partition_values,
    }
}

/// Build or extend a partition list with a new partition.
///
/// This helper takes an optional incoming partition list (from parent facets),
/// and adds a new partition for the current facet level. If no incoming list
/// exists, creates a new one.
///
/// # Arguments
/// * `incoming_list` - Optional partition list from parent coordination context
/// * `new_partition` - The partition to add for the current facet level
///
/// # Returns
/// A FacetPartitionList containing all partitions from outermost to current level
pub fn extend_partition_list(
    incoming_list: Option<&FacetPartitionList>,
    new_partition: CompiledPartition,
) -> FacetPartitionList {
    let mut list = incoming_list.cloned().unwrap_or_default();
    list.push(new_partition);
    list
}
