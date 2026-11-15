//! Locales: the names of days, months, and periods of the day, and the formats of `%c`, `%x`, and
//! `%X`, from a d3 time locale.

use avenger_format::DateTimeFormatError;
use avenger_format_locales_d3::{bundled, DateTimeLocaleSpec};

use crate::element::{check, tokens, Kind, Rejection, Token};

/// The names and the composites that a format writes and a parse reads. The default is
/// GoogleSQL's: its en-US names, `AM` and `PM`, and the composites of its C library, `%c` as
/// `%a %b %e %H:%M:%S %EY`, `%x` as `%m/%d/%y`, and `%X` as `%H:%M:%S`. A locale from a d3 time
/// locale has the d3 locale's names, periods, and composites, so the bundled en-US locale writes
/// `%x` as `3/5/2024` where GoogleSQL writes `03/05/24`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Locale {
    pub(crate) days: [String; 7],
    pub(crate) short_days: [String; 7],
    pub(crate) months: [String; 12],
    pub(crate) short_months: [String; 12],
    pub(crate) periods: [String; 2],
    pub(crate) date_time: String,
    pub(crate) date: String,
    pub(crate) time: String,
}

impl Default for Locale {
    fn default() -> Self {
        let days = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ]
        .map(String::from);
        let months = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ]
        .map(String::from);
        Locale {
            short_days: days.clone().map(|day| day[..3].to_string()),
            short_months: months.clone().map(|month| month[..3].to_string()),
            days,
            months,
            periods: ["AM".into(), "PM".into()],
            date_time: "%a %b %e %H:%M:%S %EY".into(),
            date: "%m/%d/%y".into(),
            time: "%H:%M:%S".into(),
        }
    }
}

impl Locale {
    /// The locale of the d3 time locale `spec`. Its composites are GoogleSQL formats: `dateTime`
    /// of a datetime's elements, without `%c`, `date` of a date's, and `time` of a time's.
    pub fn new(spec: &DateTimeLocaleSpec) -> Result<Locale, DateTimeFormatError> {
        let composites = [
            ("dateTime", &spec.date_time, Kind::Datetime),
            ("date", &spec.date, Kind::Date),
            ("time", &spec.time, Kind::Time),
        ];
        for (field, format, kind) in composites {
            let invalid = |message: String| DateTimeFormatError::InvalidLocaleData {
                message: format!("field `{field}` {message}"),
            };
            check(format, kind).map_err(|rejection| {
                invalid(match rejection {
                    Rejection::Unknown { element, .. } => {
                        format!("has `{element}`, which is not an element of GoogleSQL's formats")
                    }
                    Rejection::Absent { element } => {
                        format!("has `{element}`, which needs a part that the value does not have")
                    }
                    Rejection::Unsupported => "has `%J`, which GoogleSQL does not format".into(),
                })
            })?;
            let refers_to_itself = (tokens(format).iter())
                .any(|token| matches!(token, Token::Element { letter: b'c', .. }));
            if refers_to_itself {
                return Err(invalid("has `%c`, which refers to itself".into()));
            }
        }
        Ok(Locale {
            days: spec.days.clone(),
            short_days: spec.short_days.clone(),
            months: spec.months.clone(),
            short_months: spec.short_months.clone(),
            periods: spec.periods.clone(),
            date_time: spec.date_time.clone(),
            date: spec.date.clone(),
            time: spec.time.clone(),
        })
    }

    /// The bundled locale named `id`, with hyphens or underscores, such as `fr-FR`: U.S.
    /// English, or all of d3's locales with the `all-locales` feature.
    pub fn bundled(id: &str) -> Result<Locale, DateTimeFormatError> {
        let spec = bundled(id).ok_or_else(|| DateTimeFormatError::LocaleUnavailable {
            locale: id.into(),
            message: "no bundled D3 definition is available in this build".into(),
        })?;
        Locale::new(&spec)
    }
}
