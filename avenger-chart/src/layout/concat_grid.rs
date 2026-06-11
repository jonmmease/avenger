//! Chart-owned concat/wrap grid solving through the unified
//! `avenger_layout::Layout` API.
//!
//! Concat coordination works on per-track requirement data
//! ([`ChartGridData`]): exported from measured children, max-merged across
//! cousins (`layout::alignment`), and applied back by re-solving the merged
//! values against the local children. The solving itself goes through
//! `Layout`:
//!
//! - **Export** ([`solve_concat_grid`] with `export = true`) zeroes the
//!   spanned-axis content of multi-span cells so the extracted track sizes
//!   equal the pre-span-constraint requirement fold (span constraints are
//!   re-applied at apply time, per member — the historical contract).
//! - **Apply** ([`solve_concat_grid_against`]) uses a **phantom cousin**:
//!   the local grid and an equal-shape phantom reproducing the merged
//!   requirements share one key, so the solve's coordination patches the
//!   local grid to exactly the merged floors (merged ≥ local by the
//!   align-merge construction) and re-applies local span constraints.
//!
//! Per-slot reads mirror the historical grid solution: origins from track
//! starts (the solver's exact floats), edge targets from per-track demand
//! vectors, span content sizes positionally from the tracks.

use avenger_layout::{
    EdgeDemand, Edges, GridShape, GridSlot, Layout, LayoutSolution, RegionDetail, Side, Size,
    SolveOptions, Spacing,
};

use super::placement::EdgeTargets;

/// One measured concat child as a grid cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridCell {
    pub slot: GridSlot,
    pub content_size: Size,
    pub inner_edges: Edges<f32>,
    pub outer_edges: Edges<f32>,
    pub total_edges: Edges<f32>,
}

/// Per-track requirement data for one measured grid: the chart's
/// coordination payload (exported, merged across cousins, applied back).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChartGridData {
    pub shape: GridShape,
    pub column_spacing: Spacing,
    pub row_spacing: Spacing,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub column_left: Vec<EdgeDemand>,
    pub column_right: Vec<EdgeDemand>,
    pub row_top: Vec<EdgeDemand>,
    pub row_bottom: Vec<EdgeDemand>,
}

/// A solved concat grid: per-track geometry plus the requirement vectors,
/// in grid-content coordinates (the historical solution view).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SolvedConcatGrid {
    pub data: ChartGridData,
    pub column_starts: Vec<f32>,
    pub row_starts: Vec<f32>,
    pub content_size: Size,
}

impl SolvedConcatGrid {
    pub(crate) fn content_origin_for_slot(&self, slot: GridSlot) -> [f32; 2] {
        [self.column_starts[slot.column], self.row_starts[slot.row]]
    }

    pub(crate) fn edge_targets_for_slot(&self, slot: GridSlot) -> EdgeTargets {
        let last_row = slot.row_end() - 1;
        let last_column = slot.column_end() - 1;
        let top = self.data.row_top[slot.row];
        let right = self.data.column_right[last_column];
        let bottom = self.data.row_bottom[last_row];
        let left = self.data.column_left[slot.column];

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

    /// Span extent of a slot, positional (track starts are the solver's
    /// exact floats; both sides of placement change detection use this same
    /// computation).
    pub(crate) fn content_size_for_slot(&self, slot: GridSlot) -> Size {
        let last_column = slot.column_end() - 1;
        let last_row = slot.row_end() - 1;
        Size::new(
            self.column_starts[last_column] + self.data.column_widths[last_column]
                - self.column_starts[slot.column],
            self.row_starts[last_row] + self.data.row_heights[last_row] - self.row_starts[slot.row],
        )
    }
}

fn cell_leaf(cell: &GridCell, export: bool) -> Layout<usize> {
    // Export mode reproduces the pre-solve requirement fold: span items do
    // not contribute content to their tracks, so their spanned-axis content
    // is zeroed (span constraints re-apply at apply time).
    let mut content = cell.content_size;
    if export {
        if cell.slot.column_span > 1 {
            content.width = 0.0;
        }
        if cell.slot.row_span > 1 {
            content.height = 0.0;
        }
    }
    let mut leaf = Layout::leaf(content);
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        leaf = leaf.demand(
            side,
            EdgeDemand::new(
                *cell.inner_edges.side(side),
                *cell.outer_edges.side(side),
                *cell.total_edges.side(side),
            ),
        );
    }
    leaf
}

fn cells_grid(
    shape: GridShape,
    base_cell_size: Size,
    cells: &[GridCell],
    column_spacing: Spacing,
    row_spacing: Spacing,
    export: bool,
) -> Layout<usize> {
    let mut grid = Layout::grid(shape.rows, shape.columns)
        .base_cell_size(base_cell_size)
        .column_spacing(column_spacing)
        .row_spacing(row_spacing);
    for cell in cells {
        grid = grid.cell_span(
            cell.slot.row,
            cell.slot.column,
            cell.slot.row_span,
            cell.slot.column_span,
            cell_leaf(cell, export),
        );
    }
    grid
}

/// Extract the solved per-track view from a grid member region.
fn extract(
    solution: &LayoutSolution<usize>,
    member_path: &[usize],
    shape: GridShape,
    cells: &[GridCell],
    column_spacing: Spacing,
    row_spacing: Spacing,
) -> Result<SolvedConcatGrid, String> {
    let member = solution
        .at_path(member_path)
        .ok_or_else(|| "missing grid member region".to_string())?;
    let RegionDetail::Grid { tracks } = &member.detail else {
        return Err("grid member is not a grid".to_string());
    };

    let mut column_left = vec![EdgeDemand::default(); shape.columns];
    let mut column_right = vec![EdgeDemand::default(); shape.columns];
    let mut row_top = vec![EdgeDemand::default(); shape.rows];
    let mut row_bottom = vec![EdgeDemand::default(); shape.rows];
    for (index, cell) in cells.iter().enumerate() {
        let mut child_path = member_path.to_vec();
        child_path.push(index);
        let granted = &solution
            .at_path(&child_path)
            .ok_or_else(|| "missing grid child region".to_string())?
            .granted;
        // A child's granted edges ARE the folded per-track demands of the
        // tracks its slot touches (first column/row for leading, last for
        // trailing).
        column_left[cell.slot.column] = granted.left;
        column_right[cell.slot.column_end() - 1] = granted.right;
        row_top[cell.slot.row] = granted.top;
        row_bottom[cell.slot.row_end() - 1] = granted.bottom;
    }

    Ok(SolvedConcatGrid {
        data: ChartGridData {
            shape,
            column_spacing,
            row_spacing,
            column_widths: tracks.column_sizes.clone(),
            row_heights: tracks.row_sizes.clone(),
            column_left,
            column_right,
            row_top,
            row_bottom,
        },
        column_starts: tracks.column_starts.clone(),
        row_starts: tracks.row_starts.clone(),
        content_size: Size::new(member.content.width, member.content.height),
    })
}

/// Solve one concat grid's own cells. `export = true` yields the pre-solve
/// requirement fold (for the coordination payload); `export = false` yields
/// the placed solution with span constraints applied.
pub(crate) fn solve_concat_grid(
    shape: GridShape,
    base_cell_size: Size,
    cells: &[GridCell],
    column_spacing: Spacing,
    row_spacing: Spacing,
    export: bool,
) -> Result<SolvedConcatGrid, String> {
    let grid = cells_grid(
        shape,
        base_cell_size,
        cells,
        column_spacing,
        row_spacing,
        export,
    );
    let solved = grid
        .solve(&SolveOptions::default())
        .map_err(|err| err.to_string())?;
    extract(&solved, &[], shape, cells, column_spacing, row_spacing)
}

/// Solve the local cells against merged requirements (the apply path): the
/// local grid and a phantom cousin reproducing the merged values share one
/// key, so coordination patches the local grid to the merged floors and the
/// local span constraints re-apply.
pub(crate) fn solve_concat_grid_against(
    merged: &ChartGridData,
    base_cell_size: Size,
    cells: &[GridCell],
) -> Result<SolvedConcatGrid, String> {
    let shape = merged.shape;
    let local = cells_grid(
        shape,
        base_cell_size,
        cells,
        merged.column_spacing,
        merged.row_spacing,
        false,
    )
    .share(0usize);

    let mut phantom = Layout::grid(shape.rows, shape.columns)
        .column_spacing(merged.column_spacing)
        .row_spacing(merged.row_spacing)
        .share(0usize);
    for row in 0..shape.rows {
        for column in 0..shape.columns {
            let mut leaf = Layout::leaf(Size::new(
                merged.column_widths[column],
                merged.row_heights[row],
            ));
            for (side, demand) in [
                (Side::Top, merged.row_top[row]),
                (Side::Right, merged.column_right[column]),
                (Side::Bottom, merged.row_bottom[row]),
                (Side::Left, merged.column_left[column]),
            ] {
                leaf = leaf.demand(side, demand);
            }
            phantom = phantom.cell(row, column, leaf);
        }
    }

    let root: Layout<usize> = Layout::row(vec![local, phantom]);
    let solved = root
        .solve(&SolveOptions::default())
        .map_err(|err| err.to_string())?;
    extract(
        &solved,
        &[0],
        shape,
        cells,
        merged.column_spacing,
        merged.row_spacing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        width: f32,
        height: f32,
        edges: Edges<f32>,
    ) -> GridCell {
        GridCell {
            slot: GridSlot {
                row,
                column,
                row_span,
                column_span,
            },
            content_size: Size::new(width, height),
            inner_edges: Edges::default(),
            outer_edges: Edges::default(),
            total_edges: edges,
        }
    }

    fn legacy_requirements(
        shape: GridShape,
        base: Size,
        cells: &[GridCell],
        min_gap: f32,
    ) -> avenger_layout::GridRequirements {
        let items: Vec<avenger_layout::GridItem> = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| avenger_layout::GridItem {
                id: index,
                slot: cell.slot,
                content_size: cell.content_size,
                inner_edges: cell.inner_edges,
                outer_edges: cell.outer_edges,
                total_edges: cell.total_edges,
            })
            .collect();
        let mut requirements =
            avenger_layout::GridRequirements::from_items(shape, base, &items).expect("valid");
        requirements.column_spacing.min_gap = min_gap;
        requirements.row_spacing.min_gap = min_gap;
        requirements
    }

    /// Export reproduces the legacy pre-solve requirement fold exactly,
    /// including spans (no content contribution) and holes (defaults).
    #[test]
    fn export_matches_legacy_requirement_fold() {
        let shape = GridShape {
            rows: 2,
            columns: 3,
        };
        let base = Size::new(40.5, 30.25);
        let cells = vec![
            cell(0, 0, 1, 1, 101.5, 51.25, Edges::new(1.5, 5.0, 2.0, 3.25)),
            cell(0, 2, 1, 1, 120.0, 50.0, Edges::new(1.0, 4.0, 2.0, 7.5)),
            cell(1, 0, 1, 2, 199.75, 60.5, Edges::new(9.0, 5.5, 2.0, 3.0)),
        ];
        let spacing = Spacing {
            min_gap: 11.5,
            ..Default::default()
        };
        let exported =
            solve_concat_grid(shape, base, &cells, spacing, spacing, true).expect("solve");
        let legacy = legacy_requirements(shape, base, &cells, 11.5);

        assert_eq!(exported.data.column_widths, legacy.column_widths);
        assert_eq!(exported.data.row_heights, legacy.row_heights);
        assert_eq!(exported.data.column_left, legacy.column_left);
        assert_eq!(exported.data.column_right, legacy.column_right);
        assert_eq!(exported.data.row_top, legacy.row_top);
        assert_eq!(exported.data.row_bottom, legacy.row_bottom);
    }

    /// The phantom-cousin apply reproduces the legacy
    /// `merged_requirements.solve(&local_items)` per-slot reads exactly.
    #[test]
    fn apply_matches_legacy_solve_against_merged() {
        let shape = GridShape {
            rows: 2,
            columns: 2,
        };
        let base = Size::new(50.0, 40.0);
        let cells = vec![
            cell(0, 0, 1, 1, 101.5, 51.25, Edges::new(1.5, 5.0, 2.0, 3.25)),
            cell(0, 1, 1, 1, 90.0, 50.0, Edges::new(1.0, 4.0, 2.0, 7.5)),
            cell(1, 0, 1, 2, 260.25, 60.5, Edges::new(9.0, 5.5, 2.0, 3.0)),
        ];
        let spacing = Spacing {
            min_gap: 12.25,
            ..Default::default()
        };
        // A merged payload strictly above the local export (a cousin was
        // bigger).
        let local = solve_concat_grid(shape, base, &cells, spacing, spacing, true).expect("solve");
        let mut merged = local.data.clone();
        merged.column_widths[0] += 13.5;
        merged.row_heights[1] += 7.25;
        merged.column_left[0] =
            EdgeDemand::new(6.0, 2.0, 8.0).max_components(merged.column_left[0]);
        merged.column_spacing.min_gap = 14.0;

        let applied = solve_concat_grid_against(&merged, base, &cells).expect("apply");

        // Legacy: install merged values into requirements, solve local items.
        let mut legacy = legacy_requirements(shape, base, &cells, 12.25);
        legacy.column_widths = merged.column_widths.clone();
        legacy.row_heights = merged.row_heights.clone();
        legacy.column_left = merged.column_left.clone();
        legacy.column_right = merged.column_right.clone();
        legacy.row_top = merged.row_top.clone();
        legacy.row_bottom = merged.row_bottom.clone();
        legacy.column_spacing = merged.column_spacing;
        legacy.row_spacing = merged.row_spacing;
        let items: Vec<avenger_layout::GridItem> = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| avenger_layout::GridItem {
                id: index,
                slot: cell.slot,
                content_size: cell.content_size,
                inner_edges: cell.inner_edges,
                outer_edges: cell.outer_edges,
                total_edges: cell.total_edges,
            })
            .collect();
        let legacy_solution = legacy.solve(&items);

        assert_eq!(applied.column_starts, legacy_solution.column_starts);
        assert_eq!(applied.row_starts, legacy_solution.row_starts);
        assert_eq!(applied.data.column_widths, legacy_solution.column_widths);
        assert_eq!(applied.data.row_heights, legacy_solution.row_heights);
        assert_eq!(applied.content_size, legacy_solution.content_size);
        for cell in &cells {
            let new_targets = applied.edge_targets_for_slot(cell.slot);
            let legacy_targets = legacy_solution.edge_targets_for_slot(cell.slot);
            assert_eq!(new_targets.inner, legacy_targets.inner);
            assert_eq!(new_targets.total, legacy_targets.total);
            assert_eq!(
                applied.content_origin_for_slot(cell.slot),
                legacy_solution.content_origin_for_slot(cell.slot)
            );
        }
    }
}
