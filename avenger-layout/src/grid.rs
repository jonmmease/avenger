//! Grid track requirements and solving.
//!
//! This is the general solver: rectangular slots with row/column spans, track
//! sizes derived from content, and one gap rule shared by every consumer:
//! `gap(i, i + 1) = max(min_gap, trailing[i].total + leading[i + 1].total)`.

use std::fmt;

use crate::geometry::{Edges, Size};
use crate::region::EdgeGrant;

/// Two-dimensional grid track count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridShape {
    pub rows: usize,
    pub columns: usize,
}

/// Rectangular slot occupied by one child in a grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridSlot {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
}

impl GridSlot {
    pub fn row_end(self) -> usize {
        self.row + self.row_span
    }

    pub fn column_end(self) -> usize {
        self.column + self.column_span
    }
}

/// Content and edge demand exported by one grid child.
#[derive(Clone, Debug, PartialEq)]
pub struct GridItem<Id = usize> {
    pub id: Id,
    pub slot: GridSlot,
    pub content_size: Size,
    pub guide_edges: Edges<f32>,
    pub legend_edges: Edges<f32>,
    pub total_edges: Edges<f32>,
}

/// Per-axis spacing policy for a sequence of grid tracks.
///
/// `min_gap` is the floor applied to every inter-track gap:
/// `gap(i, i+1) = max(min_gap, trailing[i].total + leading[i+1].total)`.
/// The first track's leading edge and the last track's trailing edge stay
/// excluded from the content extent (they overlap container overflow).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackSpacing {
    pub outer_start: f32,
    pub outer_end: f32,
    pub min_gap: f32,
}

impl TrackSpacing {
    pub fn merge_max(self, other: Self) -> Self {
        Self {
            outer_start: self.outer_start.max(other.outer_start),
            outer_end: self.outer_end.max(other.outer_end),
            min_gap: self.min_gap.max(other.min_gap),
        }
    }

    pub fn abs_delta(self, other: Self) -> f32 {
        (other.outer_start - self.outer_start).abs()
            + (other.outer_end - self.outer_end).abs()
            + (other.min_gap - self.min_gap).abs()
    }
}

/// Effective gap between adjacent tracks `i` and `i + 1` on one axis.
#[inline]
fn track_gap(trailing_total: f32, leading_total: f32, min_gap: f32) -> f32 {
    (trailing_total + leading_total).max(min_gap.max(0.0))
}

/// Track sizes and edge requirements needed to align one measured grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GridRequirements {
    pub shape: GridShape,
    pub column_spacing: TrackSpacing,
    pub row_spacing: TrackSpacing,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub column_left: Vec<EdgeGrant>,
    pub column_right: Vec<EdgeGrant>,
    pub row_top: Vec<EdgeGrant>,
    pub row_bottom: Vec<EdgeGrant>,
}

/// Solved track starts and effective content size for one grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSolution {
    pub column_spacing: TrackSpacing,
    pub row_spacing: TrackSpacing,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub column_left: Vec<EdgeGrant>,
    pub column_right: Vec<EdgeGrant>,
    pub row_top: Vec<EdgeGrant>,
    pub row_bottom: Vec<EdgeGrant>,
    pub column_starts: Vec<f32>,
    pub row_starts: Vec<f32>,
    pub content_size: Size,
}

/// Error produced while deriving grid requirements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    /// An item's slot rectangle has a zero span or falls outside the grid
    /// shape.
    SlotOutOfBounds { slot: GridSlot, shape: GridShape },
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlotOutOfBounds { slot, shape } => write!(
                f,
                "Grid item slot {:?} exceeds grid shape {}x{}",
                slot, shape.rows, shape.columns
            ),
        }
    }
}

impl std::error::Error for GridError {}

fn grid_edge_demand<Id>(demand: &GridItem<Id>, side: crate::geometry::Side) -> EdgeGrant {
    use crate::geometry::Side;
    match side {
        Side::Top => EdgeGrant::new(
            demand.guide_edges.top,
            demand.legend_edges.top,
            demand.total_edges.top,
        ),
        Side::Right => EdgeGrant::new(
            demand.guide_edges.right,
            demand.legend_edges.right,
            demand.total_edges.right,
        ),
        Side::Bottom => EdgeGrant::new(
            demand.guide_edges.bottom,
            demand.legend_edges.bottom,
            demand.total_edges.bottom,
        ),
        Side::Left => EdgeGrant::new(
            demand.guide_edges.left,
            demand.legend_edges.left,
            demand.total_edges.left,
        ),
    }
}

impl GridRequirements {
    /// Derive per-track sizes and edge demands from measured grid items.
    pub fn from_items<Id>(
        shape: GridShape,
        base_cell_size: Size,
        demands: &[GridItem<Id>],
    ) -> Result<GridRequirements, GridError> {
        for demand in demands {
            let slot = demand.slot;
            if slot.row_span == 0
                || slot.column_span == 0
                || slot.row_end() > shape.rows
                || slot.column_end() > shape.columns
            {
                return Err(GridError::SlotOutOfBounds { slot, shape });
            }
        }
        Ok(Self::from_items_validated(shape, base_cell_size, demands))
    }

    /// Like [`GridRequirements::from_items`] for callers whose slots are
    /// correct by construction (the band adapter derives every slot from
    /// the item index, so out-of-bounds is structurally impossible).
    /// Bounds are debug-asserted only.
    pub(crate) fn from_items_validated<Id>(
        shape: GridShape,
        base_cell_size: Size,
        demands: &[GridItem<Id>],
    ) -> GridRequirements {
        use crate::geometry::Side;

        let mut requirements = GridRequirements {
            shape,
            column_spacing: TrackSpacing::default(),
            row_spacing: TrackSpacing::default(),
            column_widths: vec![base_cell_size.width; shape.columns],
            row_heights: vec![base_cell_size.height; shape.rows],
            column_left: vec![EdgeGrant::default(); shape.columns],
            column_right: vec![EdgeGrant::default(); shape.columns],
            row_top: vec![EdgeGrant::default(); shape.rows],
            row_bottom: vec![EdgeGrant::default(); shape.rows],
        };

        for demand in demands {
            let slot = demand.slot;
            debug_assert!(
                slot.row_span > 0
                    && slot.column_span > 0
                    && slot.row_end() <= shape.rows
                    && slot.column_end() <= shape.columns,
                "slot out of bounds: {slot:?} in {shape:?}"
            );

            let last_row = slot.row_end() - 1;
            let last_column = slot.column_end() - 1;
            requirements.column_left[slot.column] = requirements.column_left[slot.column]
                .max_components(grid_edge_demand(demand, Side::Left));
            requirements.column_right[last_column] = requirements.column_right[last_column]
                .max_components(grid_edge_demand(demand, Side::Right));
            requirements.row_top[slot.row] =
                requirements.row_top[slot.row].max_components(grid_edge_demand(demand, Side::Top));
            requirements.row_bottom[last_row] = requirements.row_bottom[last_row]
                .max_components(grid_edge_demand(demand, Side::Bottom));

            if slot.column_span == 1 {
                requirements.column_widths[slot.column] =
                    requirements.column_widths[slot.column].max(demand.content_size.width);
            }
            if slot.row_span == 1 {
                requirements.row_heights[slot.row] =
                    requirements.row_heights[slot.row].max(demand.content_size.height);
            }
        }

        requirements
    }

    /// Merge compatible grid requirements by component-wise maximum.
    /// Returns `None` for empty input or mismatched shapes.
    pub fn merge_max<'a, I>(requirements: I) -> Option<Self>
    where
        I: IntoIterator<Item = &'a Self>,
        Self: 'a,
    {
        let mut iter = requirements.into_iter();
        let first = iter.next()?.clone();
        iter.try_fold(first, |mut merged, next| {
            merged.merge_max_assign(next).then_some(merged)
        })
    }

    /// Merge another requirement payload into this one by component-wise
    /// maximum. Returns `false` when the shapes are incompatible.
    pub fn merge_max_assign(&mut self, other: &Self) -> bool {
        if self.shape != other.shape {
            return false;
        }

        self.column_spacing = self.column_spacing.merge_max(other.column_spacing);
        self.row_spacing = self.row_spacing.merge_max(other.row_spacing);
        max_assign_each(&mut self.column_widths, &other.column_widths);
        max_assign_each(&mut self.row_heights, &other.row_heights);
        max_assign_edge_each(&mut self.column_left, &other.column_left);
        max_assign_edge_each(&mut self.column_right, &other.column_right);
        max_assign_edge_each(&mut self.row_top, &other.row_top);
        max_assign_edge_each(&mut self.row_bottom, &other.row_bottom);
        true
    }

    /// Total absolute track-size difference against another requirement set.
    pub fn content_delta(&self, other: &Self) -> f32 {
        abs_delta_sum(&self.column_widths, &other.column_widths)
            + abs_delta_sum(&self.row_heights, &other.row_heights)
    }

    /// Total absolute edge/spacing difference against another requirement set.
    pub fn edge_delta(&self, other: &Self) -> f32 {
        abs_edge_delta_sum(&self.column_left, &other.column_left)
            + abs_edge_delta_sum(&self.column_right, &other.column_right)
            + abs_edge_delta_sum(&self.row_top, &other.row_top)
            + abs_edge_delta_sum(&self.row_bottom, &other.row_bottom)
            + self.column_spacing.abs_delta(other.column_spacing)
            + self.row_spacing.abs_delta(other.row_spacing)
    }

    /// Solve track starts, span constraints, and content size for one grid,
    /// with optional per-track growth kinds: span deficits distribute to
    /// `Auto` tracks first, then `Flex`, never `Fixed`. `None` treats every
    /// track as `Auto`.
    pub(crate) fn solve_with_growth<Id>(
        &self,
        demands: &[GridItem<Id>],
        column_growth: Option<&[TrackGrowth]>,
        row_growth: Option<&[TrackGrowth]>,
    ) -> GridSolution {
        let requirements = self;
        debug_assert_eq!(requirements.column_widths.len(), requirements.shape.columns);
        debug_assert_eq!(requirements.row_heights.len(), requirements.shape.rows);

        let mut column_widths = requirements.column_widths.clone();
        let mut row_heights = requirements.row_heights.clone();
        let column_right_totals = edge_demand_totals(&requirements.column_right);
        let column_left_totals = edge_demand_totals(&requirements.column_left);
        let row_bottom_totals = edge_demand_totals(&requirements.row_bottom);
        let row_top_totals = edge_demand_totals(&requirements.row_top);

        satisfy_span_axis_constraints(
            &mut column_widths,
            &column_right_totals,
            &column_left_totals,
            requirements.column_spacing.min_gap,
            column_growth,
            demands
                .iter()
                .map(|demand| AxisSpanConstraint {
                    start: demand.slot.column,
                    span: demand.slot.column_span,
                    target: demand.content_size.width,
                })
                .collect(),
        );
        satisfy_span_axis_constraints(
            &mut row_heights,
            &row_bottom_totals,
            &row_top_totals,
            requirements.row_spacing.min_gap,
            row_growth,
            demands
                .iter()
                .map(|demand| AxisSpanConstraint {
                    start: demand.slot.row,
                    span: demand.slot.row_span,
                    target: demand.content_size.height,
                })
                .collect(),
        );

        let (column_starts, content_width) = track_starts_and_content_size(
            &column_widths,
            &column_right_totals,
            &column_left_totals,
            requirements.column_spacing,
        );
        let (row_starts, content_height) = track_starts_and_content_size(
            &row_heights,
            &row_bottom_totals,
            &row_top_totals,
            requirements.row_spacing,
        );

        GridSolution {
            column_spacing: requirements.column_spacing,
            row_spacing: requirements.row_spacing,
            column_widths,
            row_heights,
            column_left: requirements.column_left.clone(),
            column_right: requirements.column_right.clone(),
            row_top: requirements.row_top.clone(),
            row_bottom: requirements.row_bottom.clone(),
            column_starts,
            row_starts,
            content_size: Size::new(content_width, content_height),
        }
    }
}

fn max_assign_each(target: &mut [f32], source: &[f32]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = (*target).max(*source);
    }
}

fn max_assign_edge_each(target: &mut [EdgeGrant], source: &[EdgeGrant]) {
    debug_assert_eq!(target.len(), source.len());
    for (target, source) in target.iter_mut().zip(source.iter()) {
        *target = target.max_components(*source);
    }
}

fn abs_delta_sum(local: &[f32], other: &[f32]) -> f32 {
    debug_assert_eq!(local.len(), other.len());
    local
        .iter()
        .zip(other.iter())
        .map(|(local, other)| (other - local).abs())
        .sum()
}

fn abs_edge_delta_sum(local: &[EdgeGrant], other: &[EdgeGrant]) -> f32 {
    debug_assert_eq!(local.len(), other.len());
    local
        .iter()
        .zip(other.iter())
        .map(|(local, other)| (other.total - local.total).abs())
        .sum()
}

/// Collect the `total` component of each edge demand.
pub(crate) fn edge_demand_totals(edges: &[EdgeGrant]) -> Vec<f32> {
    edges.iter().map(|edge| edge.total).collect()
}

/// How one track may absorb distributed space (span deficits, stretch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrackGrowth {
    Auto,
    Flex,
    Fixed,
}

#[derive(Clone, Debug)]
struct AxisSpanConstraint {
    start: usize,
    span: usize,
    target: f32,
}

fn satisfy_span_axis_constraints(
    sizes: &mut [f32],
    trailing_edges: &[f32],
    leading_edges: &[f32],
    min_gap: f32,
    growth: Option<&[TrackGrowth]>,
    mut constraints: Vec<AxisSpanConstraint>,
) {
    constraints.sort_by_key(|constraint| constraint.span);
    for constraint in constraints {
        let current = span_axis_extent(
            sizes,
            trailing_edges,
            leading_edges,
            min_gap,
            constraint.start,
            constraint.span,
        );
        let deficit = constraint.target - current;
        if deficit <= 0.0 || constraint.span == 0 {
            continue;
        }
        let range = constraint.start..constraint.start + constraint.span;
        // Deficits prefer Auto tracks, then Flex, never Fixed.
        let growable: Vec<usize> = match growth {
            None => range.collect(),
            Some(growth) => {
                let of_kind = |kind: TrackGrowth| -> Vec<usize> {
                    range
                        .clone()
                        .filter(|&index| {
                            growth.get(index).copied().unwrap_or(TrackGrowth::Auto) == kind
                        })
                        .collect()
                };
                let auto = of_kind(TrackGrowth::Auto);
                if !auto.is_empty() {
                    auto
                } else {
                    of_kind(TrackGrowth::Flex)
                }
            }
        };
        if growable.is_empty() {
            continue; // all Fixed: the spanning item overflows.
        }
        let extra_per_track = deficit / growable.len() as f32;
        for index in growable {
            sizes[index] += extra_per_track;
        }
    }
}

/// Extent of a span of tracks including the inter-track gaps it crosses.
pub(crate) fn span_axis_extent(
    sizes: &[f32],
    trailing_edges: &[f32],
    leading_edges: &[f32],
    min_gap: f32,
    start: usize,
    span: usize,
) -> f32 {
    let end = start + span;
    let track_sum = sizes[start..end].iter().sum::<f32>();
    let gap_sum = (start..end.saturating_sub(1))
        .map(|index| track_gap(trailing_edges[index], leading_edges[index + 1], min_gap))
        .sum::<f32>();
    track_sum + gap_sum
}

fn track_starts_and_content_size(
    sizes: &[f32],
    trailing_edges: &[f32],
    leading_edges: &[f32],
    spacing: TrackSpacing,
) -> (Vec<f32>, f32) {
    let mut starts = vec![0.0f32; sizes.len()];
    let mut cursor = spacing.outer_start.max(0.0);
    for index in 0..sizes.len() {
        if index > 0 {
            cursor += track_gap(
                trailing_edges[index - 1],
                leading_edges[index],
                spacing.min_gap,
            );
        }
        starts[index] = cursor;
        cursor += sizes[index];
    }
    (starts, cursor + spacing.outer_end.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_item(
        id: usize,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        content_size: Size,
        total_edges: Edges<f32>,
    ) -> GridItem {
        grid_item_with_edges(
            id,
            row,
            column,
            row_span,
            column_span,
            content_size,
            Edges::default(),
            Edges::default(),
            total_edges,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn grid_item_with_edges(
        id: usize,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        content_size: Size,
        guide_edges: Edges<f32>,
        legend_edges: Edges<f32>,
        total_edges: Edges<f32>,
    ) -> GridItem {
        GridItem {
            id,
            slot: GridSlot {
                row,
                column,
                row_span,
                column_span,
            },
            content_size,
            guide_edges,
            legend_edges,
            total_edges,
        }
    }

    #[test]
    fn grid_requirements_merge_max_and_delta_helpers() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 1,
        };
        let demands = vec![grid_item(
            0,
            0,
            0,
            1,
            1,
            Size::new(100.0, 50.0),
            Edges::default(),
        )];
        let first = GridRequirements::from_items(shape, Size::new(100.0, 50.0), &demands)?;
        let mut second = first.clone();
        second.column_widths[0] = 150.0;
        second.row_heights[0] = 70.0;
        second.column_spacing.min_gap = 8.0;
        second.column_left[0] = EdgeGrant::total_only(9.0);

        let merged = GridRequirements::merge_max([&first, &second]).expect("compatible");

        assert_eq!(merged.column_widths, vec![150.0]);
        assert_eq!(merged.row_heights, vec![70.0]);
        assert_eq!(merged.column_spacing.min_gap, 8.0);
        assert_eq!(merged.column_left, vec![EdgeGrant::total_only(9.0)]);
        assert_eq!(first.content_delta(&merged), 70.0);
        assert_eq!(first.edge_delta(&merged), 17.0);

        let other_shape = GridRequirements {
            shape: GridShape {
                rows: 1,
                columns: 2,
            },
            column_spacing: TrackSpacing::default(),
            row_spacing: TrackSpacing::default(),
            column_widths: vec![1.0, 1.0],
            row_heights: vec![1.0],
            column_left: vec![EdgeGrant::default(); 2],
            column_right: vec![EdgeGrant::default(); 2],
            row_top: vec![EdgeGrant::default(); 1],
            row_bottom: vec![EdgeGrant::default(); 1],
        };
        assert!(GridRequirements::merge_max([&first, &other_shape]).is_none());
        assert!(GridRequirements::merge_max(std::iter::empty::<&GridRequirements>()).is_none());
        Ok(())
    }

    #[test]
    fn grid_solver_matches_non_spanning_grid_placement() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 2,
            columns: 2,
        };
        let demands = vec![
            grid_item(
                0,
                0,
                0,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(1.0, 5.0, 2.0, 3.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size::new(120.0, 50.0),
                Edges::new(1.0, 4.0, 2.0, 7.0),
            ),
            grid_item(
                2,
                1,
                0,
                1,
                1,
                Size::new(100.0, 60.0),
                Edges::new(9.0, 5.0, 2.0, 3.0),
            ),
        ];

        let requirements = GridRequirements::from_items(shape, Size::new(100.0, 50.0), &demands)?;
        let solution = requirements.solve_with_growth(&demands, None, None);

        assert_eq!(solution.column_widths, vec![100.0, 120.0]);
        assert_eq!(solution.row_heights, vec![50.0, 60.0]);
        assert_eq!(solution.column_starts, vec![0.0, 112.0]);
        assert_eq!(solution.row_starts, vec![0.0, 61.0]);
        assert_eq!(solution.content_size, Size::new(232.0, 121.0));
        Ok(())
    }

    #[test]
    fn grid_solver_preserves_outer_offsets() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 1,
        };
        let demands = vec![grid_item(
            0,
            0,
            0,
            1,
            1,
            Size::new(100.0, 50.0),
            Edges::default(),
        )];

        let mut requirements =
            GridRequirements::from_items(shape, Size::new(100.0, 50.0), &demands)?;
        requirements.column_spacing = TrackSpacing {
            outer_start: 3.0,
            outer_end: 7.0,
            min_gap: 0.0,
        };
        requirements.row_spacing = TrackSpacing {
            outer_start: 5.0,
            outer_end: 11.0,
            min_gap: 0.0,
        };

        let solution = requirements.solve_with_growth(&demands, None, None);

        assert_eq!(solution.column_starts, vec![3.0]);
        assert_eq!(solution.row_starts, vec![5.0]);
        assert_eq!(solution.content_size, Size::new(110.0, 66.0));
        Ok(())
    }

    #[test]
    fn grid_solver_preserves_hole_track_positions() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 2,
            columns: 3,
        };
        let demands = vec![
            grid_item(0, 0, 0, 1, 1, Size::new(100.0, 100.0), Edges::default()),
            grid_item(1, 1, 2, 1, 1, Size::new(100.0, 100.0), Edges::default()),
        ];

        let requirements = GridRequirements::from_items(shape, Size::new(100.0, 100.0), &demands)?;
        let solution = requirements.solve_with_growth(&demands, None, None);

        assert_eq!(solution.column_starts, vec![0.0, 100.0, 200.0]);
        assert_eq!(solution.row_starts, vec![0.0, 100.0]);
        assert_eq!(solution.content_size, Size::new(300.0, 200.0));
        Ok(())
    }

    #[test]
    fn grid_requirements_reject_slot_outside_shape() {
        let shape = GridShape {
            rows: 1,
            columns: 1,
        };
        let demands = vec![grid_item(
            0,
            0,
            0,
            1,
            2,
            Size::new(100.0, 100.0),
            Edges::default(),
        )];

        let err = GridRequirements::from_items(shape, Size::new(100.0, 100.0), &demands)
            .expect_err("slot rect should exceed shape");
        assert!(err.to_string().contains("exceeds grid shape"));
    }

    #[test]
    fn grid_solver_satisfies_span_interval_constraints() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 3,
        };
        let demands = vec![
            grid_item(
                0,
                0,
                0,
                1,
                3,
                Size::new(190.0, 50.0),
                Edges::new(1.0, 6.0, 2.0, 4.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size::new(50.0, 50.0),
                Edges::new(0.0, 3.0, 0.0, 2.0),
            ),
        ];

        let requirements = GridRequirements::from_items(shape, Size::new(50.0, 50.0), &demands)?;
        let solution = requirements.solve_with_growth(&demands, None, None);

        assert_eq!(
            edge_demand_totals(&solution.column_left),
            vec![4.0, 2.0, 0.0]
        );
        assert_eq!(
            edge_demand_totals(&solution.column_right),
            vec![0.0, 3.0, 6.0]
        );
        let column_right_totals = edge_demand_totals(&solution.column_right);
        let column_left_totals = edge_demand_totals(&solution.column_left);
        let spanned_width = span_axis_extent(
            &solution.column_widths,
            &column_right_totals,
            &column_left_totals,
            solution.column_spacing.min_gap,
            0,
            3,
        );
        assert!((spanned_width - 190.0).abs() < 0.0001);
        assert_eq!(solution.column_starts[1], solution.column_widths[0] + 2.0);
        assert_eq!(
            solution.column_starts[2],
            solution.column_widths[0] + 2.0 + solution.column_widths[1] + 3.0
        );
        assert!((solution.content_size.width - 190.0).abs() < 0.0001);
        Ok(())
    }

    #[test]
    fn min_gap_floors_inter_track_gaps() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 3,
        };
        let demands = vec![
            grid_item(
                0,
                0,
                0,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(0.0, 2.0, 0.0, 0.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(0.0, 9.0, 0.0, 3.0),
            ),
            grid_item(
                2,
                0,
                2,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(0.0, 0.0, 0.0, 4.0),
            ),
        ];

        let mut requirements =
            GridRequirements::from_items(shape, Size::new(100.0, 50.0), &demands)?;
        requirements.column_spacing.min_gap = 10.0;

        let solution = requirements.solve_with_growth(&demands, None, None);

        // First pair: edge demand 2.0 + 3.0 = 5.0 < min_gap 10.0 -> floored.
        // Second pair: edge demand 9.0 + 4.0 = 13.0 > min_gap 10.0 -> demand wins.
        assert_eq!(solution.column_starts, vec![0.0, 110.0, 223.0]);
        assert_eq!(solution.content_size.width, 323.0);
        Ok(())
    }

    #[test]
    fn min_gap_participates_in_span_extents() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 2,
        };
        let demands = vec![
            grid_item(0, 0, 0, 1, 2, Size::new(250.0, 50.0), Edges::default()),
            grid_item(
                1,
                0,
                0,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(0.0, 1.0, 0.0, 0.0),
            ),
            grid_item(
                2,
                0,
                1,
                1,
                1,
                Size::new(100.0, 50.0),
                Edges::new(0.0, 0.0, 0.0, 2.0),
            ),
        ];

        let mut requirements =
            GridRequirements::from_items(shape, Size::new(100.0, 50.0), &demands)?;
        requirements.column_spacing.min_gap = 8.0;

        let solution = requirements.solve_with_growth(&demands, None, None);

        // The floored gap (max(8.0, 1.0 + 2.0) = 8.0) counts toward the span
        // target of 250.0, so each track absorbs (250 - 200 - 8) / 2 = 21.0.
        assert_eq!(solution.column_widths, vec![121.0, 121.0]);
        assert_eq!(solution.column_starts, vec![0.0, 129.0]);
        assert_eq!(solution.content_size.width, 250.0);

        // Slot content size agrees with the positional distance from span
        // start to span end content edge.
        let positional_extent =
            solution.column_starts[1] + solution.column_widths[1] - solution.column_starts[0];
        assert!((positional_extent - 250.0).abs() < 0.0001);
        Ok(())
    }
}
