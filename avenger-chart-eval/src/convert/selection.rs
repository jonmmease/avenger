//! The selections of a chart as avenger-selection models: the state of each selection from its
//! initial clauses, the predicates of conditional values, and the values that a selection gives a
//! selection domain.

use std::collections::{BTreeMap, HashSet};

use arrow::{
    array::{new_null_array, Array, ArrayRef, AsArray},
    compute::{concat, kernels::cmp::lt, sort_to_indices, take, SortOptions},
    row::{RowConverter, SortField},
};
use avenger_chart::{
    proto::{self, condition, predicate, Channel, SelectionResolution},
    resolve::Identity,
    Chart,
};
use avenger_scales::scalar::Scalar;
use avenger_selection::{
    Clause, Condition, Predicate, Resolution, SelectedOptions, SelectionState,
};

use crate::EvalError;

type Result<T> = std::result::Result<T, EvalError>;

/// The state of each selection of `chart` by name, with its initial clauses.
pub(crate) fn states(chart: &Chart) -> Result<BTreeMap<String, SelectionState>> {
    chart
        .selections()
        .iter()
        .map(|selection| {
            let resolution = match SelectionResolution::try_from(selection.resolution) {
                Ok(SelectionResolution::Union) => Resolution::Union,
                Ok(SelectionResolution::Intersect) => Resolution::Intersect,
                _ => Resolution::Global,
            };
            let clauses = (selection.initial.iter())
                .map(|initial| clause(chart, initial))
                .collect::<Result<Vec<_>>>()?;
            Ok((
                selection.name.clone(),
                SelectionState::new(resolution, clauses),
            ))
        })
        .collect()
}

/// A clause of the format as avenger-selection holds it, owned by the name that
/// [`Identity::owner_name`] gives it.
pub(crate) fn clause(chart: &Chart, clause: &proto::Clause) -> Result<Clause> {
    let mut converted = Clause::new(Identity::owner_name(&clause.widget, &clause.owner));
    for value in &clause.partition {
        converted = converted.partition(
            value.column.clone(),
            Scalar::new(chart.partition_value(value).clone()),
        );
    }
    let conditions = (clause.conditions.iter())
        .map(|condition| self::condition(chart, condition))
        .collect::<Result<Vec<_>>>()?;
    Ok(converted.conditions(conditions))
}

/// The label of the channel that a condition projected through, or `None` for a column that the
/// selection names directly.
fn label(channel: i32) -> Option<String> {
    match Channel::try_from(channel).unwrap_or_default() {
        Channel::Unspecified => None,
        channel => Some(channel.field_name().to_string()),
    }
}

fn condition(chart: &Chart, condition: &proto::Condition) -> Result<Condition> {
    let kind = condition
        .kind
        .as_ref()
        .expect("a valid condition has a kind");
    Ok(match kind {
        condition::Kind::Set(set) => Condition::set(
            set.columns
                .iter()
                .map(|column| column.column.clone())
                .collect(),
            set.columns
                .iter()
                .map(|column| label(column.channel))
                .collect(),
            (set.columns.iter())
                .map(|column| {
                    chart.list_elements(column.values.as_ref().expect("a set has its values"))
                })
                .collect(),
        )?,
        condition::Kind::Range(range) => {
            let low = range.low.as_ref().map(|low| chart.value(low).clone());
            let high = range.high.as_ref().map(|high| chart.value(high).clone());
            // An absent end is null, which avenger-selection does not bound.
            let data_type = (low.as_ref().or(high.as_ref()))
                .expect("a valid range has an end")
                .data_type()
                .clone();
            let end =
                |value: Option<ArrayRef>| value.unwrap_or_else(|| new_null_array(&data_type, 1));
            let (low, high) = (end(low), end(high));
            Condition::Range {
                column: range.column.clone(),
                channel: label(range.channel),
                ends: concat(&[low.as_ref(), high.as_ref()])?,
                exclude_end: range.exclude_end,
                cells: None,
            }
        }
        condition::Kind::Polygon(polygon) => Condition::Polygon {
            x_column: polygon.x_column.clone(),
            y_column: polygon.y_column.clone(),
            polygons: (polygon.vertices.iter())
                .map(|vertices| {
                    let points = chart.list_elements(vertices);
                    let points = points.as_struct();
                    (points.column(0).clone(), points.column(1).clone())
                })
                .collect(),
            cells: None,
        },
        condition::Kind::CyclicRange(range) => Condition::CyclicRange {
            column: range.column.clone(),
            channel: label(range.channel),
            start: range.start,
            end: range.end,
            period: range.period,
        },
    })
}

/// A predicate of the format for the rows of a layer whose owners, the layer and the plot that
/// draw them, are `owners`, faceted when one of them has a partition.
pub(crate) fn predicate(
    predicate: &proto::Predicate,
    owners: &[String],
    faceted: bool,
) -> Predicate {
    use predicate::Predicate as Kind;
    match predicate
        .predicate
        .as_ref()
        .expect("a valid predicate has a kind")
    {
        Kind::Selected(selected) => Predicate::selected(
            selected.selection.clone(),
            SelectedOptions {
                exclude_own: match selected.exclude_own {
                    true => owners.to_vec(),
                    false => Vec::new(),
                },
                faceted,
                empty_selects_none: selected.empty_selects_none,
                without: Vec::new(),
            },
        ),
        Kind::All(list) => Predicate::And(
            (list.predicates.iter())
                .map(|inner| self::predicate(inner, owners, faceted))
                .collect(),
        ),
        Kind::Any(list) => Predicate::Or(
            (list.predicates.iter())
                .map(|inner| self::predicate(inner, owners, faceted))
                .collect(),
        ),
        Kind::Negate(inner) => Predicate::Not(Box::new(self::predicate(inner, owners, faceted))),
    }
}

/// The values that a selection gives the selection domain of one channel.
#[derive(Clone, Debug)]
pub(crate) enum SelectionValues {
    /// The envelope of the ranges of a global or a union selection, or their intersection for an
    /// intersect selection: its low end and its high end, or `None` for an end that no range
    /// bounds, which the domain takes from its fallback.
    Range {
        low: Option<ArrayRef>,
        high: Option<ArrayRef>,
    },
    /// The union of the sets in the order of their clauses, or their intersection.
    Set(ArrayRef),
}

/// The values of the clauses of `state` for `channel`, or `None` when its clauses give none, or
/// an intersect selection's ranges or sets do not meet.
pub(crate) fn domain_values(
    state: &SelectionState,
    channel: Channel,
) -> Result<Option<SelectionValues>> {
    let label = Some(channel.field_name().to_string());
    let intersect = state.resolution == Resolution::Intersect;
    let mut ranges: Vec<ArrayRef> = Vec::new();
    let mut sets: Vec<ArrayRef> = Vec::new();
    for clause in &state.clauses {
        let mut clause_sets = Vec::new();
        for condition in &clause.conditions {
            match condition {
                Condition::Range { channel, ends, .. } if *channel == label => {
                    ranges.push(ends.clone())
                }
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
        let lows: Vec<ArrayRef> = ranges.iter().map(|ends| ends.slice(0, 1)).collect();
        let highs: Vec<ArrayRef> = ranges.iter().map(|ends| ends.slice(1, 1)).collect();
        // A union's end is open when one range leaves it open, and an intersection's when every
        // range does.
        let end = |ends: &[ArrayRef], greatest: bool| -> Result<Option<ArrayRef>> {
            let open = ends.iter().filter(|end| end.is_null(0)).count();
            let unbounded = match intersect {
                true => open == ends.len(),
                false => open > 0,
            };
            if unbounded {
                return Ok(None);
            }
            let bounded: Vec<ArrayRef> =
                ends.iter().filter(|end| end.is_valid(0)).cloned().collect();
            Ok(Some(extreme(&bounded, greatest)?))
        };
        let (low, high) = match intersect {
            true => (end(&lows, true)?, end(&highs, false)?),
            false => (end(&lows, false)?, end(&highs, true)?),
        };
        if let (true, Some(low), Some(high)) = (intersect, &low, &high) {
            if lt(high, low)?.value(0) {
                return Ok(None);
            }
        }
        return Ok(Some(SelectionValues::Range { low, high }));
    }
    let Some((first, rest)) = sets.split_first() else {
        return Ok(None);
    };
    let values = match intersect {
        true => {
            let mut kept = distinct(first)?;
            for set in rest {
                let keys: HashSet<Vec<u8>> = keys_of(set)?.into_iter().collect();
                let candidates = keys_of(&kept)?;
                let indices: Vec<u32> = (0..kept.len())
                    .filter(|index| keys.contains(&candidates[*index]))
                    .map(|index| index as u32)
                    .collect();
                kept = take(&kept, &arrow::array::UInt32Array::from(indices), None)?;
            }
            kept
        }
        false => {
            let arrays: Vec<&dyn Array> = sets.iter().map(|set| set.as_ref()).collect();
            distinct(&concat(&arrays)?)?
        }
    };
    Ok((!values.is_empty()).then_some(SelectionValues::Set(values)))
}

/// The least or the greatest of one-element `values`.
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

fn keys_of(values: &ArrayRef) -> Result<Vec<Vec<u8>>> {
    let converter = RowConverter::new(vec![SortField::new(values.data_type().clone())])?;
    let rows = converter.convert_columns(std::slice::from_ref(values))?;
    Ok(rows.iter().map(|row| row.as_ref().to_vec()).collect())
}

/// `values` without repeats, in the order of their first appearance.
fn distinct(values: &ArrayRef) -> Result<ArrayRef> {
    let mut seen = HashSet::new();
    let indices: Vec<u32> = keys_of(values)?
        .into_iter()
        .enumerate()
        .filter(|(_, key)| seen.insert(key.clone()))
        .map(|(index, _)| index as u32)
        .collect();
    Ok(take(
        values,
        &arrow::array::UInt32Array::from(indices),
        None,
    )?)
}
