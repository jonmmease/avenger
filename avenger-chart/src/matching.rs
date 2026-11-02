//! Which rows the selections of a chart select: the rules of each test of a contribution, and the
//! predicate of a plot over the contributions of its selections as a [`Matching`], whose leaves
//! are the tests. The renderer evaluates a matching to masks, and a query can translate it to a
//! filter, so both select the same rows.

use std::collections::BTreeMap;

use arrow::{
    array::{Array, ArrayRef, BooleanArray, Float64Array, Scalar},
    compute::{
        cast,
        kernels::cmp::{eq, gt_eq, lt, lt_eq},
    },
    datatypes::DataType,
};

use crate::{
    proto::{self, predicate},
    Chart, ChartError, Contribution, TupleTest,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The rows that a predicate of a plot selects, over the contributions of its selections, as a
/// tree whose leaves are the tests of the contributions.
#[derive(Clone, Debug)]
pub enum Matching<'a> {
    /// Every row.
    All,
    /// No row.
    Nothing,
    /// The rows that every part matches.
    Every(Vec<Matching<'a>>),
    /// The rows that some part matches.
    Any(Vec<Matching<'a>>),
    /// The rows that the part does not match.
    Not(Box<Matching<'a>>),
    /// The rows that match a test of a contribution to `selection`.
    Test {
        selection: &'a str,
        test: &'a TupleTest,
    },
}

impl Chart {
    /// The rows that `predicate` selects in a plot, over `contributions`, the contributions of the
    /// chart's selections by name. `owned` says whether the plot wrote a contribution, which a test
    /// with `exclude_own` skips: a contribution of the plot's unit or of an overlay that holds it.
    ///
    /// A selection without contributions selects every row, or none with `empty_selects_none`. A
    /// selection whose contributions the test skips, all of them, selects every row. Otherwise a
    /// row matches an intersect selection when it matches the contribution of every owner, and a
    /// global or a union selection when it matches the contribution of some owner. A row matches
    /// a contribution when it matches one of its tuples, and a tuple when it matches every test of
    /// the tuple. A predicate without a kind selects no row.
    pub fn matching<'a>(
        &self,
        predicate: &proto::Predicate,
        contributions: &'a BTreeMap<String, Vec<Contribution>>,
        owned: &dyn Fn(&Contribution) -> bool,
    ) -> Matching<'a> {
        let parts = |list: &proto::PredicateList| -> Vec<Matching<'a>> {
            (list.predicates.iter())
                .map(|inner| self.matching(inner, contributions, owned))
                .collect()
        };
        match predicate.predicate.as_ref() {
            Some(predicate::Predicate::Selection(test)) => {
                self.selection_matching(test, contributions, owned)
            }
            Some(predicate::Predicate::And(list)) => Matching::Every(parts(list)),
            Some(predicate::Predicate::Or(list)) => Matching::Any(parts(list)),
            Some(predicate::Predicate::Not(inner)) => {
                Matching::Not(Box::new(self.matching(inner, contributions, owned)))
            }
            None => Matching::Nothing,
        }
    }

    /// The rows that the selection of `test` selects, as [`Chart::matching`] describes.
    fn selection_matching<'a>(
        &self,
        test: &proto::SelectionTest,
        contributions: &'a BTreeMap<String, Vec<Contribution>>,
        owned: &dyn Fn(&Contribution) -> bool,
    ) -> Matching<'a> {
        let Some((selection, contributions)) = contributions
            .get_key_value(&test.selection)
            .filter(|(_, contributions)| !contributions.is_empty())
        else {
            return if test.empty_selects_none {
                Matching::Nothing
            } else {
                Matching::All
            };
        };
        let kept: Vec<Matching<'a>> = (contributions.iter())
            .filter(|contribution| !(test.exclude_own && owned(contribution)))
            .map(|contribution| {
                Matching::Any(
                    (contribution.tuples.iter())
                        .map(|tuple| {
                            Matching::Every(
                                (tuple.iter())
                                    .map(|test| Matching::Test { selection, test })
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        if kept.is_empty() {
            return Matching::All;
        }
        let resolution = (self.selections().iter())
            .find(|candidate| candidate.name == *selection)
            .and_then(|selection| proto::SelectionResolution::try_from(selection.resolution).ok());
        if resolution == Some(proto::SelectionResolution::Intersect) {
            Matching::Every(kept)
        } else {
            Matching::Any(kept)
        }
    }
}

impl Matching<'_> {
    /// Whether each of `len` rows matches, with `test` giving whether each row matches a test of
    /// a contribution to a selection, such as by [`TupleTest::matches`] over the rows' columns.
    pub fn mask<E, F>(&self, len: usize, test: &mut F) -> std::result::Result<Vec<bool>, E>
    where
        F: FnMut(&str, &TupleTest) -> std::result::Result<Vec<bool>, E>,
    {
        Ok(match self {
            Matching::All => vec![true; len],
            Matching::Nothing => vec![false; len],
            Matching::Every(parts) => {
                let mut mask = vec![true; len];
                for part in parts {
                    and(&mut mask, &part.mask(len, test)?);
                }
                mask
            }
            Matching::Any(parts) => {
                let mut mask = vec![false; len];
                for part in parts {
                    or(&mut mask, &part.mask(len, test)?);
                }
                mask
            }
            Matching::Not(part) => (part.mask(len, test)?.into_iter())
                .map(|matched| !matched)
                .collect(),
            Matching::Test {
                selection,
                test: tuple_test,
            } => test(selection, tuple_test)?,
        })
    }
}

impl TupleTest {
    /// The columns that the test reads: its column, or the two columns of a polygon.
    pub fn columns(&self) -> Vec<&str> {
        match self {
            TupleTest::Set { column, .. }
            | TupleTest::Range { column, .. }
            | TupleTest::CyclicRange { column, .. } => vec![column],
            TupleTest::Polygon {
                x_column, y_column, ..
            } => vec![x_column, y_column],
        }
    }

    /// Whether each row matches the test, from the values of the test's [`columns`](Self::columns)
    /// for the rows, in their order. A null matches no test, and a dictionary column compares its
    /// values.
    ///
    /// - A set matches the values equal to one of its values, which are cast to the column's type.
    /// - A range matches the values from its first end through its second, or to before its
    ///   second with `exclude_end`, and with a grid, the values in the cells of the grid that it
    ///   holds.
    /// - A polygon matches the points inside it by the even-odd rule, through the numbers of the
    ///   two columns.
    /// - A cyclic range matches the values whose difference from its start, wrapped into one
    ///   period, is at most its width, and a range at least one period wide matches every value.
    pub fn matches(&self, columns: &[ArrayRef]) -> Result<Vec<bool>> {
        match self {
            TupleTest::Set { values, .. } => in_set(&columns[0], values),
            TupleTest::Range {
                grid: Some(grid), ..
            } => grid.contains(&plain(&columns[0])?),
            TupleTest::Range {
                ends, exclude_end, ..
            } => in_range(&columns[0], ends, *exclude_end),
            TupleTest::Polygon { x, y, .. } => in_polygon(&columns[0], &columns[1], x, y),
            TupleTest::CyclicRange {
                start, end, period, ..
            } => in_cyclic_range(&columns[0], *start, *end, *period),
        }
    }
}

fn and(mask: &mut [bool], other: &[bool]) {
    for (selected, other) in mask.iter_mut().zip(other) {
        *selected &= *other;
    }
}

fn or(mask: &mut [bool], other: &[bool]) {
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

fn in_set(column: &ArrayRef, set: &ArrayRef) -> Result<Vec<bool>> {
    let column = plain(column)?;
    let set = like(set, &column)?;
    let mut mask = vec![false; column.len()];
    for index in 0..set.len() {
        if set.is_null(index) {
            continue;
        }
        let value = Scalar::new(set.slice(index, 1));
        or(&mut mask, &selected(&eq(&column, &value)?));
    }
    Ok(mask)
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
