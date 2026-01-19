//! Shared types and helper functions for facet guides.
//!
//! This module contains types used by both FacetRowGuide and FacetColGuide:
//! - FacetSource: representation of a compiled facet mark and data context
//! - FacetTitles: unified title references for measurement helpers
//! - Overflow aggregation and coordination helpers
//! - Scale sharing computation functions

use crate::channel::config_traits::ScaleSharing;
use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetRow};
use crate::guide::OverflowSpaceRequirement;
use crate::marks::CompiledMark;
use crate::plot::CompiledPlot;
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

    // Determine sharing mode for each channel
    for ch in all_channels {
        let mut mode = ScaleSharing::Free;
        for m in marks {
            if let Some(cv) = m.data_context().channels().get(&ch) {
                if let Some(share_mode) = cv.get_share_mode() {
                    mode = match (mode, share_mode) {
                        (ScaleSharing::Free, new_mode) => new_mode,
                        (ScaleSharing::Shared, _) => ScaleSharing::Shared,
                        (_, ScaleSharing::Shared) => ScaleSharing::Shared,
                        (existing, _) => existing,
                    };
                }
            }
        }
        scale_sharing_by_channel.insert(ch, mode);
    }

    scale_sharing_by_channel
}

/// Compute scale sharing mode for nested facet scenarios.
/// When the marks contain a nested facet (e.g., FacetRow inside FacetCol),
/// we need to look at the INNERMOST subplot's marks to get the x/y channel
/// scale sharing from the actual channel configurations (e.g., `.x_with(..., |c| c.with_scale_sharing(...))`).
pub fn compute_scale_sharing_for_nested_facet(
    marks: &[Arc<dyn CompiledMark>],
) -> HashMap<String, ScaleSharing> {
    // First try the standard approach - this won't find x/y sharing for nested facets
    // since the outer marks don't have x/y channels
    let mut scale_sharing = compute_scale_sharing_from_marks(marks);

    // Check if any mark is a nested facet and extract x/y sharing from its subplot
    for m in marks {
        let mark_type = m.mark_type();

        // Check for nested FacetRow (inside FacetCol)
        if mark_type == "facet_row" {
            if let Some(facet_row) = m.as_any().downcast_ref::<CompiledFacetRow>() {
                // Found a nested FacetRow - extract x/y sharing from its subplot's marks
                let inner_marks = &facet_row.compiled_subplot.marks;
                for inner_mark in inner_marks {
                    let channels = inner_mark.data_context().channels();
                    for channel_name in ["x", "y"] {
                        if let Some(channel_value) = channels.get(channel_name) {
                            let share_mode =
                                channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                            scale_sharing.insert(channel_name.to_string(), share_mode);
                        }
                    }
                }
                break;
            }
        }

        // Check for nested FacetCol (inside FacetRow)
        if mark_type == "facet_col" {
            if let Some(facet_col) = m.as_any().downcast_ref::<CompiledFacetCol>() {
                // Found a nested FacetCol - extract x/y sharing from its subplot's marks
                let inner_marks = &facet_col.compiled_subplot.marks;
                for inner_mark in inner_marks {
                    let channels = inner_mark.data_context().channels();
                    for channel_name in ["x", "y"] {
                        if let Some(channel_value) = channels.get(channel_name) {
                            let share_mode =
                                channel_value.get_share_mode().unwrap_or(ScaleSharing::Free);
                            scale_sharing.insert(channel_name.to_string(), share_mode);
                        }
                    }
                }
                break;
            }
        }
    }

    scale_sharing
}
