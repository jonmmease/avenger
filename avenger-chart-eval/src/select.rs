//! Selections at an evaluation: the clauses that the evaluator holds, the rows that a
//! predicate selects, and the domains that selections give scales.

use std::collections::{BTreeMap, HashSet};

use arrow::{
    array::{new_null_array, Array, ArrayRef, RecordBatch, UInt32Array},
    compute::{concat, kernels::cmp::lt, sort_to_indices, take, SortOptions},
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use avenger_chart::{
    proto::{self, Channel},
    resolve::Identity,
    Chart, ChartError, Clause, Condition,
};

use crate::{evaluator::EvaluationWarning, EvalError};

type Result<T> = std::result::Result<T, EvalError>;

/// The clauses of an evaluator's selections, by name.
pub(crate) type Clauses = BTreeMap<String, Vec<Clause>>;

/// The rows of a unit that predicates check.
pub(crate) struct Rows<'a> {
    pub(crate) table: &'a RecordBatch,
    /// The unit's partition values, its own and its ancestors', which every row holds.
    pub(crate) partition: &'a BTreeMap<String, ArrayRef>,
    /// The unit and the overlays that hold it, whose clauses `exclude_own` skips.
    pub(crate) owners: &'a [Identity],
    /// The path of the unit, which warnings name.
    pub(crate) path: &'a str,
}

impl Rows<'_> {
    fn len(&self) -> usize {
        self.table.num_rows()
    }

    /// The type of the column `column` of the table or the partition.
    fn data_type(&self, column: &str) -> Option<DataType> {
        let schema = self.table.schema();
        (schema.field_with_name(column).ok())
            .map(|field| field.data_type().clone())
            .or_else(|| {
                self.partition
                    .get(column)
                    .map(|value| value.data_type().clone())
            })
    }

    /// The value of the column `column` for each row: the table's column, or the partition value
    /// that every row holds.
    fn column(&self, column: &str) -> Option<ArrayRef> {
        if let Some(values) = self.table.column_by_name(column) {
            return Some(values.clone());
        }
        let value = self.partition.get(column)?;
        take(value, &UInt32Array::from(vec![0; self.len()]), None).ok()
    }
}

/// The predicates and the domains of selections over the clauses of an evaluator.
pub(crate) struct Selections<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) clauses: &'a Clauses,
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

    /// Whether `predicate` selects each of `rows`, through the test of the chart's
    /// [`predicate`](Chart::predicate) for the rows' owners: a `Selected` with `exclude_own` skips
    /// the clauses of the unit and the overlays that hold it, only those of the unit's panel when
    /// the owners have partition values. A condition of a column that neither the table nor the
    /// partition of the unit has matches no row, with a warning.
    pub(crate) fn mask(
        &self,
        predicate: &proto::Predicate,
        rows: &Rows,
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<Vec<bool>> {
        let owners: Vec<String> = rows.owners.iter().map(|owner| owner.id.clone()).collect();
        let faceted = (rows.owners.iter()).any(|owner| !owner.partition.is_empty());
        let states = self.chart.selection_states(self.clauses);
        let missing = self.missing(predicate, rows, warnings);
        let types = |column: &str| {
            rows.data_type(column)
                .or_else(|| missing.get(column).cloned())
        };
        let test = (self.chart.predicate(predicate, &owners, faceted))
            .prepare(&states, types)
            .map_err(ChartError::from)?;
        let len = rows.len();
        let columns = |column: &str| {
            (rows.column(column))
                .or_else(|| (missing.get(column)).map(|data_type| new_null_array(data_type, len)))
        };
        let mask = (test.mask(len, columns, |column: &str| rows.column(column)))
            .map_err(ChartError::from)?;
        Ok(mask.values().iter().collect())
    }

    /// The type of each column that a condition of a selection of `predicate` checks and that
    /// neither the table nor the partition of `rows` has, the type of the condition's values, with
    /// a warning for each, so the condition matches no row.
    fn missing(
        &self,
        predicate: &proto::Predicate,
        rows: &Rows,
        warnings: &mut Vec<EvaluationWarning>,
    ) -> BTreeMap<String, DataType> {
        let mut missing = BTreeMap::new();
        for selection in selections(predicate) {
            let clauses = self.clauses.get(selection).map(Vec::as_slice);
            for condition in clauses
                .unwrap_or_default()
                .iter()
                .flat_map(|c| &c.conditions)
            {
                for (column, data_type) in value_types(condition) {
                    if rows.data_type(column).is_some() || missing.contains_key(column) {
                        continue;
                    }
                    warnings.push(EvaluationWarning {
                        path: rows.path.to_string(),
                        message: format!(
                            "the selection `{selection}` checks the column `{column}`, which the \
                             unit's table and partition do not have, so it selects no row"
                        ),
                    });
                    missing.insert(column.to_string(), data_type);
                }
            }
        }
        missing
    }

    /// The domain that `selection` gives `channel`: the envelope of its ranges through the
    /// channel, or the union of its sets' values through the channel in the order of their
    /// clauses, for a global or a union selection, and their intersection for an intersect
    /// selection. It is `None` when the selection gives no values or an empty intersection.
    pub(crate) fn domain(&self, selection: &str, channel: Channel) -> Result<Option<ArrayRef>> {
        let clauses = self
            .clauses
            .get(selection)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let intersect = self.resolution(selection) == proto::SelectionResolution::Intersect;
        let label = channel.label();
        let mut ranges: Vec<ArrayRef> = Vec::new();
        let mut sets: Vec<ArrayRef> = Vec::new();
        for clause in clauses {
            let mut clause_sets = Vec::new();
            for condition in &clause.conditions {
                match condition {
                    Condition::Range {
                        channel: through,
                        ends,
                        ..
                    } if *through == label => ranges.push(ends.clone()),
                    // The values of a set's column that goes through the channel.
                    Condition::Set {
                        channels, values, ..
                    } => {
                        if let Some(index) = channels.iter().position(|through| *through == label) {
                            clause_sets.push(values[index].clone());
                        }
                    }
                    _ => {}
                }
            }
            if !clause_sets.is_empty() {
                let arrays: Vec<&dyn Array> = clause_sets.iter().map(|set| set.as_ref()).collect();
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

/// The names of the selections that `predicate` tests.
fn selections(predicate: &proto::Predicate) -> Vec<&str> {
    use proto::predicate::Predicate;
    match predicate.predicate.as_ref() {
        Some(Predicate::Selected(selected)) => vec![selected.selection.as_str()],
        Some(Predicate::And(list) | Predicate::Or(list)) => {
            list.predicates.iter().flat_map(selections).collect()
        }
        Some(Predicate::Not(inner)) => selections(inner),
        None => Vec::new(),
    }
}

/// Each column that `condition` checks, with the type of the values that it compares the column
/// with.
fn value_types(condition: &Condition) -> Vec<(&str, DataType)> {
    match condition {
        Condition::Set {
            columns, values, ..
        } => (columns.iter())
            .zip(values)
            .map(|(column, values)| (column.as_str(), values.data_type().clone()))
            .collect(),
        Condition::Range { column, ends, .. } => vec![(column.as_str(), ends.data_type().clone())],
        Condition::Polygon {
            x_column,
            y_column,
            polygons,
            ..
        } => {
            let type_of = |values: Option<&ArrayRef>| {
                values.map_or(DataType::Float64, |values| values.data_type().clone())
            };
            let first = polygons.first();
            vec![
                (x_column.as_str(), type_of(first.map(|(x, _)| x))),
                (y_column.as_str(), type_of(first.map(|(_, y)| y))),
            ]
        }
        Condition::CyclicRange { column, .. } => vec![(column.as_str(), DataType::Float64)],
    }
}

/// Whether each of `rows` matches `condition`. A null matches no condition, and a column that
/// neither the table nor the partition of the unit has matches no row, with a warning.
pub(crate) fn condition_mask(
    condition: &Condition,
    rows: &Rows,
    selection: &str,
    warnings: &mut Vec<EvaluationWarning>,
) -> Result<Vec<bool>> {
    let mut columns = Vec::new();
    for column in condition.columns() {
        match rows.column(column) {
            Some(values) => columns.push(values),
            None => {
                warnings.push(EvaluationWarning {
                    path: rows.path.to_string(),
                    message: format!(
                        "the selection `{selection}` checks the column `{column}`, which the \
                         unit's table and partition do not have, so it selects no row"
                    ),
                });
                return Ok(vec![false; rows.len()]);
            }
        }
    }
    let mask = condition.matches(&columns).map_err(ChartError::from)?;
    Ok(mask.iter().map(|matched| matched == Some(true)).collect())
}
