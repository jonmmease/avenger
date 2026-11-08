//! Ticks, time zones, and the calendar arithmetic of a call.

use super::*;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset,
    TimeZone,
};
use datafusion::arrow::array::temporal_conversions::{
    MICROSECONDS, MILLISECONDS, NANOSECONDS, UNIX_EPOCH_DAY, date32_to_datetime,
    timestamp_ms_to_datetime, timestamp_ns_to_datetime, timestamp_s_to_datetime,
    timestamp_us_to_datetime,
};
use datafusion::arrow::array::timezone::Tz;
use datafusion::arrow::datatypes::{
    ArrowTimestampType, DataType, Date32Type, TimeUnit as ArrowUnit, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType,
};
use datafusion::common::{Result, plan_err};

/// How an `i64` holds a value: days of a `DATE`, or the ticks of a timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Days,
    Ts(ArrowUnit),
}

impl Kind {
    pub(super) fn per_second(self) -> i64 {
        match self {
            Kind::Days => 0,
            Kind::Ts(ArrowUnit::Second) => 1,
            Kind::Ts(ArrowUnit::Millisecond) => MILLISECONDS,
            Kind::Ts(ArrowUnit::Microsecond) => MICROSECONDS,
            Kind::Ts(ArrowUnit::Nanosecond) => NANOSECONDS,
        }
    }

    /// Ticks in one millisecond.
    pub(super) fn per_ms(self) -> f64 {
        match self {
            Kind::Days => 1.0 / 8.64e7,
            _ => self.per_second() as f64 / 1e3,
        }
    }

    pub(super) fn ndt(self, v: i64) -> NaiveDateTime {
        match self {
            Kind::Days => date32_to_datetime(v as i32),
            Kind::Ts(ArrowUnit::Second) => timestamp_s_to_datetime(v),
            Kind::Ts(ArrowUnit::Millisecond) => timestamp_ms_to_datetime(v),
            Kind::Ts(ArrowUnit::Microsecond) => timestamp_us_to_datetime(v),
            Kind::Ts(ArrowUnit::Nanosecond) => timestamp_ns_to_datetime(v),
        }
        .expect("a value in range")
    }

    pub(super) fn ticks(self, d: NaiveDateTime) -> i64 {
        match self {
            Kind::Days => Some(Date32Type::from_naive_date(d.date()) as i64),
            Kind::Ts(ArrowUnit::Second) => TimestampSecondType::from_naive_datetime(d, None),
            Kind::Ts(ArrowUnit::Millisecond) => {
                TimestampMillisecondType::from_naive_datetime(d, None)
            }
            Kind::Ts(ArrowUnit::Microsecond) => {
                TimestampMicrosecondType::from_naive_datetime(d, None)
            }
            Kind::Ts(ArrowUnit::Nanosecond) => {
                TimestampNanosecondType::from_naive_datetime(d, None)
            }
        }
        .expect("a value in range")
    }

    /// The ticks of the values whose units and offsets the calendar can compute: the dates of
    /// chrono and the ticks of i64, less `margin_days` on each side.
    pub(super) fn supported(self, margin_days: i64) -> (i64, i64) {
        let days = |d: NaiveDate| (d.num_days_from_ce() as i64 - UNIX_EPOCH_DAY) as i128;
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

    pub(super) fn secs(self, v: i64) -> i64 {
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
    /// The zone of a type's time zone, as Arrow reads it. Arrow reads a time zone that
    /// starts with a sign, such as `+05:45`, as a fixed offset, and `UTC` has one offset.
    pub fn parse(tz: Option<&str>) -> Result<Zone> {
        let Some(tz) = tz else { return Ok(Zone::Civil) };
        let zone = match Tz::from_str(tz) {
            Ok(zone) => zone,
            Err(e) => return plan_err!("unknown time zone '{tz}': {e}"),
        };
        if tz.starts_with(['+', '-']) || tz == "UTC" {
            let epoch = NaiveDateTime::default();
            return Ok(Zone::Fixed(zone.offset_from_utc_datetime(&epoch).fix()));
        }
        Ok(Zone::Named(zone))
    }
}

/// Offsets and local times of a zone, with caches.
#[derive(Debug)]
pub(super) struct Clock {
    pub(super) zone: Zone,
    /// A 15-minute UTC bucket in which the offset does not change, and the offset.
    pub(super) bucket: i64,
    pub(super) bucket_off: Option<i32>,
    /// The offsets of a range of instants: the first second of each offset, and the offset.
    pub(super) table: Option<Arc<OffsetTable>>,
    /// The name of a named zone, the key of the offset tables that calls share.
    pub(super) name: Option<Arc<str>>,
    /// Instants of local times.
    pub(super) resolved: HashMap<NaiveDateTime, (NaiveDateTime, Option<NaiveDateTime>)>,
}

#[derive(Debug)]
pub(super) struct OffsetTable {
    pub(super) lo: i64,
    pub(super) hi: i64,
    pub(super) starts: Vec<i64>,
    pub(super) offsets: Vec<i32>,
}

pub(super) const BUCKET: i64 = 900;

impl Clock {
    pub(super) fn new(zone: Zone, name: Option<&str>) -> Self {
        Self {
            zone,
            bucket: i64::MIN,
            bucket_off: None,
            table: None,
            name: name.map(Arc::from),
            resolved: HashMap::new(),
        }
    }

    pub(super) fn is_named(&self) -> bool {
        matches!(self.zone, Zone::Named(_))
    }

    /// Makes the table of the offsets of the instants from `lo` to `hi` (seconds), with two
    /// days more on each side. It samples the offset at each day and finds each change to
    /// the second, so it misses two changes within one day that cancel out.
    pub(super) fn prepare(&mut self, lo: i64, hi: i64) {
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
    pub(super) fn offset(&mut self, utc_secs: i64) -> i32 {
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
    pub(super) fn span(&mut self, utc_secs: i64) -> (i32, i64, i64) {
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
    pub(super) fn first_instant_ticks(&mut self, local: i64, kind: Kind) -> i64 {
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
    pub(super) fn resolve(
        &mut self,
        local: NaiveDateTime,
    ) -> (NaiveDateTime, Option<NaiveDateTime>) {
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

    pub(super) fn first_instant(&mut self, local: NaiveDateTime) -> NaiveDateTime {
        self.resolve(local).0
    }
}

pub(super) fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_hms_opt(0, 0, 0).unwrap()
}

pub(super) fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a valid date")
}

// ---------------------------------------------------------------------------------------
// The calendar of a call
// ---------------------------------------------------------------------------------------

/// The calendar arithmetic of one call: the kind and the zone of the values, and the
/// options that move unit boundaries.
#[derive(Debug)]
pub(crate) struct Calendar {
    pub(super) kind: Kind,
    pub(super) clock: Clock,
    pub(super) week_start: WeekStart,
    pub(super) fiscal: Option<u32>,
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
    pub(super) fn day_ticks(&self) -> i64 {
        match self.kind {
            Kind::Days => 1,
            k => 86_400 * k.per_second(),
        }
    }

    /// Local time in ticks.
    pub(super) fn local_ticks(&mut self, t: i64) -> i64 {
        match self.kind {
            Kind::Days => t,
            k => t + self.clock.offset(k.secs(t)) as i64 * k.per_second(),
        }
    }

    /// The first instant of a local time, in ticks.
    pub(super) fn instant(&mut self, local: i64) -> i64 {
        self.clock.first_instant_ticks(local, self.kind)
    }

    /// The first instant of a date.
    pub(super) fn date_instant(&mut self, d: NaiveDate) -> i64 {
        let days = d.num_days_from_ce() as i64 - UNIX_EPOCH_DAY;
        self.instant(days * self.day_ticks())
    }

    /// The local date of a value.
    pub(super) fn local_date(&mut self, t: i64) -> NaiveDate {
        let days = self.local_ticks(t).div_euclid(self.day_ticks());
        Kind::Days.ndt(days).date()
    }

    /// The origin of the units of a week or shorter: the first day of a week, January 1 of
    /// the reference year.
    pub(super) fn origin(&self) -> NaiveDate {
        ymd(self.week_start.reference_year(), 1, 1)
    }

    /// The first day of the unit of one day or longer that contains `d`, and of the next.
    /// Days and weeks step from [`Calendar::origin`], months, quarters, and years from
    /// January of year 0, and fiscal units from the start of fiscal year 0.
    pub(super) fn civil_dates(&self, d: NaiveDate, t: Trunc, k: u32) -> (NaiveDate, NaiveDate) {
        let k = k as i64;
        let days = |n: i64| {
            let origin = self.origin();
            let i = (d - origin).num_days().div_euclid(n) * n;
            (origin + Duration::days(i), origin + Duration::days(i + n))
        };
        // Steps of `n` months from the month `origin`, as months since January of year 0.
        let months = |origin: i64, n: i64| {
            let m = d.year() as i64 * 12 + d.month0() as i64;
            let i = origin + (m - origin).div_euclid(n) * n;
            (month_start(i), month_start(i + n))
        };
        // Fiscal year 0, named for the calendar year in which it ends, starts in year -1
        // unless it starts in January.
        let fiscal = || match self.fiscal.expect("a fiscal unit has fiscal_year_start") {
            1 => 0,
            f => f as i64 - 1 - 12,
        };
        match t {
            Trunc::Day => days(k),
            Trunc::Week => days(7 * k),
            Trunc::Month => months(0, k),
            Trunc::Quarter => months(0, 3 * k),
            Trunc::Year => months(0, 12 * k),
            Trunc::FiscalQuarter => months(fiscal(), 3 * k),
            Trunc::FiscalYear => months(fiscal(), 12 * k),
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
        // Milliseconds that divide a second stay within whole seconds, where every offset
        // changes.
        let in_second = unit == Trunc::Millisecond && 1000 % k == 0;
        if let Some((s, e)) = self.subday_ticks(t, off, unit, k) {
            // The unit needs the offset of the value from its start to its end, and past its
            // end: a change of offset at the end can drop the local time of the end, as on Lord
            // Howe Island, where 02:00 becomes 01:30 and the hour from 01:00 lasts 90 minutes.
            let ps = self.kind.per_second();
            if in_second || (lo <= s.div_euclid(ps) && e.div_euclid(ps) < hi) {
                return (s, e);
            }
        }
        let delta = Duration::seconds(off as i64);
        let origin = midnight(self.origin());
        let l = floor_sub(origin, self.kind.ndt(t) + delta, unit, k);
        let (s, e) = (l - delta, next_sub(l, unit, k) - delta);
        if !self.clock.is_named()
            || in_second
            || (self.clock.offset(s.and_utc().timestamp()) == off
                && self.clock.offset(e.and_utc().timestamp()) == off)
        {
            return (self.kind.ticks(s), self.kind.ticks(e));
        }
        // A change of the offset: the edges of the local times around the value, within
        // one unit and the longest change of offset of the unit.
        let span = match unit {
            Trunc::Hour => Duration::days(1),
            Trunc::Minute => Duration::hours(3),
            _ => Duration::minutes(15),
        } + Duration::milliseconds(sub_ms(unit) * k as i64);
        let edges = self.sub_edges(l - span, l + span, unit, k);
        let i = edges.partition_point(|e| *e <= t);
        (edges[i - 1], edges[i])
    }

    /// The bounds of a unit shorter than one day with the offset of the value throughout,
    /// in integer arithmetic on ticks, or None when the unit is not a whole number of ticks.
    pub(super) fn subday_ticks(&self, t: i64, off: i32, unit: Trunc, k: u32) -> Option<(i64, i64)> {
        let ps = self.kind.per_second();
        if ps < 1000 && unit == Trunc::Millisecond {
            return None;
        }
        // Wide integers: the origin is far from the ends of the range of nanoseconds.
        let step = (sub_ms(unit) * k as i64) as i128 * ps as i128 / 1000;
        let shift = off as i128 * ps as i128;
        let origin = days_from_civil(self.week_start.reference_year() as i64, 1, 1) as i128
            * 86_400
            * ps as i128;
        let local = t as i128 + shift;
        let s = origin + (local - origin).div_euclid(step) * step - shift;
        Some((i64::try_from(s).ok()?, i64::try_from(s + step).ok()?))
    }

    /// All edges of a unit shorter than one day between two local times, sorted: the
    /// instants of each local multiple of the unit.
    pub(super) fn sub_edges(
        &mut self,
        lo: NaiveDateTime,
        hi: NaiveDateTime,
        unit: Trunc,
        k: u32,
    ) -> Vec<i64> {
        let mut edges = Vec::new();
        let mut l = floor_sub(midnight(self.origin()), lo, unit, k);
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
    pub(super) fn edges(
        &mut self,
        lo: i64,
        hi: i64,
        unit: Trunc,
        k: u32,
        limit: usize,
    ) -> Option<Vec<i64>> {
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
    pub(super) fn collapse(&mut self, t: i64, c: Collapse) -> (i64, i64) {
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
            Collapse::MonthDay => {
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
    pub(super) fn rank(&self, local_ticks: i64) -> u8 {
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
pub(super) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

/// The length of a unit shorter than one day in milliseconds.
pub(super) fn sub_ms(unit: Trunc) -> i64 {
    match unit {
        Trunc::Hour => 3_600_000,
        Trunc::Minute => 60_000,
        Trunc::Second => 1000,
        Trunc::Millisecond => 1,
        _ => unreachable!("a unit of a day or longer"),
    }
}

/// The last local time at or before `l` that is a multiple of `k` units from `origin`.
pub(super) fn floor_sub(
    origin: NaiveDateTime,
    l: NaiveDateTime,
    unit: Trunc,
    k: u32,
) -> NaiveDateTime {
    let step = sub_ms(unit) * k as i64;
    let d = l - origin;
    // Whole milliseconds, rounded down.
    let mut ms = d.num_milliseconds();
    if d < Duration::milliseconds(ms) {
        ms -= 1;
    }
    origin + Duration::milliseconds(ms.div_euclid(step) * step)
}

/// The next local multiple after a multiple.
pub(super) fn next_sub(l: NaiveDateTime, unit: Trunc, k: u32) -> NaiveDateTime {
    l + Duration::milliseconds(sub_ms(unit) * k as i64)
}

/// The first day of the month `i`, in months since January of year 0.
pub(super) fn month_start(i: i64) -> NaiveDate {
    ymd(i.div_euclid(12) as i32, i.rem_euclid(12) as u32 + 1, 1)
}
