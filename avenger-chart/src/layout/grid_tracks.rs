//! Neutral grid track requirements and solving.
//!
//! This module works in terms of content sizes, rectangular slots, spans, and
//! edge total_edges. Concat/facet code adapts chart-specific measurements into these
//! requirements.

use crate::{
    error::AvengerChartError,
    layout::{EdgeSlabs, EdgeTargets, OverflowSide, Size2D},
};

/// Two-dimensional grid track count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GridShape {
    pub(crate) rows: usize,
    pub(crate) columns: usize,
}

/// Rectangular slot occupied by one child in a grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GridSlot {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) row_span: usize,
    pub(crate) column_span: usize,
}

impl GridSlot {
    pub(crate) fn row_end(self) -> usize {
        self.row + self.row_span
    }

    pub(crate) fn column_end(self) -> usize {
        self.column + self.column_span
    }
}

/// Content and edge demand exported by one grid child.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridItem {
    pub(crate) child_index: usize,
    pub(crate) slot: GridSlot,
    pub(crate) content_size: Size2D,
    pub(crate) inner_edges: EdgeSlabs,
    pub(crate) outer_edges: EdgeSlabs,
    pub(crate) total_edges: EdgeSlabs,
}

/// Edge demand split into semantic layers while keeping one numeric total.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EdgeDemand {
    pub(crate) inner: f32,
    pub(crate) outer: f32,
    pub(crate) total: f32,
}

impl EdgeDemand {
    pub(crate) fn new(inner: f32, outer: f32, total: f32) -> Self {
        let inner = inner.max(0.0);
        let outer = outer.max(0.0);
        let total = total.max(inner + outer).max(0.0);
        Self {
            inner,
            outer,
            total,
        }
    }

    pub(crate) fn total(total: f32) -> Self {
        Self::new(0.0, 0.0, total)
    }

    pub(crate) fn max_components(self, other: Self) -> Self {
        Self::new(
            self.inner.max(other.inner),
            self.outer.max(other.outer),
            self.total.max(other.total),
        )
    }
}

/// Track sizes and edge requirements needed to align one measured grid.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridRequirements {
    pub(crate) shape: GridShape,
    /// Chart-adapter facet guide slot gap needed when generic grid requirements
    /// are projected back into `CoordinatedLayout`.
    ///
    /// Concat containers do not use this value, so their local requirements
    /// keep it at zero. Keeping it here avoids losing chart guide-spacing state
    /// when facet bands participate in the generic layout model.
    pub(crate) guide_slot_gap_px: f32,
    pub(crate) column_outer_start: f32,
    pub(crate) column_outer_end: f32,
    pub(crate) row_outer_start: f32,
    pub(crate) row_outer_end: f32,
    pub(crate) column_widths: Vec<f32>,
    pub(crate) row_heights: Vec<f32>,
    pub(crate) column_left: Vec<EdgeDemand>,
    pub(crate) column_right: Vec<EdgeDemand>,
    pub(crate) row_top: Vec<EdgeDemand>,
    pub(crate) row_bottom: Vec<EdgeDemand>,
}

/// Solved track starts and effective content size for one grid.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridSolution {
    pub(crate) guide_slot_gap_px: f32,
    pub(crate) column_outer_start: f32,
    pub(crate) column_outer_end: f32,
    pub(crate) row_outer_start: f32,
    pub(crate) row_outer_end: f32,
    pub(crate) column_widths: Vec<f32>,
    pub(crate) row_heights: Vec<f32>,
    pub(crate) column_left: Vec<EdgeDemand>,
    pub(crate) column_right: Vec<EdgeDemand>,
    pub(crate) row_top: Vec<EdgeDemand>,
    pub(crate) row_bottom: Vec<EdgeDemand>,
    pub(crate) column_starts: Vec<f32>,
    pub(crate) row_starts: Vec<f32>,
    pub(crate) content_size: Size2D,
}

fn grid_edge_demand(demand: &GridItem, side: OverflowSide) -> EdgeDemand {
    match side {
        OverflowSide::Top => EdgeDemand::new(
            demand.inner_edges.top,
            demand.outer_edges.top,
            demand.total_edges.top,
        ),
        OverflowSide::Right => EdgeDemand::new(
            demand.inner_edges.right,
            demand.outer_edges.right,
            demand.total_edges.right,
        ),
        OverflowSide::Bottom => EdgeDemand::new(
            demand.inner_edges.bottom,
            demand.outer_edges.bottom,
            demand.total_edges.bottom,
        ),
        OverflowSide::Left => EdgeDemand::new(
            demand.inner_edges.left,
            demand.outer_edges.left,
            demand.total_edges.left,
        ),
    }
}

pub(crate) fn grid_requirements(
    shape: GridShape,
    base_cell_size: Size2D,
    demands: &[GridItem],
) -> Result<GridRequirements, AvengerChartError> {
    let mut requirements = GridRequirements {
        shape,
        guide_slot_gap_px: 0.0,
        column_outer_start: 0.0,
        column_outer_end: 0.0,
        row_outer_start: 0.0,
        row_outer_end: 0.0,
        column_widths: vec![base_cell_size.width; shape.columns],
        row_heights: vec![base_cell_size.height; shape.rows],
        column_left: vec![EdgeDemand::default(); shape.columns],
        column_right: vec![EdgeDemand::default(); shape.columns],
        row_top: vec![EdgeDemand::default(); shape.rows],
        row_bottom: vec![EdgeDemand::default(); shape.rows],
    };

    for demand in demands {
        let slot = demand.slot;
        if slot.row_span == 0
            || slot.column_span == 0
            || slot.row_end() > shape.rows
            || slot.column_end() > shape.columns
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Grid child {} slot {:?} exceeds grid shape {}x{}",
                demand.child_index, slot, shape.rows, shape.columns
            )));
        }

        let last_row = slot.row_end() - 1;
        let last_column = slot.column_end() - 1;
        requirements.column_left[slot.column] = requirements.column_left[slot.column]
            .max_components(grid_edge_demand(demand, OverflowSide::Left));
        requirements.column_right[last_column] = requirements.column_right[last_column]
            .max_components(grid_edge_demand(demand, OverflowSide::Right));
        requirements.row_top[slot.row] = requirements.row_top[slot.row]
            .max_components(grid_edge_demand(demand, OverflowSide::Top));
        requirements.row_bottom[last_row] = requirements.row_bottom[last_row]
            .max_components(grid_edge_demand(demand, OverflowSide::Bottom));

        if slot.column_span == 1 {
            requirements.column_widths[slot.column] =
                requirements.column_widths[slot.column].max(demand.content_size.width);
        }
        if slot.row_span == 1 {
            requirements.row_heights[slot.row] =
                requirements.row_heights[slot.row].max(demand.content_size.height);
        }
    }

    Ok(requirements)
}

pub(crate) fn solve_grid_requirements(
    requirements: &GridRequirements,
    demands: &[GridItem],
) -> GridSolution {
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
        requirements.column_outer_start,
        requirements.column_outer_end,
    );
    let (row_starts, content_height) = track_starts_and_content_size(
        &row_heights,
        &row_bottom_totals,
        &row_top_totals,
        requirements.row_outer_start,
        requirements.row_outer_end,
    );

    GridSolution {
        guide_slot_gap_px: requirements.guide_slot_gap_px,
        column_outer_start: requirements.column_outer_start,
        column_outer_end: requirements.column_outer_end,
        row_outer_start: requirements.row_outer_start,
        row_outer_end: requirements.row_outer_end,
        column_widths,
        row_heights,
        column_left: requirements.column_left.clone(),
        column_right: requirements.column_right.clone(),
        row_top: requirements.row_top.clone(),
        row_bottom: requirements.row_bottom.clone(),
        column_starts,
        row_starts,
        content_size: Size2D::new(content_width, content_height),
    }
}

pub(crate) fn edge_demand_totals(edges: &[EdgeDemand]) -> Vec<f32> {
    edges.iter().map(|edge| edge.total).collect()
}

pub(crate) fn zero_edge_demands(len: usize) -> Vec<EdgeDemand> {
    vec![EdgeDemand::default(); len]
}

#[cfg(test)]
pub(crate) fn total_edge_demands(values: impl IntoIterator<Item = f32>) -> Vec<EdgeDemand> {
    values.into_iter().map(EdgeDemand::total).collect()
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
    mut constraints: Vec<AxisSpanConstraint>,
) {
    constraints.sort_by_key(|constraint| constraint.span);
    for constraint in constraints {
        let current = span_axis_extent(
            sizes,
            trailing_edges,
            leading_edges,
            constraint.start,
            constraint.span,
        );
        let deficit = constraint.target - current;
        if deficit <= 0.0 || constraint.span == 0 {
            continue;
        }
        let extra_per_track = deficit / constraint.span as f32;
        for size in &mut sizes[constraint.start..constraint.start + constraint.span] {
            *size += extra_per_track;
        }
    }
}

pub(crate) fn span_axis_extent(
    sizes: &[f32],
    trailing_edges: &[f32],
    leading_edges: &[f32],
    start: usize,
    span: usize,
) -> f32 {
    let end = start + span;
    let track_sum = sizes[start..end].iter().sum::<f32>();
    let gap_sum = (start..end.saturating_sub(1))
        .map(|index| trailing_edges[index] + leading_edges[index + 1])
        .sum::<f32>();
    track_sum + gap_sum
}

fn track_starts_and_content_size(
    sizes: &[f32],
    trailing_edges: &[f32],
    leading_edges: &[f32],
    outer_start: f32,
    outer_end: f32,
) -> (Vec<f32>, f32) {
    let mut starts = vec![0.0f32; sizes.len()];
    let mut cursor = outer_start.max(0.0);
    for index in 0..sizes.len() {
        if index > 0 {
            cursor += trailing_edges[index - 1] + leading_edges[index];
        }
        starts[index] = cursor;
        cursor += sizes[index];
    }
    (starts, cursor + outer_end.max(0.0))
}

impl GridSolution {
    pub(crate) fn content_origin_for_slot(&self, slot: GridSlot) -> [f32; 2] {
        [self.column_starts[slot.column], self.row_starts[slot.row]]
    }

    pub(crate) fn edge_targets_for_slot(&self, slot: GridSlot) -> EdgeTargets {
        let last_row = slot.row_end() - 1;
        let last_column = slot.column_end() - 1;
        let top = self.row_top[slot.row];
        let right = self.column_right[last_column];
        let bottom = self.row_bottom[last_row];
        let left = self.column_left[slot.column];

        EdgeTargets {
            inner: EdgeSlabs {
                top: top.inner,
                right: right.inner,
                bottom: bottom.inner,
                left: left.inner,
            },
            total: EdgeSlabs {
                top: top.total,
                right: right.total,
                bottom: bottom.total,
                left: left.total,
            },
        }
    }

    pub(crate) fn content_size_for_slot(&self, slot: GridSlot) -> Size2D {
        let column_right_totals = edge_demand_totals(&self.column_right);
        let column_left_totals = edge_demand_totals(&self.column_left);
        let row_bottom_totals = edge_demand_totals(&self.row_bottom);
        let row_top_totals = edge_demand_totals(&self.row_top);
        Size2D::new(
            span_axis_extent(
                &self.column_widths,
                &column_right_totals,
                &column_left_totals,
                slot.column,
                slot.column_span,
            ),
            span_axis_extent(
                &self.row_heights,
                &row_bottom_totals,
                &row_top_totals,
                slot.row,
                slot.row_span,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_item(
        child_index: usize,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        content_size: Size2D,
        total_edges: EdgeSlabs,
    ) -> GridItem {
        grid_item_with_edges(
            child_index,
            row,
            column,
            row_span,
            column_span,
            content_size,
            EdgeSlabs::default(),
            EdgeSlabs::default(),
            total_edges,
        )
    }

    fn grid_item_with_edges(
        child_index: usize,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        content_size: Size2D,
        inner_edges: EdgeSlabs,
        outer_edges: EdgeSlabs,
        total_edges: EdgeSlabs,
    ) -> GridItem {
        GridItem {
            child_index,
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
    fn edge_demand_merges_structured_components() {
        let left = EdgeDemand::new(4.0, 11.0, 2.0);
        assert_eq!(
            left,
            EdgeDemand {
                inner: 4.0,
                outer: 11.0,
                total: 15.0
            }
        );

        let right = EdgeDemand::new(9.0, 3.0, 22.0);
        assert_eq!(
            left.max_components(right),
            EdgeDemand {
                inner: 9.0,
                outer: 11.0,
                total: 22.0
            }
        );
    }

    #[test]
    fn grid_solver_matches_non_spanning_grid_placement() -> Result<(), AvengerChartError> {
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
                Size2D::new(100.0, 50.0),
                EdgeSlabs::new(1.0, 5.0, 2.0, 3.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size2D::new(120.0, 50.0),
                EdgeSlabs::new(1.0, 4.0, 2.0, 7.0),
            ),
            grid_item(
                2,
                1,
                0,
                1,
                1,
                Size2D::new(100.0, 60.0),
                EdgeSlabs::new(9.0, 5.0, 2.0, 3.0),
            ),
        ];

        let requirements = grid_requirements(shape, Size2D::new(100.0, 50.0), &demands)?;
        let solution = solve_grid_requirements(&requirements, &demands);

        assert_eq!(solution.column_widths, vec![100.0, 120.0]);
        assert_eq!(solution.row_heights, vec![50.0, 60.0]);
        assert_eq!(solution.column_starts, vec![0.0, 112.0]);
        assert_eq!(solution.row_starts, vec![0.0, 61.0]);
        assert_eq!(solution.content_size, Size2D::new(232.0, 121.0));
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
    fn grid_solver_preserves_outer_offsets() -> Result<(), AvengerChartError> {
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
            Size2D::new(100.0, 50.0),
            EdgeSlabs::default(),
        )];

        let mut requirements = grid_requirements(shape, Size2D::new(100.0, 50.0), &demands)?;
        requirements.guide_slot_gap_px = 13.0;
        requirements.column_outer_start = 3.0;
        requirements.column_outer_end = 7.0;
        requirements.row_outer_start = 5.0;
        requirements.row_outer_end = 11.0;

        let solution = solve_grid_requirements(&requirements, &demands);

        assert_eq!(solution.guide_slot_gap_px, 13.0);
        assert_eq!(solution.column_starts, vec![3.0]);
        assert_eq!(solution.row_starts, vec![5.0]);
        assert_eq!(solution.content_size, Size2D::new(110.0, 66.0));
        assert_eq!(
            solution.content_origin_for_slot(demands[0].slot),
            [3.0, 5.0]
        );
        Ok(())
    }

    #[test]
    fn grid_solver_preserves_hole_track_positions() -> Result<(), AvengerChartError> {
        let shape = GridShape {
            rows: 2,
            columns: 3,
        };
        let demands = vec![
            grid_item(
                0,
                0,
                0,
                1,
                1,
                Size2D::new(100.0, 100.0),
                EdgeSlabs::default(),
            ),
            grid_item(
                1,
                1,
                2,
                1,
                1,
                Size2D::new(100.0, 100.0),
                EdgeSlabs::default(),
            ),
        ];

        let requirements = grid_requirements(shape, Size2D::new(100.0, 100.0), &demands)?;
        let solution = solve_grid_requirements(&requirements, &demands);

        assert_eq!(solution.column_starts, vec![0.0, 100.0, 200.0]);
        assert_eq!(solution.row_starts, vec![0.0, 100.0]);
        assert_eq!(
            solution.content_origin_for_slot(demands[1].slot),
            [200.0, 100.0]
        );
        assert_eq!(solution.content_size, Size2D::new(300.0, 200.0));
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
            Size2D::new(100.0, 100.0),
            EdgeSlabs::default(),
        )];

        let err = grid_requirements(shape, Size2D::new(100.0, 100.0), &demands)
            .expect_err("slot rect should exceed shape");
        assert!(err.to_string().contains("exceeds grid shape"));
    }

    #[test]
    fn grid_solver_satisfies_span_interval_constraints() -> Result<(), AvengerChartError> {
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
                Size2D::new(190.0, 50.0),
                EdgeSlabs::new(1.0, 6.0, 2.0, 4.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size2D::new(50.0, 50.0),
                EdgeSlabs::new(0.0, 3.0, 0.0, 2.0),
            ),
        ];

        let requirements = grid_requirements(shape, Size2D::new(50.0, 50.0), &demands)?;
        let solution = solve_grid_requirements(&requirements, &demands);

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
    fn grid_solution_carries_structured_edge_targets() -> Result<(), AvengerChartError> {
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
                Size2D::new(100.0, 60.0),
                EdgeSlabs::new(0.0, 6.0, 0.0, 0.0),
                EdgeSlabs::new(0.0, 20.0, 0.0, 0.0),
                EdgeSlabs::new(0.0, 12.0, 0.0, 0.0),
            ),
            grid_item(
                1,
                0,
                1,
                1,
                1,
                Size2D::new(100.0, 60.0),
                EdgeSlabs::new(0.0, 0.0, 0.0, 3.0),
            ),
        ];

        let requirements = grid_requirements(shape, Size2D::new(100.0, 60.0), &demands)?;
        assert_eq!(
            requirements.column_right[0],
            EdgeDemand::new(6.0, 20.0, 12.0)
        );

        let solution = solve_grid_requirements(&requirements, &demands);
        let targets = solution.edge_targets_for_slot(demands[0].slot);
        assert_eq!(targets.inner.right, 6.0);
        assert_eq!(targets.total.right, 26.0);
        assert_eq!(targets.inner.left, 0.0);
        assert_eq!(targets.total.left, 0.0);
        Ok(())
    }
}
