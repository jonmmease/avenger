//! The categorical scales of columns of lumps: their keys, the order of their kept categories, and
//! their Other group, which comes last.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, UInt32Array},
    compute::{concat, take},
    datatypes::{DataType, Float64Type, Int64Type},
};
use avenger_chart::lump;
use avenger_format::PrecisionSharing;
use avenger_guides::label_variant::LabelVariant;
use avenger_scales::categories::{distinct, CategoryIndex};
use avenger_typst_label::{LabelEngine, LabelError, LabelValue, LabelValues};

use crate::{fit::MemberValues, EvalError};

type Result<T> = std::result::Result<T, EvalError>;

/// One value of every row of a key, or several.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Agreed<T> {
    One(T),
    Several,
}

impl<T: PartialEq> Agreed<T> {
    fn with(self, value: T) -> Self {
        match self {
            Agreed::One(known) if known == value => Agreed::One(known),
            _ => Agreed::Several,
        }
    }
}

/// `agreed` with `value`, one more row of its key.
fn agree<T: PartialEq>(agreed: &mut Option<Agreed<T>>, value: T) {
    *agreed = Some(match agreed.take() {
        Some(known) => known.with(value),
        None => Agreed::One(value),
    });
}

/// What the members of a scale group of lumps hold beyond their keys: the weight and the number
/// of categories of each key, and the keys of the Other group.
#[derive(Clone, Debug)]
pub(crate) struct Lumps {
    /// The type of the keys.
    key_type: DataType,
    /// The distinct keys of the rows.
    keys: ArrayRef,
    /// The positions of `keys`, which the other fields hold a value at for each key.
    index: CategoryIndex,
    /// The weight of each key: the greatest of its rows' weights, if any has one.
    weights: Vec<Option<f64>>,
    /// The weight of each key's rows, when they agree.
    weight: Vec<Option<Agreed<f64>>>,
    /// The number of categories of each key's rows, when they agree.
    count: Vec<Option<Agreed<i64>>>,
    /// The keys of the Other group among the rows, one for each label.
    others: Option<ArrayRef>,
}

impl Lumps {
    /// The lumps of a group whose members hold `values`, or `None` when the members hold no
    /// lumps.
    pub(crate) fn of(values: &[Option<MemberValues>]) -> Result<Option<Self>> {
        let Some(key_type) =
            (values.iter().flatten()).find_map(|(values, _)| lump::key_type(values.data_type()))
        else {
            return Ok(None);
        };
        let columns: Vec<(&ArrayRef, ArrayRef)> = (values.iter().flatten())
            .filter_map(|(column, _)| Some((column, lump::keys(column)?)))
            .collect();
        let all: Vec<&dyn Array> = columns.iter().map(|(_, keys)| keys.as_ref()).collect();
        let keys = match all.is_empty() {
            true => arrow::array::new_empty_array(&key_type),
            false => distinct(&concat(&all)?)?,
        };
        let index = CategoryIndex::new(&keys)?;
        let mut weights: Vec<Option<f64>> = vec![None; keys.len()];
        let mut weight_of: Vec<Option<Agreed<f64>>> = vec![None; keys.len()];
        let mut count_of: Vec<Option<Agreed<i64>>> = vec![None; keys.len()];
        let mut others: Vec<ArrayRef> = Vec::new();
        for (column, column_keys) in &columns {
            let lumps = column.as_struct();
            let weight = lumps.column(2).as_primitive::<Float64Type>();
            let count = lumps.column(3).as_primitive::<Int64Type>();
            let other = lump::others(column).expect("a lump");
            for (row, key) in index.positions(column_keys)?.into_iter().enumerate() {
                let key = key.expect("the distinct keys hold every key");
                if weight.is_valid(row) {
                    let value = weight.value(row);
                    weights[key] = Some(weights[key].map_or(value, |known| known.max(value)));
                    agree(&mut weight_of[key], value);
                }
                agree(&mut count_of[key], count.value(row));
            }
            let other_rows: UInt32Array = (0..column_keys.len())
                .filter(|&row| other.value(row))
                .map(|row| row as u32)
                .collect();
            if !other_rows.is_empty() {
                others.push(take(column_keys.as_ref(), &other_rows, None)?);
            }
        }
        let others = match others.is_empty() {
            true => None,
            false => {
                let refs: Vec<&dyn Array> = others.iter().map(|keys| keys.as_ref()).collect();
                Some(distinct(&concat(&refs)?)?)
            }
        };
        Ok(Some(Self {
            key_type,
            keys,
            index,
            weights,
            weight: weight_of,
            count: count_of,
            others,
        }))
    }

    /// The keys of the kept categories `values`, which an explicit domain or a preferred order
    /// names in the type of the lumps' `value`.
    pub(crate) fn kept_keys(&self, values: &ArrayRef) -> Result<ArrayRef> {
        lump::kept_keys(&self.key_type, values).ok_or_else(|| {
            EvalError::Unsupported(format!(
                "the categories {:?} of a column of lumps of {}",
                values.data_type(),
                self.key_type
            ))
        })
    }

    /// `domain`, keys of the group, in the order of the lumps: the kept categories as `domain`
    /// orders them when the fit gives an order, and by descending weight otherwise, then the Other
    /// group. An explicit domain that names no Other group gains the Other group of the rows.
    pub(crate) fn order(&self, domain: &ArrayRef, ordered: bool) -> Result<ArrayRef> {
        let domain = match &self.others {
            Some(others) => {
                let known = CategoryIndex::new(domain)?.positions(others)?;
                let missing: UInt32Array = (known.into_iter().enumerate())
                    .filter(|(_, position)| position.is_none())
                    .map(|(row, _)| row as u32)
                    .collect();
                let missing = take(others.as_ref(), &missing, None)?;
                concat(&[domain.as_ref(), missing.as_ref()])?
            }
            None => domain.clone(),
        };
        let positions = self.index.positions(&domain)?;
        let other = lump::others(&domain).expect("the keys of lumps");
        let mut kept: Vec<usize> = (0..domain.len()).filter(|&i| !other.value(i)).collect();
        if !ordered {
            // A stable sort keeps the domain's ascending order among equal weights.
            let weight = |i: &usize| {
                positions[*i]
                    .and_then(|key| self.weights[key])
                    .unwrap_or(0.0)
            };
            kept.sort_by(|a, b| weight(b).total_cmp(&weight(a)));
        }
        let order: UInt32Array = (kept.into_iter())
            .chain((0..domain.len()).filter(|&i| other.value(i)))
            .map(|i| i as u32)
            .collect();
        Ok(take(domain.as_ref(), &order, None)?)
    }
}

impl Lumps {
    /// The values that the labels of every key of the rows bind, for a band or a point axis, as
    /// `label_values` gives them, with the label of the Other group in `other`.
    pub(crate) fn category_labels(
        &self,
        pattern: &str,
        other: Option<LabelVariant>,
        engine: &LabelEngine,
    ) -> Result<avenger_guides::axis::CategoryLabels> {
        let others = lump::others(&self.keys).expect("the keys of lumps");
        Ok(avenger_guides::axis::CategoryLabels {
            categories: self.keys.clone(),
            values: self.label_values(&self.keys, pattern, engine)?,
            variants: (others.iter())
                .map(|is_other| is_other.unwrap_or(false).then(|| other.clone()).flatten())
                .collect(),
        })
    }

    /// The values that the label of each of `keys` binds: `value`, the kept category as
    /// `pattern` formats it or the Other group's label; `other`, whether the key is the Other
    /// group; `count`, its number of categories; and `weight`, its weight in the text engine's
    /// default number format. `count` and `weight` are `?` when the rows of the key disagree, as
    /// the rows of several partitions of a lump do.
    pub(crate) fn label_values(
        &self,
        keys: &ArrayRef,
        pattern: &str,
        engine: &LabelEngine,
    ) -> Result<Vec<LabelValues>> {
        let keys_struct = keys.as_struct();
        let (value, other) = (
            keys_struct.column(0),
            keys_struct.column(1).as_string::<i32>(),
        );
        let formatted = engine.format_array(value, pattern, PrecisionSharing::PerValue)?;
        let positions = self.index.positions(keys)?;
        let unknown = || LabelValue::Str("?".to_string());
        let mut labels = Vec::with_capacity(keys.len());
        for (index, (formatted, key)) in formatted.into_iter().zip(positions).enumerate() {
            let is_other = other.is_valid(index);
            let value = match is_other {
                true => LabelValue::Str(other.value(index).to_string()),
                false => formatted,
            };
            // A key without rows, such as a category of an explicit domain, has no count or
            // weight, and the rows of a key that disagree have `?`.
            let count = match key.and_then(|key| self.count[key]) {
                Some(Agreed::One(count)) => LabelValue::Int(count),
                Some(Agreed::Several) => unknown(),
                None => LabelValue::None,
            };
            let weight = match key.and_then(|key| self.weight[key]) {
                Some(Agreed::One(weight)) => weight_text(weight, engine)?,
                Some(Agreed::Several) => unknown(),
                None => LabelValue::None,
            };
            labels.push(bound(value, is_other, count, weight));
        }
        Ok(labels)
    }
}

/// The values that a label of a lump binds: `value`, `other`, `count`, and `weight`.
fn bound(value: LabelValue, other: bool, count: LabelValue, weight: LabelValue) -> LabelValues {
    LabelValues::from([
        ("value".to_string(), value),
        ("other".to_string(), LabelValue::Bool(other)),
        ("count".to_string(), count),
        ("weight".to_string(), weight),
    ])
}

/// `weight` in the text engine's default number format.
fn weight_text(weight: f64, engine: &LabelEngine) -> std::result::Result<LabelValue, LabelError> {
    let weight: ArrayRef = Arc::new(Float64Array::from(vec![weight]));
    Ok((engine.format_array(&weight, "", PrecisionSharing::PerValue)?).remove(0))
}

/// The values that a label of `row` of `column` binds when `column` is a column of lumps, as the
/// labels of a lump's guides bind them, from the row's own weight and number of categories: its
/// kept category as `pattern` formats it or the Other group's label as `value`, whether it is the
/// Other group as `other`, its number of categories as `count`, and its weight as `weight`. `None`
/// for a column that is not a lump.
pub fn lump_label_values(
    column: &ArrayRef,
    row: usize,
    pattern: &str,
    engine: &LabelEngine,
) -> std::result::Result<Option<LabelValues>, LabelError> {
    if !lump::is_lump(column.data_type()) {
        return Ok(None);
    }
    let lumps = column.as_struct();
    let other = lumps.column(1).as_string::<i32>();
    let weight = lumps.column(2).as_primitive::<Float64Type>();
    let count = lumps.column(3).as_primitive::<Int64Type>();
    let is_other = other.is_valid(row);
    let value = match is_other {
        true => LabelValue::Str(other.value(row).to_string()),
        false => (engine.format_array(
            &lumps.column(0).slice(row, 1),
            pattern,
            PrecisionSharing::PerValue,
        )?)
        .remove(0),
    };
    let weight = match weight.is_valid(row) {
        true => weight_text(weight.value(row), engine)?,
        false => LabelValue::None,
    };
    let count = LabelValue::Int(count.value(row));
    Ok(Some(bound(value, is_other, count, weight)))
}

/// `values`, the values of the members of a scale group, with each column of lumps as its keys,
/// which the group's categorical scale takes.
pub(crate) fn keyed(values: &[Option<MemberValues>]) -> Vec<Option<MemberValues>> {
    (values.iter())
        .map(|member| {
            member.as_ref().map(|(column, reach)| {
                (
                    lump::keys(column).unwrap_or_else(|| column.clone()),
                    reach.clone(),
                )
            })
        })
        .collect()
}

/// The values that `channel` takes from `column`: the keys of a column of lumps, which its
/// categorical scale maps, and otherwise the starts or the ends of a column of spans, or `column`
/// itself.
pub(crate) fn channel_values(
    column: &ArrayRef,
    channel: avenger_chart::proto::Channel,
) -> ArrayRef {
    lump::keys(column).unwrap_or_else(|| avenger_chart::span::channel_values(column, channel))
}
