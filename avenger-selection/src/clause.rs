//! The clauses of a selection and their conditions, with the rows that each condition matches.

use std::collections::{BTreeMap, HashSet};

use arrow::{
    array::{Array, ArrayRef, BooleanArray, Float64Array, Scalar},
    compute::{
        cast,
        kernels::cmp::{gt_eq, lt, lt_eq},
    },
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use crate::{GridCells, Result, SelectionError};

/// The part of a selection that one owner, such as a plot, wrote: conditions that a row matches
/// when it matches all of them.
#[must_use]
#[derive(Clone, Debug)]
pub struct Clause {
    /// The id of the unit or the overlay that wrote it.
    pub owner: String,
    /// The partition values of the owner, its own and those of its ancestors, by column.
    pub partition: BTreeMap<String, avenger_scales::scalar::Scalar>,
    /// The conditions, all of which a row matches.
    pub conditions: Vec<Condition>,
}

impl Clause {
    /// A clause of the unit or the overlay with the id `owner`.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            partition: BTreeMap::new(),
            conditions: Vec::new(),
        }
    }

    /// A partition value of the owner: its own values and those of its ancestors name it, with
    /// its id.
    pub fn partition(
        mut self,
        column: impl Into<String>,
        value: impl Into<avenger_scales::scalar::Scalar>,
    ) -> Self {
        self.partition.insert(column.into(), value.into());
        self
    }

    /// A condition, which a row matches together with the clause's other conditions.
    pub fn condition(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    /// Conditions, which a row matches together with the clause's other conditions.
    pub fn conditions(mut self, conditions: impl IntoIterator<Item = Condition>) -> Self {
        self.conditions.extend(conditions);
        self
    }
}

/// A check of one or more columns of a row. A condition is symbolic: it holds the columns and the
/// values, ends, cells, or polygons that a row's values fit, and [`Condition::matches`] evaluates
/// it over Arrow columns. Each column records the label of the channel that the selection
/// projected through, such as `x`, or `None`.
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
    /// or with a `grid`, when its value lies in one of the grid's cells. A brush that selects
    /// whole cells writes the grid of its plot, and its `ends` are the edges of the box that it
    /// drew.
    Range {
        column: String,
        channel: Option<String>,
        ends: ArrayRef,
        exclude_end: bool,
        grid: Option<GridCells>,
    },
    /// A row matches when the point of its values of `x_column` and `y_column` is inside one of
    /// `polygons`, each the vertices `x` and `y` in order around it.
    Polygon {
        x_column: String,
        y_column: String,
        polygons: Vec<(ArrayRef, ArrayRef)>,
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
    /// [`columns`](Self::columns) for the rows, in their order. A null matches no condition, and
    /// a dictionary column compares its values.
    ///
    /// - A set matches the rows whose values equal one of its tuples, with its values cast to
    ///   the columns' types. A tuple with a null matches no row.
    /// - A range matches the values from its first end through its second, or to before its
    ///   second with `exclude_end`, and with a grid, the values in the cells of the grid that it
    ///   holds.
    /// - A polygon condition matches the points inside one of its polygons, each by the even-odd
    ///   rule, through the numbers of the two columns.
    /// - A cyclic range matches the values whose difference from its start, wrapped into one
    ///   period, is at most its width, and a range at least one period wide matches every value.
    pub fn matches(&self, columns: &[ArrayRef]) -> Result<Vec<bool>> {
        match self {
            Condition::Set { values, .. } => in_set(columns, values),
            Condition::Range {
                grid: Some(grid), ..
            } => grid.contains(&plain(&columns[0])?),
            Condition::Range {
                ends, exclude_end, ..
            } => in_range(&columns[0], ends, *exclude_end),
            Condition::Polygon { polygons, .. } => {
                let mut mask = vec![false; columns[0].len()];
                for (x, y) in polygons {
                    or(&mut mask, &in_polygon(&columns[0], &columns[1], x, y)?);
                }
                Ok(mask)
            }
            Condition::CyclicRange {
                start, end, period, ..
            } => in_cyclic_range(&columns[0], *start, *end, *period),
        }
    }
}

pub(crate) fn and(mask: &mut [bool], other: &[bool]) {
    for (selected, other) in mask.iter_mut().zip(other) {
        *selected &= *other;
    }
}

pub(crate) fn or(mask: &mut [bool], other: &[bool]) {
    for (selected, other) in mask.iter_mut().zip(other) {
        *selected |= *other;
    }
}

/// `values` as their dictionary's values when they are a dictionary, so they compare with plain
/// values.
fn plain(values: &ArrayRef) -> Result<ArrayRef> {
    Ok(match values.data_type() {
        DataType::Dictionary(_, value_type) => cast(values, value_type)?,
        _ => values.clone(),
    })
}

/// `values` in the type of `like`.
fn like(values: &ArrayRef, like: &ArrayRef) -> Result<ArrayRef> {
    let values = plain(values)?;
    Ok(if values.data_type() == like.data_type() {
        values
    } else {
        cast(&values, like.data_type())?
    })
}

fn selected(mask: &BooleanArray) -> Vec<bool> {
    (0..mask.len())
        .map(|index| mask.is_valid(index) && mask.value(index))
        .collect()
}

/// Whether each row of `columns` equals one tuple of `values`, through the rows of Arrow's row
/// format, so the time is linear in the rows and the tuples.
fn in_set(columns: &[ArrayRef], values: &[ArrayRef]) -> Result<Vec<bool>> {
    let columns = columns.iter().map(plain).collect::<Result<Vec<_>>>()?;
    let values = (values.iter().zip(&columns))
        .map(|(values, column)| like(values, column))
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

fn in_range(column: &ArrayRef, ends: &ArrayRef, exclude_end: bool) -> Result<Vec<bool>> {
    let column = plain(column)?;
    let ends = like(ends, &column)?;
    let start = Scalar::new(ends.slice(0, 1));
    let end = Scalar::new(ends.slice(1, 1));
    let mut mask = selected(&gt_eq(&column, &start)?);
    let below = if exclude_end {
        lt(&column, &end)?
    } else {
        lt_eq(&column, &end)?
    };
    and(&mut mask, &selected(&below));
    Ok(mask)
}

fn numbers(values: &ArrayRef) -> Result<Vec<Option<f64>>> {
    let numbers = cast(&plain(values)?, &DataType::Float64)?;
    Ok(numbers
        .as_any()
        .downcast_ref::<Float64Array>()
        .expect("a Float64 array")
        .iter()
        .collect())
}

/// Whether each point of `x` and `y` is inside the polygon of the vertices `vertex_x` and
/// `vertex_y`, by the even-odd rule.
fn in_polygon(
    x: &ArrayRef,
    y: &ArrayRef,
    vertex_x: &ArrayRef,
    vertex_y: &ArrayRef,
) -> Result<Vec<bool>> {
    let vertices: Vec<(f64, f64)> = numbers(vertex_x)?
        .into_iter()
        .zip(numbers(vertex_y)?)
        .filter_map(|(x, y)| Some((x?, y?)))
        .collect();
    Ok(numbers(x)?
        .into_iter()
        .zip(numbers(y)?)
        .map(|point| match point {
            (Some(x), Some(y)) => {
                let mut inside = false;
                let mut previous = vertices.len().wrapping_sub(1);
                for (index, &(xi, yi)) in vertices.iter().enumerate() {
                    let (xj, yj) = vertices[previous];
                    if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    previous = index;
                }
                inside
            }
            _ => false,
        })
        .collect())
}

/// Whether each value is from `start` forward to `end` in a dimension whose values wrap after
/// `period`: its difference from `start`, wrapped into one period, is at most the width of the
/// range, which crosses the wrap when `end` is less than `start`. A range at least one period wide
/// holds every value.
fn in_cyclic_range(column: &ArrayRef, start: f64, end: f64, period: f64) -> Result<Vec<bool>> {
    let width = if end >= start {
        end - start
    } else {
        end - start + period
    };
    Ok(numbers(column)?
        .into_iter()
        .map(|value| {
            value
                .is_some_and(|value| width >= period || (value - start).rem_euclid(period) <= width)
        })
        .collect())
}
