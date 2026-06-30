use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocaleId(pub String);

impl LocaleId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for LocaleId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LocaleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LocaleWeekday {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl LocaleWeekday {
    pub(crate) fn ordinal_from_sunday(self) -> u32 {
        match self {
            Self::Sunday => 0,
            Self::Monday => 1,
            Self::Tuesday => 2,
            Self::Wednesday => 3,
            Self::Thursday => 4,
            Self::Friday => 5,
            Self::Saturday => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Widths12Spec {
    pub narrow: Option<[String; 12]>,
    pub abbrev: Option<[String; 12]>,
    pub wide: Option<[String; 12]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Widths7Spec {
    pub narrow: Option<[String; 7]>,
    pub abbrev: Option<[String; 7]>,
    pub wide: Option<[String; 7]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Widths4Spec {
    pub narrow: Option<[String; 4]>,
    pub abbrev: Option<[String; 4]>,
    pub wide: Option<[String; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Widths2Spec {
    pub narrow: Option<[String; 2]>,
    pub abbrev: Option<[String; 2]>,
    pub wide: Option<[String; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DayPeriodsSpec {
    pub am: Option<String>,
    pub pm: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LengthsSpec {
    pub short: Option<String>,
    pub medium: Option<String>,
    pub long: Option<String>,
    pub full: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DateTimeLocaleSpec {
    pub base: Option<LocaleId>,
    pub months: Option<Widths12Spec>,
    pub months_standalone: Option<Widths12Spec>,
    pub weekdays: Option<Widths7Spec>,
    pub weekdays_standalone: Option<Widths7Spec>,
    pub quarters: Option<Widths4Spec>,
    pub quarters_standalone: Option<Widths4Spec>,
    pub eras: Option<Widths2Spec>,
    pub day_periods: Option<DayPeriodsSpec>,
    pub date_patterns: Option<LengthsSpec>,
    pub time_patterns: Option<LengthsSpec>,
    pub datetime_glue: Option<LengthsSpec>,
    pub first_day_of_week: Option<LocaleWeekday>,
    pub digits: Option<[String; 10]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widths12 {
    pub narrow: [String; 12],
    pub abbrev: [String; 12],
    pub wide: [String; 12],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widths7 {
    pub narrow: [String; 7],
    pub abbrev: [String; 7],
    pub wide: [String; 7],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widths4 {
    pub narrow: [String; 4],
    pub abbrev: [String; 4],
    pub wide: [String; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widths2 {
    pub narrow: [String; 2],
    pub abbrev: [String; 2],
    pub wide: [String; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayPeriods {
    pub am: String,
    pub pm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lengths {
    pub short: String,
    pub medium: String,
    pub long: String,
    pub full: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDateTimeLocale {
    pub id: LocaleId,
    pub months: Widths12,
    pub months_standalone: Widths12,
    pub weekdays: Widths7,
    pub weekdays_standalone: Widths7,
    pub quarters: Widths4,
    pub quarters_standalone: Widths4,
    pub eras: Widths2,
    pub day_periods: DayPeriods,
    pub date_patterns: Lengths,
    pub time_patterns: Lengths,
    pub datetime_glue: Lengths,
    pub first_day_of_week: LocaleWeekday,
    pub digits: Option<[String; 10]>,
}

impl ResolvedDateTimeLocale {
    pub fn en_us() -> Self {
        Self {
            id: LocaleId::new("en-US"),
            months: en_us_months(),
            months_standalone: en_us_months(),
            weekdays: en_us_weekdays(),
            weekdays_standalone: en_us_weekdays(),
            quarters: Widths4 {
                narrow: str_array4(["1", "2", "3", "4"]),
                abbrev: str_array4(["Q1", "Q2", "Q3", "Q4"]),
                wide: str_array4(["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"]),
            },
            quarters_standalone: Widths4 {
                narrow: str_array4(["1", "2", "3", "4"]),
                abbrev: str_array4(["Q1", "Q2", "Q3", "Q4"]),
                wide: str_array4(["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"]),
            },
            eras: Widths2 {
                narrow: str_array2(["B", "A"]),
                abbrev: str_array2(["BC", "AD"]),
                wide: str_array2(["Before Christ", "Anno Domini"]),
            },
            day_periods: DayPeriods {
                am: "AM".to_string(),
                pm: "PM".to_string(),
            },
            date_patterns: Lengths {
                short: "M/d/yy".to_string(),
                medium: "MMM d, y".to_string(),
                long: "MMMM d, y".to_string(),
                full: "EEEE, MMMM d, y".to_string(),
            },
            time_patterns: Lengths {
                short: "h:mm a".to_string(),
                medium: "h:mm:ss a".to_string(),
                long: "h:mm:ss a O".to_string(),
                full: "h:mm:ss a O".to_string(),
            },
            datetime_glue: Lengths {
                short: "{1}, {0}".to_string(),
                medium: "{1}, {0}".to_string(),
                long: "{1} 'at' {0}".to_string(),
                full: "{1} 'at' {0}".to_string(),
            },
            first_day_of_week: LocaleWeekday::Sunday,
            digits: None,
        }
    }
}

fn en_us_months() -> Widths12 {
    Widths12 {
        narrow: str_array12(["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"]),
        abbrev: str_array12([
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ]),
        wide: str_array12([
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
        ]),
    }
}

fn en_us_weekdays() -> Widths7 {
    Widths7 {
        narrow: str_array7(["S", "M", "T", "W", "T", "F", "S"]),
        abbrev: str_array7(["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]),
        wide: str_array7([
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ]),
    }
}

pub(crate) fn str_array12(values: [&str; 12]) -> [String; 12] {
    values.map(str::to_string)
}

pub(crate) fn str_array7(values: [&str; 7]) -> [String; 7] {
    values.map(str::to_string)
}

pub(crate) fn str_array4(values: [&str; 4]) -> [String; 4] {
    values.map(str::to_string)
}

pub(crate) fn str_array2(values: [&str; 2]) -> [String; 2] {
    values.map(str::to_string)
}
