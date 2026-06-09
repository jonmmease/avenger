//! Requirement merging and deltas for aligning equivalent grids.
//!
//! Callers own node identity, traversal, grouping, and application. This
//! module only merges compatible grid requirements (component-wise max) and
//! reports numeric deltas between local and merged requirements.

use crate::grid::GridRequirements;
use crate::region::EdgeDemand;

/// Merge compatible requirements by component-wise maximum.
///
/// Returns `None` when the input is empty or any two requirements have
/// different shapes.
pub fn merge_grid_requirements<'a>(
    requirements: impl IntoIterator<Item = &'a GridRequirements>,
) -> Option<GridRequirements> {
    let mut iter = requirements.into_iter();
    let first = iter.next()?.clone();
    iter.try_fold(first, |mut merged, next| {
        if merged.shape != next.shape {
            return None;
        }

        merged.column_spacing = merged.column_spacing.merge_max(next.column_spacing);
        merged.row_spacing = merged.row_spacing.merge_max(next.row_spacing);
        max_assign_each(&mut merged.column_widths, &next.column_widths);
        max_assign_each(&mut merged.row_heights, &next.row_heights);
        max_assign_edge_each(&mut merged.column_left, &next.column_left);
        max_assign_edge_each(&mut merged.column_right, &next.column_right);
        max_assign_edge_each(&mut merged.row_top, &next.row_top);
        max_assign_edge_each(&mut merged.row_bottom, &next.row_bottom);
        Some(merged)
    })
}

/// Total absolute track-size difference between local and merged requirements.
pub fn grid_content_delta(local: &GridRequirements, merged: &GridRequirements) -> f32 {
    abs_delta_sum(&local.column_widths, &merged.column_widths)
        + abs_delta_sum(&local.row_heights, &merged.row_heights)
}

/// Total absolute edge/spacing difference between local and merged
/// requirements.
pub fn grid_edge_delta(local: &GridRequirements, merged: &GridRequirements) -> f32 {
    abs_edge_delta_sum(&local.column_left, &merged.column_left)
        + abs_edge_delta_sum(&local.column_right, &merged.column_right)
        + abs_edge_delta_sum(&local.row_top, &merged.row_top)
        + abs_edge_delta_sum(&local.row_bottom, &merged.row_bottom)
        + local.column_spacing.abs_delta(merged.column_spacing)
        + local.row_spacing.abs_delta(merged.row_spacing)
}

fn max_assign_each(target: &mut [f32], source: &[f32]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = (*target).max(*source);
    }
}

fn max_assign_edge_each(target: &mut [EdgeDemand], source: &[EdgeDemand]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = target.max_components(*source);
    }
}

fn abs_delta_sum(local: &[f32], merged: &[f32]) -> f32 {
    debug_assert_eq!(local.len(), merged.len());
    local
        .iter()
        .zip(merged.iter())
        .map(|(local, merged)| (merged - local).abs())
        .sum()
}

fn abs_edge_delta_sum(local: &[EdgeDemand], merged: &[EdgeDemand]) -> f32 {
    debug_assert_eq!(local.len(), merged.len());
    local
        .iter()
        .zip(merged.iter())
        .map(|(local, merged)| (merged.total - local.total).abs())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{GridShape, TrackSpacing, total_edge_demands, zero_edge_demands};

    fn requirements(width: f32, left_total: f32) -> GridRequirements {
        GridRequirements {
            shape: GridShape {
                rows: 1,
                columns: 1,
            },
            column_spacing: TrackSpacing::default(),
            row_spacing: TrackSpacing::default(),
            column_widths: vec![width],
            row_heights: vec![40.0],
            column_left: total_edge_demands([left_total]),
            column_right: zero_edge_demands(1),
            row_top: zero_edge_demands(1),
            row_bottom: zero_edge_demands(1),
        }
    }

    #[test]
    fn merge_grid_requirements_keeps_component_maxima() {
        let mut first = requirements(100.0, 4.0);
        first.column_spacing.min_gap = 9.0;
        let mut second = requirements(80.0, 12.0);
        second.row_heights[0] = 55.0;
        second.column_spacing.min_gap = 5.0;
        second.column_spacing.outer_start = 3.0;

        let merged = merge_grid_requirements([&first, &second]).unwrap();

        assert_eq!(merged.column_widths, vec![100.0]);
        assert_eq!(merged.row_heights, vec![55.0]);
        assert_eq!(merged.column_left[0].total, 12.0);
        assert_eq!(
            merged.column_spacing,
            TrackSpacing {
                outer_start: 3.0,
                outer_end: 0.0,
                min_gap: 9.0
            }
        );
    }

    #[test]
    fn merge_grid_requirements_rejects_different_shapes() {
        let first = requirements(100.0, 4.0);
        let mut second = requirements(80.0, 12.0);
        second.shape.columns = 2;

        assert!(merge_grid_requirements([&first, &second]).is_none());
    }

    #[test]
    fn grid_requirement_deltas_are_separated_by_content_and_edge() {
        let local = requirements(100.0, 4.0);
        let mut merged = requirements(120.0, 10.0);
        merged.row_heights[0] = 45.0;
        merged.column_spacing.outer_start = 3.0;
        merged.column_spacing.min_gap = 6.0;

        assert_eq!(grid_content_delta(&local, &merged), 25.0);
        assert_eq!(grid_edge_delta(&local, &merged), 15.0);
    }
}
