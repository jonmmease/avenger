use crate::{
    format::{local_datetime, normalize_instant},
    DateTimeFormatContext, DateTimeFormatError, NaiveDateTimeInput, PreparedDateTimeFormat,
    ZonedDateTimeInput,
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

/// Vega time multi-format overrides keyed by calendar unit. Empty patterns use defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeMultiFormatSpec {
    pub milliseconds: Option<String>,
    pub seconds: Option<String>,
    pub minutes: Option<String>,
    pub hours: Option<String>,
    /// Daily labels. A nonempty pattern takes precedence over `day`.
    pub date: Option<String>,
    /// Alias for `date`, used when `date` is absent or empty.
    pub day: Option<String>,
    /// Labels at Sunday week boundaries.
    pub week: Option<String>,
    pub month: Option<String>,
    pub quarter: Option<String>,
    pub year: Option<String>,
}

/// Vega's calendar-sensitive format selection above the scalar D3 formatter.
#[derive(Debug, Clone)]
pub struct PreparedTimeMultiFormat {
    formats: Vec<PreparedDateTimeFormat>,
    timezone: chrono_tz::Tz,
}
impl PreparedTimeMultiFormat {
    /// Prepare the supplied unit overrides and Vega's remaining default patterns.
    pub fn new(
        spec: &TimeMultiFormatSpec,
        context: DateTimeFormatContext<'_>,
    ) -> Result<Self, DateTimeFormatError> {
        let patterns = [
            (&spec.milliseconds, ".%L"),
            (&spec.seconds, ":%S"),
            (&spec.minutes, "%I:%M"),
            (&spec.hours, "%I %p"),
            (
                if spec.date.as_deref().is_some_and(|value| !value.is_empty()) {
                    &spec.date
                } else {
                    &spec.day
                },
                "%a %d",
            ),
            (&spec.week, "%b %d"),
            (&spec.month, "%B"),
            (&spec.quarter, "%B"),
            (&spec.year, "%Y"),
        ];
        let formats = patterns
            .into_iter()
            .map(|(pattern, default)| {
                PreparedDateTimeFormat::new(
                    Some(
                        pattern
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .unwrap_or(default),
                    ),
                    Default::default(),
                    context,
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            formats,
            timezone: context.timezone,
        })
    }
    /// Validate every selectable pattern for civil inputs at batch setup.
    pub fn validate_naive(&self) -> Result<(), DateTimeFormatError> {
        for format in &self.formats {
            format.validate_naive()?;
        }
        Ok(())
    }
    /// Select a pattern in the display zone, returning an error for out-of-range dates.
    pub fn format_zoned(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        let value = normalize_instant(value)?;
        self.formats[self.select_zoned(value)?].format_zoned(value)
    }
    fn select_zoned(&self, value: ZonedDateTimeInput) -> Result<usize, DateTimeFormatError> {
        let local = local_datetime(value.with_timezone(&self.timezone))?;
        if local.nanosecond() != 0 {
            return Ok(0);
        }
        if local.second() != 0 {
            return Ok(1);
        }
        if local.minute() != 0 {
            return Ok(2);
        }
        let date = local.date();
        let past_boundary = |date| {
            midnight_millis(date, self.timezone).map(|boundary| boundary < value.timestamp_millis())
        };
        if past_boundary(date)? {
            return Ok(3);
        }
        if past_boundary(date.with_day(1).unwrap())? {
            return Ok(if date.weekday() == chrono::Weekday::Sun {
                5
            } else {
                4
            });
        }
        if past_boundary(date.with_ordinal(1).unwrap())? {
            let quarter = date
                .with_month(date.month0() / 3 * 3 + 1)
                .unwrap()
                .with_day(1)
                .unwrap();
            return Ok(if past_boundary(quarter)? { 6 } else { 7 });
        }
        Ok(8)
    }
    /// Select a pattern from civil calendar fields without inventing an instant.
    pub fn format_naive(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.formats[select(value.datetime())].format_naive(value)
    }
}
fn select(value: NaiveDateTime) -> usize {
    if value.nanosecond() / 1_000_000 != 0 {
        0
    } else if value.second() != 0 {
        1
    } else if value.minute() != 0 {
        2
    } else if value.hour() != 0 {
        3
    } else if value.day() != 1 {
        if value.weekday() == chrono::Weekday::Sun {
            5
        } else {
            4
        }
    } else if value.month() != 1 {
        if value.month0().is_multiple_of(3) {
            7
        } else {
            6
        }
    } else {
        8
    }
}

// JavaScript calendar setters choose the earlier instant in a fold and move a
// nonexistent time forward by the gap. A midnight DST transition can therefore
// make 01:00 the start of a day.
fn midnight_millis(date: NaiveDate, timezone: chrono_tz::Tz) -> Result<i64, DateTimeFormatError> {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap();
    if let Some(value) = timezone.from_local_datetime(&midnight).earliest() {
        Ok(value.timestamp_millis())
    } else {
        let (_, offset) = chrono_tz::GapInfo::new(&midnight, &timezone)
            .and_then(|gap| gap.begin)
            .ok_or(DateTimeFormatError::DateTimeOutOfRange)?;
        Ok(
            midnight.and_utc().timestamp_millis()
                - i64::from(offset.fix().local_minus_utc()) * 1000,
        )
    }
}
