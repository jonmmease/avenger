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
    SolveOptions, Spacing, TrackSize,
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

#[allow(clippy::too_many_arguments)]
fn cells_grid(
    shape: GridShape,
    base_cell_size: Size,
    cells: &[GridCell],
    column_spacing: Spacing,
    row_spacing: Spacing,
    column_sizes: Option<&[TrackSize]>,
    row_sizes: Option<&[TrackSize]>,
    export: bool,
) -> Layout<usize> {
    // The base cell size is the equal-division floor for undeclared grids;
    // an axis with declared track sizing must not be floored toward equal
    // shares (Fixed pins, Flex floors at content).
    let base_cell_size = Size::new(
        if column_sizes.is_some() {
            0.0
        } else {
            base_cell_size.width
        },
        if row_sizes.is_some() {
            0.0
        } else {
            base_cell_size.height
        },
    );
    let mut grid = Layout::grid(shape.rows, shape.columns)
        .base_cell_size(base_cell_size)
        .column_spacing(column_spacing)
        .row_spacing(row_spacing);
    if let Some(sizes) = column_sizes {
        grid = grid.columns(sizes.iter().copied());
    }
    if let Some(sizes) = row_sizes {
        grid = grid.rows(sizes.iter().copied());
    }
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_concat_grid(
    shape: GridShape,
    base_cell_size: Size,
    cells: &[GridCell],
    column_spacing: Spacing,
    row_spacing: Spacing,
    column_sizes: Option<&[TrackSize]>,
    row_sizes: Option<&[TrackSize]>,
    export: bool,
) -> Result<SolvedConcatGrid, String> {
    let grid = cells_grid(
        shape,
        base_cell_size,
        cells,
        column_spacing,
        row_spacing,
        column_sizes,
        row_sizes,
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
    column_sizes: Option<&[TrackSize]>,
    row_sizes: Option<&[TrackSize]>,
) -> Result<SolvedConcatGrid, String> {
    let shape = merged.shape;
    let local = cells_grid(
        shape,
        base_cell_size,
        cells,
        merged.column_spacing,
        merged.row_spacing,
        column_sizes,
        row_sizes,
        false,
    )
    .share(0usize);

    let mut phantom = Layout::grid(shape.rows, shape.columns)
        .column_spacing(merged.column_spacing)
        .row_spacing(merged.row_spacing)
        .share(0usize);
    if let Some(sizes) = column_sizes {
        phantom = phantom.columns(sizes.iter().copied());
    }
    if let Some(sizes) = row_sizes {
        phantom = phantom.rows(sizes.iter().copied());
    }
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

    /// Export reproduces the pre-solve requirement fold: per-track maxima
    /// with base floors, span items contributing edges (first/last track)
    /// but no content, holes defaulting.
    #[test]
    fn export_is_the_pre_solve_requirement_fold() {
        let shape = GridShape {
            rows: 2,
            columns: 3,
        };
        let base = Size::new(40.0, 30.0);
        let cells = vec![
            cell(0, 0, 1, 1, 100.0, 50.0, Edges::new(0.0, 5.0, 0.0, 3.0)),
            cell(0, 2, 1, 1, 120.0, 50.0, Edges::new(0.0, 4.0, 0.0, 7.0)),
            cell(1, 0, 1, 2, 200.0, 60.0, Edges::new(0.0, 6.0, 0.0, 2.0)),
        ];
        let spacing = Spacing {
            min_gap: 10.0,
            ..Default::default()
        };
        let exported = solve_concat_grid(shape, base, &cells, spacing, spacing, None, None, true)
            .expect("solve");

        // Span content does not reach the tracks; the hole keeps the base.
        assert_eq!(exported.data.column_widths, vec![100.0, 40.0, 120.0]);
        assert_eq!(exported.data.row_heights, vec![50.0, 60.0]);
        // Span edges land on the first/last spanned tracks.
        let totals = |edges: &[EdgeDemand]| edges.iter().map(|edge| edge.total).collect::<Vec<_>>();
        assert_eq!(totals(&exported.data.column_left), vec![3.0, 0.0, 7.0]);
        assert_eq!(totals(&exported.data.column_right), vec![5.0, 6.0, 4.0]);
    }

    /// The phantom-cousin apply: merged floors install exactly, local span
    /// constraints re-apply on top, and per-slot reads follow the laws.
    #[test]
    fn apply_installs_merged_floors_and_reapplies_spans() {
        let shape = GridShape {
            rows: 2,
            columns: 3,
        };
        let base = Size::new(40.0, 30.0);
        let cells = vec![
            cell(0, 0, 1, 1, 100.0, 50.0, Edges::new(0.0, 5.0, 0.0, 3.0)),
            cell(0, 2, 1, 1, 120.0, 50.0, Edges::new(0.0, 4.0, 0.0, 7.0)),
            cell(1, 0, 1, 2, 200.0, 60.0, Edges::new(0.0, 6.0, 0.0, 2.0)),
        ];
        let spacing = Spacing {
            min_gap: 10.0,
            ..Default::default()
        };
        let mut merged = solve_concat_grid(shape, base, &cells, spacing, spacing, None, None, true)
            .expect("solve")
            .data;
        // A cousin was bigger: wider first column, taller second row, a
        // larger min gap.
        merged.column_widths[0] = 120.0;
        merged.row_heights[1] = 70.0;
        merged.column_spacing.min_gap = 12.0;
        merged.row_spacing.min_gap = 12.0;

        let applied = solve_concat_grid_against(&merged, base, &cells, None, None).expect("apply");

        // Span deficit: 120 + gap(12) + 40 = 172 against 200 -> +14 per
        // spanned track.
        assert_eq!(applied.data.column_widths, vec![134.0, 54.0, 120.0]);
        assert_eq!(applied.data.row_heights, vec![50.0, 70.0]);
        // Gaps: max(12, 5+0) = 12 and max(12, 6+7) = 13.
        assert_eq!(applied.column_starts, vec![0.0, 146.0, 213.0]);
        assert_eq!(applied.row_starts, vec![0.0, 62.0]);
        assert_eq!(applied.content_size, Size::new(333.0, 132.0));
        // The span slot's positional extent meets its constraint exactly.
        let span_slot = cells[2].slot;
        assert_eq!(applied.content_size_for_slot(span_slot).width, 200.0);
        assert_eq!(applied.content_origin_for_slot(span_slot), [0.0, 62.0]);
        // Per-track fold: max(cell00's left 3.0, the span's left 2.0).
        assert_eq!(applied.edge_targets_for_slot(span_slot).total.left, 3.0);
    }
}
