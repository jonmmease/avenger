use avenger_format::DateTimeFormatError;
use icu_calendar::{preferences::CalendarAlgorithm, AnyCalendarKind};
use icu_datetime::{
    provider::{names::DatetimeNamesWeekdayV1, semantic_skeletons::marker_attrs::WIDE, Baked},
    DateTimeFormatterPreferences,
};
use icu_locale_core::Locale;
use icu_provider::{
    DataError, DataErrorKind, DataIdentifierBorrowed, DataMarker, DataProvider, DataRequest,
};

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
        self.validate_locale(prefs)?;
        if let Some(calendar) = self.calendar {
            prefs.calendar_algorithm = Some(calendar);
        }
        let kind =
            AnyCalendarKind::try_new((&prefs).into()).map_err(|error| self.data_error(error))?;
        Ok((prefs, kind))
    }

    fn validate_locale(
        &self,
        prefs: DateTimeFormatterPreferences,
    ) -> Result<(), DateTimeFormatError> {
        // Wide weekday names provide calendar-independent localized data. ICU reports
        // root fallback as a successful load, so check the resolved locale as well.
        let locale = DatetimeNamesWeekdayV1::INFO.make_locale(prefs.locale_preferences);
        let response = DataProvider::<DatetimeNamesWeekdayV1>::load(
            &Baked,
            DataRequest {
                id: DataIdentifierBorrowed::for_marker_attributes_and_locale(WIDE, &locale),
                ..Default::default()
            },
        )
        .map_err(|error| self.data_error(error))?;
        if response
            .metadata
            .locale
            .is_some_and(|locale| locale.is_unknown())
        {
            return Err(self.locale_error("ICU has no localized datetime data".into()));
        }
        Ok(())
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
