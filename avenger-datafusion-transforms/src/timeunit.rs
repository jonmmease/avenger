//! `timeunit`: the calendar unit, such as the month or the quarter, that contains each date
//! or timestamp.
//!
//! ```sql
//! SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
//! SELECT *, timeunit(t, 'unit', 'month') AS u FROM events
//! ```
//!
//! The options are pairs of a name and a constant value, after the value, as for `bin`:
//!
//! - `unit`: a truncating unit, such as `'month'`, `'15 minutes'`, `'fiscal_quarter'`, or a
//!   collapsing unit, such as `'month_of_year'`. Without it, the window form chooses a unit
//!   for each window partition.
//! - `maxbins` (20): the largest number of units that the chosen unit permits.
//! - `min_unit`: the shortest unit that the window form may choose. Without it, the floor is
//!   the resolution of the data: the finest calendar field that is not at its start in some
//!   value (`month` for values that are all first days of months), and a multiple of the
//!   greatest common divisor of the differences of the values (`30 minutes` for values
//!   every 10 minutes).
//! - `week_start` (`'monday'`): `'monday'` or `'sunday'`.
//! - `fiscal_year_start`: a month name. It sets the start of the units `fiscal_year` and
//!   `fiscal_quarter`, which need it.
//!
//! The result is `struct<start, end, unit>`: the start of the unit, the start of the next
//! unit, both with the type of the value (a `DATE` stays a `DATE`, a time zone stays), and
//! the name of the unit, such as `'month'` or `'3 months'`. The field has the metadata key
//! `avenger.time_unit`.
//!
//! Time zones come from the type. A timestamp with a time zone is binned in civil time in
//! that zone. A `DATE` and a timestamp without a time zone hold civil time. The edges of a
//! unit shorter than a day are the instants whose local time is a multiple of the unit within
//! its parent unit: a local time that occurs two times gives two edges, and one that the zone
//! skips moves forward by the length of the skip. A unit of a day or longer starts at the
//! first instant of its first date. Collapsing units map each value into a reference year, a
//! leap year that starts on the first day of the week: 2024 for Monday, 2012 for Sunday.
//!
//! For a named time zone, a table of the offsets over the range of the values, which the
//! batches of a process share, holds each change of offset. It samples the offset once a day,
//! so it misses two changes within one day that cancel out.
//!
//! The functions:
//!
//! - `timeunit(x, ..options) OVER (..)`: the window function.
//! - `__timeunit(x, ..options)`: the scalar form, with a `unit`. DataFusion 55.2 plans a
//!   scalar function before a window function of the same name and drops the `OVER`
//!   clause, so [`sql`] renames `timeunit` calls without `OVER` to `__timeunit`.
//! - `__timeunit_choose(x, ..options)`: internal aggregate; the unit of a partition.
//! - `__timeunit_apply(x, unit, ..options)`: internal; the unit of each value.
//!
//! The rule [`TimeUnitWindowRewrite`] plans `timeunit(..) OVER (..)` as `__timeunit` (with a
//! `unit`) or as `__timeunit_apply` with the unit from a scalar subquery (no `PARTITION BY`)
//! or from a join with the unit of each partition.

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, LocalResult, Months, NaiveDate, NaiveDateTime,
    Offset, TimeZone, Timelike,
};
use datafusion::arrow::array::timezone::Tz;
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, DictionaryArray, Int64Array, StringArray, StructArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray, UInt8Array, UInt64Array,
};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{
    DataType, Date32Type, Field, FieldRef, Fields, Int32Type, Int64Type, TimeUnit as ArrowUnit,
    TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
    TimestampSecondType, UInt8Type, UInt64Type,
};
use datafusion::common::tree_node::Transformed;
use datafusion::common::{
    Column, JoinType, NullEquality, Result, ScalarValue, Spans, internal_err, plan_datafusion_err,
    plan_err,
};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::expr::WindowFunction;
use datafusion::logical_expr::function::{
    AccumulatorArgs, PartitionEvaluatorArgs, StateFieldsArgs, WindowUDFFieldArgs,
};
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, ColumnarValue, Expr, LogicalPlan,
    LogicalPlanBuilder, PartitionEvaluator, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF,
    ScalarUDFImpl, Signature, Subquery, Volatility, WindowUDFImpl,
};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};
use datafusion::physical_expr::expressions::Literal;
use datafusion::prelude::SessionContext;
use datafusion::sql::parser::{DFParser, Statement as DFStatement};
use datafusion::sql::sqlparser::ast::{
    Expr as SqlExpr, Ident, ObjectNamePart, visit_expressions_mut,
};
use datafusion::sql::sqlparser::dialect::GenericDialect;

pub const TAG: &str = "avenger.time_unit";

// ---------------------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------------------

/// A truncating unit: values go to the unit that contains them on the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Trunc {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
    FiscalQuarter,
    FiscalYear,
}

const TRUNCS: [(Trunc, &str); 11] = [
    (Trunc::Millisecond, "millisecond"),
    (Trunc::Second, "second"),
    (Trunc::Minute, "minute"),
    (Trunc::Hour, "hour"),
    (Trunc::Day, "day"),
    (Trunc::Week, "week"),
    (Trunc::Month, "month"),
    (Trunc::Quarter, "quarter"),
    (Trunc::Year, "year"),
    (Trunc::FiscalQuarter, "fiscal_quarter"),
    (Trunc::FiscalYear, "fiscal_year"),
];

/// A collapsing unit: the values of every period go to one reference calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Collapse {
    QuarterOfYear,
    FiscalQuarterOfYear,
    MonthOfYear,
    Monthdate,
    DayOfYear,
    WeekOfYear,
    DayOfMonth,
    DayOfWeek,
    HourOfDay,
    MinuteOfDay,
    SecondOfDay,
    TimeOfDay,
    MinuteOfHour,
    SecondOfMinute,
}

const COLLAPSES: [(Collapse, &str); 14] = [
    (Collapse::QuarterOfYear, "quarter_of_year"),
    (Collapse::FiscalQuarterOfYear, "fiscal_quarter_of_year"),
    (Collapse::MonthOfYear, "month_of_year"),
    (Collapse::Monthdate, "monthdate"),
    (Collapse::DayOfYear, "day_of_year"),
    (Collapse::WeekOfYear, "week_of_year"),
    (Collapse::DayOfMonth, "day_of_month"),
    (Collapse::DayOfWeek, "day_of_week"),
    (Collapse::HourOfDay, "hour_of_day"),
    (Collapse::MinuteOfDay, "minute_of_day"),
    (Collapse::SecondOfDay, "second_of_day"),
    (Collapse::TimeOfDay, "time_of_day"),
    (Collapse::MinuteOfHour, "minute_of_hour"),
    (Collapse::SecondOfMinute, "second_of_minute"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Trunc(Trunc, u32),
    Collapse(Collapse),
}

impl Unit {
    pub fn parse(text: &str) -> Result<Unit> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let (count, name) = match words[..] {
            [name] => (1, name),
            [count, name] => match count.parse::<u32>() {
                Ok(c) if c > 0 => (c, name),
                _ => return plan_err!("the count of the unit '{text}' must be a positive integer"),
            },
            _ => return plan_err!("unknown time unit '{text}'"),
        };
        let name = name.to_ascii_lowercase();
        if let Some((c, _)) = COLLAPSES.iter().find(|(_, n)| *n == name) {
            if words.len() > 1 {
                return plan_err!("the collapsing unit '{name}' takes no count");
            }
            return Ok(Unit::Collapse(*c));
        }
        let singular = name.strip_suffix('s').unwrap_or(&name);
        let found = TRUNCS.iter().find(|(_, n)| *n == name || *n == singular);
        match found {
            Some((t, _)) => {
                // A unit restarts at each of its parent unit, so a count past the size of the
                // parent is the parent. Weeks and years have no parent.
                let parent = match t {
                    Trunc::Millisecond => Some((1000, "second")),
                    Trunc::Second => Some((60, "minute")),
                    Trunc::Minute => Some((60, "hour")),
                    Trunc::Hour => Some((24, "day")),
                    Trunc::Day => Some((31, "month")),
                    Trunc::Month => Some((12, "year")),
                    Trunc::Quarter | Trunc::FiscalQuarter => Some((4, "year")),
                    Trunc::Week | Trunc::Year | Trunc::FiscalYear => None,
                };
                if let Some((max, parent)) = parent.filter(|(max, _)| count > *max) {
                    return plan_err!(
                        "the unit '{text}' restarts at each {parent}, so its count is at most {max}"
                    );
                }
                Ok(Unit::Trunc(*t, count))
            }
            None => {
                let truncs: Vec<&str> = TRUNCS.iter().map(|(_, n)| *n).collect();
                let collapses: Vec<&str> = COLLAPSES.iter().map(|(_, n)| *n).collect();
                plan_err!(
                    "unknown time unit '{text}': the truncating units are {}, with an optional count, and the collapsing units are {}",
                    truncs.join(", "),
                    collapses.join(", ")
                )
            }
        }
    }

    pub fn text(&self) -> String {
        match self {
            Unit::Trunc(t, k) => {
                let name = TRUNCS.iter().find(|(u, _)| u == t).unwrap().1;
                if *k == 1 {
                    name.to_string()
                } else {
                    format!("{k} {name}s")
                }
            }
            Unit::Collapse(c) => COLLAPSES
                .iter()
                .find(|(u, _)| u == c)
                .unwrap()
                .1
                .to_string(),
        }
    }

    /// Shorter than one day.
    pub fn is_subday(&self) -> bool {
        match self {
            Unit::Trunc(t, _) => *t <= Trunc::Hour,
            Unit::Collapse(c) => matches!(
                c,
                Collapse::HourOfDay
                    | Collapse::MinuteOfDay
                    | Collapse::SecondOfDay
                    | Collapse::TimeOfDay
                    | Collapse::MinuteOfHour
                    | Collapse::SecondOfMinute
            ),
        }
    }

    pub fn is_fiscal(&self) -> bool {
        matches!(
            self,
            Unit::Trunc(Trunc::FiscalQuarter | Trunc::FiscalYear, _)
                | Unit::Collapse(Collapse::FiscalQuarterOfYear)
        )
    }

    /// The approximate length in milliseconds: a month is 30 days and a year 365.
    pub fn approx_ms(&self) -> f64 {
        let Unit::Trunc(t, k) = self else {
            return f64::INFINITY;
        };
        let one = match t {
            Trunc::Millisecond => 1.0,
            Trunc::Second => 1e3,
            Trunc::Minute => 6e4,
            Trunc::Hour => 3.6e6,
            Trunc::Day => 8.64e7,
            Trunc::Week => 6.048e8,
            Trunc::Month => 2.592e9,
            Trunc::Quarter | Trunc::FiscalQuarter => 7.776e9,
            Trunc::Year | Trunc::FiscalYear => 3.1536e10,
        };
        one * *k as f64
    }

    /// The exact length in milliseconds of a unit of fixed length (up to weeks).
    fn exact_ms(&self) -> Option<i64> {
        match self {
            Unit::Trunc(t, _) if *t <= Trunc::Week => Some(self.approx_ms() as i64),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

impl WeekStart {
    /// A leap year whose January 1 is the first day of a week: the reference calendar of
    /// the collapsing units and the anchor of weeks with a count.
    pub fn reference_year(self) -> i32 {
        match self {
            WeekStart::Monday => 2024,
            WeekStart::Sunday => 2012,
        }
    }

    fn name(self) -> &'static str {
        match self {
            WeekStart::Monday => "monday",
            WeekStart::Sunday => "sunday",
        }
    }
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TimeUnitOptions {
    pub unit: Option<Unit>,
    pub maxbins: Option<u32>,
    pub min_unit: Option<Unit>,
    pub week_start: WeekStart,
    /// The first month of the fiscal year, 1 to 12.
    pub fiscal_year_start: Option<u32>,
}

impl TimeUnitOptions {
    pub fn parse(pairs: &[ScalarValue]) -> Result<Self> {
        if !pairs.len().is_multiple_of(2) {
            return plan_err!(
                "timeunit options must be pairs of a name and a value, but the last name has no value"
            );
        }
        let mut o = Self::default();
        let mut seen: Vec<&str> = Vec::new();
        for pair in pairs.chunks(2) {
            let Some(Some(name)) = pair[0].try_as_str() else {
                return plan_err!("a timeunit option name must be a string, not {}", pair[0]);
            };
            if seen.contains(&name) {
                return plan_err!("the timeunit option '{name}' is given twice");
            }
            seen.push(name);
            let v = &pair[1];
            let text = || -> Result<&str> {
                match v.try_as_str() {
                    Some(Some(s)) => Ok(s),
                    _ => plan_err!("the timeunit option '{name}' must be a string, not {v}"),
                }
            };
            match name {
                "unit" => o.unit = Some(Unit::parse(text()?)?),
                "min_unit" => {
                    let u = Unit::parse(text()?)?;
                    if matches!(u, Unit::Collapse(_)) {
                        return plan_err!(
                            "'min_unit' must be a truncating unit, not '{}'",
                            u.text()
                        );
                    }
                    o.min_unit = Some(u);
                }
                "maxbins" => {
                    let n = match v.cast_to(&DataType::Float64) {
                        Ok(ScalarValue::Float64(Some(f))) if v.data_type().is_numeric() => f,
                        _ => {
                            return plan_err!(
                                "the timeunit option 'maxbins' must be a number, not {v}"
                            );
                        }
                    };
                    if n < 1.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
                        return plan_err!(
                            "the timeunit option 'maxbins' must be a positive integer, not {v}"
                        );
                    }
                    o.maxbins = Some(n as u32);
                }
                "week_start" => {
                    o.week_start = match text()?.to_ascii_lowercase().as_str() {
                        "monday" => WeekStart::Monday,
                        "sunday" => WeekStart::Sunday,
                        other => {
                            return plan_err!(
                                "'week_start' must be 'monday' or 'sunday', not '{other}'"
                            );
                        }
                    }
                }
                "fiscal_year_start" => {
                    let m = text()?.to_ascii_lowercase();
                    let Some(i) = MONTHS.iter().position(|n| *n == m) else {
                        return plan_err!(
                            "'fiscal_year_start' must be the name of a month, such as 'october', not '{m}'"
                        );
                    };
                    o.fiscal_year_start = Some(i as u32 + 1);
                }
                _ => {
                    return plan_err!(
                        "unknown timeunit option '{name}': the options are unit, maxbins, min_unit, week_start, fiscal_year_start"
                    );
                }
            }
        }
        if o.unit.is_some() && o.maxbins.is_some() {
            return plan_err!(
                "timeunit takes 'unit' or 'maxbins', not both: an explicit unit has no automatic choice"
            );
        }
        if o.unit.is_some() && o.min_unit.is_some() {
            return plan_err!(
                "timeunit takes 'unit' or 'min_unit', not both: an explicit unit has no automatic choice"
            );
        }
        for u in o.unit.iter().chain(o.min_unit.iter()) {
            if u.is_fiscal() && o.fiscal_year_start.is_none() {
                return plan_err!(
                    "the unit '{}' needs 'fiscal_year_start'; without it, use '{}'",
                    u.text(),
                    u.text().replace("fiscal_", "")
                );
            }
        }
        Ok(o)
    }

    /// Checks the options against the type of the value.
    pub fn check(&self, dt: &DataType) -> Result<()> {
        for u in self.unit.iter().chain(self.min_unit.iter()) {
            if *dt == DataType::Date32 && u.is_subday() {
                return plan_err!(
                    "the unit '{}' is shorter than one day, so it does not apply to a DATE",
                    u.text()
                );
            }
            if matches!(dt, DataType::Timestamp(ArrowUnit::Second, _))
                && matches!(u, Unit::Trunc(Trunc::Millisecond, _))
            {
                return plan_err!(
                    "the unit '{}' is shorter than one second, so it does not apply to a timestamp in seconds",
                    u.text()
                );
            }
        }
        Ok(())
    }

    /// The value of the metadata key `avenger.time_unit`.
    pub fn tag(&self) -> String {
        let fiscal = self
            .fiscal_year_start
            .map(|m| format!(",\"fiscal_year_start\":\"{}\"", MONTHS[m as usize - 1]))
            .unwrap_or_default();
        format!("{{\"week_start\":\"{}\"{fiscal}}}", self.week_start.name())
    }
}

fn scalars_of_literals(
    exprs: &[Arc<dyn datafusion::physical_expr::PhysicalExpr>],
) -> Result<Vec<ScalarValue>> {
    exprs
        .iter()
        .map(|e| {
            e.downcast_ref::<Literal>()
                .map(|l| l.value().clone())
                .ok_or_else(|| plan_datafusion_err!("timeunit options must be constants, not {e}"))
        })
        .collect()
}

fn literals(exprs: &[Expr]) -> Result<Vec<ScalarValue>> {
    exprs
        .iter()
        .map(|e| match e {
            Expr::Literal(v, _) => Ok(v.clone()),
            _ => plan_err!("timeunit options must be constants, not {e}"),
        })
        .collect()
}

fn scalar_args(args: &[ColumnarValue]) -> Result<Vec<ScalarValue>> {
    args.iter()
        .map(|a| match a {
            ColumnarValue::Scalar(v) => Ok(v.clone()),
            ColumnarValue::Array(_) => plan_err!("timeunit options must be constants"),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Values: ticks, kinds, and zones
// ---------------------------------------------------------------------------------------

/// How an `i64` holds a value: days of a `DATE`, or the ticks of a timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Days,
    Ts(ArrowUnit),
}

const UNIX_DAYS_FROM_CE: i64 = 719_163;

impl Kind {
    fn per_second(self) -> i64 {
        match self {
            Kind::Days => 0,
            Kind::Ts(ArrowUnit::Second) => 1,
            Kind::Ts(ArrowUnit::Millisecond) => 1_000,
            Kind::Ts(ArrowUnit::Microsecond) => 1_000_000,
            Kind::Ts(ArrowUnit::Nanosecond) => 1_000_000_000,
        }
    }

    /// Ticks in one millisecond.
    fn per_ms(self) -> f64 {
        match self {
            Kind::Days => 1.0 / 8.64e7,
            _ => self.per_second() as f64 / 1e3,
        }
    }

    fn ndt(self, v: i64) -> NaiveDateTime {
        match self {
            Kind::Days => NaiveDate::from_num_days_from_ce_opt((v + UNIX_DAYS_FROM_CE) as i32)
                .expect("a date in range")
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            _ => {
                let ps = self.per_second();
                let s = v.div_euclid(ps);
                let sub = v.rem_euclid(ps) * (1_000_000_000 / ps);
                DateTime::from_timestamp(s, sub as u32)
                    .expect("a timestamp in range")
                    .naive_utc()
            }
        }
    }

    fn ticks(self, d: NaiveDateTime) -> i64 {
        match self {
            Kind::Days => d.date().num_days_from_ce() as i64 - UNIX_DAYS_FROM_CE,
            _ => {
                let ps = self.per_second();
                let u = d.and_utc();
                u.timestamp() * ps + u.timestamp_subsec_nanos() as i64 / (1_000_000_000 / ps)
            }
        }
    }

    /// The ticks of the values whose units and offsets the calendar can compute: the dates of
    /// chrono and the ticks of i64, less `margin_days` on each side.
    fn supported(self, margin_days: i64) -> (i64, i64) {
        let days = |d: NaiveDate| (d.num_days_from_ce() as i64 - UNIX_DAYS_FROM_CE) as i128;
        let (lo, hi) = (
            days(NaiveDate::MIN) + margin_days as i128,
            days(NaiveDate::MAX) - margin_days as i128,
        );
        let day = match self {
            Kind::Days => 1,
            k => 86_400 * k.per_second() as i128,
        };
        let clamp = |v: i128| v.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let margin = margin_days as i128 * day;
        (
            clamp((lo * day).max(i64::MIN as i128 + margin)),
            clamp((hi * day).min(i64::MAX as i128 - margin)),
        )
    }

    fn secs(self, v: i64) -> i64 {
        match self {
            Kind::Days => v * 86_400,
            _ => v.div_euclid(self.per_second()),
        }
    }
}

/// The time zone of the civil time of a value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Zone {
    /// A `DATE` or a timestamp without a time zone: the value is civil time.
    Civil,
    Fixed(FixedOffset),
    Named(Tz),
}

impl Zone {
    pub fn parse(tz: Option<&str>) -> Result<Zone> {
        let Some(tz) = tz else { return Ok(Zone::Civil) };
        if let Some(off) = parse_fixed(tz) {
            return Ok(Zone::Fixed(off));
        }
        match Tz::from_str(tz) {
            Ok(t) => Ok(Zone::Named(t)),
            Err(e) => plan_err!("unknown time zone '{tz}': {e}"),
        }
    }
}

fn parse_fixed(tz: &str) -> Option<FixedOffset> {
    if matches!(tz, "UTC" | "utc" | "Z" | "z") {
        return FixedOffset::east_opt(0);
    }
    let (sign, rest) = match tz.as_bytes().first()? {
        b'+' => (1, &tz[1..]),
        b'-' => (-1, &tz[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

/// Offsets and local times of a zone, with caches.
#[derive(Debug)]
struct Clock {
    zone: Zone,
    /// A 15-minute UTC bucket in which the offset does not change, and the offset.
    bucket: i64,
    bucket_off: Option<i32>,
    /// The offsets of a range of instants: the first second of each offset, and the offset.
    table: Option<Arc<OffsetTable>>,
    /// The name of a named zone, the key of the offset tables that calls share.
    name: Option<Arc<str>>,
    /// Instants of local times.
    resolved: HashMap<NaiveDateTime, (NaiveDateTime, Option<NaiveDateTime>)>,
}

#[derive(Debug)]
struct OffsetTable {
    lo: i64,
    hi: i64,
    starts: Vec<i64>,
    offsets: Vec<i32>,
}

const BUCKET: i64 = 900;

impl Clock {
    fn new(zone: Zone, name: Option<&str>) -> Self {
        Self {
            zone,
            bucket: i64::MIN,
            bucket_off: None,
            table: None,
            name: name.map(Arc::from),
            resolved: HashMap::new(),
        }
    }

    fn is_named(&self) -> bool {
        matches!(self.zone, Zone::Named(_))
    }

    /// Makes the table of the offsets of the instants from `lo` to `hi` (seconds), with two
    /// days more on each side. It samples the offset at each day and finds each change to
    /// the second, so it misses two changes within one day that cancel out.
    fn prepare(&mut self, lo: i64, hi: i64) {
        let Zone::Named(tz) = self.zone else { return };
        let (mut lo, mut hi) = (lo - 2 * 86_400, hi + 2 * 86_400);
        if self
            .table
            .as_ref()
            .is_some_and(|t| t.lo <= lo && hi <= t.hi)
        {
            return;
        }
        // The batches of one query share their tables.
        static TABLES: OnceLock<Mutex<HashMap<Arc<str>, Arc<OffsetTable>>>> = OnceLock::new();
        let tables = TABLES.get_or_init(Default::default);
        let name = self.name.clone().expect("a named zone has a name");
        if let Some(t) = tables
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&name)
        {
            if t.lo <= lo && hi <= t.hi {
                self.table = Some(t.clone());
                return;
            }
            (lo, hi) = (lo.min(t.lo), hi.max(t.hi));
        }
        let at = |s: i64| {
            let t = DateTime::from_timestamp(s, 0)
                .expect("in range")
                .naive_utc();
            tz.offset_from_utc_datetime(&t).fix().local_minus_utc()
        };
        let mut starts = vec![lo];
        let mut offsets = vec![at(lo)];
        let mut a = lo;
        while a < hi {
            let b = (a + 86_400).min(hi);
            let off = at(b);
            if off != *offsets.last().unwrap() {
                // The first second of the new offset.
                let (mut x, mut y) = (a, b);
                while y - x > 1 {
                    let m = x + (y - x) / 2;
                    if at(m) == off { y = m } else { x = m }
                }
                starts.push(y);
                offsets.push(off);
            }
            a = b;
        }
        let table = Arc::new(OffsetTable {
            lo,
            hi,
            starts,
            offsets,
        });
        tables
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name, table.clone());
        self.table = Some(table);
    }

    /// The offset in seconds from UTC at an instant.
    fn offset(&mut self, utc_secs: i64) -> i32 {
        let tz = match self.zone {
            Zone::Civil => return 0,
            Zone::Fixed(o) => return o.local_minus_utc(),
            Zone::Named(tz) => tz,
        };
        if let Some(t) = &self.table
            && t.lo <= utc_secs
            && utc_secs <= t.hi
        {
            return t.offsets[t.starts.partition_point(|s| *s <= utc_secs) - 1];
        }
        let b = utc_secs.div_euclid(BUCKET);
        if b == self.bucket
            && let Some(o) = self.bucket_off
        {
            return o;
        }
        let at = |s: i64| {
            let t = DateTime::from_timestamp(s, 0)
                .expect("in range")
                .naive_utc();
            tz.offset_from_utc_datetime(&t).fix().local_minus_utc()
        };
        let (lo, hi) = (at(b * BUCKET), at(b * BUCKET + BUCKET - 1));
        if lo == hi {
            self.bucket = b;
            self.bucket_off = Some(lo);
            lo
        } else {
            at(utc_secs)
        }
    }

    /// The offset at an instant, and the seconds `[lo, hi)` around it in which it holds.
    fn span(&mut self, utc_secs: i64) -> (i32, i64, i64) {
        match self.zone {
            Zone::Civil => return (0, i64::MIN, i64::MAX),
            Zone::Fixed(o) => return (o.local_minus_utc(), i64::MIN, i64::MAX),
            Zone::Named(_) => {}
        }
        if let Some(t) = &self.table
            && t.lo <= utc_secs
            && utc_secs <= t.hi
        {
            let i = t.starts.partition_point(|s| *s <= utc_secs) - 1;
            return (
                t.offsets[i],
                t.starts[i],
                t.starts.get(i + 1).copied().unwrap_or(t.hi),
            );
        }
        let off = self.offset(utc_secs);
        let b = utc_secs.div_euclid(BUCKET);
        if b == self.bucket && self.bucket_off.is_some() {
            (off, b * BUCKET, b * BUCKET + BUCKET)
        } else {
            (off, utc_secs, utc_secs + 1)
        }
    }

    /// The first instant of a local time, both in ticks of `kind`.
    fn first_instant_ticks(&mut self, local: i64, kind: Kind) -> i64 {
        let ps = kind.per_second();
        match self.zone {
            Zone::Civil => return local,
            Zone::Fixed(o) => return local - o.local_minus_utc() as i64 * ps,
            Zone::Named(_) => {}
        }
        if let Some(t) = &self.table {
            let ls = local.div_euclid(ps);
            if t.lo + 86_400 <= ls && ls <= t.hi - 86_400 {
                // The offsets of the instants within 14 hours of the local time, in order:
                // the first that maps the local time into its own interval.
                let a = t
                    .starts
                    .partition_point(|s| *s <= ls - 50_400)
                    .saturating_sub(1);
                let b = t.starts.partition_point(|s| *s <= ls + 50_400);
                for i in a..b {
                    let inst = ls - t.offsets[i] as i64;
                    let end = t.starts.get(i + 1).copied().unwrap_or(i64::MAX);
                    if t.starts[i] <= inst && inst < end {
                        return local - t.offsets[i] as i64 * ps;
                    }
                }
            }
        }
        // A skipped local time, or one outside the table.
        let n = kind.ndt(local);
        kind.ticks(self.first_instant(n))
    }

    /// The instants of a local time: one, or two for a local time that occurs two times. A
    /// local time that the zone skips moves forward by the length of the skip.
    fn resolve(&mut self, local: NaiveDateTime) -> (NaiveDateTime, Option<NaiveDateTime>) {
        let tz = match self.zone {
            Zone::Civil => return (local, None),
            Zone::Fixed(o) => return (local - Duration::seconds(o.local_minus_utc() as i64), None),
            Zone::Named(tz) => tz,
        };
        if let Some(r) = self.resolved.get(&local) {
            return *r;
        }
        let r = match tz.from_local_datetime(&local) {
            LocalResult::Single(d) => (d.naive_utc(), None),
            LocalResult::Ambiguous(a, b) => {
                let (a, b) = (a.naive_utc(), b.naive_utc());
                (a.min(b), Some(a.max(b)))
            }
            LocalResult::None => {
                let before = tz
                    .offset_from_utc_datetime(&(local - Duration::days(1)))
                    .fix();
                (
                    local - Duration::seconds(before.local_minus_utc() as i64),
                    None,
                )
            }
        };
        if self.resolved.len() > 1 << 16 {
            self.resolved.clear();
        }
        self.resolved.insert(local, r);
        r
    }

    fn first_instant(&mut self, local: NaiveDateTime) -> NaiveDateTime {
        self.resolve(local).0
    }
}

fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_hms_opt(0, 0, 0).unwrap()
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a valid date")
}

fn add_months(d: NaiveDate, m: i64) -> NaiveDate {
    if m >= 0 {
        d.checked_add_months(Months::new(m as u32))
    } else {
        d.checked_sub_months(Months::new((-m) as u32))
    }
    .expect("a date in range")
}

// ---------------------------------------------------------------------------------------
// The calendar of a call
// ---------------------------------------------------------------------------------------

/// The calendar arithmetic of one call: the kind and the zone of the values, and the
/// options that move unit boundaries.
#[derive(Debug)]
pub(crate) struct Calendar {
    kind: Kind,
    clock: Clock,
    week_start: WeekStart,
    fiscal: Option<u32>,
}

impl Calendar {
    pub fn new(dt: &DataType, o: &TimeUnitOptions) -> Result<Self> {
        let (kind, zone) = match dt {
            DataType::Date32 => (Kind::Days, Zone::Civil),
            DataType::Timestamp(u, tz) => (Kind::Ts(*u), Zone::parse(tz.as_deref())?),
            other => return plan_err!("timeunit takes a DATE or a TIMESTAMP, not {other}"),
        };
        o.check(dt)?;
        let name = match dt {
            DataType::Timestamp(_, Some(tz)) => Some(tz.as_ref()),
            _ => None,
        };
        Ok(Self {
            kind,
            clock: Clock::new(zone, name),
            week_start: o.week_start,
            fiscal: o.fiscal_year_start,
        })
    }

    /// Ticks in one day; a `DATE` counts days.
    fn day_ticks(&self) -> i64 {
        match self.kind {
            Kind::Days => 1,
            k => 86_400 * k.per_second(),
        }
    }

    /// Local time in ticks.
    fn local_ticks(&mut self, t: i64) -> i64 {
        match self.kind {
            Kind::Days => t,
            k => t + self.clock.offset(k.secs(t)) as i64 * k.per_second(),
        }
    }

    /// The first instant of a local time, in ticks.
    fn instant(&mut self, local: i64) -> i64 {
        self.clock.first_instant_ticks(local, self.kind)
    }

    /// The first instant of a date.
    fn date_instant(&mut self, d: NaiveDate) -> i64 {
        let days = d.num_days_from_ce() as i64 - UNIX_DAYS_FROM_CE;
        self.instant(days * self.day_ticks())
    }

    /// The local date of a value.
    fn local_date(&mut self, t: i64) -> NaiveDate {
        let days = self.local_ticks(t).div_euclid(self.day_ticks());
        Kind::Days.ndt(days).date()
    }

    /// The first instant of a fiscal year that contains `d`.
    fn fiscal_year_of(&self, d: NaiveDate) -> NaiveDate {
        let f = self.fiscal.expect("a fiscal unit has fiscal_year_start");
        let off = (d.month0() as i64 - (f as i64 - 1)).rem_euclid(12);
        add_months(ymd(d.year(), d.month(), 1), -off)
    }

    /// The first day of a unit of one day or longer that contains `d`, and of the next.
    fn civil_dates(&self, d: NaiveDate, t: Trunc, k: u32) -> (NaiveDate, NaiveDate) {
        let k64 = k as i64;
        match t {
            Trunc::Day => {
                let first = ymd(d.year(), d.month(), 1);
                let s = first + Duration::days((d.day0() / k * k) as i64);
                (s, (s + Duration::days(k64)).min(add_months(first, 1)))
            }
            Trunc::Week => {
                let r = ymd(self.week_start.reference_year(), 1, 1);
                let w = (d - r).num_days().div_euclid(7);
                let s = r + Duration::days(7 * (w - w.rem_euclid(k64)));
                (s, s + Duration::days(7 * k64))
            }
            Trunc::Month | Trunc::Quarter => {
                let k = if t == Trunc::Quarter { 3 * k } else { k };
                let s = ymd(d.year(), d.month0() / k * k + 1, 1);
                (s, add_months(s, k as i64).min(ymd(d.year() + 1, 1, 1)))
            }
            Trunc::Year => {
                let y = d.year().div_euclid(k as i32) * k as i32;
                (ymd(y, 1, 1), ymd(y + k as i32, 1, 1))
            }
            Trunc::FiscalQuarter => {
                let fy = self.fiscal_year_of(d);
                let q = ((d.year() - fy.year()) as i64 * 12 + d.month0() as i64
                    - fy.month0() as i64)
                    / 3;
                let s = add_months(fy, 3 * (q - q % k64));
                (s, add_months(s, 3 * k64).min(add_months(fy, 12)))
            }
            Trunc::FiscalYear => {
                let f = self.fiscal.unwrap();
                let fy = self.fiscal_year_of(d);
                // A fiscal year is named for the calendar year in which it ends.
                let shift = if f > 1 { 1 } else { 0 };
                let n = fy.year() + shift;
                let n0 = n.div_euclid(k as i32) * k as i32;
                let s = ymd(n0 - shift, f, 1);
                (s, add_months(s, 12 * k64))
            }
            _ => unreachable!("a unit shorter than one day"),
        }
    }

    /// The bounds of the unit that contains `t`, in ticks: `[start, end)`.
    pub fn bounds(&mut self, t: i64, unit: Trunc, k: u32) -> (i64, i64) {
        if unit >= Trunc::Day {
            let d = self.local_date(t);
            let (s, e) = self.civil_dates(d, unit, k);
            return (self.date_instant(s), self.date_instant(e));
        }
        let secs = self.kind.secs(t);
        let (off, lo, hi) = self.clock.span(secs);
        if let Some((s, e)) = self.subday_ticks(t, off, unit, k) {
            // The unit needs the offset of the value from its start to its end, and past its
            // end: a change of offset at the end can drop the local time of the end, as on Lord
            // Howe Island, where 02:00 becomes 01:30 and the hour from 01:00 lasts 90 minutes.
            // Milliseconds start at whole seconds, where every offset changes.
            let ps = self.kind.per_second();
            if unit == Trunc::Millisecond || (lo <= s.div_euclid(ps) && e.div_euclid(ps) < hi) {
                return (s, e);
            }
        }
        let delta = Duration::seconds(off as i64);
        let l = floor_sub(self.kind.ndt(t) + delta, unit, k);
        let (s, e) = (l - delta, next_sub(l, unit, k) - delta);
        if !self.clock.is_named()
            || unit == Trunc::Millisecond
            || (self.clock.offset(s.and_utc().timestamp()) == off
                && self.clock.offset(e.and_utc().timestamp()) == off)
        {
            return (self.kind.ticks(s), self.kind.ticks(e));
        }
        // A change of the offset: the edges of the local times around the value.
        let span = match unit {
            Trunc::Hour => Duration::days(1),
            Trunc::Minute => Duration::hours(3),
            _ => Duration::minutes(15),
        };
        let edges = self.sub_edges(l - span, l + span, unit, k);
        let i = edges.partition_point(|e| *e <= t);
        (edges[i - 1], edges[i])
    }

    /// The bounds of a unit shorter than one day with the offset of the value throughout,
    /// in integer arithmetic on ticks, or None when the unit is not a whole number of ticks.
    fn subday_ticks(&self, t: i64, off: i32, unit: Trunc, k: u32) -> Option<(i64, i64)> {
        let ps = self.kind.per_second();
        let (one, parent) = match unit {
            Trunc::Hour => (3600 * ps, 86_400 * ps),
            Trunc::Minute => (60 * ps, 3600 * ps),
            Trunc::Second => (ps, 60 * ps),
            Trunc::Millisecond if ps >= 1000 => (ps / 1000, ps),
            _ => return None,
        };
        let shift = off as i64 * ps;
        let local = t + shift;
        let p = local - local.rem_euclid(parent);
        let step = one * k as i64;
        let s = p + (local - p) / step * step;
        let e = (s + step).min(p + parent);
        Some((s - shift, e - shift))
    }

    /// All edges of a unit shorter than one day between two local times, sorted: the
    /// instants of each local multiple of the unit.
    fn sub_edges(&mut self, lo: NaiveDateTime, hi: NaiveDateTime, unit: Trunc, k: u32) -> Vec<i64> {
        let mut edges = Vec::new();
        let mut l = floor_sub(lo, unit, k);
        while l <= hi {
            let (a, b) = self.clock.resolve(l);
            edges.push(self.kind.ticks(a));
            if let Some(b) = b {
                edges.push(self.kind.ticks(b));
            }
            l = next_sub(l, unit, k);
        }
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    /// The edges of a unit of a day or longer over `[lo, hi]`, or None when there would be
    /// more than `limit`.
    fn edges(&mut self, lo: i64, hi: i64, unit: Trunc, k: u32, limit: usize) -> Option<Vec<i64>> {
        let span_ms = hi.abs_diff(lo) as f64 / self.kind.per_ms();
        let per_unit = Unit::Trunc(unit, k).approx_ms();
        if span_ms / per_unit > limit as f64 {
            return None;
        }
        let first = self.local_date(lo);
        let mut d = self.civil_dates(first, unit, k).0;
        let mut edges = Vec::new();
        loop {
            let e = self.date_instant(d);
            edges.push(e);
            if e > hi {
                break;
            }
            d = self.civil_dates(d, unit, k).1;
        }
        Some(edges)
    }

    /// The reference unit of a collapsing unit, as local times in ticks.
    fn collapse(&mut self, t: i64, c: Collapse) -> (i64, i64) {
        let dt = self.day_ticks();
        let lt = self.local_ticks(t);
        let (days, tod) = (lt.div_euclid(dt), lt.rem_euclid(dt));
        let (y, m, d) = civil_from_days(days);
        let ry = self.week_start.reference_year() as i64;
        let jan1 = days_from_civil(ry, 1, 1);
        let week_index = |days: i64| match self.week_start {
            WeekStart::Monday => (days + 3).rem_euclid(7),
            WeekStart::Sunday => (days + 4).rem_euclid(7),
        };
        // The first day of the month with index `y * 12 + month0`.
        let month = |i: i64| days_from_civil(i.div_euclid(12), i.rem_euclid(12) as u32 + 1, 1);
        let ps = self.kind.per_second();
        let time = |unit: i64| {
            let s = jan1 * dt + tod / unit * unit;
            (s, s + unit)
        };
        let (s, e) = match c {
            Collapse::QuarterOfYear => {
                let i = ry * 12 + (m as i64 - 1) / 3 * 3;
                (month(i), month(i + 3))
            }
            Collapse::FiscalQuarterOfYear => {
                let f = self.fiscal.expect("a fiscal unit has fiscal_year_start") as i64;
                let q = (m as i64 - f).rem_euclid(12) / 3;
                let i = ry * 12 + f - 1 + 3 * q;
                (month(i), month(i + 3))
            }
            Collapse::MonthOfYear => {
                let i = ry * 12 + m as i64 - 1;
                (month(i), month(i + 1))
            }
            Collapse::Monthdate => {
                let s = days_from_civil(ry, m, d);
                (s, s + 1)
            }
            Collapse::DayOfYear => {
                let s = jan1 + days - days_from_civil(y, 1, 1);
                (s, s + 1)
            }
            Collapse::WeekOfYear => {
                let jan = days_from_civil(y, 1, 1);
                let first = jan + (7 - week_index(jan)) % 7;
                let w = if days < first {
                    0
                } else {
                    (days - first) / 7 + 1
                };
                let s = jan1 + 7 * (w - 1);
                (s, s + 7)
            }
            Collapse::DayOfMonth => (jan1 + d as i64 - 1, jan1 + d as i64),
            Collapse::DayOfWeek => (jan1 + week_index(days), jan1 + week_index(days) + 1),
            Collapse::HourOfDay => return time(3600 * ps),
            Collapse::MinuteOfDay => return time(60 * ps),
            Collapse::SecondOfDay => return time(ps),
            Collapse::TimeOfDay => return time((ps / 1000).max(1)),
            Collapse::MinuteOfHour => {
                let s = jan1 * dt + tod % (3600 * ps) / (60 * ps) * (60 * ps);
                return (s, s + 60 * ps);
            }
            Collapse::SecondOfMinute => {
                let s = jan1 * dt + tod % (60 * ps) / ps * ps;
                return (s, s + ps);
            }
        };
        (s * dt, e * dt)
    }

    /// The finest calendar field of a local time that is not at its start: 0 for a
    /// fraction of a second, then second, minute, hour, day, month, quarter, and 7 for none.
    fn rank(&self, local_ticks: i64) -> u8 {
        let (sub, tod, days) = match self.kind {
            Kind::Days => (0, 0, local_ticks),
            k => {
                let ps = k.per_second();
                let secs = local_ticks.div_euclid(ps);
                (
                    local_ticks.rem_euclid(ps),
                    secs.rem_euclid(86_400),
                    secs.div_euclid(86_400),
                )
            }
        };
        if sub != 0 {
            return 0;
        }
        if tod % 60 != 0 {
            return 1;
        }
        if tod % 3600 != 0 {
            return 2;
        }
        if tod != 0 {
            return 3;
        }
        let d = Kind::Days.ndt(days).date();
        if d.day() != 1 {
            return 4;
        }
        let off = match self.fiscal {
            Some(f) => (d.month0() as i64 - (f as i64 - 1)).rem_euclid(12),
            None => d.month0() as i64,
        };
        if off % 3 != 0 {
            5
        } else if off != 0 {
            6
        } else {
            7
        }
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn floor_sub(l: NaiveDateTime, unit: Trunc, k: u32) -> NaiveDateTime {
    let d = l.date();
    let (h, m, s) = (l.hour(), l.minute(), l.second());
    let ms = l.nanosecond() / 1_000_000 % 1000;
    match unit {
        Trunc::Hour => d.and_hms_opt(h / k * k, 0, 0),
        Trunc::Minute => d.and_hms_opt(h, m / k * k, 0),
        Trunc::Second => d.and_hms_opt(h, m, s / k * k),
        Trunc::Millisecond => d.and_hms_milli_opt(h, m, s, ms / k * k),
        _ => unreachable!(),
    }
    .unwrap()
}

/// The next local multiple after a multiple: the next multiple in its parent unit, or the
/// start of the next parent unit.
fn next_sub(l: NaiveDateTime, unit: Trunc, k: u32) -> NaiveDateTime {
    let d = l.date();
    let (h, m, s) = (l.hour(), l.minute(), l.second());
    let ms = l.nanosecond() / 1_000_000 % 1000;
    match unit {
        Trunc::Hour if h + k < 24 => d.and_hms_opt(h + k, 0, 0).unwrap(),
        Trunc::Hour => midnight(d + Duration::days(1)),
        Trunc::Minute if m + k < 60 => d.and_hms_opt(h, m + k, 0).unwrap(),
        Trunc::Minute => d.and_hms_opt(h, 0, 0).unwrap() + Duration::hours(1),
        Trunc::Second if s + k < 60 => d.and_hms_opt(h, m, s + k).unwrap(),
        Trunc::Second => d.and_hms_opt(h, m, 0).unwrap() + Duration::minutes(1),
        Trunc::Millisecond if ms + k < 1000 => d.and_hms_milli_opt(h, m, s, ms + k).unwrap(),
        Trunc::Millisecond => d.and_hms_opt(h, m, s).unwrap() + Duration::seconds(1),
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------------------
// Applying units to arrays
// ---------------------------------------------------------------------------------------

fn ticks_of(a: &ArrayRef) -> Result<Vec<i64>> {
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

fn temporal(dt: &DataType, v: Vec<i64>) -> ArrayRef {
    match dt {
        DataType::Date32 => Arc::new(Date32Array::from(
            v.into_iter().map(|x| x as i32).collect::<Vec<_>>(),
        )),
        DataType::Timestamp(u, tz) => match u {
            ArrowUnit::Second => {
                Arc::new(TimestampSecondArray::from(v).with_timezone_opt(tz.clone()))
            }
            ArrowUnit::Millisecond => {
                Arc::new(TimestampMillisecondArray::from(v).with_timezone_opt(tz.clone()))
            }
            ArrowUnit::Microsecond => {
                Arc::new(TimestampMicrosecondArray::from(v).with_timezone_opt(tz.clone()))
            }
            ArrowUnit::Nanosecond => {
                Arc::new(TimestampNanosecondArray::from(v).with_timezone_opt(tz.clone()))
            }
        },
        _ => unreachable!(),
    }
}

pub fn result_fields(dt: &DataType) -> Fields {
    Fields::from(vec![
        Field::new("start", dt.clone(), false),
        Field::new("end", dt.clone(), false),
        Field::new(
            "unit",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
    ])
}

fn result_field(name: &str, dt: &DataType, nullable: bool, tag: String) -> FieldRef {
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
            temporal(&dt, starts),
            temporal(&dt, ends),
            Arc::new(unit_array),
        ],
        (nulls.null_count() > 0).then_some(nulls),
    )?))
}

/// Makes the offset table of a zone for the range of the values.
fn prepare(cal: &mut Calendar, x: &ArrayRef, ticks: &[i64]) {
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
fn check_range(
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

fn apply_unit(
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

// ---------------------------------------------------------------------------------------
// Choosing a unit
// ---------------------------------------------------------------------------------------

/// What the choice of a unit needs to know about the values of a partition. All of it
/// merges, so it is the state of an aggregate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stats {
    pub n: u64,
    pub min: i64,
    pub max: i64,
    /// The finest calendar field of any value in civil time (`Calendar::rank`).
    pub rank: u8,
    /// The greatest common divisor of the differences of the values in civil time, in ticks.
    pub gcd: u64,
    /// One value in civil time, for the differences.
    pub reference: i64,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            n: 0,
            min: i64::MAX,
            max: i64::MIN,
            rank: 7,
            gcd: 0,
            reference: 0,
        }
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Stats {
    fn add(&mut self, cal: &mut Calendar, t: i64) {
        let local = cal.local_ticks(t);
        if self.n == 0 {
            self.reference = local;
        } else {
            self.gcd = gcd(self.gcd, local.abs_diff(self.reference));
        }
        self.n += 1;
        self.min = self.min.min(t);
        self.max = self.max.max(t);
        if self.rank > 0 {
            self.rank = self.rank.min(cal.rank(local));
        }
    }

    pub fn merge(&mut self, o: &Stats) {
        if o.n == 0 {
            return;
        }
        if self.n == 0 {
            *self = *o;
            return;
        }
        self.gcd = gcd(gcd(self.gcd, o.gcd), self.reference.abs_diff(o.reference));
        self.n += o.n;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.rank = self.rank.min(o.rank);
    }

    pub fn of(x: &ArrayRef, cal: &mut Calendar) -> Result<Stats> {
        let mut s = Stats::default();
        s.add_array(x, cal)?;
        Ok(s)
    }

    /// Adds the values of an array.
    fn add_array(&mut self, x: &ArrayRef, cal: &mut Calendar) -> Result<()> {
        let ticks = ticks_of(x)?;
        check_range(
            cal,
            x.data_type(),
            &ticks,
            (0..ticks.len()).filter(|i| x.is_valid(*i)),
            None,
        )?;
        prepare(cal, x, &ticks);
        for (i, t) in ticks.into_iter().enumerate() {
            if x.is_valid(i) {
                self.add(cal, t);
            }
        }
        Ok(())
    }
}

/// The automatic units, in order. Years go on as 1, 2, or 5 times a power of ten.
fn table(fiscal: bool) -> Vec<Unit> {
    let mut t = Vec::new();
    let mut add = |u: Trunc, ks: &[u32]| t.extend(ks.iter().map(|k| Unit::Trunc(u, *k)));
    add(Trunc::Millisecond, &[1, 2, 5, 10, 20, 50, 100, 200, 500]);
    add(Trunc::Second, &[1, 5, 15, 30]);
    add(Trunc::Minute, &[1, 5, 15, 30]);
    add(Trunc::Hour, &[1, 3, 6, 12]);
    add(Trunc::Day, &[1, 2]);
    add(Trunc::Week, &[1]);
    add(Trunc::Month, &[1]);
    if fiscal {
        add(Trunc::FiscalQuarter, &[1]);
        add(Trunc::FiscalYear, &[1, 2, 5, 10]);
    } else {
        add(Trunc::Quarter, &[1]);
        add(Trunc::Year, &[1, 2, 5, 10]);
    }
    t
}

fn first_at_least(target_ms: f64, fiscal: bool) -> Unit {
    if let Some(u) = table(fiscal)
        .into_iter()
        .find(|u| u.approx_ms() >= target_ms)
    {
        return u;
    }
    let year = if fiscal {
        Trunc::FiscalYear
    } else {
        Trunc::Year
    };
    let mut p = 10u32;
    loop {
        for m in [2, 5, 10] {
            let u = Unit::Trunc(year, p * m);
            // Past a billion years, the range check of the unit fails.
            if u.approx_ms() >= target_ms || p >= 100_000_000 {
                return u;
            }
        }
        p *= 10;
    }
}

fn longer(a: Unit, b: Unit) -> Unit {
    if b.approx_ms() > a.approx_ms() { b } else { a }
}

/// The shortest unit that the resolution of the data permits: the unit of the finest
/// calendar field of the values, or a unit whose length is a multiple of the common
/// difference of the values, whichever is longer.
pub(crate) fn resolution_floor(s: &Stats, kind: Kind, fiscal: bool) -> Unit {
    use Trunc::*;
    let mut floor = match s.rank {
        0 => Unit::Trunc(Millisecond, 1),
        1 => Unit::Trunc(Second, 1),
        2 => Unit::Trunc(Minute, 1),
        3 => Unit::Trunc(Hour, 1),
        4 => Unit::Trunc(Day, 1),
        5 => Unit::Trunc(Month, 1),
        6 => Unit::Trunc(if fiscal { FiscalQuarter } else { Quarter }, 1),
        _ => Unit::Trunc(if fiscal { FiscalYear } else { Year }, 1),
    };
    if s.rank <= 4 && s.gcd > 0 {
        let g = s.gcd as f64;
        let ticks = |u: &Unit| u.exact_ms().map(|ms| ms as f64 * kind.per_ms());
        let units = table(fiscal);
        // A unit that holds the same number of values, up to 12 times the difference.
        let multiple = units.iter().find(|u| {
            ticks(u).is_some_and(|t| {
                t >= g && t <= 12.0 * g && t.fract() == 0.0 && (t as u64).is_multiple_of(s.gcd)
            })
        });
        let at_least = || {
            units
                .iter()
                .find(|u| u.approx_ms() * kind.per_ms() >= g)
                .copied()
        };
        if let Some(u) = multiple.copied().or_else(at_least) {
            floor = longer(floor, u);
        }
    }
    if kind == Kind::Days {
        floor = longer(floor, Unit::Trunc(Day, 1));
    }
    floor
}

/// The unit of a partition: the first unit of `table` with at least
/// `(max - min) / maxbins`, and no shorter than the floor.
pub(crate) fn choose(s: &Stats, o: &TimeUnitOptions, kind: Kind) -> Option<Unit> {
    if s.n == 0 {
        return None;
    }
    let fiscal = o.fiscal_year_start.is_some();
    let floor = match o.min_unit {
        Some(u) => u,
        None => resolution_floor(s, kind, fiscal),
    };
    let span_ms = s.max.abs_diff(s.min) as f64 / kind.per_ms();
    if span_ms == 0.0 {
        return Some(longer(Unit::Trunc(Trunc::Day, 1), floor));
    }
    let target = span_ms / o.maxbins.unwrap_or(20) as f64;
    if target <= floor.approx_ms() {
        return Some(floor);
    }
    Some(first_at_least(target, fiscal))
}

// ---------------------------------------------------------------------------------------
// DataFusion functions
// ---------------------------------------------------------------------------------------

/// `timeunit(x, ..options) OVER (..)`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TimeUnitWindow {
    signature: Signature,
}

impl Default for TimeUnitWindow {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl WindowUDFImpl for TimeUnitWindow {
    fn name(&self) -> &str {
        "timeunit"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn partition_evaluator(
        &self,
        args: PartitionEvaluatorArgs,
    ) -> Result<Box<dyn PartitionEvaluator>> {
        let options = TimeUnitOptions::parse(&scalars_of_literals(&args.input_exprs()[1..])?)?;
        options.check(args.input_fields()[0].data_type())?;
        Ok(Box::new(TimeUnitEvaluator { options }))
    }
    fn field(&self, args: WindowUDFFieldArgs) -> Result<FieldRef> {
        let Some(x) = args.input_fields().first() else {
            return plan_err!("timeunit needs a value");
        };
        Calendar::new(x.data_type(), &TimeUnitOptions::default())?;
        // A window function does not see its constant arguments here, so the metadata
        // cannot hold the options. The rule plans the call as `__timeunit_apply`, which
        // sets them.
        Ok(result_field(
            args.name(),
            x.data_type(),
            x.is_nullable(),
            "{}".to_string(),
        ))
    }
}

#[derive(Debug)]
struct TimeUnitEvaluator {
    options: TimeUnitOptions,
}

impl PartitionEvaluator for TimeUnitEvaluator {
    fn evaluate_all(&mut self, values: &[ArrayRef], _num_rows: usize) -> Result<ArrayRef> {
        let x = &values[0];
        let unit = match self.options.unit {
            Some(u) => Some(u),
            None => {
                let mut cal = Calendar::new(x.data_type(), &self.options)?;
                let stats = Stats::of(x, &mut cal)?;
                choose(&stats, &self.options, cal.kind)
            }
        };
        apply(x, &self.options, |_| unit)
    }
}

/// `__timeunit(x, ..options)`: the scalar form, which needs a `unit`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitScalar {
    signature: Signature,
}

impl Default for TimeUnitScalar {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

fn scalar_options(args: &ReturnFieldArgs, from: usize) -> Result<Option<TimeUnitOptions>> {
    let options: Option<Vec<ScalarValue>> = args.scalar_arguments[from..]
        .iter()
        .map(|v| v.cloned())
        .collect();
    options.map(|o| TimeUnitOptions::parse(&o)).transpose()
}

impl ScalarUDFImpl for TimeUnitScalar {
    fn name(&self) -> &str {
        "__timeunit"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        internal_err!(
            "{} computes its field in return_field_from_args",
            self.name()
        )
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let Some(x) = args.arg_fields.first() else {
            return plan_err!("timeunit needs a value");
        };
        let o = scalar_options(&args, 1)?.unwrap_or_default();
        if o.unit.is_none() && args.scalar_arguments[1..].iter().all(|v| v.is_some()) {
            return plan_err!(
                "timeunit without 'unit' chooses a unit from other rows, so it needs an OVER clause, for example timeunit(t) OVER ()"
            );
        }
        Calendar::new(x.data_type(), &o)?;
        Ok(result_field(
            self.name(),
            x.data_type(),
            x.is_nullable(),
            o.tag(),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let o = TimeUnitOptions::parse(&scalar_args(&args.args[1..])?)?;
        let Some(unit) = o.unit else {
            return plan_err!("timeunit without 'unit' needs an OVER clause");
        };
        let x = args.args[0].to_array(args.number_rows)?;
        Ok(ColumnarValue::Array(apply(&x, &o, |_| Some(unit))?))
    }
}

/// `__timeunit_apply(x, unit, ..options)`: internal; `unit` is the text of a unit for each
/// row, from `__timeunit_choose`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitApply {
    signature: Signature,
}

impl Default for TimeUnitApply {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for TimeUnitApply {
    fn name(&self) -> &str {
        "__timeunit_apply"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        internal_err!(
            "{} computes its field in return_field_from_args",
            self.name()
        )
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let x = &args.arg_fields[0];
        let o = scalar_options(&args, 2)?.unwrap_or_default();
        Ok(result_field(
            self.name(),
            x.data_type(),
            x.is_nullable(),
            o.tag(),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let o = TimeUnitOptions::parse(&scalar_args(&args.args[2..])?)?;
        let n = args.number_rows;
        let x = args.args[0].to_array(n)?;
        let units = cast(&args.args[1].to_array(n)?, &DataType::Utf8)?;
        let units = units.as_string::<i32>();
        let mut parsed: HashMap<&str, Unit> = HashMap::new();
        for i in 0..n {
            if units.is_valid(i) && !parsed.contains_key(units.value(i)) {
                parsed.insert(units.value(i), Unit::parse(units.value(i))?);
            }
        }
        let array = apply(&x, &o, |i| {
            units.is_valid(i).then(|| parsed[units.value(i)])
        })?;
        Ok(ColumnarValue::Array(array))
    }
}

/// `__timeunit_choose(x, ..options)`: internal aggregate; the text of the unit that
/// `timeunit` chooses for the values.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimeUnitChoose {
    signature: Signature,
}

impl Default for TimeUnitChoose {
    fn default() -> Self {
        Self {
            signature: Signature::variadic_any(Volatility::Immutable),
        }
    }
}

impl AggregateUDFImpl for TimeUnitChoose {
    fn name(&self) -> &str {
        "__timeunit_choose"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let options = TimeUnitOptions::parse(&scalars_of_literals(&args.exprs[1..])?)?;
        let cal = Calendar::new(args.expr_fields[0].data_type(), &options)?;
        Ok(Box::new(ChooseAccumulator {
            options,
            cal,
            stats: Stats::default(),
        }))
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        let f = |n: &str, t: DataType| Arc::new(Field::new(format!("{}[{n}]", args.name), t, true));
        Ok(vec![
            f("n", DataType::UInt64),
            f("min", DataType::Int64),
            f("max", DataType::Int64),
            f("rank", DataType::UInt8),
            f("gcd", DataType::UInt64),
            f("reference", DataType::Int64),
        ])
    }
}

#[derive(Debug)]
struct ChooseAccumulator {
    options: TimeUnitOptions,
    cal: Calendar,
    stats: Stats,
}

impl Accumulator for ChooseAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.stats.add_array(&values[0], &mut self.cal)
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Utf8(
            choose(&self.stats, &self.options, self.cal.kind).map(|u| u.text()),
        ))
    }
    fn size(&self) -> usize {
        size_of_val(self)
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        let s = &self.stats;
        Ok(vec![
            ScalarValue::UInt64(Some(s.n)),
            ScalarValue::Int64(Some(s.min)),
            ScalarValue::Int64(Some(s.max)),
            ScalarValue::UInt8(Some(s.rank)),
            ScalarValue::UInt64(Some(s.gcd)),
            ScalarValue::Int64(Some(s.reference)),
        ])
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let n: &UInt64Array = states[0].as_primitive::<UInt64Type>();
        let min: &Int64Array = states[1].as_primitive::<Int64Type>();
        let max: &Int64Array = states[2].as_primitive::<Int64Type>();
        let rank: &UInt8Array = states[3].as_primitive::<UInt8Type>();
        let g: &UInt64Array = states[4].as_primitive::<UInt64Type>();
        let r: &Int64Array = states[5].as_primitive::<Int64Type>();
        for i in 0..n.len() {
            self.stats.merge(&Stats {
                n: n.value(i),
                min: min.value(i),
                max: max.value(i),
                rank: rank.value(i),
                gcd: g.value(i),
                reference: r.value(i),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// The rule: timeunit(x, ..options) OVER (..) -> __timeunit or __timeunit_apply
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct TimeUnitWindowRewrite {
    scalar: Arc<ScalarUDF>,
    apply: Arc<ScalarUDF>,
    choose: Arc<AggregateUDF>,
}

impl Default for TimeUnitWindowRewrite {
    fn default() -> Self {
        Self {
            scalar: Arc::new(ScalarUDF::new_from_impl(TimeUnitScalar::default())),
            apply: Arc::new(ScalarUDF::new_from_impl(TimeUnitApply::default())),
            choose: Arc::new(AggregateUDF::new_from_impl(TimeUnitChoose::default())),
        }
    }
}

const UNITS: &str = "__timeunit_units";

impl OptimizerRule for TimeUnitWindowRewrite {
    fn name(&self) -> &str {
        "timeunit_window_rewrite"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Window(window) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if let Some(split) =
            crate::split_window(window, |_, e| crate::is_call::<TimeUnitWindow>(e))?
        {
            let lower = self.rewrite(split.lower.clone(), _config)?.data;
            return Ok(Transformed::yes(split.rebuild(lower, Ok)?));
        }
        let mut calls: Vec<&WindowFunction> = Vec::new();
        for expr in &window.window_expr {
            let Some(wf) =
                crate::window_call(expr).filter(|_| crate::is_call::<TimeUnitWindow>(expr))
            else {
                return Ok(Transformed::no(plan));
            };
            crate::whole_partition("timeunit", &wf.params)?;
            if wf.params.args.is_empty() {
                return Ok(Transformed::no(plan));
            }
            calls.push(wf);
        }
        // The rewrite reads the input twice: for the units, and for the rows.
        if crate::is_volatile(&window.input)?
            || calls.iter().any(|c| c.params.args[0].is_volatile())
        {
            return Ok(Transformed::no(plan));
        }
        let partition_by = &calls[0].params.partition_by;
        if calls.iter().any(|c| &c.params.partition_by != partition_by) {
            return Ok(Transformed::no(plan));
        }
        let Some(keys) = partition_by
            .iter()
            .map(|e| match e {
                Expr::Column(c) => Some(c.clone()),
                _ => None,
            })
            .collect::<Option<Vec<Column>>>()
        else {
            return Ok(Transformed::no(plan));
        };
        let mut options = Vec::new();
        for call in &calls {
            let exprs = call.params.args[1..].to_vec();
            let o = TimeUnitOptions::parse(&literals(&exprs)?)?;
            options.push((exprs, o));
        }

        let input = window.input.as_ref().clone();
        let n_in = input.schema().fields().len();
        let mut exprs: Vec<Expr> = input
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        let out = |i: usize| window.schema.qualified_field(n_in + i);

        // The unit of each call: None for an explicit unit, which is the scalar function.
        let mut units: Vec<Option<Expr>> = vec![None; calls.len()];
        let auto: Vec<usize> = (0..calls.len())
            .filter(|&i| options[i].1.unit.is_none())
            .collect();
        let choose = |i: usize| {
            let mut args = vec![calls[i].params.args[0].clone()];
            args.extend(options[i].0.iter().cloned());
            self.choose.call(args)
        };

        let builder = if auto.is_empty() {
            LogicalPlanBuilder::from(input)
        } else if keys.is_empty() {
            for &i in &auto {
                let subquery = LogicalPlanBuilder::from(input.clone())
                    .aggregate(Vec::<Expr>::new(), vec![choose(i).alias("unit")])?
                    .build()?;
                units[i] = Some(Expr::ScalarSubquery(Subquery {
                    subquery: Arc::new(subquery),
                    outer_ref_columns: vec![],
                    spans: Spans::new(),
                }));
            }
            LogicalPlanBuilder::from(input)
        } else {
            let group: Vec<Expr> = keys
                .iter()
                .enumerate()
                .map(|(k, c)| Expr::Column(c.clone()).alias(format!("__timeunit_key_{k}")))
                .collect();
            let aggs: Vec<Expr> = auto
                .iter()
                .map(|&i| choose(i).alias(format!("__timeunit_unit_{i}")))
                .collect();
            for &i in &auto {
                units[i] = Some(Expr::Column(Column::new(
                    Some(UNITS),
                    format!("__timeunit_unit_{i}"),
                )));
            }
            let chosen = LogicalPlanBuilder::from(input.clone())
                .aggregate(group, aggs)?
                .alias(UNITS)?
                .build()?;
            let right: Vec<Column> = (0..keys.len())
                .map(|k| Column::new(Some(UNITS), format!("__timeunit_key_{k}")))
                .collect();
            LogicalPlanBuilder::from(input).join_detailed(
                chosen,
                JoinType::Inner,
                (keys, right),
                None,
                NullEquality::NullEqualsNull,
            )?
        };

        for (i, call) in calls.iter().enumerate() {
            let (q, f) = out(i);
            let x = call.params.args[0].clone();
            let e = match units[i].take() {
                None => {
                    let mut args = vec![x];
                    args.extend(options[i].0.iter().cloned());
                    self.scalar.call(args)
                }
                Some(unit) => {
                    let mut args = vec![x, unit];
                    args.extend(options[i].0.iter().cloned());
                    self.apply.call(args)
                }
            };
            exprs.push(e.alias_qualified(q.cloned(), f.name()));
        }
        Ok(Transformed::yes(builder.project(exprs)?.build()?))
    }
}

// ---------------------------------------------------------------------------------------
// SQL
// ---------------------------------------------------------------------------------------

/// The scalar function of `timeunit`, `__timeunit`, for [`crate::session`].
pub fn scalar_udf() -> ScalarUDF {
    ScalarUDF::new_from_impl(TimeUnitScalar::default())
}

/// Runs a query, with `timeunit` calls without `OVER` renamed to `__timeunit`.
///
/// DataFusion 55.2 resolves a function name to a scalar function before a window function,
/// and then drops the `OVER` clause, so `timeunit` cannot name both. The avenger-sql
/// pipeline can make this rename while it rewrites the query.
pub async fn sql(ctx: &SessionContext, query: &str) -> Result<DataFrame> {
    let mut statements = DFParser::parse_sql_with_dialect(query, &GenericDialect {})?;
    if statements.len() != 1 {
        return plan_err!("one statement at a time");
    }
    let mut statement = statements.pop_front().unwrap();
    if let DFStatement::Statement(s) = &mut statement {
        let _ = visit_expressions_mut(s.as_mut(), |e| {
            if let SqlExpr::Function(f) = e
                && f.over.is_none()
                && f.name.0.len() == 1
                && matches!(&f.name.0[0], ObjectNamePart::Identifier(i) if i.value.eq_ignore_ascii_case("timeunit"))
            {
                f.name.0[0] = ObjectNamePart::Identifier(Ident::new("__timeunit"));
            }
            ControlFlow::<()>::Continue(())
        });
    }
    let plan = ctx.state().statement_to_plan(statement).await?;
    ctx.execute_logical_plan(plan).await
}
