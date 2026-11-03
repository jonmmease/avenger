//! The pixel grids of the ranges and the lassos that brushes write: the scale of a plot's
//! dimension at a gesture, the cells of its range that a range holds whole, and the cells of two
//! dimensions that a lasso covers. The renderer and a query that filters by the selection compute
//! the cells alike, through this module.

use arrow::array::{Array, ArrayRef, AsArray, BooleanArray, BooleanBufferBuilder, Int64Array};
use arrow::buffer::{BooleanBuffer, NullBuffer};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float32Type, Float64Type};
use avenger_scales::scales::ConfiguredScale;

use crate::{Result, SelectionError};

/// The grid of a plot's dimension at a gesture: cells `cell` wide in the units of the scale's
/// range, which are logical pixels for the scales of a plot's positions, counted from `origin`,
/// the smaller end of the range.
#[derive(Clone, Debug)]
pub struct PixelGrid {
    /// The scale of the dimension at the gesture, with the domain of a navigated view.
    pub scale: ConfiguredScale,
    pub origin: f64,
    pub cell: f64,
}

impl PixelGrid {
    /// The grid of `scale` with cells `cell` wide, or an error for a scale whose range is not
    /// numeric, or a cell that is not positive and finite.
    pub fn new(scale: ConfiguredScale, cell: f64) -> Result<Self> {
        if !(cell.is_finite() && cell > 0.0) {
            return Err(SelectionError::Grid(format!(
                "a cell is positive and finite, not {cell}"
            )));
        }
        let (a, b) = (scale.numeric_interval_range()).map_err(|error| {
            SelectionError::Grid(format!("a grid needs a numeric range: {error}"))
        })?;
        Ok(Self {
            scale,
            origin: a.min(b),
            cell,
        })
    }

    /// The cell of each of `values`: `floor((s(v) − origin) / cell)` of the value's position
    /// `s(v)` through the scale, or null for a null value, a position that is null or not finite,
    /// and a cell outside `i64`.
    pub fn cells(&self, values: &ArrayRef) -> Result<Int64Array> {
        let positions = self.positions(values)?;
        let (cells, valid) = match positions.data_type() {
            DataType::Float32 => {
                self.cells_of(positions.as_primitive::<Float32Type>().values().as_ref())
            }
            _ => self.cells_of(positions.as_primitive::<Float64Type>().values().as_ref()),
        };
        let valid = valid_where(valid, positions.nulls());
        Ok(Int64Array::new(cells.into(), Some(NullBuffer::new(valid))))
    }

    /// The positions of `values` through the scale, as 32- or 64-bit floats.
    fn positions(&self, values: &ArrayRef) -> Result<ArrayRef> {
        let positions = (self.scale.scale(values)).map_err(|error| {
            SelectionError::Grid(format!("a grid cannot scale values: {error}"))
        })?;
        Ok(match positions.data_type() {
            DataType::Float32 | DataType::Float64 => positions,
            _ => cast(&positions, &DataType::Float64)?,
        })
    }

    /// The cells of `positions`, and whether each has one, as [`cell_of`](Self::cell_of) gives
    /// it.
    fn cells_of<T: Floats + ?Sized>(&self, positions: &T) -> (Vec<i64>, BooleanBuffer) {
        let len = positions.len();
        let cells = (0..len)
            .map(|index| self.cell(positions.at(index)).0)
            .collect();
        let valid = BooleanBuffer::collect_bool(len, |index| self.cell(positions.at(index)).1);
        (cells, valid)
    }

    /// The cell of `position`, `floor((position − origin) / cell)`, and whether the cell is a
    /// finite cell within `i64`.
    #[inline]
    fn cell(&self, position: f64) -> (i64, bool) {
        let cell = ((position - self.origin) / self.cell).floor();
        let valid = cell.is_finite() && cell >= i64::MIN as f64 && cell < i64::MAX as f64;
        (cell as i64, valid)
    }

    /// The cell of the position `position`, in the units of the range.
    fn cell_of(&self, position: f64) -> Option<i64> {
        let (cell, valid) = self.cell(position);
        valid.then_some(cell)
    }

    /// The cells of a box from its near edge `low` to its far edge `high`, in the units of the
    /// range: the cell of `low`, and the last cell that the box reaches into,
    /// `ceil((high − origin) / cell) − 1`, so a far edge on a cell boundary adds no cell. A box
    /// narrower than a cell holds the cell of `low`.
    pub fn span(self, low: f64, high: f64) -> Result<GridCells> {
        let (Some(first), Some(last)) = (
            self.cell_of(low),
            self.cell_of(high).map(|cell| {
                let on_edge = (high - self.origin) / self.cell == cell as f64;
                if on_edge {
                    cell - 1
                } else {
                    cell
                }
            }),
        ) else {
            return Err(SelectionError::Grid(format!(
                "a box from {low} to {high} has no cells"
            )));
        };
        Ok(GridCells {
            first,
            last: last.max(first),
            grid: self,
        })
    }

    /// Whether `other` is the same grid: the same type of scale, domain, range, and options, the
    /// same origin, and the same cell.
    pub fn same(&self, other: &PixelGrid) -> bool {
        let (scale, theirs) = (&self.scale, &other.scale);
        scale.scale_impl.scale_type() == theirs.scale_impl.scale_type()
            && scale.domain() == theirs.domain()
            && scale.range() == theirs.range()
            && scale.config.options == theirs.config.options
            && self.origin == other.origin
            && self.cell == other.cell
    }
}

/// The grid of a range and the cells of the grid that the range holds, from `first` to `last`.
#[derive(Clone, Debug)]
pub struct GridCells {
    pub grid: PixelGrid,
    pub first: i64,
    pub last: i64,
}

impl GridCells {
    /// Whether each of `values` lies in a cell from `first` to `last`, as a mask without nulls. A
    /// value without a cell lies in none.
    pub fn contains(&self, values: &ArrayRef) -> Result<BooleanArray> {
        let positions = self.grid.positions(values)?;
        let within = match positions.data_type() {
            DataType::Float32 => {
                self.within(positions.as_primitive::<Float32Type>().values().as_ref())
            }
            _ => self.within(positions.as_primitive::<Float64Type>().values().as_ref()),
        };
        Ok(BooleanArray::new(
            valid_where(within, positions.nulls()),
            None,
        ))
    }

    /// Whether each of `positions` has a cell from `first` to `last`, in one pass.
    fn within<T: Floats + ?Sized>(&self, positions: &T) -> BooleanBuffer {
        BooleanBuffer::collect_bool(positions.len(), |index| {
            let (cell, valid) = self.grid.cell(positions.at(index));
            valid & (self.first <= cell) & (cell <= self.last)
        })
    }
}

/// The cells that a lasso covers, with a grid for each of a plot's two dimensions. Each row of
/// cells of the second dimension, from `first_row`, holds runs of cells of the first dimension.
/// The cells decide membership: a point matches when its cell in each grid is one of them, so
/// the renderer and a query that groups rows by their cells select the same rows.
#[derive(Clone, Debug)]
pub struct PolygonCells {
    pub grids: [PixelGrid; 2],
    pub first_row: i64,
    /// The runs of each row, from `first_row`: the first and the last cell of each run, in
    /// ascending order, apart from each other.
    pub rows: Vec<Vec<[i64; 2]>>,
}

impl PolygonCells {
    /// The cells whose centers are inside one of `polygons`, by the even-odd rule for each
    /// polygon. Each polygon is its vertices `[x, y]` in order around it, in the units of the
    /// grids' ranges, which are logical pixels for a plot's positions.
    pub fn cover(grids: [PixelGrid; 2], polygons: &[Vec<[f64; 2]>]) -> Self {
        let [columns, rows] = &grids;
        let center = |grid: &PixelGrid, cell: i64| grid.origin + (cell as f64 + 0.5) * grid.cell;
        // The first cell whose center is at or after `position`.
        let from = |grid: &PixelGrid, position: f64| {
            ((position - grid.origin) / grid.cell - 0.5).ceil() as i64
        };
        let mut covered: std::collections::BTreeMap<i64, Vec<[i64; 2]>> = Default::default();
        for vertices in polygons {
            let edges: Vec<[f64; 4]> = (0..vertices.len())
                .map(|index| {
                    let [xi, yi] = vertices[index];
                    let [xj, yj] = vertices[(index + vertices.len() - 1) % vertices.len()];
                    [xi, yi, xj, yj]
                })
                .filter(|[xi, yi, xj, yj]| {
                    yi != yj && [xi, yi, xj, yj].iter().all(|value| value.is_finite())
                })
                .collect();
            let (low, high) = (edges.iter()).fold((f64::INFINITY, f64::NEG_INFINITY), |b, e| {
                (b.0.min(e[1].min(e[3])), b.1.max(e[1].max(e[3])))
            });
            if edges.is_empty() {
                continue;
            }
            let mut crossings = Vec::new();
            for row in from(rows, low)..from(rows, high) {
                let y = center(rows, row);
                crossings.clear();
                crossings.extend(
                    (edges.iter())
                        .filter(|[_, yi, _, yj]| (*yi > y) != (*yj > y))
                        .map(|[xi, yi, xj, yj]| (xj - xi) * (y - yi) / (yj - yi) + xi),
                );
                crossings.sort_by(f64::total_cmp);
                let runs = covered.entry(row).or_default();
                // A center is inside from an odd crossing to before the next one.
                for pair in crossings.chunks_exact(2) {
                    let (first, last) = (from(columns, pair[0]), from(columns, pair[1]) - 1);
                    if first <= last {
                        runs.push([first, last]);
                    }
                }
            }
        }
        covered.retain(|_, runs| !runs.is_empty());
        let Some((&first_row, _)) = covered.first_key_value() else {
            return Self {
                grids,
                first_row: 0,
                rows: Vec::new(),
            };
        };
        let last_row = *covered.last_key_value().expect("a row").0;
        let rows = (first_row..=last_row)
            .map(|row| merge(covered.remove(&row).unwrap_or_default()))
            .collect();
        Self {
            grids,
            first_row,
            rows,
        }
    }

    /// Whether the point of each of `x` and `y` lies in one of the cells, as a mask without
    /// nulls. A point without a cell in each grid lies in none.
    pub fn contains(&self, x: &ArrayRef, y: &ArrayRef) -> Result<BooleanArray> {
        let (x, y) = (self.grids[0].positions(x)?, self.grids[1].positions(y)?);
        let raster = Raster::new(self);
        let inside = match (x.data_type(), y.data_type()) {
            (DataType::Float32, DataType::Float32) => raster.contain(
                x.as_primitive::<Float32Type>().values().as_ref(),
                y.as_primitive::<Float32Type>().values().as_ref(),
            ),
            (DataType::Float32, _) => raster.contain(
                x.as_primitive::<Float32Type>().values().as_ref(),
                y.as_primitive::<Float64Type>().values().as_ref(),
            ),
            (_, DataType::Float32) => raster.contain(
                x.as_primitive::<Float64Type>().values().as_ref(),
                y.as_primitive::<Float32Type>().values().as_ref(),
            ),
            _ => raster.contain(
                x.as_primitive::<Float64Type>().values().as_ref(),
                y.as_primitive::<Float64Type>().values().as_ref(),
            ),
        };
        let inside = valid_where(valid_where(inside, x.nulls()), y.nulls());
        Ok(BooleanArray::new(inside, None))
    }

    /// Whether each pair of a cell of the first grid in `columns` and a cell of the second grid
    /// in `rows` is one of the cells, as a mask without nulls, as a table of rows grouped by their
    /// cells holds them. A null cell is none of the cells.
    pub fn covers(&self, columns: &Int64Array, rows: &Int64Array) -> BooleanArray {
        let raster = Raster::new(self);
        let (column, row) = (columns.values(), rows.values());
        let covered = BooleanBuffer::collect_bool(columns.len(), |index| {
            raster.holds(column[index], row[index])
        });
        let covered = valid_where(valid_where(covered, columns.nulls()), rows.nulls());
        BooleanArray::new(covered, None)
    }
}

/// The cells of a [`PolygonCells`] as one bit for each cell of the runs' bounds, row by row.
struct Raster<'a> {
    cells: &'a PolygonCells,
    bits: BooleanBuffer,
    left: i64,
    width: u64,
    height: u64,
}

impl<'a> Raster<'a> {
    fn new(cells: &'a PolygonCells) -> Self {
        let runs = cells.rows.iter().flatten();
        let left = runs.clone().map(|run| run[0]).min().unwrap_or(0);
        let right = runs.map(|run| run[1]).max().unwrap_or(-1);
        let width = (right - left + 1) as u64;
        let mut bits = BooleanBufferBuilder::new(width as usize * cells.rows.len());
        for runs in &cells.rows {
            let mut next = left;
            for &[first, last] in runs {
                bits.append_n((first - next) as usize, false);
                bits.append_n((last - first + 1) as usize, true);
                next = last + 1;
            }
            bits.append_n((right + 1 - next) as usize, false);
        }
        // One unset bit for cells that hold no cell, so a lookup always reads a bit.
        let bits = if bits.is_empty() {
            BooleanBuffer::new_unset(1)
        } else {
            bits.finish()
        };
        Self {
            cells,
            bits,
            left,
            width,
            height: cells.rows.len() as u64,
        }
    }

    /// Whether the point of each of `xs` and `ys` has a cell in each grid that is one of the
    /// cells, in one pass over the positions.
    fn contain<X: Floats + ?Sized, Y: Floats + ?Sized>(&self, xs: &X, ys: &Y) -> BooleanBuffer {
        let [columns, rows] = &self.cells.grids;
        BooleanBuffer::collect_bool(xs.len(), |index| {
            let (column, column_valid) = columns.cell(xs.at(index));
            let (row, row_valid) = rows.cell(ys.at(index));
            column_valid & row_valid & self.holds(column, row)
        })
    }

    /// Whether the cell `column` of the first grid and the cell `row` of the second grid are one
    /// of the cells.
    #[inline]
    fn holds(&self, column: i64, row: i64) -> bool {
        let column = column.wrapping_sub(self.left) as u64;
        let row = row.wrapping_sub(self.cells.first_row) as u64;
        let within = (column < self.width) & (row < self.height);
        let bit = if within { row * self.width + column } else { 0 };
        within & self.bits.value(bit as usize)
    }
}

/// A slice of 32- or 64-bit floats, read as 64-bit floats.
trait Floats {
    fn len(&self) -> usize;
    fn at(&self, index: usize) -> f64;
}

impl Floats for [f32] {
    fn len(&self) -> usize {
        <[f32]>::len(self)
    }
    #[inline]
    fn at(&self, index: usize) -> f64 {
        self[index] as f64
    }
}

impl Floats for [f64] {
    fn len(&self) -> usize {
        <[f64]>::len(self)
    }
    #[inline]
    fn at(&self, index: usize) -> f64 {
        self[index]
    }
}

/// `mask` without the rows that `nulls` marks null.
fn valid_where(mask: BooleanBuffer, nulls: Option<&NullBuffer>) -> BooleanBuffer {
    match nulls {
        Some(nulls) => &mask & nulls.inner(),
        None => mask,
    }
}

/// `runs` in ascending order, with the runs that overlap or touch merged.
fn merge(mut runs: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    runs.sort_unstable();
    let mut merged: Vec<[i64; 2]> = Vec::with_capacity(runs.len());
    for run in runs {
        match merged.last_mut() {
            Some(last) if run[0] <= last[1] + 1 => last[1] = last[1].max(run[1]),
            _ => merged.push(run),
        }
    }
    merged
}
