//! Chart-owned concat/wrap grid solving through the unified
//! `avenger_layout::Layout` API.
//!
//! Concat coordination solves cousin grids together on a real share key
//! ([`solve_concat_grid_group`]): every member is rebuilt from its own
//! measured cells ([`GridMemberSpec`]), the solver merges the per-track
//! requirement folds across the group, and each member's solved region is
//! extracted as its own [`SolvedConcatGrid`] (merged floors plus that
//! member's span constraints).
//!
//! [`ChartGridData`] remains the exported requirement payload for
//! diagnostics and delta gating: [`solve_concat_grid`] with `export = true`
//! zeroes the spanned-axis content of multi-span cells so the extracted
//! track sizes equal the pre-span-constraint requirement fold (span
//! constraints re-apply per member in the group solve — the historical
//! contract).
//!
//! Per-slot reads mirror the historical grid solution: origins from track
//! starts (the solver's exact floats), edge targets from per-track demand
//! vectors, span content sizes positionally from the tracks.

use avenger_layout::{
    EdgeDemand, EdgeGrant, Edges, GridShape, GridSlot, Layout, LayoutSolution, RegionDetail, Side,
    Size, SolveOptions, Spacing, TrackSize,
};

use super::placement::EdgeTargets;

/// One measured concat child as a grid cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridCell {
    pub slot: GridSlot,
    pub content_size: Size,
    /// Per-side edge demand declarations (layered guide/legend pairs at
    /// production sites; unlayered totals in fixtures).
    pub edges: Edges<EdgeDemand>,
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
    pub column_left: Vec<EdgeGrant>,
    pub column_right: Vec<EdgeGrant>,
    pub row_top: Vec<EdgeGrant>,
    pub row_bottom: Vec<EdgeGrant>,
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
        leaf = leaf.demand(side, *cell.edges.side(side));
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

/// Everything needed to rebuild one member grid for a coordination group
/// solve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GridMemberSpec {
    pub shape: GridShape,
    pub base_cell_size: Size,
    pub cells: Vec<GridCell>,
    pub column_spacing: Spacing,
    pub row_spacing: Spacing,
    pub column_sizes: Option<Vec<TrackSize>>,
    pub row_sizes: Option<Vec<TrackSize>>,
}

/// Extract the solved per-track view from a grid member region. Spacing
/// comes from the solved tracks: the values the solve actually used,
/// post-merge for share-group members.
fn extract(
    solution: &LayoutSolution<usize>,
    member_path: &[usize],
    shape: GridShape,
    cells: &[GridCell],
) -> Result<SolvedConcatGrid, String> {
    let member = solution
        .at_path(member_path)
        .ok_or_else(|| "missing grid member region".to_string())?;
    let RegionDetail::Grid { tracks } = &member.detail else {
        return Err("grid member is not a grid".to_string());
    };

    let mut column_left = vec![EdgeGrant::default(); shape.columns];
    let mut column_right = vec![EdgeGrant::default(); shape.columns];
    let mut row_top = vec![EdgeGrant::default(); shape.rows];
    let mut row_bottom = vec![EdgeGrant::default(); shape.rows];
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
            column_spacing: tracks.column_spacing,
            row_spacing: tracks.row_spacing,
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
    extract(&solved, &[], shape, cells)
}

/// Solve a coordination group of cousin grids together on one share key.
/// The solver merges the members' per-track requirement folds (merged ≥
/// every local fold by construction), then each member re-solves at the
/// merged floors with its own span constraints. Returns one solved view per
/// member, in input order.
pub(crate) fn solve_concat_grid_group(
    members: &[GridMemberSpec],
) -> Result<Vec<SolvedConcatGrid>, String> {
    let root: Layout<usize> = Layout::row(
        members
            .iter()
            .map(|member| {
                cells_grid(
                    member.shape,
                    member.base_cell_size,
                    &member.cells,
                    member.column_spacing,
                    member.row_spacing,
                    member.column_sizes.as_deref(),
                    member.row_sizes.as_deref(),
                    false,
                )
                .share(0usize)
            })
            .collect::<Vec<_>>(),
    );
    let solved = root
        .solve(&SolveOptions::default())
        .map_err(|err| err.to_string())?;
    if !solved.diagnostics().skipped_groups.is_empty() {
        return Err("concat coordination group members had incompatible shapes".to_string());
    }
    members
        .iter()
        .enumerate()
        .map(|(index, member)| extract(&solved, &[index], member.shape, &member.cells))
        .collect()
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
            edges: Edges::new(
                EdgeDemand::Unlayered(edges.top),
                EdgeDemand::Unlayered(edges.right),
                EdgeDemand::Unlayered(edges.bottom),
                EdgeDemand::Unlayered(edges.left),
            ),
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
        let totals = |edges: &[EdgeGrant]| edges.iter().map(|edge| edge.total).collect::<Vec<_>>();
        assert_eq!(totals(&exported.data.column_left), vec![3.0, 0.0, 7.0]);
        assert_eq!(totals(&exported.data.column_right), vec![5.0, 6.0, 4.0]);
    }

    /// The group solve on a real share key: the bigger cousin's folds become
    /// the group floors, local span constraints re-apply on top, and
    /// per-slot reads follow the laws. The literals are the former
    /// phantom-cousin apply values (the group solve is the same float path).
    #[test]
    fn group_solve_installs_merged_floors_and_reapplies_spans() {
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
        // The bigger cousin: wider first column, taller second row, a larger
        // min gap (its requirement fold is [120, 40, 120] x [50, 70]).
        let cousin_cells = vec![
            cell(0, 0, 1, 1, 120.0, 50.0, Edges::default()),
            cell(0, 2, 1, 1, 120.0, 50.0, Edges::default()),
            cell(1, 0, 1, 1, 120.0, 70.0, Edges::default()),
        ];
        let spacing = |min_gap: f32| Spacing {
            min_gap,
            ..Default::default()
        };
        let members = vec![
            GridMemberSpec {
                shape,
                base_cell_size: base,
                cells: cells.clone(),
                column_spacing: spacing(10.0),
                row_spacing: spacing(10.0),
                column_sizes: None,
                row_sizes: None,
            },
            GridMemberSpec {
                shape,
                base_cell_size: base,
                cells: cousin_cells,
                column_spacing: spacing(12.0),
                row_spacing: spacing(12.0),
                column_sizes: None,
                row_sizes: None,
            },
        ];

        let solved = solve_concat_grid_group(&members).expect("group solve");
        let applied = &solved[0];

        // Merged spacing reported on every member.
        assert_eq!(applied.data.column_spacing.min_gap, 12.0);
        assert_eq!(applied.data.row_spacing.min_gap, 12.0);
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
        // The cousin holds the merged floors without span growth.
        assert_eq!(solved[1].data.column_widths, vec![120.0, 40.0, 120.0]);
        assert_eq!(solved[1].data.row_heights, vec![50.0, 70.0]);
    }
}
