//! Selections at an evaluation: the contributions that the evaluator holds, the rows that a
//! predicate selects, and the domains that selections give scales.

use std::collections::{BTreeMap, HashSet};

use arrow::{
    array::{Array, ArrayRef, BooleanArray, Float64Array, RecordBatch, Scalar as ArrowScalar},
    compute::{
        cast, concat,
        kernels::cmp::{eq, gt_eq, lt, lt_eq},
        sort_to_indices, SortOptions,
    },
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use avenger_chart::{
    proto::{self, predicate, Channel},
    resolve::Identity,
    Chart, Contribution, TupleTest,
};

use crate::{evaluator::EvaluationWarning, EvalError};

type Result<T> = std::result::Result<T, EvalError>;

/// The contributions of an evaluator's selections, by name.
pub(crate) type Contributions = BTreeMap<String, Vec<Contribution>>;

/// The rows of a unit that predicates test.
pub(crate) struct Rows<'a> {
    pub(crate) table: &'a RecordBatch,
    /// The unit's partition values, its own and its ancestors', which every row holds.
    pub(crate) partition: &'a BTreeMap<String, ArrayRef>,
    /// The unit and the overlays that hold it, whose contributions `exclude_own` skips.
    pub(crate) owners: &'a [Identity],
    /// The path of the unit, which warnings name.
    pub(crate) path: &'a str,
}

/// The value of a column for each row: the rows of the table, or the partition value that every
/// row holds.
enum Values {
    Rows(ArrayRef),
    Constant(ArrayRef),
}

impl Rows<'_> {
    fn len(&self) -> usize {
        self.table.num_rows()
    }

    fn values(&self, column: &str) -> Option<Values> {
        if let Some(values) = self.table.column_by_name(column) {
            return Some(Values::Rows(values.clone()));
        }
        self.partition
            .get(column)
            .map(|value| Values::Constant(value.clone()))
    }
}

/// The predicates and the domains of selections over the contributions of an evaluator.
pub(crate) struct Selections<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) contributions: &'a Contributions,
}

impl Selections<'_> {
    fn resolution(&self, selection: &str) -> proto::SelectionResolution {
        self.chart
            .selections()
            .iter()
            .find(|candidate| candidate.name == selection)
            .and_then(|selection| proto::SelectionResolution::try_from(selection.resolution).ok())
            .unwrap_or(proto::SelectionResolution::Union)
    }

    /// Whether `predicate` selects each of `rows`.
    pub(crate) fn mask(
        &self,
        predicate: &proto::Predicate,
        rows: &Rows,
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<Vec<bool>> {
        let len = rows.len();
        Ok(match predicate.predicate.as_ref() {
            Some(predicate::Predicate::Selection(test)) => self.test(test, rows, warnings)?,
            Some(predicate::Predicate::And(list)) => {
                let mut mask = vec![true; len];
                for inner in &list.predicates {
                    and(&mut mask, &self.mask(inner, rows, warnings)?);
                }
                mask
            }
            Some(predicate::Predicate::Or(list)) => {
                let mut mask = vec![false; len];
                for inner in &list.predicates {
                    or(&mut mask, &self.mask(inner, rows, warnings)?);
                }
                mask
            }
            Some(predicate::Predicate::Not(inner)) => self
                .mask(inner, rows, warnings)?
                .into_iter()
                .map(|selected| !selected)
                .collect(),
            None => vec![false; len],
        })
    }

    /// Whether a selection selects each of `rows`, with its empty behavior and `exclude_own`.
    fn test(
        &self,
        test: &proto::SelectionTest,
        rows: &Rows,
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<Vec<bool>> {
        let len = rows.len();
        let contributions = self
            .contributions
            .get(&test.selection)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if contributions.is_empty() {
            return Ok(vec![!test.empty_selects_none; len]);
        }
        let kept: Vec<&Contribution> = contributions
            .iter()
            .filter(|contribution| {
                !(test.exclude_own && rows.owners.iter().any(|owner| owner.wrote(contribution)))
            })
            .collect();
        if kept.is_empty() {
            return Ok(vec![true; len]);
        }
        let intersect = self.resolution(&test.selection) == proto::SelectionResolution::Intersect;
        let mut mask = vec![intersect; len];
        for contribution in kept {
            let selected = contribution_mask(contribution, rows, &test.selection, warnings)?;
            if intersect {
                and(&mut mask, &selected);
            } else {
                or(&mut mask, &selected);
            }
        }
        Ok(mask)
    }

    /// The domain that `selection` gives `channel`: the envelope of its ranges through the
    /// channel, or the union of its sets in the order of their contributions and tuples, for a
    /// global or a union selection, and their intersection for an intersect selection. It is
    /// `None` when the selection gives no values or an empty intersection.
    pub(crate) fn domain(&self, selection: &str, channel: Channel) -> Result<Option<ArrayRef>> {
        let contributions = self
            .contributions
            .get(selection)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let intersect = self.resolution(selection) == proto::SelectionResolution::Intersect;
        let mut ranges: Vec<ArrayRef> = Vec::new();
        let mut sets: Vec<ArrayRef> = Vec::new();
        for contribution in contributions {
            let mut contribution_sets = Vec::new();
            for test in contribution.tuples.iter().flatten() {
                match test {
                    TupleTest::Range {
                        channel: through,
                        ends,
                        ..
                    } if *through == channel => ranges.push(ends.clone()),
                    TupleTest::Set {
                        channel: through,
                        values,
                        ..
                    } if *through == channel => contribution_sets.push(values.clone()),
                    _ => {}
                }
            }
            if !contribution_sets.is_empty() {
                let arrays: Vec<&dyn Array> =
                    contribution_sets.iter().map(|set| set.as_ref()).collect();
                sets.push(concat(&arrays)?);
            }
        }
        if !ranges.is_empty() {
            let starts: Vec<ArrayRef> = ranges.iter().map(|ends| ends.slice(0, 1)).collect();
            let ends: Vec<ArrayRef> = ranges.iter().map(|ends| ends.slice(1, 1)).collect();
            let (start, end) = if intersect {
                (extreme(&starts, true)?, extreme(&ends, false)?)
            } else {
                (extreme(&starts, false)?, extreme(&ends, true)?)
            };
            if intersect && lt(&end, &start)?.value(0) {
                return Ok(None);
            }
            return Ok(Some(concat(&[start.as_ref(), end.as_ref()])?));
        }
        let Some((first, rest)) = sets.split_first() else {
            return Ok(None);
        };
        let values = if intersect {
            let mut kept = distinct(first)?;
            for set in rest {
                let keys = keys(set)?;
                let candidates = keys_of(&kept)?;
                let indices: Vec<u32> = (0..kept.len())
                    .filter(|index| keys.contains(&candidates[*index]))
                    .map(|index| index as u32)
                    .collect();
                kept =
                    arrow::compute::take(&kept, &arrow::array::UInt32Array::from(indices), None)?;
            }
            kept
        } else {
            let arrays: Vec<&dyn Array> = sets.iter().map(|set| set.as_ref()).collect();
            distinct(&concat(&arrays)?)?
        };
        Ok((!values.is_empty()).then_some(values))
    }
}

/// The least of `values`, each one element, or the greatest when `greatest`.
fn extreme(values: &[ArrayRef], greatest: bool) -> Result<ArrayRef> {
    let arrays: Vec<&dyn Array> = values.iter().map(|value| value.as_ref()).collect();
    let all = concat(&arrays)?;
    let order = sort_to_indices(
        &all,
        Some(SortOptions {
            descending: greatest,
            nulls_first: false,
        }),
        None,
    )?;
    Ok(all.slice(order.value(0) as usize, 1))
}

/// The row key of each value of `values`, which is equal for equal values.
fn keys_of(values: &ArrayRef) -> Result<Vec<Vec<u8>>> {
    let converter = RowConverter::new(vec![SortField::new(values.data_type().clone())])?;
    let rows = converter.convert_columns(std::slice::from_ref(values))?;
    Ok(rows.iter().map(|row| row.as_ref().to_vec()).collect())
}

fn keys(values: &ArrayRef) -> Result<HashSet<Vec<u8>>> {
    Ok(keys_of(values)?.into_iter().collect())
}

/// The values of `values` without repeats, in the order of their first occurrence.
fn distinct(values: &ArrayRef) -> Result<ArrayRef> {
    let mut seen = HashSet::new();
    let indices: Vec<u32> = keys_of(values)?
        .into_iter()
        .enumerate()
        .filter(|(_, key)| seen.insert(key.clone()))
        .map(|(index, _)| index as u32)
        .collect();
    Ok(arrow::compute::take(
        values,
        &arrow::array::UInt32Array::from(indices),
        None,
    )?)
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

/// Whether `contribution` selects each of `rows`: when a row matches one of its tuples, and a
/// tuple when the row matches each of its tests.
fn contribution_mask(
    contribution: &Contribution,
    rows: &Rows,
    selection: &str,
    warnings: &mut Vec<EvaluationWarning>,
) -> Result<Vec<bool>> {
    let mut mask = vec![false; rows.len()];
    for tuple in &contribution.tuples {
        let mut matched = vec![true; rows.len()];
        for test in tuple {
            and(&mut matched, &test_mask(test, rows, selection, warnings)?);
        }
        or(&mut mask, &matched);
    }
    Ok(mask)
}

/// Whether each of `rows` matches `test`. A null matches no test, and a column that neither the
/// table nor the partition of the unit has matches no row, with a warning.
pub(crate) fn test_mask(
    test: &TupleTest,
    rows: &Rows,
    selection: &str,
    warnings: &mut Vec<EvaluationWarning>,
) -> Result<Vec<bool>> {
    let len = rows.len();
    let columns: Vec<&str> = match test {
        TupleTest::Set { column, .. }
        | TupleTest::Range { column, .. }
        | TupleTest::LongitudeRange { column, .. } => vec![column],
        TupleTest::Polygon {
            x_column, y_column, ..
        } => vec![x_column, y_column],
    };
    let mut values = Vec::new();
    for column in columns {
        match rows.values(column) {
            Some(column_values) => values.push(column_values),
            None => {
                warnings.push(EvaluationWarning {
                    path: rows.path.to_string(),
                    message: format!(
                        "the selection `{selection}` tests the column `{column}`, which the \
                         unit's table and partition do not have, so it selects no row"
                    ),
                });
                return Ok(vec![false; len]);
            }
        }
    }
    // A test of partition values gives one answer for every row.
    let constant = values
        .iter()
        .all(|values| matches!(values, Values::Constant(_)));
    let arrays: Vec<ArrayRef> = values
        .into_iter()
        .map(|values| match values {
            Values::Rows(array) => array,
            Values::Constant(value) if constant => value,
            Values::Constant(value) => {
                let indices = arrow::array::UInt32Array::from(vec![0; len]);
                arrow::compute::take(&value, &indices, None).expect("a value of one element")
            }
        })
        .collect();
    let mask = match test {
        TupleTest::Set { values, .. } => in_set(&arrays[0], values)?,
        TupleTest::Range {
            ends, exclude_end, ..
        } => in_range(&arrays[0], ends, *exclude_end)?,
        TupleTest::Polygon { x, y, .. } => in_polygon(&arrays[0], &arrays[1], x, y)?,
        TupleTest::LongitudeRange { west, east, .. } => in_longitudes(&arrays[0], *west, *east)?,
    };
    Ok(if constant {
        vec![mask.first().copied().unwrap_or(false); len]
    } else {
        mask
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
        let value = ArrowScalar::new(set.slice(index, 1));
        or(&mut mask, &selected(&eq(&column, &value)?));
    }
    Ok(mask)
}

fn in_range(column: &ArrayRef, ends: &ArrayRef, exclude_end: bool) -> Result<Vec<bool>> {
    let column = plain(column)?;
    let ends = like(ends, &column)?;
    let start = ArrowScalar::new(ends.slice(0, 1));
    let end = ArrowScalar::new(ends.slice(1, 1));
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

/// Whether each longitude is from `west` eastward to `east`, across the antimeridian when `west`
/// is larger than `east`.
fn in_longitudes(column: &ArrayRef, west: f64, east: f64) -> Result<Vec<bool>> {
    let wrap = |longitude: f64| (longitude + 180.0).rem_euclid(360.0) - 180.0;
    let (west, east) = (wrap(west), wrap(east));
    Ok(numbers(column)?
        .into_iter()
        .map(|longitude| {
            longitude.is_some_and(|longitude| {
                let longitude = wrap(longitude);
                if west <= east {
                    (west..=east).contains(&longitude)
                } else {
                    longitude >= west || longitude <= east
                }
            })
        })
        .collect())
}
