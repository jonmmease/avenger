//! The conditions of clauses, and the rows that each condition matches.

use std::collections::HashSet;

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanArray, Float64Array, Scalar},
    buffer::BooleanBuffer,
    compute::{
        and, cast,
        kernels::cmp::{gt_eq, lt, lt_eq},
        prep_null_mask_filter,
    },
    datatypes::{DataType, Float64Type, Int64Type},
    row::{RowConverter, SortField},
};

use crate::{grid::Raster, set::SetIndex, CellSpan, PolygonCells, Result, SelectionError};

/// A check of one or more columns of a row. A condition is symbolic: it holds the columns and the
/// values, ends, cells, or polygons that a row's values fit, and [`Condition::matches`] evaluates
/// it over Arrow columns. A set, a range, and a cyclic range record, for each column, the label of
/// the channel that the selection projected through, such as `x`, or `None`.
#[derive(Clone, Debug)]
pub enum Condition {
    /// A row matches when its values of `columns` equal one tuple of `values`: the rows of the
    /// lists in `values`, one list for each column, all of one length. A set of one column holds
    /// the values of that column.
    Set {
        columns: Vec<String>,
        channels: Vec<Option<String>>,
        values: Vec<ArrayRef>,
    },
    /// A row matches when its value of `column` is between the two `ends`, in ascending order,
    /// or with `cells`, when its value lies in one of the cells. A brush that selects whole cells
    /// writes the span of its plot's cells that the box covers, and its `ends` are the edges of
    /// the box that it drew.
    Range {
        column: String,
        channel: Option<String>,
        ends: ArrayRef,
        exclude_end: bool,
        cells: Option<CellSpan>,
    },
    /// A row matches when the point of its values of `x_column` and `y_column` is inside one of
    /// `polygons`, each the vertices `x` and `y` in order around it, or with `cells`, when the
    /// point lies in one of the cells. A lasso that selects whole cells writes the cells of its
    /// plot, and its `polygons` are the paths that it drew.
    Polygon {
        x_column: String,
        y_column: String,
        polygons: Vec<(ArrayRef, ArrayRef)>,
        cells: Option<PolygonCells>,
    },
    /// A row matches when its value of `column` is from `start` forward to `end`, in a dimension
    /// whose values wrap after `period`, such as longitudes in degrees with a period of 360. When
    /// `end` is less than `start`, the range crosses the wrap.
    CyclicRange {
        column: String,
        channel: Option<String>,
        start: f64,
        end: f64,
        period: f64,
    },
}

impl Condition {
    /// A set of the tuples of `values` in `columns`, with the channel label of each column, or an
    /// error when the columns, the labels, and the lists of values differ in number, or the lists
    /// differ in length.
    pub fn set(
        columns: Vec<String>,
        channels: Vec<Option<String>>,
        values: Vec<ArrayRef>,
    ) -> Result<Self> {
        if columns.is_empty() || channels.len() != columns.len() || values.len() != columns.len() {
            return Err(SelectionError::Condition(format!(
                "a set has a label and a list of values for each of its columns, and at least \
                 one column: {} columns, {} labels, {} lists",
                columns.len(),
                channels.len(),
                values.len()
            )));
        }
        if values.iter().any(|list| list.len() != values[0].len()) {
            return Err(SelectionError::Condition(
                "the lists of values of a set have one length".to_string(),
            ));
        }
        Ok(Condition::Set {
            columns,
            channels,
            values,
        })
    }

    /// A set of the values `values` of the one column `column`.
    pub fn values(column: impl Into<String>, channel: Option<String>, values: ArrayRef) -> Self {
        Condition::Set {
            columns: vec![column.into()],
            channels: vec![channel],
            values: vec![values],
        }
    }

    /// The columns that the condition reads: its column, the columns of a set, or the two
    /// columns of a polygon.
    pub fn columns(&self) -> Vec<&str> {
        match self {
            Condition::Set { columns, .. } => columns.iter().map(String::as_str).collect(),
            Condition::Range { column, .. } | Condition::CyclicRange { column, .. } => {
                vec![column]
            }
            Condition::Polygon {
                x_column, y_column, ..
            } => vec![x_column, y_column],
        }
    }

    /// Whether each row matches the condition, from the values of the condition's
    /// [`columns`](Self::columns) for the rows, in their order, as a mask without nulls. A null
    /// matches no condition, and a dictionary column compares its values.
    ///
    /// - A set matches the rows whose values equal one of its tuples, with its values cast to
    ///   the columns' types. A tuple with a null matches no row.
    /// - A range matches the values from its first end through its second, or to before its
    ///   second with `exclude_end`, and with cells, the values in its cells.
    /// - A polygon condition matches the points inside one of its polygons, each by the even-odd
    ///   rule, through the numbers of the two columns, and with cells, the points in its cells.
    /// - A cyclic range matches the values whose difference from its start, wrapped into one
    ///   period, is at most its width, and a range at least one period wide matches every value.
    pub fn matches(&self, columns: &[ArrayRef]) -> Result<BooleanArray> {
        let types: Vec<DataType> = (columns.iter())
            .map(|column| column.data_type().clone())
            .collect();
        self.prepare(&types)?.matches(columns)
    }

    /// The condition prepared to match columns of the types `types`, one for each of its
    /// [`columns`](Self::columns), many times: a set's index of its tuples, a polygon's edges,
    /// and a lasso's bitmap of its cells, built once.
    pub fn prepare(&self, types: &[DataType]) -> Result<PreparedCondition> {
        let prepared = match self {
            Condition::Set { values, .. } => {
                let plain_types: Vec<DataType> = types.iter().map(value_type).collect();
                let lists = (values.iter().zip(&plain_types))
                    .map(|(list, data_type)| cast_to(list, data_type))
                    .collect::<Result<Vec<_>>>()?;
                Prepared::Set {
                    index: SetIndex::new(&lists),
                    types: plain_types,
                }
            }
            Condition::Polygon {
                cells: Some(cells), ..
            } => Prepared::Cells(Box::new(Raster::new(cells))),
            Condition::Polygon { polygons, .. } => Prepared::Polygons(
                (polygons.iter())
                    .map(|(x, y)| {
                        let vertices: Vec<(f64, f64)> = (numbers(x)?.into_iter())
                            .zip(numbers(y)?)
                            .filter_map(|(x, y)| Some((x?, y?)))
                            .collect();
                        Ok(Edges::new(&vertices))
                    })
                    .collect::<Result<_>>()?,
            ),
            _ => Prepared::Nothing,
        };
        Ok(PreparedCondition {
            condition: self.clone(),
            prepared,
        })
    }
}

/// A condition prepared to match columns of given types many times, which matches the rows that
/// [`Condition::matches`] does: a set's index of its tuples, a polygon's edges, and a lasso's
/// bitmap of its cells are built once, when it is prepared. A query prepares each condition once
/// and matches it for each batch.
pub struct PreparedCondition {
    condition: Condition,
    prepared: Prepared,
}

enum Prepared {
    /// The index of a set's tuples for columns of the value types `types`, or `None` for types
    /// that match through Arrow's row format.
    Set {
        types: Vec<DataType>,
        index: Option<SetIndex>,
    },
    Polygons(Vec<Edges>),
    Cells(Box<Raster>),
    Nothing,
}

impl PreparedCondition {
    /// The condition.
    pub fn condition(&self) -> &Condition {
        &self.condition
    }

    /// Whether each row of a table of keys matches the condition, as a mask without nulls, as a
    /// query that groups rows by their keys checks them. `keys` holds a column for each of the
    /// condition's [`columns`](Condition::columns), in order: the row's cell, as a 64-bit integer,
    /// for a range with cells and each column of a polygon with cells, or the row's value for a
    /// set. A row whose key is null matches none. Other conditions have no keys, and a key that is
    /// not a 64-bit integer where a cell belongs is an error.
    pub fn matches_keys(&self, keys: &[ArrayRef]) -> Result<BooleanArray> {
        let cells = |index: usize| {
            keys[index].as_primitive_opt::<Int64Type>().ok_or_else(|| {
                SelectionError::Condition(format!(
                    "a key of cells is a 64-bit integer, not {}",
                    keys[index].data_type()
                ))
            })
        };
        match (&self.condition, &self.prepared) {
            (Condition::Set { .. }, _) => self.matches(keys),
            (
                Condition::Range {
                    cells: Some(span), ..
                },
                _,
            ) => {
                let key = cells(0)?;
                let cell = key.values();
                let within = BooleanBuffer::collect_bool(key.len(), |row| {
                    (span.first <= cell[row]) & (cell[row] <= span.last)
                });
                let within = match key.nulls() {
                    Some(nulls) => &within & nulls.inner(),
                    None => within,
                };
                Ok(BooleanArray::new(within, None))
            }
            (Condition::Polygon { .. }, Prepared::Cells(raster)) => {
                Ok(raster.covers(cells(0)?, cells(1)?))
            }
            (condition, _) => Err(SelectionError::Condition(format!(
                "only ranges and polygons with cells, and sets, have keys, not the condition of {:?}",
                condition.columns()
            ))),
        }
    }

    /// Whether each row matches the condition, as [`Condition::matches`] gives it. Columns of
    /// other types than the condition was prepared for match without the prepared index.
    pub fn matches(&self, columns: &[ArrayRef]) -> Result<BooleanArray> {
        match (&self.condition, &self.prepared) {
            (Condition::Set { values, .. }, Prepared::Set { types, index }) => {
                let same = (columns.iter().zip(types))
                    .all(|(column, data_type)| &value_type(column.data_type()) == data_type);
                match index {
                    Some(index) if same => Ok(index.contains(columns)),
                    _ => Ok(BooleanArray::from(in_set(columns, values)?)),
                }
            }
            (
                Condition::Range {
                    cells: Some(cells), ..
                },
                _,
            ) => cells.contains(&plain(&columns[0])?),
            (
                Condition::Range {
                    ends, exclude_end, ..
                },
                _,
            ) => in_range(&columns[0], ends, *exclude_end),
            (Condition::Polygon { .. }, Prepared::Cells(raster)) => {
                raster.contains(&plain(&columns[0])?, &plain(&columns[1])?)
            }
            (Condition::Polygon { .. }, Prepared::Polygons(polygons)) => {
                in_polygons(&columns[0], &columns[1], polygons)
            }
            (
                Condition::CyclicRange {
                    start, end, period, ..
                },
                _,
            ) => in_cyclic_range(&columns[0], *start, *end, *period),
            _ => unreachable!("a condition's own preparation"),
        }
    }
}

/// The type of the values of `data_type`: a dictionary's values, or the type itself.
fn value_type(data_type: &DataType) -> DataType {
    match data_type {
        DataType::Dictionary(_, values) => values.as_ref().clone(),
        other => other.clone(),
    }
}

/// `values` as plain values of the type `data_type`.
pub(crate) fn cast_to(values: &ArrayRef, data_type: &DataType) -> Result<ArrayRef> {
    let values = plain(values)?;
    Ok(if values.data_type() == data_type {
        values
    } else {
        cast(&values, data_type)?
    })
}

/// `values` as their dictionary's values when they are a dictionary, so they compare with plain
/// values.
fn plain(values: &ArrayRef) -> Result<ArrayRef> {
    Ok(match values.data_type() {
        DataType::Dictionary(_, value_type) => cast(values, value_type)?,
        _ => values.clone(),
    })
}

/// `mask` with each null as `false`.
pub(crate) fn without_nulls(mask: BooleanArray) -> BooleanArray {
    if mask.null_count() == 0 {
        mask
    } else {
        prep_null_mask_filter(&mask)
    }
}

/// Whether each row of `columns` equals one tuple of `values`, through the rows of Arrow's row
/// format, so the time is linear in the rows and the tuples.
fn in_set(columns: &[ArrayRef], values: &[ArrayRef]) -> Result<Vec<bool>> {
    let columns = columns.iter().map(plain).collect::<Result<Vec<_>>>()?;
    let values = (values.iter().zip(&columns))
        .map(|(values, column)| cast_to(values, column.data_type()))
        .collect::<Result<Vec<_>>>()?;
    let fields = (columns.iter())
        .map(|column| SortField::new(column.data_type().clone()))
        .collect();
    let converter = RowConverter::new(fields)?;
    let tuples = converter.convert_columns(&values)?;
    let tuple_count = values.first().map_or(0, |list| list.len());
    let set: HashSet<_> = (0..tuple_count)
        .filter(|&index| values.iter().all(|list| list.is_valid(index)))
        .map(|index| tuples.row(index))
        .collect();
    let rows = converter.convert_columns(&columns)?;
    let row_count = columns.first().map_or(0, |column| column.len());
    Ok((0..row_count)
        .map(|index| {
            columns.iter().all(|column| column.is_valid(index)) && set.contains(&rows.row(index))
        })
        .collect())
}

/// Whether each value is from the first of `ends` through the second, or to before the second
/// with `exclude_end`, through Arrow's comparison kernels.
fn in_range(column: &ArrayRef, ends: &ArrayRef, exclude_end: bool) -> Result<BooleanArray> {
    let column = plain(column)?;
    let ends = cast_to(ends, column.data_type())?;
    let start = Scalar::new(ends.slice(0, 1));
    let end = Scalar::new(ends.slice(1, 1));
    let below = if exclude_end {
        lt(&column, &end)?
    } else {
        lt_eq(&column, &end)?
    };
    Ok(without_nulls(and(&gt_eq(&column, &start)?, &below)?))
}

/// `values` as 64-bit floats, cast when they are of another type.
fn floats(values: &ArrayRef) -> Result<Float64Array> {
    let values = plain(values)?;
    let values = if values.data_type() == &DataType::Float64 {
        values
    } else {
        cast(&values, &DataType::Float64)?
    };
    Ok(values.as_primitive::<Float64Type>().clone())
}

fn numbers(values: &ArrayRef) -> Result<Vec<Option<f64>>> {
    Ok(floats(values)?.iter().collect())
}

/// Whether each point of `x` and `y` is inside one of `polygons`, by the even-odd rule for each
/// polygon. A point with a null coordinate is inside none.
fn in_polygons(x: &ArrayRef, y: &ArrayRef, polygons: &[Edges]) -> Result<BooleanArray> {
    let (x, y) = (floats(x)?, floats(y)?);
    let mut inside = BooleanBuffer::new_unset(x.len());
    for edges in polygons {
        inside = &inside | &edges.contain(x.values(), y.values());
    }
    for nulls in [x.nulls(), y.nulls()].into_iter().flatten() {
        inside = &inside & nulls.inner();
    }
    Ok(BooleanArray::new(inside, None))
}

/// The edges of a polygon, indexed by horizontal bands so that a point checks only the edges of
/// its band. An edge runs from a vertex to the vertex before it, and a point toggles the even-odd
/// rule at each edge that crosses its `y` to its right:
/// `(yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi`.
///
/// Neither the index nor the bounds change the answer. Only an edge whose `y` range, from its
/// lower end to before its upper end, holds the point's `y` can cross it, so a horizontal edge
/// never crosses and a point outside the edges' `y` bounds crosses none. A point's band is
/// monotonic in `y`, so every edge that can cross it is in its band. A point left of every edge
/// crosses an even number of them, and a point right of every edge crosses none, so both are
/// outside. The `x` bounds widen by a few rounding errors of the crossing's `x`. A polygon with a
/// coordinate that is not finite checks every edge, since these arguments need finite values.
struct Edges {
    /// Each edge that can cross a point: `[xi, yi, xj, yj]`, from a vertex `i` to the vertex
    /// `j` before it.
    edges: Vec<[f64; 4]>,
    /// The bounds of the points that can be inside: `x` from `left` through `right`, and `y` from
    /// `low` to before `high`.
    left: f64,
    right: f64,
    low: f64,
    high: f64,
    /// The bands per unit of `y`, and the edges of band `b`, `members[starts[b]..starts[b + 1]]`,
    /// or `None` to check every edge.
    bands: Option<(f64, Vec<usize>, Vec<u32>)>,
}

impl Edges {
    fn new(vertices: &[(f64, f64)]) -> Self {
        let mut edges = Vec::with_capacity(vertices.len());
        let mut previous = vertices.len().wrapping_sub(1);
        for (index, &(xi, yi)) in vertices.iter().enumerate() {
            let (xj, yj) = vertices[previous];
            if yi != yj {
                edges.push([xi, yi, xj, yj]);
            }
            previous = index;
        }
        let finite = (vertices.iter()).all(|(x, y)| x.is_finite() && y.is_finite());
        if !finite || edges.is_empty() {
            return Self {
                edges,
                left: f64::NEG_INFINITY,
                right: f64::INFINITY,
                low: f64::NEG_INFINITY,
                high: f64::INFINITY,
                bands: None,
            };
        }
        let (mut left, mut right) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for [xi, yi, xj, yj] in &edges {
            left = left.min(xi.min(*xj));
            right = right.max(xi.max(*xj));
            low = low.min(yi.min(*yj));
            high = high.max(yi.max(*yj));
        }
        let rounding = 16.0 * f64::EPSILON * left.abs().max(right.abs()) + f64::MIN_POSITIVE;
        let count = edges.len().min(4096);
        let scale = count as f64 / (high - low);
        let band = |y: f64| (((y - low) * scale) as usize).min(count - 1);
        let mut starts = vec![0; count + 1];
        for [_, yi, _, yj] in &edges {
            for band in band(yi.min(*yj))..=band(yi.max(*yj)) {
                starts[band + 1] += 1;
            }
        }
        for band in 0..count {
            starts[band + 1] += starts[band];
        }
        let mut filled = starts.clone();
        let mut members = vec![0; starts[count]];
        for (edge, [_, yi, _, yj]) in edges.iter().enumerate() {
            for band in band(yi.min(*yj))..=band(yi.max(*yj)) {
                members[filled[band]] = edge as u32;
                filled[band] += 1;
            }
        }
        Self {
            edges,
            left: left - rounding,
            right: right + rounding,
            low,
            high,
            bands: Some((scale, starts, members)),
        }
    }

    /// Whether each point of `xs` and `ys` is inside the polygon.
    fn contain(&self, xs: &[f64], ys: &[f64]) -> BooleanBuffer {
        let toggles = |x: f64, y: f64, edge: &[f64; 4]| {
            let [xi, yi, xj, yj] = *edge;
            (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi
        };
        let Some((scale, starts, members)) = &self.bands else {
            return BooleanBuffer::collect_bool(xs.len(), |row| {
                let (x, y) = (xs[row], ys[row]);
                (self.edges.iter()).fold(false, |inside, edge| inside ^ toggles(x, y, edge))
            });
        };
        let last = starts.len() - 2;
        BooleanBuffer::collect_bool(xs.len(), |row| {
            let (x, y) = (xs[row], ys[row]);
            if !(y >= self.low && y < self.high && x >= self.left && x <= self.right) {
                return false;
            }
            let band = (((y - self.low) * scale) as usize).min(last);
            (members[starts[band]..starts[band + 1]].iter()).fold(false, |inside, &edge| {
                inside ^ toggles(x, y, &self.edges[edge as usize])
            })
        })
    }
}

/// Whether each value is from `start` forward to `end` in a dimension whose values wrap after
/// `period`: its difference from `start`, wrapped into one period, is at most the width of the
/// range, which crosses the wrap when `end` is less than `start`. A range at least one period wide
/// holds every value.
fn in_cyclic_range(column: &ArrayRef, start: f64, end: f64, period: f64) -> Result<BooleanArray> {
    let width = if end >= start {
        end - start
    } else {
        end - start + period
    };
    let values = floats(column)?;
    let values = values.values();
    let mut inside = BooleanBuffer::collect_bool(values.len(), |row| {
        width >= period || (values[row] - start).rem_euclid(period) <= width
    });
    if let Some(nulls) = column.logical_nulls() {
        inside = &inside & nulls.inner();
    }
    Ok(BooleanArray::new(inside, None))
}
