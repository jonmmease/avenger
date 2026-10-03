use avenger_format::DateTimeFormatError;
use icu_calendar::{preferences::CalendarAlgorithm, AnyCalendarKind};
use icu_datetime::DateTimeFormatterPreferences;
use icu_locale_core::Locale;
use icu_provider::{DataError, DataErrorKind};

/// Borrow provider settings while validating preferences and loading locale data.
pub(crate) struct Settings<'a> {
    pub locale: Option<&'a str>,
    pub calendar: Option<CalendarAlgorithm>,
}

impl Settings<'_> {
    pub(crate) fn resolve(
        &self,
    ) -> Result<(DateTimeFormatterPreferences, AnyCalendarKind), DateTimeFormatError> {
        let locale = self
            .locale_name()
            .replace('_', "-")
            .parse::<Locale>()
            .map_err(|error| self.locale_error(error.to_string()))?;
        let mut prefs = DateTimeFormatterPreferences::from_locale_strict(&locale)
            .map_err(|_| self.locale_error("invalid Unicode locale preference".into()))?;
        if let Some(calendar) = self.calendar {
            prefs.calendar_algorithm = Some(calendar);
        }
        let kind =
            AnyCalendarKind::try_new((&prefs).into()).map_err(|error| self.data_error(error))?;
        Ok((prefs, kind))
    }

    pub(crate) fn locale_name(&self) -> &str {
        self.locale.unwrap_or("en-US")
    }

    pub(crate) fn locale_error(&self, message: String) -> DateTimeFormatError {
        DateTimeFormatError::LocaleUnavailable {
            locale: self.locale_name().into(),
            message,
        }
    }

    pub(crate) fn data_error(&self, error: DataError) -> DateTimeFormatError {
        match error.kind {
            DataErrorKind::IdentifierNotFound => self.locale_error(error.to_string()),
            _ => DateTimeFormatError::FormattingFailed,
        }
    }

    pub(crate) fn calendar_error(&self, kind: AnyCalendarKind) -> DateTimeFormatError {
        let message = format!("ICU datetime formatting does not support the {kind} calendar");
        if self.calendar.is_some() {
            DateTimeFormatError::InvalidOption {
                option: "calendar".into(),
                message,
            }
        } else {
            self.locale_error(message)
        }
    }
}

/// Store the optional ICU calendar as its CLDR identifier.
pub(crate) mod calendar_serde {
    use super::CalendarAlgorithm;
    use icu_locale_core::extensions::unicode::Value;
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        calendar: &Option<CalendarAlgorithm>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        calendar.map(|c| c.as_str()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<CalendarAlgorithm>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|name| {
                let value = name.parse::<Value>().map_err(D::Error::custom)?;
                CalendarAlgorithm::try_from(&value).map_err(D::Error::custom)
            })
            .transpose()
    }
}
