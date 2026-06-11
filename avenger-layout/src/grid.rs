//! Grid track requirements and solving.
//!
//! This is the general solver: rectangular slots with row/column spans, track
//! sizes derived from content, and one gap rule shared by every consumer:
//! `gap(i, i + 1) = max(min_gap, trailing[i].total + leading[i + 1].total)`.
//! One-dimensional bands are 1xN grids (see [`crate::band`]).

use std::fmt;

use crate::geometry::{Edges, Size};
use crate::region::{EdgeDemand, EdgeTargets};

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
    pub inner_edges: Edges<f32>,
    pub outer_edges: Edges<f32>,
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

/// Uniform single-span tracks: the policy-level requirement for `count`
/// equally sized slots sharing one [`TrackSpacing`].
///
/// This is the neutral form of band/facet layout policy, where per-track
/// geometry is uniform by construction and merging coordinates the policy
/// (max count, max spacing) rather than per-track values. Unlike
/// [`GridRequirements`], merging across different counts is well-defined.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UniformTracks {
    pub count: usize,
    pub spacing: TrackSpacing,
}

impl UniformTracks {
    pub fn merge_max(self, other: Self) -> Self {
        Self {
            count: self.count.max(other.count),
            spacing: self.spacing.merge_max(other.spacing),
        }
    }

    /// Solve the uniform arrangement for a given per-track size: starts form
    /// an arithmetic progression with the spacing's `min_gap` between
    /// tracks, offset by `outer_start`, and the extent includes both outers.
    pub fn solve(&self, track_size: f32) -> UniformTrackSolution {
        let track_size = track_size.max(0.0);
        let gap = self.spacing.min_gap.max(0.0);
        let outer_start = self.spacing.outer_start.max(0.0);
        let starts = (0..self.count)
            .map(|index| outer_start + index as f32 * (track_size + gap))
            .collect::<Vec<_>>();
        let extent = if self.count == 0 {
            outer_start + self.spacing.outer_end.max(0.0)
        } else {
            outer_start
                + self.count as f32 * track_size
                + (self.count - 1) as f32 * gap
                + self.spacing.outer_end.max(0.0)
        };
        UniformTrackSolution {
            starts,
            track_size,
            extent,
        }
    }
}

/// Solved positions for a uniform arrangement.
#[derive(Clone, Debug, PartialEq)]
pub struct UniformTrackSolution {
    pub starts: Vec<f32>,
    pub track_size: f32,
    pub extent: f32,
}

/// Track sizes and edge requirements needed to align one measured grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GridRequirements {
    pub shape: GridShape,
    pub column_spacing: TrackSpacing,
    pub row_spacing: TrackSpacing,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub column_left: Vec<EdgeDemand>,
    pub column_right: Vec<EdgeDemand>,
    pub row_top: Vec<EdgeDemand>,
    pub row_bottom: Vec<EdgeDemand>,
}

/// Solved track starts and effective content size for one grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSolution {
    pub column_spacing: TrackSpacing,
    pub row_spacing: TrackSpacing,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub column_left: Vec<EdgeDemand>,
    pub column_right: Vec<EdgeDemand>,
    pub row_top: Vec<EdgeDemand>,
    pub row_bottom: Vec<EdgeDemand>,
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

fn grid_edge_demand<Id>(demand: &GridItem<Id>, side: crate::geometry::Side) -> EdgeDemand {
    use crate::geometry::Side;
    match side {
        Side::Top => EdgeDemand::new(
            demand.inner_edges.top,
            demand.outer_edges.top,
            demand.total_edges.top,
        ),
        Side::Right => EdgeDemand::new(
            demand.inner_edges.right,
            demand.outer_edges.right,
            demand.total_edges.right,
        ),
        Side::Bottom => EdgeDemand::new(
            demand.inner_edges.bottom,
            demand.outer_edges.bottom,
            demand.total_edges.bottom,
        ),
        Side::Left => EdgeDemand::new(
            demand.inner_edges.left,
            demand.outer_edges.left,
            demand.total_edges.left,
        ),
    }
}

impl<Id> GridItem<Id> {
    /// The overflow this item carries into a solve, as inner/total edge
    /// targets (totals lifted to `max(total, inner + outer)`, matching the
    /// merge laws).
    pub fn requested_targets(&self) -> EdgeTargets {
        let lifted =
            |inner: f32, outer: f32, total: f32| EdgeDemand::new(inner, outer, total).total;
        EdgeTargets {
            inner: self.inner_edges,
            total: Edges::new(
                lifted(
                    self.inner_edges.top,
                    self.outer_edges.top,
                    self.total_edges.top,
                ),
                lifted(
                    self.inner_edges.right,
                    self.outer_edges.right,
                    self.total_edges.right,
                ),
                lifted(
                    self.inner_edges.bottom,
                    self.outer_edges.bottom,
                    self.total_edges.bottom,
                ),
                lifted(
                    self.inner_edges.left,
                    self.outer_edges.left,
                    self.total_edges.left,
                ),
            ),
        }
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
            column_left: vec![EdgeDemand::default(); shape.columns],
            column_right: vec![EdgeDemand::default(); shape.columns],
            row_top: vec![EdgeDemand::default(); shape.rows],
            row_bottom: vec![EdgeDemand::default(); shape.rows],
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

    /// Solve track starts, span constraints, and content size for one grid.
    pub fn solve<Id>(&self, demands: &[GridItem<Id>]) -> GridSolution {
        self.solve_with_growth(demands, None, None)
    }

    /// Like [`GridRequirements::solve`], with optional per-track growth
    /// kinds: span deficits distribute to `Auto` tracks first, then `Flex`,
    /// never `Fixed`. `None` treats every track as `Auto`.
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

/// Collect the `total` component of each edge demand.
pub(crate) fn edge_demand_totals(edges: &[EdgeDemand]) -> Vec<f32> {
    edges.iter().map(|edge| edge.total).collect()
}

/// Build a vector of zero edge demands.
#[cfg(test)]
pub(crate) fn zero_edge_demands(len: usize) -> Vec<EdgeDemand> {
    vec![EdgeDemand::default(); len]
}

/// Build edge demands carrying only totals.
#[cfg(test)]
pub(crate) fn total_edge_demands(values: impl IntoIterator<Item = f32>) -> Vec<EdgeDemand> {
    values.into_iter().map(EdgeDemand::total).collect()
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

impl GridSolution {
    pub fn content_origin_for_slot(&self, slot: GridSlot) -> [f32; 2] {
        [self.column_starts[slot.column], self.row_starts[slot.row]]
    }

    pub fn edge_targets_for_slot(&self, slot: GridSlot) -> EdgeTargets {
        let last_row = slot.row_end() - 1;
        let last_column = slot.column_end() - 1;
        let top = self.row_top[slot.row];
        let right = self.column_right[last_column];
        let bottom = self.row_bottom[last_row];
        let left = self.column_left[slot.column];

        EdgeTargets {
            inner: Edges {
                top: top.inner,
                right: right.inner,
                bottom: bottom.inner,
                left: left.inner,
            },
            total: Edges {
                top: top.total,
                right: right.total,
                bottom: bottom.total,
                left: left.total,
            },
        }
    }

    pub fn content_size_for_slot(&self, slot: GridSlot) -> Size {
        let column_right_totals = edge_demand_totals(&self.column_right);
        let column_left_totals = edge_demand_totals(&self.column_left);
        let row_bottom_totals = edge_demand_totals(&self.row_bottom);
        let row_top_totals = edge_demand_totals(&self.row_top);
        Size::new(
            span_axis_extent(
                &self.column_widths,
                &column_right_totals,
                &column_left_totals,
                self.column_spacing.min_gap,
                slot.column,
                slot.column_span,
            ),
            span_axis_extent(
                &self.row_heights,
                &row_bottom_totals,
                &row_top_totals,
                self.row_spacing.min_gap,
                slot.row,
                slot.row_span,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_tracks_solve_matches_uniform_grid_solve() {
        let tracks = UniformTracks {
            count: 3,
            spacing: TrackSpacing {
                outer_start: 5.0,
                outer_end: 7.0,
                min_gap: 10.0,
            },
        };
        let solved = tracks.solve(40.0);

        let items = (0..3)
            .map(|index| GridItem {
                id: index,
                slot: GridSlot {
                    row: 0,
                    column: index,
                    row_span: 1,
                    column_span: 1,
                },
                content_size: Size::new(40.0, 20.0),
                inner_edges: Edges::default(),
                outer_edges: Edges::default(),
                total_edges: Edges::default(),
            })
            .collect::<Vec<_>>();
        let mut requirements = GridRequirements::from_items(
            GridShape {
                rows: 1,
                columns: 3,
            },
            Size::default(),
            &items,
        )
        .expect("uniform items fit the shape");
        requirements.column_spacing = tracks.spacing;
        let grid = requirements.solve(&items);

        assert_eq!(solved.starts, grid.column_starts);
        assert_eq!(solved.extent, grid.content_size.width);
        assert_eq!(solved.track_size, 40.0);
    }

    #[test]
    fn uniform_tracks_merge_coordinates_policy() {
        let first = UniformTracks {
            count: 2,
            spacing: TrackSpacing {
                outer_start: 11.0,
                outer_end: 12.0,
                min_gap: 24.0,
            },
        };
        let second = UniformTracks {
            count: 4,
            spacing: TrackSpacing {
                outer_start: 91.0,
                outer_end: 2.0,
                min_gap: 18.0,
            },
        };

        let merged = first.merge_max(second);
        assert_eq!(merged.count, 4);
        assert_eq!(
            merged.spacing,
            TrackSpacing {
                outer_start: 91.0,
                outer_end: 12.0,
                min_gap: 24.0,
            }
        );
        assert_eq!(
            UniformTracks::default().merge_max(first),
            first,
            "merging from the default seed is the identity"
        );
    }

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
        inner_edges: Edges<f32>,
        outer_edges: Edges<f32>,
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
            inner_edges,
            outer_edges,
            total_edges,
        }
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
        let solution = requirements.solve(&demands);

        assert_eq!(solution.column_widths, vec![100.0, 120.0]);
        assert_eq!(solution.row_heights, vec![50.0, 60.0]);
        assert_eq!(solution.column_starts, vec![0.0, 112.0]);
        assert_eq!(solution.row_starts, vec![0.0, 61.0]);
        assert_eq!(solution.content_size, Size::new(232.0, 121.0));
        assert_eq!(
            solution.content_origin_for_slot(demands[1].slot),
            [112.0, 0.0]
        );
        assert_eq!(
            solution.content_origin_for_slot(demands[2].slot),
            [0.0, 61.0]
        );
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

        let solution = requirements.solve(&demands);

        assert_eq!(solution.column_starts, vec![3.0]);
        assert_eq!(solution.row_starts, vec![5.0]);
        assert_eq!(solution.content_size, Size::new(110.0, 66.0));
        assert_eq!(
            solution.content_origin_for_slot(demands[0].slot),
            [3.0, 5.0]
        );
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
        let solution = requirements.solve(&demands);

        assert_eq!(solution.column_starts, vec![0.0, 100.0, 200.0]);
        assert_eq!(solution.row_starts, vec![0.0, 100.0]);
        assert_eq!(
            solution.content_origin_for_slot(demands[1].slot),
            [200.0, 100.0]
        );
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
        let solution = requirements.solve(&demands);

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

        let solution = requirements.solve(&demands);

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

        let solution = requirements.solve(&demands);

        // The floored gap (max(8.0, 1.0 + 2.0) = 8.0) counts toward the span
        // target of 250.0, so each track absorbs (250 - 200 - 8) / 2 = 21.0.
        assert_eq!(solution.column_widths, vec![121.0, 121.0]);
        assert_eq!(solution.column_starts, vec![0.0, 129.0]);
        assert_eq!(solution.content_size.width, 250.0);

        // Slot content size agrees with the positional distance from span
        // start to span end content edge.
        let span_slot = demands[0].slot;
        let positional_extent =
            solution.column_starts[1] + solution.column_widths[1] - solution.column_starts[0];
        assert_eq!(
            solution.content_size_for_slot(span_slot).width,
            positional_extent
        );
        Ok(())
    }

    #[test]
    fn grid_solution_carries_structured_edge_targets() -> Result<(), GridError> {
        let shape = GridShape {
            rows: 1,
            columns: 2,
        };
        let demands = vec![
            grid_item_with_edges(
                0,
                0,
                0,
                1,
                1,
                Size::new(100.0, 60.0),
                Edges::new(0.0, 6.0, 0.0, 0.0),
                Edges::new(0.0, 20.0, 0.0, 0.0),
                Edges::new(0.0, 12.0, 0.0, 0.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size::new(100.0, 60.0),
                Edges::new(0.0, 0.0, 0.0, 3.0),
            ),
        ];

        let requirements = GridRequirements::from_items(shape, Size::new(100.0, 60.0), &demands)?;
        assert_eq!(
            requirements.column_right[0],
            EdgeDemand::new(6.0, 20.0, 12.0)
        );

        let solution = requirements.solve(&demands);
        let targets = solution.edge_targets_for_slot(demands[0].slot);
        assert_eq!(targets.inner.right, 6.0);
        assert_eq!(targets.total.right, 26.0);
        assert_eq!(targets.inner.left, 0.0);
        assert_eq!(targets.total.left, 0.0);
        Ok(())
    }
}
