//! `fill_gaps`: the keys of a sequence that no row has.
//!
//! The planner reads the argument into [`Gaps`] with [`Gaps::plan`], and `ImputeExec` adds
//! the keys of the gaps of each partition with [`Gaps::keys`].

use std::collections::HashSet;
use std::sync::Arc;

use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Float64Array, Int64Array, StringArray, StructArray, new_null_array,
};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{
    DataType, Field, Fields, Float64Type, Int64Type, TimeUnit as ArrowUnit,
};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{Result, ScalarValue, exec_err};

use crate::timeunit::{self, TimeUnitOptions, Unit, UnitSequence};

/// The most positions of the gaps of one partition.
pub const LIMIT: usize = 1_000_000;

/// The kind of a key whose gaps `fill_gaps` fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Hash)]
pub enum GapKind {
    /// A struct `{start, end, unit}`, as `timeunit` returns.
    TimeUnit,
    /// A struct `{start, end}` of numbers, dates, or timestamps, as `bin` returns.
    Interval,
    /// A DATE or a timestamp.
    Date,
    /// A number.
    Number,
}

/// The step of a sequence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub enum Step {
    /// The width of an interval or the distance of numbers: a `Float64` for keys of floating
    /// point numbers, else an `Int64` of the ticks of the key's type.
    Width(ScalarValue),
    /// The unit of a time unit or a date key.
    Unit(String),
}

/// `fill_gaps`: the sequence whose gaps `impute` fills. A field that the sequence leaves out
/// comes from the keys of each partition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash)]
pub struct Gaps {
    pub kind: GapKind,
    /// The start of the range, of the type of the key or of its `start` field.
    pub start: Option<ScalarValue>,
    /// The end of the range, exclusive, of the same type.
    pub end: Option<ScalarValue>,
    pub step: Option<Step>,
}

/// The type of the positions of a key: the key's type, or the type of its `start` field.
fn coordinate(kind: GapKind, key: &DataType) -> DataType {
    match (kind, key) {
        (GapKind::TimeUnit | GapKind::Interval, DataType::Struct(fields)) => {
            field(fields, "start").map_or(DataType::Null, |f| f.data_type().clone())
        }
        _ => key.clone(),
    }
}

fn field<'a>(fields: &'a Fields, name: &str) -> Option<&'a Arc<Field>> {
    fields.iter().find(|f| f.name() == name)
}

fn is_number(dt: &DataType) -> bool {
    dt.is_integer() || dt.is_floating()
}

fn is_float(dt: &DataType) -> bool {
    dt.is_floating()
}

fn is_temporal(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(..)
    )
}

fn is_string(dt: &DataType) -> bool {
    match dt {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => true,
        DataType::Dictionary(_, v) => is_string(v),
        _ => false,
    }
}

/// The kind of a key, or why `fill_gaps` does not apply to it.
fn kind_of(key: &DataType) -> std::result::Result<GapKind, String> {
    match key {
        DataType::Struct(fields) => {
            let (Some(start), Some(end)) = (field(fields, "start"), field(fields, "end")) else {
                return Err(format!(
                    "a struct key needs the fields `start` and `end`, not {key}"
                ));
            };
            if start.data_type() != end.data_type() {
                return Err(format!(
                    "the fields `start` and `end` of the key differ in type: {key}"
                ));
            }
            let dt = start.data_type();
            let unit = field(fields, "unit");
            let kind = match unit {
                Some(u)
                    if is_string(u.data_type())
                        && matches!(dt, DataType::Date32 | DataType::Timestamp(..)) =>
                {
                    GapKind::TimeUnit
                }
                _ if is_number(dt) || is_temporal(dt) => GapKind::Interval,
                _ => {
                    return Err(format!(
                        "the `start` and `end` of the key must be numbers, dates, or timestamps, not {dt}"
                    ));
                }
            };
            let made = ["start", "end"]
                .iter()
                .chain((kind == GapKind::TimeUnit).then_some(&"unit"));
            let made: Vec<&str> = made.copied().collect();
            if let Some(f) = fields
                .iter()
                .find(|f| !made.contains(&f.name().as_str()) && !f.is_nullable())
            {
                return Err(format!(
                    "the added keys are NULL in the field `{}` of the key, which cannot be NULL",
                    f.name()
                ));
            }
            Ok(kind)
        }
        DataType::Date32 | DataType::Timestamp(..) => Ok(GapKind::Date),
        dt if is_number(dt) => Ok(GapKind::Number),
        dt => Err(format!(
            "`fill_gaps` fills the gaps of bins, time units, dates, timestamps, and numbers, not of a key of type {dt}"
        )),
    }
}

/// Ticks of a temporal type in one nanosecond, as a divisor: the nanoseconds of one tick.
fn nanos_per_tick(dt: &DataType) -> Option<i128> {
    Some(match dt {
        DataType::Date32 => 86_400_000_000_000,
        DataType::Date64 => 1_000_000,
        DataType::Timestamp(u, _) => match u {
            ArrowUnit::Second => 1_000_000_000,
            ArrowUnit::Millisecond => 1_000_000,
            ArrowUnit::Microsecond => 1_000,
            ArrowUnit::Nanosecond => 1,
        },
        _ => return None,
    })
}

/// The ticks of `dt` in an interval or a duration, if it has a fixed length that is a whole
/// number of ticks.
fn interval_ticks(v: &ScalarValue, dt: &DataType) -> Option<i64> {
    let nanos: i128 = match v {
        ScalarValue::IntervalDayTime(Some(x)) => {
            x.days as i128 * 86_400_000_000_000 + x.milliseconds as i128 * 1_000_000
        }
        ScalarValue::IntervalMonthDayNano(Some(x)) if x.months == 0 => {
            x.days as i128 * 86_400_000_000_000 + x.nanoseconds as i128
        }
        ScalarValue::DurationSecond(Some(x)) => *x as i128 * 1_000_000_000,
        ScalarValue::DurationMillisecond(Some(x)) => *x as i128 * 1_000_000,
        ScalarValue::DurationMicrosecond(Some(x)) => *x as i128 * 1_000,
        ScalarValue::DurationNanosecond(Some(x)) => *x as i128,
        _ => return None,
    };
    let per = nanos_per_tick(dt)?;
    (nanos % per == 0)
        .then(|| i64::try_from(nanos / per).ok())
        .flatten()
}

impl Gaps {
    /// Reads the constant value of `fill_gaps` for a key of type `key`: `true`, a unit, or a
    /// struct `{start, end, step}`. `false` is None. The error is the message for the argument.
    pub fn plan(value: &ScalarValue, key: &DataType) -> std::result::Result<Option<Gaps>, String> {
        let kind = kind_of(key)?;
        let coordinate = coordinate(kind, key);
        let mut gaps = Gaps {
            kind,
            start: None,
            end: None,
            step: None,
        };
        match value {
            ScalarValue::Boolean(Some(false)) => return Ok(None),
            ScalarValue::Boolean(Some(true)) => {}
            v if v.try_as_str().flatten().is_some() => {
                gaps.step = Some(Self::plan_step(v, kind, &coordinate, key)?);
            }
            ScalarValue::Struct(s) => {
                for (f, column) in s.fields().iter().zip(s.columns()) {
                    let v = ScalarValue::try_from_array(column, 0).map_err(|e| e.to_string())?;
                    if v.is_null() {
                        continue;
                    }
                    match f.name().as_str() {
                        "start" | "end" => {
                            let cast = super::cast_exact(&v, &coordinate).ok_or_else(|| {
                                format!(
                                    "the `{}` {v} of `fill_gaps` does not coerce to {coordinate}",
                                    f.name()
                                )
                            })?;
                            if f.name() == "start" {
                                gaps.start = Some(cast);
                            } else {
                                gaps.end = Some(cast);
                            }
                        }
                        "step" => gaps.step = Some(Self::plan_step(&v, kind, &coordinate, key)?),
                        other => {
                            return Err(format!(
                                "unknown field `{other}` of `fill_gaps`: a sequence has the fields start, end, and step"
                            ));
                        }
                    }
                }
            }
            v => {
                return Err(format!(
                    "`fill_gaps` must be true, a unit such as 'month', or a struct {{start, end, step}}, not {v}"
                ));
            }
        }
        match (kind, &gaps.step) {
            (GapKind::Date, None) => Err(format!(
                "`fill_gaps` of a key of type {key} needs a unit, such as `fill_gaps => 'month'`"
            )),
            (GapKind::Number, None) => Err(format!(
                "`fill_gaps` of a key of type {key} needs a step, such as `fill_gaps => named_struct('step', 1)`"
            )),
            _ => Ok(Some(gaps)),
        }
    }

    fn plan_step(
        v: &ScalarValue,
        kind: GapKind,
        coordinate: &DataType,
        key: &DataType,
    ) -> std::result::Result<Step, String> {
        match kind {
            GapKind::TimeUnit | GapKind::Date => {
                let Some(Some(text)) = v.try_as_str() else {
                    return Err(format!(
                        "the step of a key of type {key} is a unit, such as 'month', not {v}"
                    ));
                };
                let unit = Unit::parse(text).map_err(|e| e.strip_backtrace())?;
                if kind == GapKind::Date {
                    if matches!(unit, Unit::Collapse(_)) {
                        return Err(format!(
                            "the collapsing unit '{text}' does not apply to a date key; key impute by the result of timeunit"
                        ));
                    }
                    if unit.is_fiscal() {
                        return Err(format!(
                            "the fiscal unit '{text}' needs a fiscal year start; key impute by the result of timeunit with 'fiscal_year_start'"
                        ));
                    }
                    let o = TimeUnitOptions {
                        unit: Some(unit),
                        ..Default::default()
                    };
                    o.check(key).map_err(|e| e.strip_backtrace())?;
                }
                Ok(Step::Unit(text.to_string()))
            }
            GapKind::Interval | GapKind::Number => {
                if v.try_as_str().flatten().is_some() {
                    return Err(format!(
                        "a unit applies to a date key or a time unit key; the step of a key of type {key} is a width"
                    ));
                }
                let width = if is_temporal(coordinate) {
                    interval_ticks(v, coordinate).map(ScalarValue::from).ok_or_else(|| {
                        format!("the step of a key of {coordinate} must be an interval of a fixed length in whole ticks, not {v}")
                    })?
                } else if is_float(coordinate) {
                    v.cast_to(&DataType::Float64)
                        .map_err(|_| format!("the step {v} of `fill_gaps` must be a number"))?
                } else {
                    super::cast_exact(v, &DataType::Int64).ok_or_else(|| {
                        format!("the step {v} of a key of type {coordinate} must be an integer")
                    })?
                };
                let positive = match &width {
                    ScalarValue::Float64(Some(f)) => f.is_finite() && *f > 0.0,
                    ScalarValue::Int64(Some(i)) => *i > 0,
                    _ => false,
                };
                if !positive {
                    return Err(format!("the step {v} of `fill_gaps` must be positive"));
                }
                Ok(Step::Width(width))
            }
        }
    }

    /// The keys of the positions of the sequence that `observed` (the distinct, non-null keys
    /// of a partition and `extra_keys`) does not have, of the type of `field`, the key.
    pub fn keys(&self, observed: &ArrayRef, field: &Field) -> Result<ArrayRef> {
        match self.kind {
            GapKind::TimeUnit | GapKind::Date => self.unit_keys(observed, field),
            GapKind::Interval | GapKind::Number => {
                let coordinate = coordinate(self.kind, field.data_type());
                let (starts, ends) = match observed.data_type() {
                    DataType::Struct(_) => {
                        let s = observed.as_struct();
                        (
                            s.column_by_name("start").unwrap().clone(),
                            Some(s.column_by_name("end").unwrap().clone()),
                        )
                    }
                    _ => (observed.clone(), None),
                };
                let (starts, ends) = if is_float(&coordinate) {
                    let width = match &self.step {
                        Some(Step::Width(ScalarValue::Float64(Some(w)))) => Some(*w),
                        _ => None,
                    };
                    let (s, e) = self.positions(
                        floats(&starts)?,
                        ends.as_ref().map(floats).transpose()?,
                        width,
                        self.bound_f64(&self.start)?,
                        self.bound_f64(&self.end)?,
                    )?;
                    (from_floats(s, &coordinate)?, from_floats(e, &coordinate)?)
                } else {
                    let width = match &self.step {
                        Some(Step::Width(ScalarValue::Int64(Some(w)))) => Some(*w),
                        _ => None,
                    };
                    let (s, e) = self.positions(
                        ints(&starts)?,
                        ends.as_ref().map(ints).transpose()?,
                        width,
                        self.bound_i64(&self.start)?,
                        self.bound_i64(&self.end)?,
                    )?;
                    (from_ints(s, &coordinate)?, from_ints(e, &coordinate)?)
                };
                match field.data_type() {
                    DataType::Struct(fields) => build_struct(fields, starts, ends, None),
                    _ => Ok(starts),
                }
            }
        }
    }

    fn bound_f64(&self, v: &Option<ScalarValue>) -> Result<Option<f64>> {
        v.as_ref()
            .map(|v| Ok(floats(&v.to_array()?)?[0].unwrap_or(f64::NAN)))
            .transpose()
    }

    fn bound_i64(&self, v: &Option<ScalarValue>) -> Result<Option<i64>> {
        v.as_ref()
            .map(|v| Ok(ints(&v.to_array()?)?[0].unwrap_or(0)))
            .transpose()
    }

    /// The starts and ends of the missing positions of an interval or number sequence.
    fn positions<C: Coordinate>(
        &self,
        starts: Vec<Option<C>>,
        ends: Option<Vec<Option<C>>>,
        width: Option<C>,
        start: Option<C>,
        end: Option<C>,
    ) -> Result<(Vec<C>, Vec<C>)> {
        // The keys of the sequence: finite, not NULL, with their ends.
        let keys: Vec<(C, C)> = match &ends {
            Some(ends) => starts
                .iter()
                .zip(ends)
                .filter_map(|(s, e)| Some(((*s)?, (*e)?)))
                .filter(|(s, e)| s.finite() && e.finite())
                .collect(),
            None => starts
                .iter()
                .flatten()
                .filter(|s| s.finite())
                .map(|s| (*s, *s))
                .collect(),
        };
        let Some(step) = width.or_else(|| {
            keys.first()
                .filter(|_| ends.is_some())
                .map(|(s, e)| e.minus(*s))
        }) else {
            return Ok((vec![], vec![]));
        };
        let origin = match start.or_else(|| {
            keys.iter()
                .map(|k| k.0)
                .reduce(|a, b| if b < a { b } else { a })
        }) {
            Some(o) => o,
            None => return Ok((vec![], vec![])),
        };
        let mut have = HashSet::new();
        let mut greatest = None::<i64>;
        for (s, e) in &keys {
            if ends.is_some() && !C::same_width(e.minus(*s), step) {
                return exec_err!(
                    "impute fills the gaps of intervals of one width in a partition, but the key [{s}, {e}) is not {step} wide; \
                     partition impute with `partition_by` by the partitions of the bins"
                );
            }
            let Some(i) = s.index(origin, step) else {
                return exec_err!(
                    "impute fills the gaps of keys a whole number of steps apart in a partition, but the key at {s} is not \
                     a whole number of {step} from {origin}; partition impute with `partition_by` by the partitions of the bins"
                );
            };
            have.insert(i);
            greatest = Some(greatest.map_or(i, |g| g.max(i)));
        }
        // Positions 0 to `last`: from the origin, which is the start or the least key, to the
        // last position that starts before the end, or to the greatest key.
        let last = match end {
            Some(e) => C::positions_before(e, origin, step) - 1,
            None => match greatest {
                Some(g) => g,
                None => return Ok((vec![], vec![])),
            },
        };
        if last >= LIMIT as i64 {
            return exec_err!(
                "impute would fill more than {LIMIT} positions from {origin} by {step} in a partition; use a longer step or a shorter range"
            );
        }
        let explicit = start.is_some() && width.is_some();
        let (mut out_s, mut out_e) = (Vec::new(), Vec::new());
        for i in 0..=last.max(-1) {
            if !have.contains(&i) {
                let (s, e) = C::nth(origin, step, i, explicit);
                out_s.push(s);
                out_e.push(e);
            }
        }
        Ok((out_s, out_e))
    }

    fn unit_keys(&self, observed: &ArrayRef, field: &Field) -> Result<ArrayRef> {
        let dt = coordinate(self.kind, field.data_type());
        let (starts, ends, units) = match observed.data_type() {
            DataType::Struct(_) => {
                let s = observed.as_struct();
                let units = cast(s.column_by_name("unit").unwrap(), &DataType::Utf8)?;
                (
                    s.column_by_name("start").unwrap().clone(),
                    Some(s.column_by_name("end").unwrap().clone()),
                    Some(units),
                )
            }
            _ => (observed.clone(), None, None),
        };
        // The unit: of the keys of a time unit key, else of the step.
        let mut texts: Vec<String> = Vec::new();
        if let Some(units) = &units {
            let units = units.as_string::<i32>();
            for i in 0..units.len() {
                if starts.is_valid(i)
                    && units.is_valid(i)
                    && !texts.iter().any(|t| t == units.value(i))
                {
                    texts.push(units.value(i).to_string());
                }
            }
        }
        if texts.len() > 1 {
            return exec_err!(
                "impute fills the gaps of one time unit in a partition, but the keys have the units '{}' and '{}'; \
                 partition impute with `partition_by` by the partitions of the timeunit call",
                texts[0],
                texts[1]
            );
        }
        let step = match &self.step {
            Some(Step::Unit(text)) => Some(text.as_str()),
            _ => None,
        };
        let text = match (texts.first(), step) {
            (Some(key), Some(step)) => {
                if Unit::parse(key)? != Unit::parse(step)? {
                    return exec_err!(
                        "the step '{step}' of `fill_gaps` differs from the unit '{key}' of the keys"
                    );
                }
                key.clone()
            }
            (Some(key), None) => key.clone(),
            (None, Some(step)) => step.to_string(),
            (None, None) => return Ok(new_null_array(field.data_type(), 0)),
        };
        let unit = Unit::parse(&text)?;
        let options = match self.kind {
            GapKind::TimeUnit => {
                timeunit::options_of_tag(field.metadata().get(timeunit::TAG).map(String::as_str))?
            }
            _ => TimeUnitOptions::default(),
        };
        let mut seq = UnitSequence::new(&dt, &options, unit)?;

        // Each key must be a unit of the sequence.
        let start_ticks = timeunit::ticks_of(&starts)?;
        let end_ticks = ends.as_ref().map(timeunit::ticks_of).transpose()?;
        let valid: Vec<usize> = (0..starts.len()).filter(|i| starts.is_valid(*i)).collect();
        if let (Some(lo), Some(hi)) = (
            valid.iter().map(|i| start_ticks[*i]).min(),
            valid.iter().map(|i| start_ticks[*i]).max(),
        ) {
            seq.prepare(lo, hi);
        }
        let mut have = HashSet::new();
        for &i in &valid {
            let (s, e) = seq.unit_at(start_ticks[i])?;
            let ok = s == start_ticks[i] && end_ticks.as_ref().is_none_or(|ends| ends[i] == e);
            if !ok {
                let shown = array_value_to_string(observed, i)?;
                return match self.kind {
                    GapKind::Date => exec_err!(
                        "the key {shown} is not the start of a '{text}'; truncate the keys with timeunit or date_trunc, or key impute by the result of timeunit"
                    ),
                    _ => exec_err!(
                        "the key {shown} is not a unit '{text}' of its calendar options; key impute by the result of timeunit, whose rule tags the calendar"
                    ),
                };
            }
            have.insert(s);
        }

        // The units of the range: from the unit of the start, or of the least key, while a
        // unit starts before the end, or to the greatest key.
        let bound = |v: &Option<ScalarValue>| -> Result<Option<i64>> {
            v.as_ref()
                .map(|v| Ok(timeunit::ticks_of(&cast(&v.to_array()?, &dt)?)?[0]))
                .transpose()
        };
        let (start, end) = (bound(&self.start)?, bound(&self.end)?);
        let first = match start {
            Some(s) => Some(seq.unit_at(s)?.0),
            None => valid.iter().map(|i| start_ticks[*i]).min(),
        };
        let greatest = valid.iter().map(|i| start_ticks[*i]).max();
        let Some(mut t) = first else {
            return Ok(new_null_array(field.data_type(), 0));
        };
        let (mut out_s, mut out_e) = (Vec::new(), Vec::new());
        let mut count = 0usize;
        loop {
            let more = match (end, greatest) {
                (Some(e), _) => t < e,
                (None, Some(g)) => t <= g,
                (None, None) => false,
            };
            if !more {
                break;
            }
            let (s, e) = seq.unit_at(t)?;
            // A collapsing unit wraps at the end of its reference year, where the unit at the
            // end of the last one starts before it.
            if count > 0 && s != t {
                break;
            }
            count += 1;
            if count > LIMIT {
                return exec_err!(
                    "impute would fill more than {LIMIT} units '{text}' in a partition; use a longer unit or a shorter range"
                );
            }
            if !have.contains(&s) {
                out_s.push(s);
                out_e.push(e);
            }
            if e <= t {
                break;
            }
            t = e;
        }
        match field.data_type() {
            DataType::Struct(fields) => {
                let n = out_s.len();
                build_struct(
                    fields,
                    timeunit::temporal(&dt, out_s),
                    timeunit::temporal(&dt, out_e),
                    Some(Arc::new(StringArray::from(vec![text.as_str(); n]))),
                )
            }
            key => Ok(cast(&timeunit::temporal(&dt, out_s), key)?),
        }
    }
}

/// A key struct of the given fields: `start`, `end`, `unit` if any, and NULL in the others.
fn build_struct(
    fields: &Fields,
    starts: ArrayRef,
    ends: ArrayRef,
    units: Option<ArrayRef>,
) -> Result<ArrayRef> {
    let n = starts.len();
    let columns = fields
        .iter()
        .map(|f| {
            let column = match (f.name().as_str(), &units) {
                ("start", _) => starts.clone(),
                ("end", _) => ends.clone(),
                ("unit", Some(u)) => u.clone(),
                _ => return Ok(new_null_array(f.data_type(), n)),
            };
            Ok(cast(&column, f.data_type())?)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Arc::new(StructArray::try_new(
        fields.clone(),
        columns,
        None,
    )?))
}

fn floats(a: &ArrayRef) -> Result<Vec<Option<f64>>> {
    let f = cast(a, &DataType::Float64)?;
    Ok(f.as_primitive::<Float64Type>().iter().collect())
}

fn ints(a: &ArrayRef) -> Result<Vec<Option<i64>>> {
    let a = match a.data_type() {
        DataType::Date32 => cast(a, &DataType::Int32)?,
        _ => a.clone(),
    };
    let i = cast(&a, &DataType::Int64)?;
    Ok(i.as_primitive::<Int64Type>().iter().collect())
}

fn from_floats(v: Vec<f64>, dt: &DataType) -> Result<ArrayRef> {
    Ok(cast(&(Arc::new(Float64Array::from(v)) as ArrayRef), dt)?)
}

fn from_ints(v: Vec<i64>, dt: &DataType) -> Result<ArrayRef> {
    let a: ArrayRef = Arc::new(Int64Array::from(v));
    Ok(match dt {
        DataType::Date32 => cast(&cast(&a, &DataType::Int32)?, dt)?,
        _ => cast(&a, dt)?,
    })
}

/// The arithmetic of the positions of a sequence, for floating point numbers and for ticks.
trait Coordinate: Copy + PartialOrd + std::fmt::Display {
    fn finite(self) -> bool;
    fn minus(self, o: Self) -> Self;
    /// Whether a width is the step.
    fn same_width(w: Self, step: Self) -> bool;
    /// The number of steps from the origin, if it is a whole number.
    fn index(self, origin: Self, step: Self) -> Option<i64>;
    /// The number of positions from the origin that start before `bound`.
    fn positions_before(bound: Self, origin: Self, step: Self) -> i64;
    /// The start and the end of position `i`. With an explicit origin and step, the formulas
    /// of `bin`, so that an empty bin gets the key that `bin` gives a value in it.
    fn nth(origin: Self, step: Self, i: i64, explicit: bool) -> (Self, Self);
}

impl Coordinate for f64 {
    fn finite(self) -> bool {
        self.is_finite()
    }
    fn minus(self, o: Self) -> Self {
        self - o
    }
    fn same_width(w: Self, step: Self) -> bool {
        (w - step).abs() <= 1e-9 * step.abs()
    }
    fn index(self, origin: Self, step: Self) -> Option<i64> {
        let x = (self - origin) / step;
        let r = x.round();
        ((x - r).abs() <= 1e-6 && r.abs() < 9e15).then_some(r as i64)
    }
    fn positions_before(bound: Self, origin: Self, step: Self) -> i64 {
        let n = ((bound - origin) / step).ceil();
        if n.is_nan() {
            0
        } else {
            n.clamp(-1.0, 9e15) as i64
        }
    }
    fn nth(origin: Self, step: Self, i: i64, explicit: bool) -> (Self, Self) {
        let s = origin + step * i as f64;
        let e = if explicit {
            origin + step * (1.0 + (s - origin) / step)
        } else {
            s + step
        };
        (s, e)
    }
}

impl Coordinate for i64 {
    fn finite(self) -> bool {
        true
    }
    fn minus(self, o: Self) -> Self {
        self.saturating_sub(o)
    }
    fn same_width(w: Self, step: Self) -> bool {
        w == step
    }
    fn index(self, origin: Self, step: Self) -> Option<i64> {
        let d = self as i128 - origin as i128;
        (d % step as i128 == 0)
            .then(|| i64::try_from(d / step as i128).ok())
            .flatten()
    }
    fn positions_before(bound: Self, origin: Self, step: Self) -> i64 {
        let d = bound as i128 - origin as i128;
        let n = d.div_euclid(step as i128) + i128::from(d.rem_euclid(step as i128) != 0);
        n.clamp(-1, i64::MAX as i128) as i64
    }
    fn nth(origin: Self, step: Self, i: i64, _explicit: bool) -> (Self, Self) {
        let s = origin.saturating_add(step.saturating_mul(i));
        (s, s.saturating_add(step))
    }
}
