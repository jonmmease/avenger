//! The units of the values of an array, and sequences of units.

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, DictionaryArray, Int64Array, StringArray, StructArray,
};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{
    DataType, Date32Type, Field, FieldRef, Fields, Int32Type, TimeUnit as ArrowUnit,
    TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
    TimestampSecondType,
};
use datafusion::common::{Result, ScalarValue, plan_err};

pub(crate) fn ticks_of(a: &ArrayRef) -> Result<Vec<i64>> {
    Ok(match a.data_type() {
        DataType::Date32 => a
            .as_primitive::<Date32Type>()
            .values()
            .iter()
            .map(|v| *v as i64)
            .collect(),
        DataType::Timestamp(u, _) => match u {
            ArrowUnit::Second => a.as_primitive::<TimestampSecondType>().values().to_vec(),
            ArrowUnit::Millisecond => a
                .as_primitive::<TimestampMillisecondType>()
                .values()
                .to_vec(),
            ArrowUnit::Microsecond => a
                .as_primitive::<TimestampMicrosecondType>()
                .values()
                .to_vec(),
            ArrowUnit::Nanosecond => a
                .as_primitive::<TimestampNanosecondType>()
                .values()
                .to_vec(),
        },
        other => return plan_err!("timeunit takes a DATE or a TIMESTAMP, not {other}"),
    })
}

/// An array of the type of the values, a `DATE` or a timestamp with its time zone, of ticks.
pub(crate) fn temporal(dt: &DataType, ticks: Vec<i64>) -> Result<ArrayRef> {
    Ok(cast(&(Arc::new(Int64Array::from(ticks)) as ArrayRef), dt)?)
}

pub fn result_fields(dt: &DataType) -> Fields {
    Fields::from(vec![
        Field::new("start", dt.clone(), false),
        Field::new("end", dt.clone(), false),
        Field::new("unit", unit_type(), false),
    ])
}

/// The type of the text of units: a dictionary, as a column has few units.
pub(crate) fn unit_type() -> DataType {
    DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8))
}

pub(super) fn result_field(name: &str, dt: &DataType, nullable: bool, tag: String) -> FieldRef {
    Arc::new(
        Field::new(name, DataType::Struct(result_fields(dt)), nullable)
            .with_metadata(HashMap::from([(TAG.to_string(), tag)])),
    )
}

/// The units of the values of `x`, with the unit of each row from `unit_of` (None for a
/// null result).
pub fn apply(
    x: &ArrayRef,
    o: &TimeUnitOptions,
    unit_of: impl Fn(usize) -> Option<Unit>,
) -> Result<ArrayRef> {
    let dt = x.data_type().clone();
    let mut cal = Calendar::new(&dt, o)?;
    let ticks = ticks_of(x)?;
    let n = ticks.len();
    let mut starts = vec![0i64; n];
    let mut ends = vec![0i64; n];
    let mut keys = vec![0i32; n];
    let mut valid = vec![false; n];
    let mut units: Vec<Unit> = Vec::new();
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        if x.is_null(i) {
            continue;
        }
        let Some(u) = unit_of(i) else { continue };
        let k = match units.iter().position(|v| *v == u) {
            Some(k) => k,
            None => {
                if dt == DataType::Date32 && u.is_subday() {
                    return plan_err!(
                        "the unit '{}' is shorter than one day, so it does not apply to a DATE",
                        u.text()
                    );
                }
                if u.is_fiscal() && o.fiscal_year_start.is_none() {
                    return plan_err!("the unit '{}' needs 'fiscal_year_start'", u.text());
                }
                units.push(u);
                rows.push(Vec::new());
                units.len() - 1
            }
        };
        keys[i] = k as i32;
        valid[i] = true;
        rows[k].push(i);
    }
    for (u, rows) in units.iter().zip(&rows) {
        check_range(&cal, &dt, &ticks, rows.iter().copied(), Some(*u))?;
    }
    prepare(&mut cal, x, &ticks);
    for (u, rows) in units.iter().zip(&rows) {
        apply_unit(&mut cal, &ticks, rows, *u, &mut starts, &mut ends);
    }
    let mut texts: Vec<String> = units.iter().map(|u| u.text()).collect();
    if texts.is_empty() {
        texts.push(String::new());
    }
    let unit_array =
        DictionaryArray::<Int32Type>::try_new(keys.into(), Arc::new(StringArray::from(texts)))?;
    let nulls = NullBuffer::from(valid);
    Ok(Arc::new(StructArray::try_new(
        result_fields(&dt),
        vec![
            temporal(&dt, starts)?,
            temporal(&dt, ends)?,
            Arc::new(unit_array),
        ],
        (nulls.null_count() > 0).then_some(nulls),
    )?))
}

/// Makes the offset table of a zone for the range of the values.
pub(super) fn prepare(cal: &mut Calendar, x: &ArrayRef, ticks: &[i64]) {
    if !cal.clock.is_named() {
        return;
    }
    let mut range: Option<(i64, i64)> = None;
    for (i, t) in ticks.iter().enumerate() {
        if x.is_valid(i) {
            range = Some(range.map_or((*t, *t), |(a, b)| (a.min(*t), b.max(*t))));
        }
    }
    if let Some((lo, hi)) = range {
        cal.clock.prepare(cal.kind.secs(lo), cal.kind.secs(hi));
    }
}

/// Fails unless the values of the rows are in [`Kind::supported`] with a margin of one unit
/// of `u` and three days, so that the units around them and their offsets have ticks.
pub(super) fn check_range(
    cal: &Calendar,
    dt: &DataType,
    ticks: &[i64],
    rows: impl Iterator<Item = usize>,
    u: Option<Unit>,
) -> Result<()> {
    // A collapsing unit is a unit of the reference year, which needs no margin.
    let unit_days = match u {
        Some(u @ Unit::Trunc(..)) => u.approx_ms() / 8.64e7,
        _ => 0.0,
    };
    let margin = (1.01 * unit_days).ceil().min(1e9) as i64 + 3;
    let (lo, hi) = cal.kind.supported(margin);
    if let Some(t) = rows.map(|i| ticks[i]).find(|t| *t < lo || *t > hi) {
        let what = u.map_or(String::new(), |u| format!("the unit '{}' of ", u.text()));
        return plan_err!(
            "timeunit cannot compute {what}the value {t} of type {dt}: the unit is too close to the limits of the dates it can represent"
        );
    }
    Ok(())
}

/// The units of one unit and calendar over values of one type, one at a time. `impute`
/// fills the gaps of a sequence of units with it, so its units are those of `timeunit`.
pub(crate) struct UnitSequence {
    pub(super) cal: Calendar,
    pub(super) dt: DataType,
    pub(super) unit: Unit,
}

impl UnitSequence {
    pub(crate) fn new(dt: &DataType, o: &TimeUnitOptions, unit: Unit) -> Result<Self> {
        let o = TimeUnitOptions {
            unit: Some(unit),
            ..o.clone()
        };
        if unit.is_fiscal() && o.fiscal_year_start.is_none() {
            return plan_err!("the unit '{}' needs 'fiscal_year_start'", unit.text());
        }
        Ok(Self {
            cal: Calendar::new(dt, &o)?,
            dt: dt.clone(),
            unit,
        })
    }

    /// Makes the table of the offsets of a named zone for the instants from `lo` to `hi`.
    pub(crate) fn prepare(&mut self, lo: i64, hi: i64) {
        if self.cal.clock.is_named() {
            self.cal
                .clock
                .prepare(self.cal.kind.secs(lo), self.cal.kind.secs(hi));
        }
    }

    /// The unit that contains `t`, as the ticks `[start, end)`.
    pub(crate) fn unit_at(&mut self, t: i64) -> Result<(i64, i64)> {
        check_range(&self.cal, &self.dt, &[t], 0..1, Some(self.unit))?;
        Ok(match self.unit {
            Unit::Trunc(tr, k) => self.cal.bounds(t, tr, k),
            Unit::Collapse(c) => {
                let (s, e) = self.cal.collapse(t, c);
                (self.cal.instant(s), self.cal.instant(e))
            }
        })
    }
}

/// The calendar options of the metadata tag of a `timeunit` result: `week_start` and
/// `fiscal_year_start`, or the defaults without a tag.
pub(crate) fn options_of_tag(tag: Option<&str>) -> Result<TimeUnitOptions> {
    // The JSON value of a key: up to the next comma or brace.
    let value = |key: &str| -> Option<&str> {
        let tag = tag?;
        let at = tag.find(&format!("\"{key}\":"))? + key.len() + 3;
        tag[at..].split([',', '}']).next()
    };
    let mut pairs = Vec::new();
    if let Some(v) = value("week_start") {
        pairs.push(ScalarValue::from("week_start"));
        pairs.push(ScalarValue::from(v.trim_matches('"')));
    }
    if let Some(v) = value("fiscal_year_start") {
        pairs.push(ScalarValue::from("fiscal_year_start"));
        pairs.push(ScalarValue::Int64(v.parse().ok()));
    }
    TimeUnitOptions::parse(&pairs)
}

pub(super) fn apply_unit(
    cal: &mut Calendar,
    ticks: &[i64],
    rows: &[usize],
    u: Unit,
    starts: &mut [i64],
    ends: &mut [i64],
) {
    match u {
        Unit::Collapse(c) => {
            for &i in rows {
                let (s, e) = cal.collapse(ticks[i], c);
                (starts[i], ends[i]) = (cal.instant(s), cal.instant(e));
            }
        }
        Unit::Trunc(t, k) => {
            // Units of a day or longer: the edges of the range of the rows, then a binary
            // search for each row. Shorter units: integer arithmetic for each row.
            if t >= Trunc::Day && !rows.is_empty() {
                let lo = rows.iter().map(|&i| ticks[i]).min().unwrap();
                let hi = rows.iter().map(|&i| ticks[i]).max().unwrap();
                let limit = (4 * rows.len()).max(4096);
                if let Some(edges) = cal.edges(lo, hi, t, k, limit) {
                    for &i in rows {
                        let j = edges.partition_point(|e| *e <= ticks[i]);
                        (starts[i], ends[i]) = if j > 0 && j < edges.len() {
                            (edges[j - 1], edges[j])
                        } else {
                            cal.bounds(ticks[i], t, k)
                        };
                    }
                    return;
                }
            }
            for &i in rows {
                (starts[i], ends[i]) = cal.bounds(ticks[i], t, k);
            }
        }
    }
}
