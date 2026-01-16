//! Visibility resolution for facet guides.
//!
//! This module provides a single source of truth for facet guide visibility decisions,
//! ensuring that `measure_overflow()` and `evaluate()` make identical decisions about
//! what elements to include/render.

use crate::cartesian::axis::AxisPosition;
use crate::channel::config_traits::ScaleSharing;
use crate::facet::context::FacetContext;

/// Visibility decisions for FacetRowGuide elements.
///
/// Created via `resolve()` to ensure both measurement and rendering
/// use the same logic for determining what elements are visible.
#[derive(Debug, Clone)]
pub struct FacetRowVisibility {
    /// Whether the y-axis is positioned on the right side of subplots
    pub axis_on_right: bool,

    /// Whether this FacetRowGuide is at the left edge of the parent grid
    pub is_left_edge: bool,

    /// Whether this FacetRowGuide is at the right edge of the parent grid
    pub is_right_edge: bool,

    /// Whether the y-axis scale is shared across columns
    pub y_is_shared_across_cols: bool,

    /// Whether the facet title (e.g., "species") should be rendered
    /// Title is only rendered on outer edge, unified across the grid
    pub render_facet_title: bool,

    /// Whether facet labels (e.g., "setosa", "versicolor") should be rendered
    /// Labels repeat when nested because inner domains can differ per outer cell
    pub render_facet_labels: bool,

    /// Whether facet labels are placed on the left side (opposite of y-axis)
    pub facet_labels_on_left: bool,

    /// Whether unified y-axis title should be rendered
    pub render_unified_y_title: bool,

    /// Whether the parent facet has unified y-axis
    pub parent_unified_y: bool,

    /// Whether this guide is nested inside a column facet
    pub nested_in_col_facet: bool,
}

/// Input values needed to resolve FacetRowGuide visibility.
///
/// These are pre-extracted from the guide's internal structures by the caller,
/// allowing the visibility module to remain decoupled from internal types.
#[derive(Debug, Clone)]
pub struct FacetRowVisibilityInput {
    /// Y-axis position from the subplot's compiled guide, if available
    pub y_axis_position: Option<AxisPosition>,

    /// Maximum left overflow from subplot measurement
    pub max_left: f32,

    /// Maximum right overflow from subplot measurement
    pub max_right: f32,

    /// Whether a unified y-axis title is configured
    pub has_unified_y_title: bool,

    /// Scale sharing mode for this row facet variable (not the data y-scale).
    /// This determines whether the facet arrangement is shared (same rows in all columns)
    /// or free (different rows per column based on filtered data).
    pub facet_scale_sharing: Option<ScaleSharing>,
}

impl FacetRowVisibility {
    /// Resolve visibility decisions based on input values and context.
    ///
    /// This function extracts the common logic used by both `measure_overflow()`
    /// and `evaluate()` to determine visibility of facet elements.
    ///
    /// # Arguments
    /// * `input` - Pre-extracted values from the guide
    /// * `parent_ctx` - Parent facet context, if available
    pub fn resolve(input: &FacetRowVisibilityInput, parent_ctx: Option<&FacetContext>) -> Self {
        // Determine y-axis position from subplot guide (default is left)
        let axis_on_right = match input.y_axis_position {
            Some(AxisPosition::Right) => true,
            Some(AxisPosition::Left) => false,
            None => {
                // axis_position returns None when position is explicit expression
                // Infer from overflow: if right > left, y-axis is likely at right
                input.max_right > input.max_left
            }
            _ => false, // fallback: left (Top/Bottom variants)
        };
        // Check parent FacetContext for unified y and grid position
        let parent_unified_y = parent_ctx
            .map(|ctx| ctx.is_channel_unified("y"))
            .unwrap_or(false);

        // Check if we're nested inside a column facet
        let nested_in_col_facet = parent_ctx
            .map(|ctx| ctx.grid_dimensions.1 > 1) // num_cols > 1 means we're inside a column facet
            .unwrap_or(false);

        // Determine if we're on left/right edge of parent grid
        let (is_left_edge, is_right_edge) = if let Some(ctx) = parent_ctx {
            let col = ctx.position.1;
            let num_cols = ctx.grid_dimensions.1;
            (col == 0, col == num_cols - 1)
        } else {
            (true, true) // No parent = standalone FacetRowGuide, both edges
        };

        // Check if y-axis scale is shared across columns (for axis visibility, not facet labels)
        let y_sharing_mode = parent_ctx
            .and_then(|ctx| ctx.scale_sharing.get("y"))
            .copied()
            .unwrap_or(ScaleSharing::Free);

        let y_is_shared_across_cols = matches!(
            y_sharing_mode,
            ScaleSharing::Shared | ScaleSharing::Level(1..)
        );

        // Facet labels go on the opposite side from the y-axis
        let facet_labels_on_left = axis_on_right;

        // Facet TITLE (e.g., "species") is only rendered on outer edge
        // Left labels: only on leftmost column (col == 0)
        // Right labels: only on rightmost column (col == num_cols - 1)
        let render_facet_title = if facet_labels_on_left {
            is_left_edge
        } else {
            is_right_edge
        };

        // Facet LABELS (e.g., "setosa", "versicolor") behavior depends on nesting AND FACET scale sharing:
        // - When nested AND facet arrangement is FREE: labels repeat (each outer cell may have different rows)
        // - When nested AND facet arrangement is SHARED: labels only on outer edge (same rows everywhere)
        // - When not nested: only on outer edge (same as title)
        //
        // IMPORTANT: This checks the FACET VARIABLE's scale_sharing (from `.row_with(..., |c| c.facet(|f| f.share_scale()))`),
        // NOT the data y-axis scale sharing. These are different concepts:
        // - Facet arrangement sharing: Whether the row structure is the same across all columns
        // - Data scale sharing: Whether the y-axis domain is the same across all columns
        let facet_is_shared = matches!(input.facet_scale_sharing, Some(ScaleSharing::Shared));
        let render_facet_labels = if nested_in_col_facet && !facet_is_shared {
            true // Inner facet labels repeat because row arrangement can differ per outer column
        } else {
            render_facet_title // Same as title when not nested or when facet is shared
        };

        // Unified y title is rendered only if:
        // 1. A unified y title is configured
        // 2. Parent hasn't already unified y
        // 3. Not nested inside a column facet (which handles unified y title)
        let render_unified_y_title =
            input.has_unified_y_title && !parent_unified_y && !nested_in_col_facet;

        Self {
            axis_on_right,
            is_left_edge,
            is_right_edge,
            y_is_shared_across_cols,
            render_facet_title,
            render_facet_labels,
            facet_labels_on_left,
            render_unified_y_title,
            parent_unified_y,
            nested_in_col_facet,
        }
    }
}

/// Visibility decisions for FacetColGuide elements.
///
/// Created via `resolve()` to ensure both measurement and rendering
/// use the same logic for determining what elements are visible.
/// This is the column facet counterpart to FacetRowVisibility.
#[derive(Debug, Clone)]
pub struct FacetColVisibility {
    /// Whether facet labels should be placed below the plot (true) or above (false)
    pub place_below: bool,

    /// Whether the x-axis is positioned at the top of subplots
    pub x_axis_at_top: bool,

    /// Whether the y-axis is positioned on the right side of subplots
    pub y_axis_on_right: bool,

    /// Whether this FacetColGuide is at the top edge of the parent grid
    pub is_top_edge: bool,

    /// Whether this FacetColGuide is at the bottom edge of the parent grid
    pub is_bottom_edge: bool,

    /// Whether the x-axis scale is shared across rows
    pub x_is_shared_across_rows: bool,

    /// Whether the facet title (e.g., "petal_width_bin") should be rendered
    /// Title is only rendered on outer edge, unified across the grid
    pub render_facet_title: bool,

    /// Whether facet labels (e.g., "narrow", "medium") should be rendered
    /// Labels repeat when nested because inner domains can differ per outer cell
    pub render_facet_labels: bool,

    /// Whether unified x-axis title should be rendered
    pub render_unified_x_title: bool,

    /// Whether unified y-axis title should be rendered
    pub render_unified_y_title: bool,

    /// Whether the parent facet has unified x-axis
    pub parent_unified_x: bool,

    /// Whether the parent facet has unified y-axis
    pub parent_unified_y: bool,

    /// Whether this guide is nested inside a row facet
    pub nested_in_row_facet: bool,
}

/// Input values needed to resolve FacetColGuide visibility.
///
/// These are pre-extracted from the guide's internal structures by the caller,
/// allowing the visibility module to remain decoupled from internal types.
#[derive(Debug, Clone)]
pub struct FacetColVisibilityInput {
    /// X-axis position from the subplot's compiled guide, if available
    pub x_axis_position: Option<AxisPosition>,

    /// Y-axis position from the subplot's compiled guide, if available
    pub y_axis_position: Option<AxisPosition>,

    /// Maximum top overflow from subplot measurement
    pub max_top: f32,

    /// Maximum bottom overflow from subplot measurement
    pub max_bottom: f32,

    /// Whether a unified x-axis title is configured
    pub has_unified_x_title: bool,

    /// Whether a unified y-axis title is configured
    pub has_unified_y_title: bool,

    /// Scale sharing mode for this column facet variable (not the data x-scale).
    /// This determines whether the facet arrangement is shared (same columns in all rows)
    /// or free (different columns per row based on filtered data).
    pub facet_scale_sharing: Option<ScaleSharing>,
}

impl FacetColVisibility {
    /// Resolve visibility decisions based on input values and context.
    ///
    /// This function extracts the common logic used by both `measure_overflow()`
    /// and `evaluate()` to determine visibility of facet elements.
    ///
    /// # Arguments
    /// * `input` - Pre-extracted values from the guide
    /// * `parent_ctx` - Parent facet context, if available
    pub fn resolve(input: &FacetColVisibilityInput, parent_ctx: Option<&FacetContext>) -> Self {
        // Determine x-axis position from subplot guide (default is bottom)
        let x_axis_at_top = match input.x_axis_position {
            Some(AxisPosition::Top) => true,
            Some(AxisPosition::Bottom) => false,
            None => {
                // axis_position returns None when position is explicit expression
                // Infer from overflow: if top > bottom, x-axis is likely at top
                input.max_top > input.max_bottom
            }
            _ => false, // fallback: bottom (Left/Right variants)
        };

        // Determine y-axis position from subplot guide (default is left)
        let y_axis_on_right = match input.y_axis_position {
            Some(AxisPosition::Right) => true,
            Some(AxisPosition::Left) => false,
            None | _ => false, // fallback: left
        };

        // Facet labels go on the opposite side from the x-axis
        // x-axis at bottom → labels above (place_below=false)
        // x-axis at top → labels below (place_below=true)
        let place_below = x_axis_at_top;

        // Check parent FacetContext for unified axes and grid position
        let parent_unified_x = parent_ctx
            .map(|ctx| ctx.is_channel_unified("x"))
            .unwrap_or(false);
        let parent_unified_y = parent_ctx
            .map(|ctx| ctx.is_channel_unified("y"))
            .unwrap_or(false);

        // Check if we're nested inside a row facet
        let nested_in_row_facet = parent_ctx
            .map(|ctx| ctx.grid_dimensions.0 > 1) // num_rows > 1 means we're inside a row facet
            .unwrap_or(false);

        // Determine if we're on top/bottom edge of parent grid
        let (is_top_edge, is_bottom_edge) = if let Some(ctx) = parent_ctx {
            let row = ctx.position.0;
            let num_rows = ctx.grid_dimensions.0;
            (row == 0, row == num_rows - 1)
        } else {
            (true, true) // No parent = standalone FacetColGuide, both edges
        };

        // Check if x-axis scale is shared across rows (for axis visibility, not facet labels)
        let x_sharing_mode = parent_ctx
            .and_then(|ctx| ctx.scale_sharing.get("x"))
            .copied()
            .unwrap_or(ScaleSharing::Free);

        let x_is_shared_across_rows = matches!(
            x_sharing_mode,
            ScaleSharing::Shared | ScaleSharing::Level(1..)
        );

        // Facet TITLE (e.g., "petal_width_bin") is only rendered on outer edge
        // Title above (place_below=false): only on topmost row (row == 0)
        // Title below (place_below=true): only on bottommost row (row == num_rows - 1)
        let render_facet_title = if place_below {
            is_bottom_edge
        } else {
            is_top_edge
        };

        // Facet LABELS (e.g., "narrow", "medium") behavior depends on nesting AND FACET scale sharing:
        // - When nested AND facet arrangement is FREE: labels repeat (each outer cell may have different columns)
        // - When nested AND facet arrangement is SHARED: labels only on outer edge (same columns everywhere)
        // - When not nested: only on outer edge (same as title)
        //
        // IMPORTANT: This checks the FACET VARIABLE's scale_sharing (from `.col_with(..., |c| c.facet(|f| f.share_scale()))`),
        // NOT the data x-axis scale sharing. These are different concepts:
        // - Facet arrangement sharing: Whether the column structure is the same across all rows
        // - Data scale sharing: Whether the x-axis domain is the same across all rows
        let facet_is_shared = matches!(input.facet_scale_sharing, Some(ScaleSharing::Shared));
        let render_facet_labels = if nested_in_row_facet && !facet_is_shared {
            true // Inner facet labels repeat because column arrangement can differ per outer row
        } else {
            render_facet_title // Same as title when not nested or when facet is shared
        };

        // Unified x title is rendered only if:
        // 1. A unified x title is configured
        // 2. Parent hasn't already unified x
        // 3. This is the correct edge (bottom for bottom axis, top for top axis)
        // Note: FacetRowGuide doesn't handle unified x title (only y), so we render it
        // even when nested in a row facet. The parent_unified_x check handles the case
        // where another FacetColGuide parent has already rendered the unified x title.
        let is_x_title_edge = if x_axis_at_top {
            is_top_edge
        } else {
            is_bottom_edge
        };
        let render_unified_x_title =
            input.has_unified_x_title && !parent_unified_x && is_x_title_edge;

        // Unified y title is rendered only if:
        // 1. A unified y title is configured
        // 2. Parent hasn't already unified y
        let render_unified_y_title = input.has_unified_y_title && !parent_unified_y;

        Self {
            place_below,
            x_axis_at_top,
            y_axis_on_right,
            is_top_edge,
            is_bottom_edge,
            x_is_shared_across_rows,
            render_facet_title,
            render_facet_labels,
            render_unified_x_title,
            render_unified_y_title,
            parent_unified_x,
            parent_unified_y,
            nested_in_row_facet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_facet_row_visibility_standalone() {
        // Standalone FacetRowGuide (no parent context)
        let input = FacetRowVisibilityInput {
            y_axis_position: None,
            max_left: 50.0,
            max_right: 10.0,
            has_unified_y_title: true,
            facet_scale_sharing: None,
        };

        let vis = FacetRowVisibility::resolve(&input, None);

        // No parent = both edges
        assert!(vis.is_left_edge);
        assert!(vis.is_right_edge);
        // No parent unified y
        assert!(!vis.parent_unified_y);
        // Should render facet labels (standalone)
        assert!(vis.render_facet_labels);
        // Should render unified y title
        assert!(vis.render_unified_y_title);
        // axis_on_right should be false (left overflow > right overflow)
        assert!(!vis.axis_on_right);
    }

    #[test]
    fn test_axis_on_right_inference() {
        // When y_axis_position is None, infer from overflow
        let input_right = FacetRowVisibilityInput {
            y_axis_position: None,
            max_left: 10.0,
            max_right: 50.0, // Right overflow larger = axis likely on right
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        let vis = FacetRowVisibility::resolve(&input_right, None);
        assert!(vis.axis_on_right);

        // When y_axis_position is explicitly Left
        let input_left = FacetRowVisibilityInput {
            y_axis_position: Some(AxisPosition::Left),
            max_left: 10.0,
            max_right: 50.0, // Even though right is larger, explicit position wins
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        let vis = FacetRowVisibility::resolve(&input_left, None);
        assert!(!vis.axis_on_right);
    }

    #[test]
    fn test_render_facet_labels_left_placement() {
        // When axis is on right, labels go on left
        // Only leftmost column should render labels
        let input = FacetRowVisibilityInput {
            y_axis_position: Some(AxisPosition::Right),
            max_left: 10.0,
            max_right: 50.0,
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        // No parent context means both edges = true
        let vis = FacetRowVisibility::resolve(&input, None);
        assert!(vis.axis_on_right);
        assert!(vis.facet_labels_on_left);
        assert!(vis.render_facet_labels); // Both edges true for standalone
    }

    // ============================================================================
    // FacetColVisibility Tests
    // ============================================================================

    #[test]
    fn test_facet_col_visibility_standalone() {
        // Standalone FacetColGuide (no parent context)
        let input = FacetColVisibilityInput {
            x_axis_position: None,
            y_axis_position: None,
            max_top: 10.0,
            max_bottom: 50.0, // Bottom overflow larger = x-axis at bottom
            has_unified_x_title: true,
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        let vis = FacetColVisibility::resolve(&input, None);

        // No parent = both edges
        assert!(vis.is_top_edge);
        assert!(vis.is_bottom_edge);
        // No parent unified x
        assert!(!vis.parent_unified_x);
        // Should render facet labels (standalone)
        assert!(vis.render_facet_labels);
        // Should render unified x title
        assert!(vis.render_unified_x_title);
        // x_axis_at_top should be false (bottom overflow > top overflow)
        assert!(!vis.x_axis_at_top);
        // Labels above when x-axis at bottom
        assert!(!vis.place_below);
    }

    #[test]
    fn test_x_axis_at_top_inference() {
        // When x_axis_position is None, infer from overflow
        let input_top = FacetColVisibilityInput {
            x_axis_position: None,
            y_axis_position: None,
            max_top: 50.0, // Top overflow larger = axis likely at top
            max_bottom: 10.0,
            has_unified_x_title: false,
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        let vis = FacetColVisibility::resolve(&input_top, None);
        assert!(vis.x_axis_at_top);
        assert!(vis.place_below); // Labels below when x-axis at top

        // When x_axis_position is explicitly Bottom
        let input_bottom = FacetColVisibilityInput {
            x_axis_position: Some(AxisPosition::Bottom),
            y_axis_position: None,
            max_top: 50.0, // Even though top is larger, explicit position wins
            max_bottom: 10.0,
            has_unified_x_title: false,
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        let vis = FacetColVisibility::resolve(&input_bottom, None);
        assert!(!vis.x_axis_at_top);
        assert!(!vis.place_below); // Labels above when x-axis at bottom
    }

    #[test]
    fn test_render_col_facet_labels_below_placement() {
        // When x-axis is at top, labels go below
        // Only bottommost row should render labels
        let input = FacetColVisibilityInput {
            x_axis_position: Some(AxisPosition::Top),
            y_axis_position: None,
            max_top: 50.0,
            max_bottom: 10.0,
            has_unified_x_title: false,
            has_unified_y_title: false,
            facet_scale_sharing: None,
        };

        // No parent context means both edges = true
        let vis = FacetColVisibility::resolve(&input, None);
        assert!(vis.x_axis_at_top);
        assert!(vis.place_below);
        assert!(vis.render_facet_labels); // Both edges true for standalone
    }
}
