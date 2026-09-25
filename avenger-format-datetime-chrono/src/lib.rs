#![doc = include_str!("../README.md")]

use avenger_format::{
    DateTimeFormatConfig, DateTimeFormatError, DateTimeFormatProvider, DateTimeFormatRequest,
    NaiveDateTimeInput, PreparedCivilDateTimeFormatter, PreparedInstantFormatter,
    ZonedDateTimeInput,
};
use chrono::{
    format::{DelayedFormat, Item, Numeric, StrftimeItems},
    DateTime, Locale, Offset,
};
use chrono_tz::Tz;
use std::{slice, sync::Arc};

/// Chrono patterns, built-in locales, and IANA display timezones through the shared interface.
#[derive(Debug, Default)]
pub struct ChronoDateTimeFormatProvider;

impl DateTimeFormatProvider for ChronoDateTimeFormatProvider {
    fn prepare_naive(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        if request.timezone.is_some() {
            return Err(error(
                "civil datetime formatting cannot override the timezone",
            ));
        }
        let pattern = Pattern::new(config, request)?;
        // Chrono assumes UTC for naive timestamps. Civil values have no implied instant.
        if pattern
            .items
            .iter()
            .any(|item| matches!(item, Item::Numeric(Numeric::Timestamp, _)))
        {
            return Err(error("Chrono pattern `%s` requires a zoned datetime"));
        }
        // Rendering detects timezone-dependent and parsing-only items, including locale expansions.
        pattern
            .format(NaiveDateTimeInput::DateTime(
                DateTime::UNIX_EPOCH.naive_utc(),
            ))
            .map_err(|_| error("Chrono pattern cannot format a civil datetime"))?;
        Ok(Arc::new(pattern))
    }

    fn prepare_zoned(
        &self,
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        let name = request
            .timezone
            .as_deref()
            .or(config.timezone.as_deref())
            .unwrap_or("UTC");
        let timezone = name
            .parse::<Tz>()
            .map_err(|_| error(format!("invalid IANA timezone `{name}`")))?;
        let formatter = ZonedFormat {
            pattern: Pattern::new(config, request)?,
            timezone,
        };
        // Chrono parses some directives, such as %#z, that it cannot use for formatting.
        formatter
            .format(DateTime::UNIX_EPOCH)
            .map_err(|_| error("Chrono pattern contains a parsing-only directive"))?;
        Ok(Arc::new(formatter))
    }
}

/// Owned Chrono items with locale expansions resolved during preparation.
#[derive(Debug)]
struct Pattern {
    items: Vec<Item<'static>>,
    locale: Locale,
}

impl Pattern {
    fn new(
        config: &DateTimeFormatConfig,
        request: &DateTimeFormatRequest,
    ) -> Result<Self, DateTimeFormatError> {
        if let Some(name) = request.options.keys().next() {
            return Err(error(format!(
                "unsupported Chrono datetime format option `{name}`"
            )));
        }
        if !config.locales.is_empty() {
            return Err(error(
                "Chrono datetime formatting does not accept custom locale definitions",
            ));
        }
        let locale = match config.locale.as_deref() {
            Some(name) => name
                .parse::<Locale>()
                .map_err(|_| error(format!("unknown Chrono locale `{name}`")))?,
            None => Locale::POSIX,
        };
        let spec = request
            .spec
            .as_str()
            .ok_or_else(|| error("Chrono datetime specification must be a string"))?;
        let items = StrftimeItems::new_with_locale(spec, locale)
            .parse_to_owned()
            .map_err(|err| error(format!("invalid Chrono datetime pattern: {err}")))?;
        Ok(Self { items, locale })
    }
}

impl PreparedCivilDateTimeFormatter for Pattern {
    fn format(&self, value: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
        let value = value.datetime();
        render(DelayedFormat::new_with_locale(
            Some(value.date()),
            Some(value.time()),
            self.items.iter(),
            self.locale,
        ))
    }
}

#[derive(Debug)]
struct ZonedFormat {
    pattern: Pattern,
    timezone: Tz,
}

impl PreparedInstantFormatter for ZonedFormat {
    fn format(&self, value: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
        let display = value.with_timezone(&self.timezone);
        let local = display
            .naive_utc()
            .checked_add_offset(display.offset().fix())
            .ok_or_else(|| error("display datetime is outside the supported calendar range"))?;
        render(DelayedFormat::new_with_offset_and_locale(
            Some(local.date()),
            Some(local.time()),
            display.offset(),
            self.pattern.items.iter(),
            self.pattern.locale,
        ))
    }
}

fn render(
    format: DelayedFormat<slice::Iter<'_, Item<'static>>>,
) -> Result<String, DateTimeFormatError> {
    let mut text = String::new();
    format
        .write_to(&mut text)
        .map_err(|_| error("datetime cannot be rendered with the prepared Chrono pattern"))?;
    Ok(text)
}

fn error(message: impl Into<String>) -> DateTimeFormatError {
    DateTimeFormatError(message.into())
}
