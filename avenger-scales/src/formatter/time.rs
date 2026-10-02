use avenger_format::{
    DateTimeFormatBinding, DateTimeFormatError, NaiveDateTimeInput, PreparedCivilDateTimeFormatter,
    PreparedInstantFormatter, ZonedDateTimeInput,
};
use avenger_format_config::{
    ChronoDateTimeFormatConfig, D3DateTimeFormatConfig, DateTimeFormatConfig,
};
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use chrono::{Datelike, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Calendar tick patterns. Missing or empty fields use the provider's defaults.
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

impl TimeMultiFormatSpec {
    fn patterns(&self, fraction: &str) -> [String; 9] {
        let overrides = [
            &self.milliseconds,
            &self.seconds,
            &self.minutes,
            &self.hours,
            if self.date.as_deref().is_some_and(|p| !p.is_empty()) {
                &self.date
            } else {
                &self.day
            },
            &self.week,
            &self.month,
            &self.quarter,
            &self.year,
        ];
        let defaults = [
            fraction, ":%S", "%I:%M", "%I %p", "%a %d", "%b %d", "%B", "%B", "%Y",
        ];
        std::array::from_fn(|i| {
            overrides[i]
                .as_deref()
                .filter(|p| !p.is_empty())
                .unwrap_or(defaults[i])
                .to_owned()
        })
    }
}

/// Explicit patterns and calendar selection for a configured datetime provider.
#[derive(Debug, Clone)]
pub struct DateTimeFormatAdapter {
    binding: DateTimeFormatBinding,
    patterns: [String; 9],
    timezone: Result<chrono_tz::Tz, DateTimeFormatError>,
    milliseconds: bool,
}

impl DateTimeFormatAdapter {
    /// Supply patterns for fractions, seconds, minutes, hours, days, weeks, months,
    /// quarters, and years, in that order. Selection uses full nanosecond precision.
    /// The timezone must match the binding's display timezone.
    pub fn new(
        binding: DateTimeFormatBinding,
        patterns: [String; 9],
        timezone: chrono_tz::Tz,
    ) -> Self {
        Self {
            binding,
            patterns,
            timezone: Ok(timezone),
            milliseconds: false,
        }
    }

    /// Use D3 patterns and JavaScript millisecond clipping for instant selection.
    pub fn d3(config: D3DateTimeFormatConfig, spec: TimeMultiFormatSpec) -> Self {
        let timezone = timezone(config.timezone.as_deref());
        Self {
            binding: DateTimeFormatBinding::new(D3DateTimeFormatProvider, config),
            patterns: spec.patterns(".%L"),
            timezone,
            milliseconds: true,
        }
    }

    /// Use Chrono patterns and retain fractional-second precision during selection.
    pub fn chrono(config: ChronoDateTimeFormatConfig, spec: TimeMultiFormatSpec) -> Self {
        let timezone = timezone(config.timezone.as_deref());
        Self {
            binding: DateTimeFormatBinding::new(ChronoDateTimeFormatProvider, config),
            patterns: spec.patterns("%.f"),
            timezone,
            milliseconds: false,
        }
    }

    /// Construct calendar policy for a saved built-in selection.
    pub fn from_config(config: &DateTimeFormatConfig, spec: TimeMultiFormatSpec) -> Self {
        match config {
            DateTimeFormatConfig::D3(config) => Self::d3(config.clone(), spec),
            DateTimeFormatConfig::Chrono(config) => Self::chrono(config.clone(), spec),
        }
    }

    /// Settings used to prepare explicit patterns in text labels.
    pub fn binding(&self) -> &DateTimeFormatBinding {
        &self.binding
    }

    /// Prepare an explicit civil pattern or every candidate for automatic calendar labels.
    pub fn prepare_naive(
        &self,
        pattern: Option<&str>,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        if let Some(pattern) = pattern {
            return self.binding.prepare_naive(pattern);
        }
        Ok(Arc::new(CivilCalendarFormat {
            formats: self
                .patterns
                .iter()
                .map(|p| self.binding.prepare_naive(p))
                .collect::<Result<_, _>>()?,
            milliseconds: self.milliseconds,
        }))
    }

    /// Prepare an explicit instant pattern or calendar labels in the display timezone.
    pub fn prepare_zoned(
        &self,
        pattern: Option<&str>,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        if let Some(pattern) = pattern {
            return self.binding.prepare_zoned(pattern);
        }
        Ok(Arc::new(InstantCalendarFormat {
            formats: self
                .patterns
                .iter()
                .map(|p| self.binding.prepare_zoned(p))
                .collect::<Result<_, _>>()?,
            timezone: self.timezone.clone()?,
            milliseconds: self.milliseconds,
        }))
    }
}

fn timezone(name: Option<&str>) -> Result<chrono_tz::Tz, DateTimeFormatError> {
    let name = name.unwrap_or("UTC");
    name.parse()
        .map_err(|_| DateTimeFormatError(format!("invalid IANA timezone `{name}`")))
}

#[derive(Debug)]
struct CivilCalendarFormat {
    formats: Vec<Arc<dyn PreparedCivilDateTimeFormatter>>,
    milliseconds: bool,
}
impl PreparedCivilDateTimeFormatter for CivilCalendarFormat {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        self.formats[select(value.datetime(), self.milliseconds)].format(value)
    }
}

#[derive(Debug)]
struct InstantCalendarFormat {
    formats: Vec<Arc<dyn PreparedInstantFormatter>>,
    timezone: chrono_tz::Tz,
    milliseconds: bool,
}
impl PreparedInstantFormatter for InstantCalendarFormat {
    fn format(&self, mut value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        if self.milliseconds {
            if value.nanosecond() >= 1_000_000_000 {
                return Err(DateTimeFormatError(
                    "leap seconds are not supported by D3 datetime formatting".into(),
                ));
            }
            // JavaScript Date clips fractional epoch milliseconds toward zero.
            let millis = value.timestamp_millis();
            let millis = millis
                + i64::from(
                    millis < 0 && !value.timestamp_subsec_nanos().is_multiple_of(1_000_000),
                );
            value = chrono::DateTime::from_timestamp_millis(millis).ok_or_else(out_of_range)?;
        }
        self.formats[self.select(value)?].format(value)
    }
}

impl InstantCalendarFormat {
    fn select(&self, value: ZonedDateTimeInput) -> Result<usize, DateTimeFormatError> {
        let zoned = value.with_timezone(&self.timezone);
        let local = zoned
            .naive_utc()
            .checked_add_offset(zoned.offset().fix())
            .ok_or_else(out_of_range)?;
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
}

fn select(value: NaiveDateTime, milliseconds: bool) -> usize {
    if (if milliseconds {
        value.nanosecond() / 1_000_000
    } else {
        value.nanosecond()
    }) != 0
    {
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

fn out_of_range() -> DateTimeFormatError {
    DateTimeFormatError("datetime exceeds the supported calendar range".into())
}

// Calendar boundaries choose the earlier instant in a fold and move a nonexistent
// midnight forward by the gap. A midnight DST transition can start a day at 01:00.
fn midnight_millis(date: NaiveDate, timezone: chrono_tz::Tz) -> Result<i64, DateTimeFormatError> {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap();
    if let Some(value) = timezone.from_local_datetime(&midnight).earliest() {
        Ok(value.timestamp_millis())
    } else {
        let (_, offset) = chrono_tz::GapInfo::new(&midnight, &timezone)
            .and_then(|gap| gap.begin)
            .ok_or_else(out_of_range)?;
        Ok(
            midnight.and_utc().timestamp_millis()
                - i64::from(offset.fix().local_minus_utc()) * 1000,
        )
    }
}
