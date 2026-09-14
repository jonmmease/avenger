use avenger_format_datetime::{
    DateTimeFormatContext, DateTimeFormatError, DateTimeFormatOverrides, FormattedDateTime,
    NaiveDateTimeInput, PreparedDateTimeFormat, ResolvedDateTimeLocale,
};
use avenger_format_number::{
    FormatError, FormattedNumber, NumberFormatContext, NumberFormatOverrides, PreparedNumberFormat,
    ResolvedNumberLocale,
};
use std::sync::Mutex;

#[derive(Debug)]
struct NumberEntry {
    spec: String,
    overrides: NumberFormatOverrides,
    locale: ResolvedNumberLocale,
    formatter: PreparedNumberFormat,
}
#[derive(Debug)]
struct DateTimeEntry {
    spec: String,
    overrides: DateTimeFormatOverrides,
    locale: ResolvedDateTimeLocale,
    timezone: chrono_tz::Tz,
    formatter: PreparedDateTimeFormat,
}

/// The most recent number and datetime formats are reused across parameterized labels.
#[derive(Debug, Default)]
pub(crate) struct FormattingCache {
    number: Mutex<Option<NumberEntry>>,
    datetime: Mutex<Option<DateTimeEntry>>,
}
impl FormattingCache {
    pub(crate) fn number(
        &self,
        value: f64,
        spec: &str,
        overrides: NumberFormatOverrides,
        context: NumberFormatContext<'_>,
    ) -> Result<FormattedNumber, FormatError> {
        let mut slot = self.number.lock().expect("formatting cache lock");
        if !slot.as_ref().is_some_and(|entry| {
            entry.spec == spec && entry.overrides == overrides && entry.locale == *context.locale
        }) {
            let formatter = PreparedNumberFormat::new(Some(spec), overrides.clone(), context)?;
            *slot = Some(NumberEntry {
                spec: spec.into(),
                overrides,
                locale: context.locale.clone(),
                formatter,
            });
        }
        Ok(slot
            .as_ref()
            .expect("prepared number formatter")
            .formatter
            .format(value))
    }
    pub(super) fn datetime(
        &self,
        value: super::markup::DatefmtValue,
        spec: &str,
        overrides: DateTimeFormatOverrides,
        context: DateTimeFormatContext<'_>,
    ) -> Result<FormattedDateTime, DateTimeFormatError> {
        let mut slot = self.datetime.lock().expect("formatting cache lock");
        if !slot.as_ref().is_some_and(|entry| {
            entry.spec == spec
                && entry.overrides == overrides
                && entry.locale == *context.locale
                && entry.timezone == context.timezone
        }) {
            let formatter = PreparedDateTimeFormat::new(Some(spec), overrides.clone(), context)?;
            *slot = Some(DateTimeEntry {
                spec: spec.into(),
                overrides,
                locale: context.locale.clone(),
                timezone: context.timezone,
                formatter,
            });
        }
        let formatter = &slot
            .as_ref()
            .expect("prepared datetime formatter")
            .formatter;
        match value {
            super::markup::DatefmtValue::UtcDateTime(value) => Ok(formatter.format_zoned(value)),
            super::markup::DatefmtValue::Date(value) => {
                formatter.format_naive(NaiveDateTimeInput::Date(value))
            }
            super::markup::DatefmtValue::DateTime(value) => {
                formatter.format_naive(NaiveDateTimeInput::DateTime(value))
            }
        }
    }
}
