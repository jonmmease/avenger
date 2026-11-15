//! The datetime format provider of GoogleSQL's formats, for the labels of charts.

use std::collections::BTreeMap;
use std::sync::Arc;

use avenger_format::{
    CalendarPatterns, DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind,
    PreparedDateFormatter, PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use avenger_format_locales_d3::DateTimeLocaleSpec;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::element::{check, Kind, Rejection};
use crate::{Format, Locale};

/// Prepares GoogleSQL's format strings, with the syntax name `googlesql`, with the names and the
/// composites of a locale, and displays zoned datetimes in a time zone. The provider is stricter
/// than GoogleSQL's functions: it rejects an element that GoogleSQL does not document and an
/// element of a part that the value lacks, such as the hour of a date, which GoogleSQL prints as
/// written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleSqlDateTimeFormatProvider {
    /// Locale name. An omitted name selects GoogleSQL's names and composites, which differ from the
    /// composites of the d3 locale `en-US`.
    pub locale: Option<String>,
    /// Custom d3 definitions, keyed by locale name.
    pub locales: BTreeMap<String, DateTimeLocaleSpec>,
    /// The time zone that zoned datetimes display in. Defaults to UTC.
    pub timezone: Tz,
}

impl Default for GoogleSqlDateTimeFormatProvider {
    fn default() -> Self {
        Self {
            locale: None,
            locales: BTreeMap::new(),
            timezone: Tz::UTC,
        }
    }
}

impl GoogleSqlDateTimeFormatProvider {
    /// Use GoogleSQL's names and composites, and UTC for zoned formatting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a locale, accepting hyphens or underscores in its name.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Add or replace a custom definition without selecting it.
    pub fn with_custom_locale(
        mut self,
        name: impl Into<String>,
        definition: DateTimeLocaleSpec,
    ) -> Self {
        self.locales.insert(name.into(), definition);
        self
    }

    /// Displays zoned datetimes in `timezone`.
    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    /// The selected locale: a custom definition, by its name with hyphens or underscores, or a
    /// bundled one.
    fn resolve_locale(&self) -> Result<Locale, DateTimeFormatError> {
        let Some(id) = self.locale.as_deref() else {
            return Ok(Locale::default());
        };
        let normalized = id.replace('_', "-");
        let custom = self.locales.get(id).or_else(|| {
            (self.locales.iter())
                .find_map(|(name, spec)| (name.replace('_', "-") == normalized).then_some(spec))
        });
        match custom {
            Some(spec) => Locale::new(spec),
            None => Locale::bundled(&normalized),
        }
    }
}

/// A format prepared for one kind of value, with the time zone of zoned datetimes.
#[derive(Clone, Debug)]
struct Prepared {
    format: Format,
    timezone: Tz,
}

/// `pattern` prepared for values of `kind` with the names of `locale`, after the strict check.
fn prepare(
    pattern: &str,
    kind: Kind,
    locale: &Locale,
    timezone: Tz,
) -> Result<Prepared, DateTimeFormatError> {
    let input = match kind {
        Kind::Date => DateTimeInputKind::Date,
        Kind::Datetime | Kind::Time => DateTimeInputKind::Naive,
        Kind::Timestamp => DateTimeInputKind::Zoned,
    };
    check(pattern, kind).map_err(|rejection| match rejection {
        Rejection::Unknown { element, position } => DateTimeFormatError::InvalidPattern {
            message: format!("`{element}` is not an element of GoogleSQL's formats"),
            position: Some(position),
        },
        Rejection::Absent { element } => DateTimeFormatError::UnsupportedPattern {
            input,
            message: format!("`{element}` needs a part that the value does not have"),
        },
        Rejection::Unsupported => DateTimeFormatError::UnsupportedPattern {
            input,
            message: "GoogleSQL does not format `%J`".to_string(),
        },
    })?;
    let format = Format::new(pattern, kind, locale).map_err(|error| {
        DateTimeFormatError::UnsupportedPattern {
            input,
            message: error.to_string(),
        }
    })?;
    Ok(Prepared { format, timezone })
}

impl DateTimeFormatProvider for GoogleSqlDateTimeFormatProvider {
    fn syntax(&self) -> &str {
        "googlesql"
    }

    fn prepare_date(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        Ok(Arc::new(prepare(
            spec,
            Kind::Date,
            &self.resolve_locale()?,
            Tz::UTC,
        )?))
    }

    fn prepare_naive(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        Ok(Arc::new(prepare(
            spec,
            Kind::Datetime,
            &self.resolve_locale()?,
            Tz::UTC,
        )?))
    }

    fn prepare_zoned(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        Ok(Arc::new(prepare(
            spec,
            Kind::Timestamp,
            &self.resolve_locale()?,
            self.timezone,
        )?))
    }

    fn timezone(&self) -> Tz {
        self.timezone
    }

    /// The time axis patterns of the d3 provider in GoogleSQL's elements. GoogleSQL has no element
    /// of the fraction of a second alone, so the label between seconds is `%E3S`, such as
    /// `09.123`.
    fn default_calendar_patterns(&self) -> CalendarPatterns {
        CalendarPatterns {
            year: "%Y".into(),
            month: "%B".into(),
            week: "%b %d".into(),
            day: "%a %d".into(),
            hour: "%I %p".into(),
            minute: "%I:%M".into(),
            second: ":%S".into(),
            millisecond: "%E3S".into(),
        }
    }
}

impl PreparedDateFormatter for Prepared {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        Ok(self.format.format_date(value))
    }
}

impl PreparedNaiveDateTimeFormatter for Prepared {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        Ok(self.format.format_datetime(value))
    }
}

impl PreparedZonedDateTimeFormatter for Prepared {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        Ok(self.format.format_timestamp(value, self.timezone.into()))
    }

    fn timezone(&self) -> Tz {
        self.timezone
    }
}
