use crate::{
    DateTimeFormatError, DateTimeFormatProvider, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use chrono::{
    DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Timelike, Utc,
    Weekday,
};
use chrono_tz::Tz;
use std::sync::Arc;

/// One pattern per calendar boundary, in a provider's syntax, for labels such as a time axis's.
/// Each value uses the pattern for the coarsest boundary it falls on, as Vega labels time axes:
/// Jan 1 uses `year`, the 1st of another month `month`, other Sundays `week`, other days `day`,
/// and times within a day `hour`, `minute`, `second`, or `millisecond`.
///
/// Boundaries use the Gregorian calendar and Sunday weeks, so preparation first calls the
/// provider's [`check_gregorian_months`](DateTimeFormatProvider::check_gregorian_months), which
/// rejects calendars whose months start on other days. Values are truncated to whole
/// milliseconds, toward zero as JavaScript dates are, before a pattern is chosen and applied.
/// Zoned values are tested in the formatter's timezone, where a day starts at its first instant
/// even when a timezone transition skips or repeats midnight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarPatterns {
    /// Midnight on January 1.
    pub year: String,
    /// Midnight on the 1st of another month.
    pub month: String,
    /// Midnight on another Sunday.
    pub week: String,
    /// Midnight on another day.
    pub day: String,
    /// The start of another hour.
    pub hour: String,
    /// The start of another minute.
    pub minute: String,
    /// The start of another second.
    pub second: String,
    /// A value between seconds.
    pub millisecond: String,
}

/// Calendar boundaries from coarsest to finest. Dates only reach the first four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

impl CalendarPatterns {
    pub fn with_year(mut self, pattern: impl Into<String>) -> Self {
        self.year = pattern.into();
        self
    }

    pub fn with_month(mut self, pattern: impl Into<String>) -> Self {
        self.month = pattern.into();
        self
    }

    pub fn with_week(mut self, pattern: impl Into<String>) -> Self {
        self.week = pattern.into();
        self
    }

    pub fn with_day(mut self, pattern: impl Into<String>) -> Self {
        self.day = pattern.into();
        self
    }

    pub fn with_hour(mut self, pattern: impl Into<String>) -> Self {
        self.hour = pattern.into();
        self
    }

    pub fn with_minute(mut self, pattern: impl Into<String>) -> Self {
        self.minute = pattern.into();
        self
    }

    pub fn with_second(mut self, pattern: impl Into<String>) -> Self {
        self.second = pattern.into();
        self
    }

    pub fn with_millisecond(mut self, pattern: impl Into<String>) -> Self {
        self.millisecond = pattern.into();
        self
    }

    /// Prepare the date patterns, `year` through `day`, through the provider.
    pub fn prepare_date(
        &self,
        provider: &dyn DateTimeFormatProvider,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        provider.check_gregorian_months()?;
        let [year, month, week, day, ..] = self.fields();
        let formats = [year, month, week, day]
            .map(|(field, pattern)| in_field(field, provider.prepare_date(pattern)));
        Ok(Arc::new(Dates(collect(formats)?)))
    }

    /// Prepare every pattern through the provider.
    pub fn prepare_naive(
        &self,
        provider: &dyn DateTimeFormatProvider,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        provider.check_gregorian_months()?;
        let formats = self
            .fields()
            .map(|(field, pattern)| in_field(field, provider.prepare_naive(pattern)));
        Ok(Arc::new(Naive(collect(formats)?)))
    }

    /// Prepare every pattern through the provider. Boundaries are tested in the timezone the
    /// prepared formatters display values in.
    pub fn prepare_zoned(
        &self,
        provider: &dyn DateTimeFormatProvider,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        provider.check_gregorian_months()?;
        let formats = collect(
            self.fields()
                .map(|(field, pattern)| in_field(field, provider.prepare_zoned(pattern))),
        )?;
        let timezone = formats[0].timezone();
        if formats.iter().any(|format| format.timezone() != timezone) {
            return Err(DateTimeFormatError::InvalidOption {
                option: "timezone".into(),
                message: "the provider prepared formatters in different timezones".into(),
            });
        }
        Ok(Arc::new(Zoned { formats, timezone }))
    }

    fn fields(&self) -> [(&'static str, &str); 8] {
        [
            ("year", &self.year),
            ("month", &self.month),
            ("week", &self.week),
            ("day", &self.day),
            ("hour", &self.hour),
            ("minute", &self.minute),
            ("second", &self.second),
            ("millisecond", &self.millisecond),
        ]
    }
}

/// Name the pattern that failed, since each provider reports positions within one pattern.
fn in_field<T>(
    field: &str,
    result: Result<T, DateTimeFormatError>,
) -> Result<T, DateTimeFormatError> {
    result.map_err(|error| match error {
        DateTimeFormatError::InvalidPattern { message, position } => {
            DateTimeFormatError::InvalidPattern {
                message: format!("`{field}` pattern: {message}"),
                position,
            }
        }
        DateTimeFormatError::UnsupportedPattern { input, message } => {
            DateTimeFormatError::UnsupportedPattern {
                input,
                message: format!("`{field}` pattern: {message}"),
            }
        }
        error => error,
    })
}

fn collect<T, const N: usize>(
    results: [Result<T, DateTimeFormatError>; N],
) -> Result<[T; N], DateTimeFormatError> {
    let values = results.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(values
        .try_into()
        .unwrap_or_else(|_| unreachable!("one value per result")))
}

#[derive(Debug)]
struct Dates([Arc<dyn PreparedDateFormatter>; 4]);

impl PreparedDateFormatter for Dates {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        self.0[civil_boundary(value.and_time(NaiveTime::MIN)) as usize].format(value)
    }
}

#[derive(Debug)]
struct Naive([Arc<dyn PreparedNaiveDateTimeFormatter>; 8]);

impl PreparedNaiveDateTimeFormatter for Naive {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        let value = value
            .with_nanosecond(value.nanosecond() / 1_000_000 * 1_000_000)
            .expect("truncation keeps a valid nanosecond");
        self.0[civil_boundary(value) as usize].format(value)
    }
}

#[derive(Debug)]
struct Zoned {
    formats: [Arc<dyn PreparedZonedDateTimeFormatter>; 8],
    timezone: Tz,
}

impl PreparedZonedDateTimeFormatter for Zoned {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        let value = DateTime::from_timestamp_millis(truncated_millis(value))
            .ok_or(DateTimeFormatError::OutOfRange)?;
        self.formats[zoned_boundary(value, self.timezone)? as usize].format(value)
    }

    fn timezone(&self) -> Tz {
        self.timezone
    }
}

/// The boundary of civil calendar fields.
fn civil_boundary(value: NaiveDateTime) -> Boundary {
    if value.nanosecond() != 0 {
        Boundary::Millisecond
    } else if value.second() != 0 {
        Boundary::Second
    } else if value.minute() != 0 {
        Boundary::Minute
    } else if value.hour() != 0 {
        Boundary::Hour
    } else if value.day() != 1 {
        sunday_week(value.weekday())
    } else if value.month() != 1 {
        Boundary::Month
    } else {
        Boundary::Year
    }
}

/// The boundary of a UTC datetime in a timezone. Days start at their first instant, so a day
/// whose midnight a transition skips starts at the time the clocks jump to.
fn zoned_boundary(value: DateTime<Utc>, timezone: Tz) -> Result<Boundary, DateTimeFormatError> {
    let millis = value.timestamp_millis();
    let offset = value.with_timezone(&timezone).offset().fix();
    let local = value
        .naive_utc()
        .checked_add_offset(offset)
        .ok_or(DateTimeFormatError::OutOfRange)?;
    if local.nanosecond() != 0 {
        return Ok(Boundary::Millisecond);
    }
    if local.second() != 0 {
        return Ok(Boundary::Second);
    }
    if local.minute() != 0 {
        return Ok(Boundary::Minute);
    }
    let date = local.date();
    let after_start = |date| day_start_millis(date, timezone).map(|start| start < millis);
    if after_start(date)? {
        return Ok(Boundary::Hour);
    }
    if after_start(date.with_day(1).expect("every month has a 1st"))? {
        return Ok(sunday_week(date.weekday()));
    }
    if after_start(date.with_ordinal(1).expect("every year has a first day"))? {
        return Ok(Boundary::Month);
    }
    Ok(Boundary::Year)
}

fn sunday_week(weekday: Weekday) -> Boundary {
    if weekday == Weekday::Sun {
        Boundary::Week
    } else {
        Boundary::Day
    }
}

/// Epoch milliseconds truncated toward zero, as JavaScript dates truncate.
fn truncated_millis(value: DateTime<Utc>) -> i64 {
    let millis = value.timestamp_millis();
    if millis < 0 && !value.timestamp_subsec_nanos().is_multiple_of(1_000_000) {
        millis + 1
    } else {
        millis
    }
}

// JavaScript calendar setters choose the earlier instant in a fold and move a nonexistent time
// forward by the gap. A midnight DST transition can therefore make 01:00 the start of a day.
fn day_start_millis(date: NaiveDate, timezone: Tz) -> Result<i64, DateTimeFormatError> {
    let midnight = date.and_time(NaiveTime::MIN);
    if let Some(start) = timezone.from_local_datetime(&midnight).earliest() {
        return Ok(start.timestamp_millis());
    }
    let (_, offset) = chrono_tz::GapInfo::new(&midnight, &timezone)
        .and_then(|gap| gap.begin)
        .ok_or(DateTimeFormatError::OutOfRange)?;
    Ok(midnight.and_utc().timestamp_millis() - i64::from(offset.fix().local_minus_utc()) * 1000)
}
