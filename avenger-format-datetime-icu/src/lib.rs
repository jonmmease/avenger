#![doc = include_str!("../README.md")]

mod input;
mod pattern;
mod semantic;
mod settings;

use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, DateTimeInputKind, PreparedDateFormatter,
    PreparedNaiveDateTimeFormatter, PreparedZonedDateTimeFormatter,
};
use chrono::DateTime;
use chrono_tz::Tz;
use icu_calendar::AnyCalendar;
use icu_datetime::fieldsets::enums::{CompositeDateTimeFieldSet, DateFieldSet};
use pattern::{Pattern, ZonedFormat};
use serde::{Deserialize, Serialize};
use settings::{calendar_serde, Settings};
use std::sync::Arc;

pub use icu_calendar::preferences::{CalendarAlgorithm, HijriCalendarAlgorithm};

/// Prepare Unicode datetime patterns with ICU's locale data and calendar systems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IcuPatternDateTimeFormatProvider {
    /// Unicode locale identifier. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Explicit calendar, overriding the locale's calendar preference or default.
    #[serde(with = "calendar_serde")]
    pub calendar: Option<CalendarAlgorithm>,
    /// IANA display timezone for zoned datetimes. Defaults to UTC.
    pub timezone: Tz,
}

impl Default for IcuPatternDateTimeFormatProvider {
    fn default() -> Self {
        Self {
            locale: None,
            calendar: None,
            timezone: Tz::UTC,
        }
    }
}

impl IcuPatternDateTimeFormatProvider {
    /// Use the `en-US` locale, its default calendar, and UTC.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a locale, accepting hyphens or underscores in its identifier.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Select a calendar independently of the locale and pattern.
    pub fn with_calendar(mut self, calendar: CalendarAlgorithm) -> Self {
        self.calendar = Some(calendar);
        self
    }

    /// Set the display timezone for zoned datetimes. Date and Naive ignore it.
    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    fn settings(&self) -> Settings<'_> {
        Settings {
            locale: self.locale.as_deref(),
            calendar: self.calendar,
        }
    }
}

/// Prepare locale-appropriate datetime layouts from named options in a `{...}` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IcuSemanticDateTimeFormatProvider {
    /// Unicode locale identifier. An omitted name selects `en-US`.
    pub locale: Option<String>,
    /// Explicit calendar, overriding the locale's calendar preference or default.
    #[serde(with = "calendar_serde")]
    pub calendar: Option<CalendarAlgorithm>,
    /// IANA display timezone for zoned datetimes. Defaults to UTC.
    pub timezone: Tz,
}

impl Default for IcuSemanticDateTimeFormatProvider {
    fn default() -> Self {
        Self {
            locale: None,
            calendar: None,
            timezone: Tz::UTC,
        }
    }
}

impl IcuSemanticDateTimeFormatProvider {
    /// Use the `en-US` locale, its default calendar, and UTC.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a locale, accepting hyphens or underscores in its identifier.
    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// Select a calendar independently of the locale and format specification.
    pub fn with_calendar(mut self, calendar: CalendarAlgorithm) -> Self {
        self.calendar = Some(calendar);
        self
    }

    /// Set the display timezone for zoned datetimes. Date and Naive ignore it.
    pub fn with_timezone(mut self, timezone: Tz) -> Self {
        self.timezone = timezone;
        self
    }

    fn settings(&self) -> Settings<'_> {
        Settings {
            locale: self.locale.as_deref(),
            calendar: self.calendar,
        }
    }
}

// ICU's explicit-pattern API requires a concrete calendar type at preparation.
macro_rules! dispatch_calendar {
    ($calendar:expr, $cal:ident => $body:expr, $provider:expr) => {
        match $calendar {
            AnyCalendar::Buddhist($cal) => $body,
            AnyCalendar::Chinese($cal) => $body,
            AnyCalendar::Coptic($cal) => $body,
            AnyCalendar::Dangi($cal) => $body,
            AnyCalendar::Ethiopian($cal) => $body,
            AnyCalendar::Gregorian($cal) => $body,
            AnyCalendar::Hebrew($cal) => $body,
            AnyCalendar::HijriTabular($cal) => $body,
            AnyCalendar::HijriUmmAlQura($cal) => $body,
            AnyCalendar::Indian($cal) => $body,
            AnyCalendar::Japanese($cal) => $body,
            AnyCalendar::Persian($cal) => $body,
            AnyCalendar::Roc($cal) => $body,
            other => return Err($provider.calendar_error(other.kind())),
        }
    };
}

impl DateTimeFormatProvider for IcuPatternDateTimeFormatProvider {
    fn prepare_date(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        let settings = self.settings();
        let (prefs, kind) = settings.resolve()?;
        let calendar = AnyCalendar::new(kind);
        let formatter: Arc<dyn PreparedDateFormatter> = dispatch_calendar!(
            calendar, cal => Arc::new(Pattern::<_, DateFieldSet>::new(
                &settings, prefs, cal, pattern, DateTimeInputKind::Date,
            )?), settings
        );
        formatter.format(DateTime::UNIX_EPOCH.date_naive())?;
        Ok(formatter)
    }

    fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        let settings = self.settings();
        let (prefs, kind) = settings.resolve()?;
        let calendar = AnyCalendar::new(kind);
        let formatter: Arc<dyn PreparedNaiveDateTimeFormatter> = dispatch_calendar!(
            calendar, cal => Arc::new(Pattern::<_, CompositeDateTimeFieldSet>::new(
                &settings, prefs, cal, pattern, DateTimeInputKind::Naive,
            )?), settings
        );
        formatter.format(DateTime::UNIX_EPOCH.naive_utc())?;
        Ok(formatter)
    }

    fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        let settings = self.settings();
        let (prefs, kind) = settings.resolve()?;
        let calendar = AnyCalendar::new(kind);
        let formatter: Arc<dyn PreparedZonedDateTimeFormatter> = dispatch_calendar!(
            calendar, cal => Arc::new(ZonedFormat::new(&settings, self.timezone, prefs, cal, pattern)?), settings
        );
        formatter.format(DateTime::UNIX_EPOCH)?;
        Ok(formatter)
    }
}

impl DateTimeFormatProvider for IcuSemanticDateTimeFormatProvider {
    fn prepare_date(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        Ok(Arc::new(semantic::Prepared::new(
            &self.settings(),
            spec,
            DateTimeInputKind::Date,
        )?))
    }

    fn prepare_naive(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        Ok(Arc::new(semantic::Prepared::new(
            &self.settings(),
            spec,
            DateTimeInputKind::Naive,
        )?))
    }

    fn prepare_zoned(
        &self,
        spec: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        Ok(Arc::new(semantic::Zoned::new(
            &self.settings(),
            self.timezone,
            spec,
        )?))
    }
}

pub(crate) fn unsupported(input: DateTimeInputKind, message: String) -> DateTimeFormatError {
    DateTimeFormatError::UnsupportedPattern { input, message }
}
