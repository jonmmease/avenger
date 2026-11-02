//! The pixel grids of the ranges that brushes write: the scale of a plot's dimension at a gesture,
//! and the cells of its range that a range holds whole. The renderer and a query that filters by
//! the selection compute the cells alike, through the scale's `scale`, which avenger-scales'
//! `scale_expr` runs in SQL.

use arrow::array::{ArrayRef, AsArray, Int64Array};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float64Type};
use avenger_scales::scales::ConfiguredScale;

use crate::error::ChartError;

type Result<T> = std::result::Result<T, ChartError>;

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
            return Err(ChartError::Grid(format!(
                "a cell is positive and finite, not {cell}"
            )));
        }
        let (a, b) = (scale.numeric_interval_range())
            .map_err(|error| ChartError::Grid(format!("a grid needs a numeric range: {error}")))?;
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
        let positions = (self.scale.scale(values))
            .map_err(|error| ChartError::Grid(format!("a grid cannot scale values: {error}")))?;
        let positions = cast(&positions, &DataType::Float64)?;
        let positions = positions.as_primitive::<Float64Type>();
        Ok(positions
            .iter()
            .map(|position| self.cell_of(position?))
            .collect())
    }

    /// The cell of the position `position`, in the units of the range.
    fn cell_of(&self, position: f64) -> Option<i64> {
        let cell = ((position - self.origin) / self.cell).floor();
        (cell.is_finite() && cell >= i64::MIN as f64 && cell < i64::MAX as f64)
            .then_some(cell as i64)
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
            return Err(ChartError::Grid(format!(
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
    /// Whether each of `values` lies in a cell from `first` to `last`. A value without a cell lies
    /// in none.
    pub fn contains(&self, values: &ArrayRef) -> Result<Vec<bool>> {
        let cells = self.grid.cells(values)?;
        Ok(cells
            .iter()
            .map(|cell| cell.is_some_and(|cell| self.first <= cell && cell <= self.last))
            .collect())
    }
}
