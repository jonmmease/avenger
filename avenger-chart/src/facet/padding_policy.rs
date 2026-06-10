//! Padding policy for parent/child facet coordination.

/// Maximum tolerated child-padding delta before parent keeps its own padding.
pub(crate) const MAX_CHILD_PADDING_PROPAGATION_DELTA: f32 = 16.0;

/// Minimum main-axis gap between adjacent renderable facet subplots.
pub(crate) const MIN_SUBPLOT_MAIN_GAP: f32 = 12.0;

/// The effective main-axis gap for a requested inner padding: facet policy
/// floors every inter-subplot gap at [`MIN_SUBPLOT_MAIN_GAP`].
pub(crate) fn main_axis_gap(padding_inner_px: f32) -> f32 {
    padding_inner_px.max(MIN_SUBPLOT_MAIN_GAP)
}

/// Derive parent padding from parent-local and child-observed padding values.
///
/// Preserves nested-grid alignment for moderate child deltas while preventing
/// large legend-driven child spikes from forcing oversized parent gaps.
pub(crate) fn derive_parent_padding(parent_padding: f32, child_padding: f32) -> f32 {
    if child_padding <= parent_padding + MAX_CHILD_PADDING_PROPAGATION_DELTA {
        parent_padding.max(child_padding)
    } else {
        parent_padding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_padding_within_delta_propagates() {
        assert_eq!(derive_parent_padding(10.0, 22.0), 22.0);
    }

    #[test]
    fn child_padding_above_delta_is_ignored() {
        assert_eq!(derive_parent_padding(10.0, 40.0), 10.0);
    }
}
