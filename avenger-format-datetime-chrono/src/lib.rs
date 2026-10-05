#![doc = include_str!("../README.md")]

use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
#[cfg(feature = "all-locales")]
use chrono::Locale;
use chrono::{
    format::{DelayedFormat, Item, Numeric, StrftimeItems},
    DateTime, NaiveDate, NaiveDateTime, Offset, Utc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::{slice, sync::Arc};

pub use avenger_format::CalendarPatterns;

/// Prepare Chrono datetime patterns with a built-in locale and display timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChronoDateTimeFormatProvider {
    /// Chrono locale name. Names other than `POSIX` require `all-locales`.
    /// An omitted name selects `POSIX`.
    pub locale: Option<String>,
    /// Resolved IANA display timezone for zoned datetimes. Defaults to UTC.
    pub timezone: Tz,
}

impl Default for ChronoDateTimeFormatProvider {
    fn default() -> Self {
        Self {
            locale: None,
            timezone: Tz::UTC,
        }
    }
}

impl ChronoDateTimeFormatProvider {
    /// Use the POSIX locale and UTC for zoned formatting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a built-in locale, accepting hyphens or underscores in its name.
    /// Names other than `POSIX` require the `all-locales` feature.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Set the IANA display timezone for zoned datetimes. Naive fields are unchanged.
    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }
}

impl DateTimeFormatProvider for ChronoDateTimeFormatProvider {
    fn prepare_date(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        let pattern = Pattern::new(pattern, self.locale.as_deref())?;
        // Rendering without a time or offset rejects incompatible fields after locale expansion.
        PreparedDateFormatter::format(&pattern, DateTime::UNIX_EPOCH.date_naive())
            .map_err(|error| preparation_error(error, DateTimeInputKind::Date))?;
        Ok(Arc::new(pattern))
    }

    fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        let pattern = Pattern::new(pattern, self.locale.as_deref())?;
        // Chrono assumes UTC for naive timestamps. Naive values have no timezone.
        if pattern
            .items
            .iter()
            .any(|item| matches!(item, Item::Numeric(Numeric::Timestamp, _)))
        {
            return Err(DateTimeFormatError::UnsupportedPattern {
                input: DateTimeInputKind::Naive,
                message: "`%s` requires a zoned datetime".into(),
            });
        }
        // Rendering detects timezone-dependent and parsing-only items, including locale expansions.
        PreparedNaiveDateTimeFormatter::format(&pattern, DateTime::UNIX_EPOCH.naive_utc())
            .map_err(|error| preparation_error(error, DateTimeInputKind::Naive))?;
        Ok(Arc::new(pattern))
    }

    fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        let formatter = ZonedFormat {
            pattern: Pattern::new(pattern, self.locale.as_deref())?,
            timezone: self.timezone,
        };
        // Chrono parses some directives, such as %#z, that it cannot use for formatting.
        formatter
            .format(DateTime::UNIX_EPOCH)
            .map_err(|error| preparation_error(error, DateTimeInputKind::Zoned))?;
        Ok(Arc::new(formatter))
    }

    /// Vega's time axis patterns in strftime syntax. Names and `%p` follow the provider's locale.
    fn default_calendar_patterns(&self) -> CalendarPatterns {
        CalendarPatterns {
            year: "%Y".into(),
            month: "%B".into(),
            week: "%b %d".into(),
            day: "%a %d".into(),
            hour: "%I %p".into(),
            minute: "%I:%M".into(),
            second: ":%S".into(),
            millisecond: "%.3f".into(),
        }
    }
}

/// Owned Chrono items with locale expansions resolved during preparation.
#[derive(Debug)]
struct Pattern {
    items: Vec<Item<'static>>,
    #[cfg(feature = "all-locales")]
    locale: Locale,
}

impl Pattern {
    fn new(pattern: &str, locale: Option<&str>) -> Result<Self, DateTimeFormatError> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "all-locales")] {
                let locale = match locale {
                    Some(name) => name
                        .replace('-', "_")
                        .parse::<Locale>()
                        .map_err(|_| DateTimeFormatError::LocaleUnavailable {
                            locale: name.into(),
                            message: "unknown Chrono locale".into(),
                        })?,
                    None => Locale::POSIX,
                };
                let items = StrftimeItems::new_with_locale(pattern, locale);
            } else {
                if let Some(name) = locale.filter(|name| *name != "POSIX") {
                    return Err(DateTimeFormatError::LocaleUnavailable {
                        locale: name.into(),
                        message: "requires the `all-locales` feature".into(),
                    });
                }
                let items = StrftimeItems::new(pattern);
            }
        }
        let items =
            items
                .parse_to_owned()
                .map_err(|error| DateTimeFormatError::InvalidPattern {
                    position: None,
                    message: error.to_string(),
                })?;
        Ok(Self {
            items,
            #[cfg(feature = "all-locales")]
            locale,
        })
    }
}

impl PreparedDateFormatter for Pattern {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "all-locales")] {
                let format = DelayedFormat::new_with_locale(
                    Some(value), None, self.items.iter(), self.locale,
                );
            } else {
                let format = DelayedFormat::new(Some(value), None, self.items.iter());
            }
        }
        render(format)
    }
}

impl PreparedNaiveDateTimeFormatter for Pattern {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        cfg_if::cfg_if! {
            if #[cfg(feature = "all-locales")] {
                let format = DelayedFormat::new_with_locale(
                    Some(value.date()),
                    Some(value.time()),
                    self.items.iter(),
                    self.locale,
                );
            } else {
                let format = DelayedFormat::new(
                    Some(value.date()), Some(value.time()), self.items.iter(),
                );
            }
        }
        render(format)
    }
}

#[derive(Debug)]
struct ZonedFormat {
    pattern: Pattern,
    timezone: Tz,
}

impl PreparedZonedDateTimeFormatter for ZonedFormat {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        let display = value.with_timezone(&self.timezone);
        let local = display
            .naive_utc()
            .checked_add_offset(display.offset().fix())
            .ok_or(DateTimeFormatError::OutOfRange)?;
        cfg_if::cfg_if! {
            if #[cfg(feature = "all-locales")] {
                let format = DelayedFormat::new_with_offset_and_locale(
                    Some(local.date()),
                    Some(local.time()),
                    display.offset(),
                    self.pattern.items.iter(),
                    self.pattern.locale,
                );
            } else {
                let format = DelayedFormat::new_with_offset(
                    Some(local.date()),
                    Some(local.time()),
                    display.offset(),
                    self.pattern.items.iter(),
                );
            }
        }
        render(format)
    }

    fn timezone(&self) -> Tz {
        self.timezone
    }
}

fn render(
    format: DelayedFormat<slice::Iter<'_, Item<'static>>>,
) -> Result<String, DateTimeFormatError> {
    let mut text = String::new();
    format
        .write_to(&mut text)
        .map_err(|_| DateTimeFormatError::FormattingFailed)?;
    Ok(text)
}

/// Classify a failed preparation render while preserving value errors.
fn preparation_error(error: DateTimeFormatError, input: DateTimeInputKind) -> DateTimeFormatError {
    match error {
        DateTimeFormatError::FormattingFailed => DateTimeFormatError::UnsupportedPattern {
            input,
            message: "Chrono could not render this pattern".into(),
        },
        other => other,
    }
}
