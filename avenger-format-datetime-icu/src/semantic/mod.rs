mod spec;

use crate::{input, settings::Settings, unsupported};
use avenger_format::{
    DateTimeFormatError, DateTimeInputKind, PreparedDateFormatter, PreparedNaiveDateTimeFormatter,
    PreparedZonedDateTimeFormatter,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use icu_datetime::{
    fieldsets::enums::CompositeFieldSet,
    pattern::PatternLoadError,
    unchecked::{DateTimeInputUnchecked, FormattedDateTimeUncheckedError},
    DateTimeFormatter, DateTimeFormatterLoadError,
};
use spec::Spec;
use writeable::TryWriteable;

/// Retain locale data and let ICU choose a pattern for each supplied value.
#[derive(Debug)]
pub(crate) struct Prepared {
    formatter: DateTimeFormatter<CompositeFieldSet>,
    prefix: String,
    suffix: String,
}

impl Prepared {
    pub fn new(
        settings: &Settings<'_>,
        source: &str,
        input: DateTimeInputKind,
    ) -> Result<Self, DateTimeFormatError> {
        let spec = Spec::parse(source, input)?;
        let (mut prefs, kind) = settings.resolve()?;
        if let Some(hour_cycle) = spec.hour_cycle {
            prefs.hour_cycle = Some(hour_cycle);
        }
        let formatter =
            DateTimeFormatter::try_new(prefs, spec.fields).map_err(|error| match error {
                DateTimeFormatterLoadError::Data(error)
                | DateTimeFormatterLoadError::Names(PatternLoadError::Data(error, _)) => {
                    settings.data_error(error)
                }
                DateTimeFormatterLoadError::Names(error) => unsupported(input, error.to_string()),
                _ => DateTimeFormatError::FormattingFailed,
            })?;
        // ICU can fall back to a locale's default for an unsupported calendar.
        if formatter.calendar().kind() != kind {
            return Err(settings.calendar_error(kind));
        }
        Ok(Self {
            formatter,
            prefix: spec.prefix,
            suffix: spec.suffix,
        })
    }

    fn date_input(&self, value: NaiveDate) -> DateTimeInputUnchecked {
        let mut input = DateTimeInputUnchecked::default();
        input.set_date_fields_unchecked(input::date(value, self.formatter.calendar()));
        input
    }

    fn render(
        &self,
        value: DateTimeInputUnchecked,
        kind: DateTimeInputKind,
    ) -> Result<String, DateTimeFormatError> {
        // Preparation checks the requested fields. Date conversion uses this formatter's calendar.
        let formatted = self.formatter.format_unchecked(value);
        let text = formatted.try_write_to_string().map_err(|(error, _)| {
            use FormattedDateTimeUncheckedError::*;
            match error {
                UnsupportedField(_) | UnsupportedLength(_) | MissingInputField(_) => {
                    unsupported(kind, error.to_string())
                }
                InvalidMonthCode(_) | InvalidEra(_) | InvalidCyclicYear { .. } => {
                    DateTimeFormatError::UnsupportedValue {
                        input: kind,
                        message: error.to_string(),
                    }
                }
                _ => DateTimeFormatError::FormattingFailed,
            }
        })?;
        Ok(format!("{}{}{}", self.prefix, text, self.suffix))
    }
}

impl PreparedDateFormatter for Prepared {
    fn format(&self, value: NaiveDate) -> Result<String, DateTimeFormatError> {
        self.render(self.date_input(value), DateTimeInputKind::Date)
    }
}

impl PreparedNaiveDateTimeFormatter for Prepared {
    fn format(&self, value: NaiveDateTime) -> Result<String, DateTimeFormatError> {
        let mut input = self.date_input(value.date());
        input.set_time_fields(input::time(value.time(), DateTimeInputKind::Naive)?);
        self.render(input, DateTimeInputKind::Naive)
    }
}

#[derive(Debug)]
pub(crate) struct Zoned {
    prepared: Prepared,
    zone: input::Zone,
}

impl Zoned {
    pub fn new(
        settings: &Settings<'_>,
        timezone: Tz,
        source: &str,
    ) -> Result<Self, DateTimeFormatError> {
        Ok(Self {
            prepared: Prepared::new(settings, source, DateTimeInputKind::Zoned)?,
            zone: input::Zone::new(timezone),
        })
    }
}

impl PreparedZonedDateTimeFormatter for Zoned {
    fn format(&self, value: DateTime<Utc>) -> Result<String, DateTimeFormatError> {
        let (local, zone) = self.zone.localize(value)?;
        let mut input = self.prepared.date_input(local.date());
        input.set_time_fields(input::time(local.time(), DateTimeInputKind::Zoned)?);
        input.set_time_zone_info_at_time_fields(zone);
        self.prepared.render(input, DateTimeInputKind::Zoned)
    }
}
