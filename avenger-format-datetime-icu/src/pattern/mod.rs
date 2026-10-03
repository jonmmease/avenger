mod spec;

use crate::{input, settings::Settings, unsupported};
use avenger_format::{
    DateTimeFormatError, DateTimeInputKind, PreparedDateFormatter, PreparedNaiveDateTimeFormatter,
    PreparedZonedDateTimeFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use icu_calendar::{Calendar, Ref};
use icu_datetime::{
    fieldsets::enums::{CompositeDateTimeFieldSet, CompositeFieldSet, DateFieldSet},
    pattern::{
        DateTimePattern, FixedCalendarDateTimeNames, FormattedDateTimePattern,
        FormattedDateTimePatternError, PatternLoadError,
    },
    provider::Baked,
    scaffold::{CldrCalendar, DateTimeNamesMarker},
    DateTimeFormatterPreferences,
};
use icu_provider::DataProvider;
use icu_time::{DateTime as IcuDateTime, ZonedDateTime};
use std::fmt::Debug;
use writeable::TryWriteable;

/// Own the calendar, pattern, and names loaded during preparation.
#[derive(Debug)]
pub(crate) struct Pattern<C, F: DateTimeNamesMarker> {
    calendar: C,
    pattern: DateTimePattern,
    names: FixedCalendarDateTimeNames<C, F>,
}

impl<C: CldrCalendar, F: DateTimeNamesMarker> Pattern<C, F> {
    pub(crate) fn new(
        settings: &Settings<'_>,
        prefs: DateTimeFormatterPreferences,
        calendar: C,
        source: &str,
        input: DateTimeInputKind,
    ) -> Result<Self, DateTimeFormatError>
    where
        Baked: DataProvider<C::YearNamesV1> + DataProvider<C::MonthNamesV1>,
    {
        let pattern = spec::parse(source, input)?;
        let mut names = FixedCalendarDateTimeNames::try_new(prefs)
            .map_err(|error| settings.data_error(error))?;
        names
            .include_for_pattern(&pattern)
            .map_err(|error| match error {
                PatternLoadError::Data(error, _) => settings.data_error(error),
                _ => unsupported(input, error.to_string()),
            })?;
        Ok(Self {
            calendar,
            pattern,
            names,
        })
    }
}

impl<C> PreparedDateFormatter for Pattern<C, DateFieldSet>
where
    C: Calendar + CldrCalendar + Debug + Send + Sync + 'static,
{
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        render(
            self.names
                .with_pattern_unchecked(&self.pattern)
                .format(&input::date(value, Ref(&self.calendar))),
            DateTimeInputKind::Date,
        )
    }
}

impl<C> PreparedNaiveDateTimeFormatter for Pattern<C, CompositeDateTimeFieldSet>
where
    C: Calendar + CldrCalendar + Debug + Send + Sync + 'static,
{
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        let datetime = IcuDateTime {
            date: input::date(value.date(), Ref(&self.calendar)),
            time: input::time(value.time(), DateTimeInputKind::Naive)?,
        };
        render(
            self.names
                .with_pattern_unchecked(&self.pattern)
                .format(&datetime),
            DateTimeInputKind::Naive,
        )
    }
}

/// Retain both the transition rules and ICU's ID for localized timezone names.
#[derive(Debug)]
pub(crate) struct ZonedFormat<C: CldrCalendar> {
    pattern: Pattern<C, CompositeFieldSet>,
    zone: input::Zone,
}

impl<C: CldrCalendar> ZonedFormat<C> {
    pub(crate) fn new(
        settings: &Settings<'_>,
        timezone: Tz,
        prefs: DateTimeFormatterPreferences,
        calendar: C,
        source: &str,
    ) -> Result<Self, DateTimeFormatError>
    where
        Baked: DataProvider<C::YearNamesV1> + DataProvider<C::MonthNamesV1>,
    {
        Ok(Self {
            pattern: Pattern::new(settings, prefs, calendar, source, DateTimeInputKind::Zoned)?,
            zone: input::Zone::new(timezone),
        })
    }
}

impl<C> PreparedZonedDateTimeFormatter for ZonedFormat<C>
where
    C: Calendar + CldrCalendar + Debug + Send + Sync + 'static,
{
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        let (local, zone) = self.zone.localize(value)?;
        let datetime = ZonedDateTime {
            date: input::date(local.date(), Ref(&self.pattern.calendar)),
            time: input::time(local.time(), DateTimeInputKind::Zoned)?,
            zone,
        };
        render(
            self.pattern
                .names
                .with_pattern_unchecked(&self.pattern.pattern)
                .format(&datetime),
            DateTimeInputKind::Zoned,
        )
    }
}

/// Preserve ICU errors instead of returning their fallback output.
fn render(
    formatted: FormattedDateTimePattern<'_>,
    input: DateTimeInputKind,
) -> Result<String, DateTimeFormatError> {
    formatted
        .try_write_to_string()
        .map(|text| text.into_owned())
        .map_err(|(error, _)| {
            use FormattedDateTimePatternError::*;
            match error {
                UnsupportedField(_) | UnsupportedLength(_) | MissingInputField(_) => {
                    unsupported(input, error.to_string())
                }
                InvalidMonthCode(_) | InvalidEra(_) | InvalidCyclicYear { .. } => {
                    DateTimeFormatError::UnsupportedValue {
                        input,
                        message: error.to_string(),
                    }
                }
                _ => DateTimeFormatError::FormattingFailed,
            }
        })
}
