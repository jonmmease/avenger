//! `CAST(value AS type FORMAT format [AT TIME ZONE zone])`, as GoogleSQL defines it: a date or a
//! time as a string and a string as a date or a time, through the format elements of dates and
//! times; bytes as a string and a string as bytes, through an encoding such as `BASE64`; and a
//! number as a string, through the number format models of [`super::number_format`].
//!
//! Internal functions format and parse each row, with GoogleSQL's messages where a format or a
//! value is invalid, at run time as GoogleSQL raises them. `SAFE_CAST` gives `NULL` there.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, AsArray, BinaryBuilder, Int64Builder, StringBuilder, StructArray,
        },
        compute::cast as arrow_cast,
        datatypes::{DataType, Field, Fields, Int64Type, TimeUnit},
    },
    common::{exec_err, Result, ScalarValue},
    functions::core::expr_fn::get_field,
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::{cast, form::Ty, lit_i64, lit_str};
use crate::types;

const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SECOND;

// ---------------------------------------------------------------------------------------------
// The format elements of dates and times.
// ---------------------------------------------------------------------------------------------

/// The category of a format element, as GoogleSQL's messages name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Literal,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Meridian,
    TimeZone,
    Century,
    Quarter,
    Week,
    Era,
    Misc,
}

impl Category {
    fn name(self) -> &'static str {
        match self {
            Category::Literal => "LITERAL",
            Category::Year => "YEAR",
            Category::Month => "MONTH",
            Category::Day => "DAY",
            Category::Hour => "HOUR",
            Category::Minute => "MINUTE",
            Category::Second => "SECOND",
            Category::Meridian => "MERIDIAN_INDICATOR",
            Category::TimeZone => "TIME_ZONE",
            Category::Century => "CENTURY",
            Category::Quarter => "QUARTER",
            Category::Week => "WEEK",
            Category::Era => "ERA_INDICATOR",
            Category::Misc => "MISC",
        }
    }
}

/// A format element of dates and times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Text that the output repeats and the input matches, such as `-` or `"T"`.
    Text,
    /// A run of spaces, which matches one or more whitespace characters of the input.
    Space,
    Yyyy,
    Yyy,
    Yy,
    Y,
    Rrrr,
    Rr,
    YCommaYyy,
    Mm,
    Mon,
    Month,
    D,
    Dd,
    Ddd,
    DayName,
    Dy,
    Hh,
    Hh12,
    Hh24,
    Mi,
    Ss,
    Sssss,
    /// `FFn`, the first `n` digits of the fraction of the second.
    Ff(u8),
    /// `AM` or `PM`, without dots.
    Am,
    /// `A.M.` or `P.M.`.
    AmDots,
    Tzh,
    Tzm,
    /// An element that GoogleSQL knows and neither formats nor parses, such as `Q`.
    Other,
}

/// The names of the elements, each with its kind and its category. The tokenizer takes the
/// longest name that the format starts with, without regard to case.
const ELEMENTS: &[(&str, Kind, Category)] = &[
    ("YYYY", Kind::Yyyy, Category::Year),
    ("YYY", Kind::Yyy, Category::Year),
    ("YY", Kind::Yy, Category::Year),
    ("Y", Kind::Y, Category::Year),
    ("RRRR", Kind::Rrrr, Category::Year),
    ("RR", Kind::Rr, Category::Year),
    ("Y,YYY", Kind::YCommaYyy, Category::Year),
    ("IYYY", Kind::Other, Category::Year),
    ("IYY", Kind::Other, Category::Year),
    ("IY", Kind::Other, Category::Year),
    ("I", Kind::Other, Category::Year),
    ("SYYYY", Kind::Other, Category::Year),
    ("YEAR", Kind::Other, Category::Year),
    ("SYEAR", Kind::Other, Category::Year),
    ("MM", Kind::Mm, Category::Month),
    ("MON", Kind::Mon, Category::Month),
    ("MONTH", Kind::Month, Category::Month),
    ("RM", Kind::Other, Category::Month),
    ("D", Kind::D, Category::Day),
    ("DD", Kind::Dd, Category::Day),
    ("DDD", Kind::Ddd, Category::Day),
    ("DAY", Kind::DayName, Category::Day),
    ("DY", Kind::Dy, Category::Day),
    ("J", Kind::Other, Category::Day),
    ("HH", Kind::Hh, Category::Hour),
    ("HH12", Kind::Hh12, Category::Hour),
    ("HH24", Kind::Hh24, Category::Hour),
    ("MI", Kind::Mi, Category::Minute),
    ("SS", Kind::Ss, Category::Second),
    ("SSSSS", Kind::Sssss, Category::Second),
    ("FF1", Kind::Ff(1), Category::Second),
    ("FF2", Kind::Ff(2), Category::Second),
    ("FF3", Kind::Ff(3), Category::Second),
    ("FF4", Kind::Ff(4), Category::Second),
    ("FF5", Kind::Ff(5), Category::Second),
    ("FF6", Kind::Ff(6), Category::Second),
    ("FF7", Kind::Ff(7), Category::Second),
    ("FF8", Kind::Ff(8), Category::Second),
    ("FF9", Kind::Ff(9), Category::Second),
    ("FF10", Kind::Ff(10), Category::Second),
    ("FF11", Kind::Ff(11), Category::Second),
    ("FF12", Kind::Ff(12), Category::Second),
    ("AM", Kind::Am, Category::Meridian),
    ("PM", Kind::Am, Category::Meridian),
    ("A.M.", Kind::AmDots, Category::Meridian),
    ("P.M.", Kind::AmDots, Category::Meridian),
    ("TZH", Kind::Tzh, Category::TimeZone),
    ("TZM", Kind::Tzm, Category::TimeZone),
    ("CC", Kind::Other, Category::Century),
    ("SCC", Kind::Other, Category::Century),
    ("Q", Kind::Other, Category::Quarter),
    ("W", Kind::Other, Category::Week),
    ("WW", Kind::Other, Category::Week),
    ("IW", Kind::Other, Category::Week),
    ("AD", Kind::Other, Category::Era),
    ("BC", Kind::Other, Category::Era),
    ("A.D.", Kind::Other, Category::Era),
    ("B.C.", Kind::Other, Category::Era),
    ("SP", Kind::Other, Category::Misc),
    ("TH", Kind::Other, Category::Misc),
    ("FM", Kind::Other, Category::Misc),
];

/// An element of a format, with its text as the format writes it.
#[derive(Clone, Debug)]
struct Element {
    kind: Kind,
    category: Category,
    /// The element's name in upper case, or a literal's text as the format writes it, quotes
    /// and all, as GoogleSQL's messages show them.
    name: String,
    /// The element as the format writes it, whose case the output of a name matches.
    written: String,
    /// The text that a literal writes and matches.
    text: String,
}

/// The elements of the format `format`, or GoogleSQL's message for text that no element
/// matches.
fn elements(format: &str) -> Result<Vec<Element>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < format.len() {
        let rest = &format[at..];
        let c = rest.chars().next().expect("a character");
        if c == '"' {
            let mut text = String::new();
            let mut end = None;
            let mut chars = rest.char_indices().skip(1);
            while let Some((i, c)) = chars.next() {
                match c {
                    '"' => {
                        end = Some(i);
                        break;
                    }
                    '\\' => match chars.next() {
                        Some((_, escaped @ ('"' | '\\'))) => text.push(escaped),
                        Some((_, escaped)) => {
                            return Err(format!(
                                "Unsupported escape sequence \\{escaped} in text at {at}"
                            ))
                        }
                        None => break,
                    },
                    c => text.push(c),
                }
            }
            let Some(end) = end else {
                return Err(format!(
                    "Cannot find matching \" for quoted literal at {at}"
                ));
            };
            out.push(Element {
                kind: Kind::Text,
                category: Category::Literal,
                name: rest[..=end].to_string(),
                written: rest[..=end].to_string(),
                text,
            });
            at += end + 1;
            continue;
        }
        if c == ' ' {
            let spaces = rest.len() - rest.trim_start_matches(' ').len();
            out.push(Element {
                kind: Kind::Space,
                category: Category::Literal,
                name: rest[..spaces].to_string(),
                written: rest[..spaces].to_string(),
                text: rest[..spaces].to_string(),
            });
            at += spaces;
            continue;
        }
        let upper = rest.to_ascii_uppercase();
        let matched = (ELEMENTS.iter())
            .filter(|(name, ..)| upper.starts_with(name))
            .max_by_key(|(name, ..)| name.len());
        if let Some((name, kind, category)) = matched {
            out.push(Element {
                kind: *kind,
                category: *category,
                name: name.to_string(),
                written: rest[..name.len()].to_string(),
                text: String::new(),
            });
            at += name.len();
            continue;
        }
        if matches!(c, '-' | '.' | '/' | ',' | '\'' | ';' | ':') {
            out.push(Element {
                kind: Kind::Text,
                category: Category::Literal,
                name: c.to_string(),
                written: c.to_string(),
                text: c.to_string(),
            });
            at += 1;
            continue;
        }
        return Err(format!("Cannot find matched format element at {at}"));
    }
    Ok(out)
}

/// A type of a date or a time, which a format formats or parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Temporal {
    Date,
    Time,
    Datetime,
    Timestamp,
}

impl Temporal {
    fn of(ty: &Ty) -> Option<Temporal> {
        Some(match ty {
            Ty::Date => Temporal::Date,
            Ty::Time => Temporal::Time,
            Ty::Datetime => Temporal::Datetime,
            Ty::Timestamp => Temporal::Timestamp,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Temporal::Date => "DATE",
            Temporal::Time => "TIME",
            Temporal::Datetime => "DATETIME",
            Temporal::Timestamp => "TIMESTAMP",
        }
    }

    fn has_date(self) -> bool {
        self != Temporal::Time
    }

    fn has_time(self) -> bool {
        self != Temporal::Date
    }

    /// Whether a value of this type has the parts of `category`, for formatting.
    fn formats(self, category: Category) -> bool {
        match category {
            Category::Literal => true,
            Category::Year | Category::Month | Category::Day => self.has_date(),
            Category::Hour | Category::Minute | Category::Second | Category::Meridian => {
                self.has_time()
            }
            _ => self == Temporal::Timestamp,
        }
    }

    /// Whether a value of this type has the parts of `category`, for parsing.
    fn parses(self, category: Category) -> bool {
        match category {
            Category::TimeZone => self == Temporal::Timestamp,
            Category::Year | Category::Month | Category::Day => self.has_date(),
            Category::Hour | Category::Minute | Category::Second | Category::Meridian => {
                self.has_time()
            }
            _ => true,
        }
    }
}

/// The elements of `format` for formatting a value of the type `ty`, or GoogleSQL's message.
fn format_elements(format: &str, ty: Temporal) -> Result<Vec<Element>, String> {
    let elements = elements(format)?;
    if let Some(element) = (elements.iter()).find(|element| !ty.formats(element.category)) {
        return Err(format!("{} does not support '{}'", ty.name(), element.name));
    }
    if let Some(element) =
        (elements.iter()).find(|element| matches!(element.kind, Kind::Other | Kind::YCommaYyy))
    {
        return Err(format!("Unsupported format element '{}'", element.name));
    }
    Ok(elements)
}

/// The elements of `format` for parsing a value of the type `ty`, checked against GoogleSQL's
/// rules of a format model, or GoogleSQL's message.
fn parse_elements(format: &str, ty: Temporal) -> Result<Vec<Element>, String> {
    let elements = elements(format)?;
    let mut seen: Vec<&str> = Vec::new();
    for element in &elements {
        match element.kind {
            Kind::Text | Kind::Space => continue,
            Kind::Ff(n) if n > 9 => {
                return Err(format!(
                    "Format element '{}' is not supported",
                    element.name
                ))
            }
            Kind::Other | Kind::D | Kind::Ddd | Kind::DayName | Kind::Dy | Kind::Am => {
                return Err(format!(
                    "Format element '{}' is not supported for parsing",
                    element.name
                ))
            }
            _ => {}
        }
        if seen.contains(&element.name.as_str()) {
            return Err(format!(
                "Format element '{}' appears more than once in the format string",
                element.name
            ));
        }
        seen.push(&element.name);
    }
    if let Some(element) = (elements.iter()).find(|element| !ty.parses(element.category)) {
        return Err(format!(
            "Format element in category {} ('{}') is not allowed for output type {}",
            element.category.name(),
            element.name,
            ty.name()
        ));
    }
    let find = |kinds: &[Kind]| {
        elements
            .iter()
            .find(|element| kinds.contains(&element.kind))
    };
    for (index, element) in elements.iter().enumerate() {
        if matches!(element.kind, Kind::Ss | Kind::Sssss) {
            if let Some(other) = (elements[index + 1..].iter())
                .find(|other| matches!(other.kind, Kind::Ss | Kind::Sssss))
            {
                let (sssss, ss) = match element.kind {
                    Kind::Sssss => (element, other),
                    _ => (other, element),
                };
                return Err(format!(
                    "Format elements '{}' and '{}' cannot exist simultaneously",
                    sssss.name, ss.name
                ));
            }
            continue;
        }
        let single = matches!(
            element.category,
            Category::Year | Category::Month | Category::Day | Category::Hour | Category::Meridian
        );
        if !single {
            continue;
        }
        if let Some(other) =
            (elements[index + 1..].iter()).find(|other| other.category == element.category)
        {
            return Err(format!(
                "More than one format element in category {} exist: '{}' and '{}'",
                element.category.name(),
                element.name,
                other.name
            ));
        }
    }
    let meridian = elements.iter().find(|e| e.category == Category::Meridian);
    let twelve = find(&[Kind::Hh, Kind::Hh12]);
    if let (Some(meridian), Some(hh24)) = (meridian, find(&[Kind::Hh24])) {
        return Err(format!(
            "Format element in category MERIDIAN_INDICATOR ('{}') and format element '{}' cannot exist simultaneously",
            meridian.name, hh24.name
        ));
    }
    if let (Some(hh), None) = (twelve, meridian) {
        return Err(format!(
            "Format element in category MERIDIAN_INDICATOR is required when format element '{}' exists",
            hh.name
        ));
    }
    if let (None, Some(meridian)) = (twelve, meridian) {
        return Err(format!(
            "Format element of type HH/HH12 is required when format element in category MERIDIAN_INDICATOR ('{}') exists",
            meridian.name
        ));
    }
    if find(&[Kind::Sssss]).is_some() {
        for category in [Category::Hour, Category::Minute, Category::Meridian] {
            if let Some(element) = elements.iter().find(|e| e.category == category) {
                return Err(format!(
                    "Format element in category {} ('{}') and format element 'SSSSS' cannot exist simultaneously",
                    category.name(),
                    element.name
                ));
            }
        }
    }
    Ok(elements)
}

// ---------------------------------------------------------------------------------------------
// Civil dates.
// ---------------------------------------------------------------------------------------------

/// The days since 1970-01-01 of a civil date, after Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date of the days since 1970-01-01, after Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

const MONTHS: [&str; 12] = [
    "JANUARY",
    "FEBRUARY",
    "MARCH",
    "APRIL",
    "MAY",
    "JUNE",
    "JULY",
    "AUGUST",
    "SEPTEMBER",
    "OCTOBER",
    "NOVEMBER",
    "DECEMBER",
];

const DAYS: [&str; 7] = [
    "SUNDAY",
    "MONDAY",
    "TUESDAY",
    "WEDNESDAY",
    "THURSDAY",
    "FRIDAY",
    "SATURDAY",
];

/// `word`, an upper-case name, in the case of the element `written`: upper case when its first
/// two letters are, capitalized when only its first letter is, and lower case otherwise.
fn matched_case(word: &str, written: &str) -> String {
    let mut letters = written.chars().filter(|c| c.is_ascii_alphabetic());
    let first = letters.next().is_some_and(|c| c.is_ascii_uppercase());
    let second = letters.next().is_some_and(|c| c.is_ascii_uppercase());
    match (first, second) {
        (true, true) => word.to_string(),
        (true, false) => {
            let mut out = word[..1].to_string();
            out.push_str(&word[1..].to_ascii_lowercase());
            out
        }
        (false, _) => word.to_ascii_lowercase(),
    }
}

// ---------------------------------------------------------------------------------------------
// Formatting.
// ---------------------------------------------------------------------------------------------

/// The text of a date or a time: `micros`, the microseconds since 1970-01-01 of its civil date
/// and time, or of midnight for a `TIME`, and `offset`, the seconds of its time zone's offset.
fn format_value(elements: &[Element], micros: i64, offset: i64) -> String {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let day_micros = micros.rem_euclid(MICROS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = day_micros / (3600 * MICROS_PER_SECOND);
    let minute = day_micros / (60 * MICROS_PER_SECOND) % 60;
    let second = day_micros / MICROS_PER_SECOND % 60;
    let fraction = day_micros % MICROS_PER_SECOND;
    let mut out = String::new();
    for element in elements {
        let text = match element.kind {
            Kind::Text | Kind::Space => element.text.clone(),
            Kind::Yyyy | Kind::Rrrr => format!("{year:04}"),
            Kind::Yyy => format!("{:03}", year % 1000),
            Kind::Yy | Kind::Rr => format!("{:02}", year % 100),
            Kind::Y => format!("{}", year % 10),
            Kind::Mm => format!("{month:02}"),
            Kind::Mon => matched_case(&MONTHS[month as usize - 1][..3], &element.written),
            Kind::Month => matched_case(MONTHS[month as usize - 1], &element.written),
            Kind::D => format!("{}", (days + 4).rem_euclid(7) + 1),
            Kind::Dd => format!("{day:02}"),
            Kind::Ddd => format!("{:03}", days - days_from_civil(year, 1, 1) + 1),
            Kind::DayName => {
                matched_case(DAYS[(days + 4).rem_euclid(7) as usize], &element.written)
            }
            Kind::Dy => matched_case(
                &DAYS[(days + 4).rem_euclid(7) as usize][..3],
                &element.written,
            ),
            Kind::Hh | Kind::Hh12 => format!("{:02}", (hour + 11) % 12 + 1),
            Kind::Hh24 => format!("{hour:02}"),
            Kind::Mi => format!("{minute:02}"),
            Kind::Ss => format!("{second:02}"),
            Kind::Sssss => format!("{:05}", day_micros / MICROS_PER_SECOND),
            Kind::Ff(n) => {
                // Nine digits of nanoseconds, then zeros, truncated to `n`.
                let mut digits = format!("{:09}", fraction * 1000);
                while digits.len() < n as usize {
                    digits.push('0');
                }
                digits.truncate(n as usize);
                digits
            }
            Kind::Am | Kind::AmDots => {
                let text = match (hour < 12, element.kind) {
                    (true, Kind::Am) => "AM",
                    (false, Kind::Am) => "PM",
                    (true, _) => "A.M.",
                    (false, _) => "P.M.",
                };
                match element
                    .written
                    .starts_with(|c: char| c.is_ascii_uppercase())
                {
                    true => text.to_string(),
                    false => text.to_ascii_lowercase(),
                }
            }
            Kind::Tzh => {
                let sign = if offset < 0 { '-' } else { '+' };
                format!("{sign}{:02}", offset.abs() / 3600)
            }
            Kind::Tzm => format!("{:02}", offset.abs() / 60 % 60),
            Kind::YCommaYyy | Kind::Other => unreachable!("rejected before formatting"),
        };
        out.push_str(&text);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Parsing.
// ---------------------------------------------------------------------------------------------

/// What a string parses to: a civil date and time in microseconds since 1970-01-01, and the
/// seconds of the offset of its time zone when the string has one.
struct Parsed {
    micros: i64,
    offset: Option<i64>,
}

/// Whether an element reads digits, so the element before it reads its exact width.
fn reads_digits(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Yyyy
            | Kind::Yyy
            | Kind::Yy
            | Kind::Y
            | Kind::Rrrr
            | Kind::Rr
            | Kind::YCommaYyy
            | Kind::Mm
            | Kind::Dd
            | Kind::Hh
            | Kind::Hh12
            | Kind::Hh24
            | Kind::Mi
            | Kind::Ss
            | Kind::Sssss
            | Kind::Ff(_)
            | Kind::Tzm
    )
}

/// The date and time of `input` through `elements`, with the year and the month of `now`, a
/// civil date and time in microseconds, where `input` has none, or GoogleSQL's message.
fn parse_value(elements: &[Element], input: &str, now: i64) -> Result<Parsed, String> {
    let (now_year, now_month, _) = civil_from_days(now.div_euclid(MICROS_PER_DAY));
    let bytes = input.as_bytes();
    let mut at = input.len() - input.trim_start().len();
    let mut year: Option<i64> = None;
    let mut month: Option<i64> = None;
    let mut day: Option<i64> = None;
    let (mut hour, mut hour12, mut minute, mut second) = (None, None, None, None);
    let mut seconds_of_day: Option<i64> = None;
    let mut fraction_micros: Option<i64> = None;
    let mut pm: Option<bool> = None;
    let (mut tz_sign, mut tz_hour, mut tz_minute) = (1, None, None);
    for (index, element) in elements.iter().enumerate() {
        if at >= input.len() {
            return Err(format!(
                "Entire timestamp string has been parsed before dealing with format element '{}'",
                element.name
            ));
        }
        // An element that doesn't match fails at its start.
        let start = at;
        let failed = || {
            format!(
                "Failed to parse input timestamp string at {start} with format element '{}'",
                element.name
            )
        };
        let delimited = !(elements.get(index + 1)).is_some_and(|next| reads_digits(next.kind));
        // Up to `width` digits, or exactly `width` where the next element reads digits too.
        let digits = |at: &mut usize, width: usize| -> Result<(i64, usize), String> {
            let count = (bytes[*at..].iter().take(width))
                .take_while(|b| b.is_ascii_digit())
                .count();
            if count == 0 || (!delimited && count < width) {
                return Err(failed());
            }
            let value: i64 = input[*at..*at + count].parse().map_err(|_| failed())?;
            *at += count;
            Ok((value, count))
        };
        let in_range = |value: i64, low: i64, high: i64| match (low..=high).contains(&value) {
            true => Ok(value),
            false => Err(failed()),
        };
        match element.kind {
            Kind::Text => match input[at..].starts_with(&element.text) {
                true => at += element.text.len(),
                false => return Err(failed()),
            },
            Kind::Space => {
                let rest = &input[at..];
                let skipped = rest.len() - rest.trim_start().len();
                if skipped == 0 {
                    return Err(failed());
                }
                at += skipped;
            }
            Kind::Yyyy | Kind::Rrrr => {
                let width = if delimited { 5 } else { 4 };
                let (value, _) = digits(&mut at, width)?;
                year = Some(in_range(value, 0, 10_000)?);
            }
            Kind::Yyy => year = Some(now_year / 1000 * 1000 + digits(&mut at, 3)?.0),
            Kind::Yy => year = Some(now_year / 100 * 100 + digits(&mut at, 2)?.0),
            Kind::Y => year = Some(now_year / 10 * 10 + digits(&mut at, 1)?.0),
            Kind::Rr => {
                let (value, _) = digits(&mut at, 2)?;
                let century = now_year / 100 * 100;
                let recent = now_year % 100 < 50;
                year = Some(match (value < 50, recent) {
                    (true, true) | (false, false) => century + value,
                    (true, false) => century + 100 + value,
                    (false, true) => century - 100 + value,
                });
            }
            Kind::YCommaYyy => {
                let start = at;
                let high = (bytes[at..].iter().take(2))
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                let comma = start + high;
                let low_ok = high > 0
                    && bytes.get(comma) == Some(&b',')
                    && (bytes.get(comma + 1..comma + 4))
                        .is_some_and(|low| low.iter().all(u8::is_ascii_digit));
                if !low_ok {
                    return Err(failed());
                }
                let value: i64 =
                    format!("{}{}", &input[start..comma], &input[comma + 1..comma + 4])
                        .parse()
                        .map_err(|_| failed())?;
                at = comma + 4;
                year = Some(in_range(value, 0, 10_000)?);
            }
            Kind::Mm => month = Some(in_range(digits(&mut at, 2)?.0, 1, 12)?),
            Kind::Mon | Kind::Month => {
                let rest = input[at..].to_ascii_uppercase();
                let found = (MONTHS.iter().enumerate()).find_map(|(index, name)| {
                    let name = match element.kind {
                        Kind::Mon => &name[..3],
                        _ => name,
                    };
                    rest.starts_with(name).then_some((index, name.len()))
                });
                let Some((index, len)) = found else {
                    return Err(failed());
                };
                month = Some(index as i64 + 1);
                at += len;
            }
            Kind::Dd => day = Some(in_range(digits(&mut at, 2)?.0, 1, 31)?),
            Kind::Hh | Kind::Hh12 => hour12 = Some(in_range(digits(&mut at, 2)?.0, 1, 12)?),
            Kind::Hh24 => hour = Some(in_range(digits(&mut at, 2)?.0, 0, 23)?),
            Kind::Mi => minute = Some(in_range(digits(&mut at, 2)?.0, 0, 59)?),
            Kind::Ss => second = Some(in_range(digits(&mut at, 2)?.0, 0, 59)?),
            Kind::Sssss => seconds_of_day = Some(in_range(digits(&mut at, 5)?.0, 0, 86_399)?),
            Kind::Ff(n) => {
                let (value, count) = digits(&mut at, n as usize)?;
                // The digits as a fraction, in microseconds, truncated.
                let mut nanos = value;
                for _ in count..9 {
                    nanos *= 10;
                }
                fraction_micros = Some(nanos / 1000);
            }
            Kind::AmDots => {
                let rest = input[at..].to_ascii_uppercase();
                pm = Some(match () {
                    _ if rest.starts_with("A.M.") => false,
                    _ if rest.starts_with("P.M.") => true,
                    _ => return Err(failed()),
                });
                at += 4;
            }
            Kind::Tzh => {
                tz_sign = match bytes[at] {
                    b'+' | b' ' => 1,
                    b'-' => -1,
                    _ => return Err(failed()),
                };
                at += 1;
                if at >= input.len() {
                    return Err(failed());
                }
                tz_hour = Some(in_range(digits(&mut at, 2)?.0, 0, 14)?);
            }
            Kind::Tzm => tz_minute = Some(in_range(digits(&mut at, 2)?.0, 0, 59)?),
            Kind::D | Kind::Ddd | Kind::DayName | Kind::Dy | Kind::Am | Kind::Other => {
                unreachable!("rejected before parsing")
            }
        }
    }
    let rest = input[at..].trim_start();
    if !rest.is_empty() {
        return Err(format!(
            "Illegal non-space trailing data '{rest}' in timestamp string"
        ));
    }
    let year = year.unwrap_or(now_year);
    let month = month.unwrap_or(now_month);
    let day = day.unwrap_or(1);
    if !(1..=9999).contains(&year) {
        return Err("The parsing result is out of valid time range".to_string());
    }
    if day > days_in_month(year, month) {
        return Err("Invalid result from year, month, day values after parsing".to_string());
    }
    let hour = match (hour, hour12, pm) {
        (Some(hour), ..) => hour,
        (None, Some(hour12), Some(pm)) => hour12 % 12 + if pm { 12 } else { 0 },
        _ => 0,
    };
    let seconds = match seconds_of_day {
        Some(seconds) => seconds,
        None => hour * 3600 + minute.unwrap_or(0) * 60 + second.unwrap_or(0),
    };
    let micros = days_from_civil(year, month, day) * MICROS_PER_DAY
        + seconds * MICROS_PER_SECOND
        + fraction_micros.unwrap_or(0);
    let offset = match (tz_hour, tz_minute) {
        (None, None) => None,
        (hours, minutes) => {
            let (hours, minutes) = (hours.unwrap_or(0), minutes.unwrap_or(0));
            if hours * 60 + minutes > 14 * 60 {
                let sign = if tz_sign < 0 { '-' } else { '+' };
                return Err(format!("Invalid time zone: {sign}{hours:02}{minutes:02}"));
            }
            Some(tz_sign * (hours * 3600 + minutes * 60))
        }
    };
    Ok(Parsed { micros, offset })
}

// ---------------------------------------------------------------------------------------------
// Bytes.
// ---------------------------------------------------------------------------------------------

/// An encoding of bytes as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Base2,
    Base8,
    Base16,
    Base64,
    Base64Mime,
    Ascii,
    Utf8,
}

/// The encoding that `format` names, without regard to case, or GoogleSQL's message.
fn encoding(format: &str) -> Result<Encoding, String> {
    Ok(match format.to_ascii_uppercase().as_str() {
        "BASE2" => Encoding::Base2,
        "BASE8" => Encoding::Base8,
        "HEX" | "BASE16" => Encoding::Base16,
        "BASE64" => Encoding::Base64,
        "BASE64M" => Encoding::Base64Mime,
        "ASCII" => Encoding::Ascii,
        "UTF-8" | "UTF8" => Encoding::Utf8,
        _ => return Err(format!("Invalid format '{format}'")),
    })
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The digits of `bytes` in the radix `2^bits`, in groups of `group` bytes from the right, each
/// as many digits as its bits need.
fn radix_digits(bytes: &[u8], bits: u32, group: usize) -> String {
    let mut out = String::new();
    let head = bytes.len() % group;
    let chunks = std::iter::once(&bytes[..head])
        .filter(|chunk| !chunk.is_empty())
        .chain(bytes[head..].chunks(group));
    for chunk in chunks {
        let value = chunk
            .iter()
            .fold(0u64, |value, byte| value << 8 | u64::from(*byte));
        let width = (chunk.len() as u32 * 8).div_ceil(bits);
        for index in (0..width).rev() {
            let digit = (value >> (index * bits)) & ((1 << bits) - 1);
            out.push(char::from_digit(digit as u32, 1 << bits).expect("a digit"));
        }
    }
    out
}

/// The text of `bytes` in `encoding`, or GoogleSQL's message.
fn encode(bytes: &[u8], encoding: Encoding) -> Result<String, String> {
    Ok(match encoding {
        Encoding::Base2 => radix_digits(bytes, 1, 1),
        Encoding::Base8 => radix_digits(bytes, 3, 3),
        Encoding::Base16 => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        Encoding::Base64 | Encoding::Base64Mime => {
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let value = (chunk.iter().enumerate()).fold(0u32, |value, (i, byte)| {
                    value | u32::from(*byte) << (16 - 8 * i)
                });
                for index in 0..4 {
                    match index <= chunk.len() {
                        true => out.push(BASE64[(value >> (18 - 6 * index) & 63) as usize] as char),
                        false => out.push('='),
                    }
                }
            }
            if encoding == Encoding::Base64Mime {
                let lines: Vec<&str> = (out.as_bytes().chunks(76))
                    .map(|line| std::str::from_utf8(line).expect("ASCII"))
                    .collect();
                out = lines.join("\n");
            }
            out
        }
        Encoding::Ascii => match bytes.is_ascii() {
            true => String::from_utf8(bytes.to_vec()).expect("ASCII"),
            false => return Err("Failed to decode invalid ASCII string".to_string()),
        },
        Encoding::Utf8 => String::from_utf8(bytes.to_vec())
            .map_err(|_| "A string is not valid UTF-8.".to_string())?,
    })
}

/// The bytes of `digits` in the radix `2^bits`, in groups of `group_digits` digits from the
/// right, each of `group_bytes` bytes. A partial group at the left gives `partial(n)` bytes for
/// its `n` digits, of the low bits of its value.
fn radix_bytes(
    digits: &[u32],
    bits: u32,
    group_digits: usize,
    group_bytes: usize,
    partial: fn(usize) -> usize,
) -> Vec<u8> {
    let head = digits.len() % group_digits;
    let mut out = Vec::new();
    let chunks = std::iter::once(&digits[..head])
        .filter(|chunk| !chunk.is_empty())
        .chain(digits[head..].chunks(group_digits));
    for chunk in chunks {
        let value = (chunk.iter()).fold(0u64, |value, digit| value << bits | u64::from(*digit));
        let width = match chunk.len() == group_digits {
            true => group_bytes,
            false => partial(chunk.len()),
        };
        for index in (0..width).rev() {
            out.push((value >> (index * 8)) as u8);
        }
    }
    out
}

/// The bytes of `text` in `encoding`, or GoogleSQL's message.
fn decode(text: &str, encoding: Encoding) -> Result<Vec<u8>, String> {
    Ok(match encoding {
        Encoding::Base2 => {
            let mut digits = Vec::new();
            for (offset, c) in text.char_indices() {
                match c {
                    '0' | '1' => digits.push(c as u32 - '0' as u32),
                    c => {
                        return Err(format!(
                        "Failed to decode invalid base2 string due to character '{c}' at offset {offset}"
                    ))
                    }
                }
            }
            radix_bytes(&digits, 1, 8, 1, |_| 1)
        }
        Encoding::Base8 => {
            let digits: Option<Vec<u32>> = text.chars().map(|c| c.to_digit(8)).collect();
            let digits = digits.ok_or("Failed to decode invalid base8 string")?;
            // Eight digits are three bytes, and fewer digits one to three.
            radix_bytes(&digits, 3, 8, 3, |n| n.div_ceil(3))
        }
        Encoding::Base16 => {
            let mut digits = Vec::new();
            for (offset, c) in text.char_indices() {
                match c.to_digit(16) {
                    Some(digit) => digits.push(digit),
                    None => {
                        return Err(format!(
                        "Failed to decode invalid hexadecimal string due to character at offset {offset}: {text}"
                    ))
                    }
                }
            }
            radix_bytes(&digits, 4, 2, 1, |_| 1)
        }
        Encoding::Base64 | Encoding::Base64Mime => {
            let invalid = || "Failed to decode invalid base64 string".to_string();
            let symbols: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
            let data_len = symbols
                .iter()
                .position(|b| *b == b'=')
                .unwrap_or(symbols.len());
            let padding = symbols.len() - data_len;
            // Padding is optional, and when present completes the last group.
            if symbols[data_len..].iter().any(|b| *b != b'=')
                || (padding > 0 && (padding > 2 || !symbols.len().is_multiple_of(4)))
                || data_len % 4 == 1
            {
                return Err(invalid());
            }
            let mut out = Vec::new();
            for chunk in symbols[..data_len].chunks(4) {
                let mut value = 0u32;
                for (i, symbol) in chunk.iter().enumerate() {
                    let index = BASE64
                        .iter()
                        .position(|b| b == symbol)
                        .ok_or_else(invalid)?;
                    value |= (index as u32) << (18 - 6 * i);
                }
                for i in 0..chunk.len() - 1 {
                    out.push((value >> (16 - 8 * i)) as u8);
                }
            }
            out
        }
        Encoding::Ascii => match text.is_ascii() {
            true => text.as_bytes().to_vec(),
            false => return Err("Failed to decode invalid ASCII string".to_string()),
        },
        Encoding::Utf8 => text.as_bytes().to_vec(),
    })
}

// ---------------------------------------------------------------------------------------------
// The text of a value without a format.
// ---------------------------------------------------------------------------------------------

/// `value` as C's `printf` writes it with `%.{precision}g`.
fn printf_g(value: f64, precision: usize) -> String {
    let scientific = format!("{value:.*e}", precision - 1);
    let (mantissa, exponent) = scientific.split_once('e').expect("an exponent");
    let exponent: i32 = exponent.parse().expect("an integer exponent");
    let trim = |text: &str| -> String {
        match text.contains('.') {
            true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
            false => text.to_string(),
        }
    };
    if exponent < -4 || exponent >= precision as i32 {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.abs())
    } else {
        let decimals = (precision as i32 - 1 - exponent).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    }
}

/// A `FLOAT64` as GoogleSQL writes it: with 15 significant digits, or 17 where 15 don't give
/// the value back, as `printf`'s `%g` does, and `nan`, `inf` and `-inf`. Negative zero is `0`.
fn float_text(value: f64) -> String {
    match value {
        _ if value.is_nan() => "nan".to_string(),
        f64::INFINITY => "inf".to_string(),
        f64::NEG_INFINITY => "-inf".to_string(),
        0.0 => "0".to_string(),
        _ => {
            let short = printf_g(value, 15);
            match short.parse::<f64>() == Ok(value) {
                true => short,
                false => printf_g(value, 17),
            }
        }
    }
}

/// The text of the date and time `micros`, civil microseconds, as GoogleSQL writes a `DATETIME`:
/// its fraction in 3 digits, or 6 where 3 don't hold it, or none.
fn datetime_text(micros: i64) -> String {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let day_micros = micros.rem_euclid(MICROS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let seconds = day_micros / MICROS_PER_SECOND;
    let fraction = day_micros % MICROS_PER_SECOND;
    let fraction = match fraction {
        0 => String::new(),
        _ if fraction % 1000 == 0 => format!(".{:03}", fraction / 1000),
        _ => format!(".{fraction:06}"),
    };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}{fraction}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// The text of the value at `row` of `array` as GoogleSQL's `CAST(x AS STRING)` writes it: a
/// float as [`float_text`], a decimal without trailing zeros, and a date and time with a space.
/// A timestamp's array holds its civil microseconds, and `offset` its time zone's offset in
/// seconds, which follows as `-08` or `+05:30`.
fn canonical(array: &ArrayRef, row: usize, offset: Option<i64>) -> Result<String> {
    use datafusion::arrow::datatypes::{Decimal128Type, Decimal256Type, DecimalType, Float64Type};
    let trimmed = |text: String| match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        false => text,
    };
    Ok(match array.data_type() {
        DataType::Float64 => float_text(array.as_primitive::<Float64Type>().value(row)),
        DataType::Decimal128(precision, scale) => trimmed(Decimal128Type::format_decimal(
            array.as_primitive::<Decimal128Type>().value(row),
            *precision,
            *scale,
        )),
        DataType::Decimal256(precision, scale) => trimmed(Decimal256Type::format_decimal(
            array.as_primitive::<Decimal256Type>().value(row),
            *precision,
            *scale,
        )),
        DataType::Int64 => {
            let text = datetime_text(array.as_primitive::<Int64Type>().value(row));
            match offset {
                None => text,
                Some(offset) => {
                    let sign = if offset < 0 { '-' } else { '+' };
                    let (hours, minutes) = (offset.abs() / 3600, offset.abs() / 60 % 60);
                    match minutes {
                        0 => format!("{text}{sign}{hours:02}"),
                        _ => format!("{text}{sign}{hours:02}:{minutes:02}"),
                    }
                }
            }
        }
        other => return exec_err!("$to_string takes no {other}"),
    })
}

/// The text of the bytes `value` in `encoding`, such as `"HEX"`, as `TO_HEX` gives it.
pub(crate) fn encoded(value: Expr, encoding: &str, safe: bool) -> Expr {
    let safe = lit_str(if safe { "safe" } else { "" });
    BYTES_TO_STRING.call(vec![lit_str(encoding), safe, lit_str(""), value])
}

/// The bytes of the text `value` in `encoding`, such as `"HEX"`, as `FROM_HEX` gives them, with
/// GoogleSQL's error for text outside the encoding, or `NULL` where `safe`.
pub(crate) fn decoded(value: Expr, encoding: &str, safe: bool) -> Expr {
    let safe = lit_str(if safe { "safe" } else { "" });
    STRING_TO_BYTES.call(vec![lit_str(encoding), safe, lit_str(""), value])
}

/// `CAST(value AS STRING)` of a value of the type `from`, as GoogleSQL writes it, where Arrow's
/// cast writes it otherwise: a `FLOAT64`, a `NUMERIC` or `BIGNUMERIC`, a `DATETIME`, or a
/// `TIMESTAMP` in the time zone `zone`. `None` for another type, whose Arrow cast is GoogleSQL's.
pub(crate) fn cast_to_string(value: Expr, from: &Ty, zone: &str) -> Option<Expr> {
    let args = |value: Expr, offset: Option<Expr>| {
        let mut args = vec![lit_str(""), lit_str(""), lit_str(""), value];
        args.extend(offset);
        args
    };
    Some(match from {
        Ty::Float64 => TO_STRING.call(args(cast(value, DataType::Float64), None)),
        Ty::Numeric | Ty::BigNumeric => TO_STRING.call(args(value, None)),
        Ty::Datetime => TO_STRING.call(args(cast(value, DataType::Int64), None)),
        Ty::Timestamp => {
            let local = local_micros(value.clone(), zone);
            let utc = cast(cast(value, types::timestamp()), DataType::Int64);
            let offset = (local.clone() - utc) / lit_i64(MICROS_PER_SECOND);
            TO_STRING.call(args(local, Some(offset)))
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// The internal functions.
// ---------------------------------------------------------------------------------------------

/// What an internal function of `CAST ... FORMAT` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Op {
    /// A date or a time, as civil microseconds and an offset, to a string.
    FormatTemporal,
    /// A string to a date or a time, as civil microseconds, or a struct of them and an offset
    /// for a `TIMESTAMP`.
    ParseTemporal,
    BytesToString,
    StringToBytes,
    NumberToString,
    /// A value to a string without a format, as GoogleSQL's `CAST(x AS STRING)` writes it.
    ToString,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct CastFormat {
    op: Op,
    signature: Signature,
}

fn udf(op: Op) -> Arc<ScalarUDF> {
    Arc::new(ScalarUDF::new_from_impl(CastFormat {
        op,
        signature: Signature::variadic_any(Volatility::Immutable),
    }))
}

static FORMAT_TEMPORAL: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::FormatTemporal));
static PARSE_TEMPORAL: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::ParseTemporal));
static BYTES_TO_STRING: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::BytesToString));
static STRING_TO_BYTES: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::StringToBytes));
static NUMBER_TO_STRING: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::NumberToString));
static TO_STRING: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| udf(Op::ToString));

fn parsed_fields() -> Fields {
    Fields::from(vec![
        Field::new("micros", DataType::Int64, true),
        Field::new("offset", DataType::Int64, true),
    ])
}

/// The literal string at `index` of `args`.
fn literal(args: &ScalarFunctionArgs, index: usize) -> Result<String> {
    match &args.args[index] {
        ColumnarValue::Scalar(value) => match value.try_as_str() {
            Some(Some(text)) => Ok(text.to_string()),
            _ => exec_err!("the argument {index} of a CAST FORMAT function is a string"),
        },
        _ => exec_err!("the argument {index} of a CAST FORMAT function is a literal"),
    }
}

fn temporal(name: &str) -> Temporal {
    match name {
        "DATE" => Temporal::Date,
        "TIME" => Temporal::Time,
        "DATETIME" => Temporal::Datetime,
        _ => Temporal::Timestamp,
    }
}

impl ScalarUDFImpl for CastFormat {
    fn name(&self) -> &str {
        match self.op {
            Op::FormatTemporal => "$format_temporal",
            Op::ParseTemporal => "$parse_temporal",
            Op::BytesToString => "$bytes_to_string",
            Op::StringToBytes => "$string_to_bytes",
            Op::NumberToString => "$number_to_string",
            Op::ToString => "$to_string",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(match self.op {
            Op::FormatTemporal | Op::BytesToString | Op::NumberToString | Op::ToString => {
                DataType::Utf8
            }
            Op::ParseTemporal => DataType::Struct(parsed_fields()),
            Op::StringToBytes => DataType::Binary,
        })
    }

    /// The arguments are the format, `SAFE` as `"safe"` or `""`, the type of the date or the
    /// time, and the value, then for a date or a time the offset seconds of a `TIMESTAMP`'s
    /// time zone, or the civil microseconds of now, which parsing takes the year and the month
    /// of.
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let format = literal(&args, 0)?;
        let safe = literal(&args, 1)? == "safe";
        let ty = temporal(&literal(&args, 2)?);
        let scalar = (args.args[3..].iter()).all(|arg| matches!(arg, ColumnarValue::Scalar(_)));
        let value = args.args[3].clone().into_array(rows)?;
        let extra = match args.args.get(4) {
            Some(extra) => Some(arrow_cast(
                &extra.clone().into_array(rows)?,
                &DataType::Int64,
            )?),
            None => None,
        };
        let extra_at = |row: usize| -> Option<i64> {
            let extra = extra.as_ref()?.as_primitive::<Int64Type>();
            let row = if extra.len() == 1 { 0 } else { row };
            extra.is_valid(row).then(|| extra.value(row))
        };
        // A row's error: the error, or `NULL` for `SAFE_CAST`.
        let fail = |message: String| -> Result<()> {
            match safe {
                true => Ok(()),
                false => exec_err!("{message}"),
            }
        };
        let out: ArrayRef = match self.op {
            Op::FormatTemporal => {
                let micros = arrow_cast(&value, &DataType::Int64)?;
                let micros = micros.as_primitive::<Int64Type>();
                let elements = format_elements(&format, ty);
                let mut builder = StringBuilder::new();
                for row in 0..micros.len() {
                    if micros.is_null(row) {
                        builder.append_null();
                        continue;
                    }
                    match &elements {
                        Ok(elements) => builder.append_value(format_value(
                            elements,
                            micros.value(row),
                            extra_at(row).unwrap_or(0),
                        )),
                        Err(message) => {
                            fail(message.clone())?;
                            builder.append_null();
                        }
                    }
                }
                Arc::new(builder.finish())
            }
            Op::ParseTemporal => {
                let text = arrow_cast(&value, &DataType::Utf8)?;
                let text = text.as_string::<i32>();
                let elements = parse_elements(&format, ty);
                let (mut micros, mut offsets) = (Int64Builder::new(), Int64Builder::new());
                for row in 0..text.len() {
                    let parsed = match (&elements, text.is_null(row)) {
                        (_, true) => None,
                        (Ok(elements), false) => {
                            match parse_value(elements, text.value(row), extra_at(row).unwrap_or(0))
                            {
                                Ok(parsed) => Some(parsed),
                                Err(message) => {
                                    fail(message)?;
                                    None
                                }
                            }
                        }
                        (Err(message), false) => {
                            fail(message.clone())?;
                            None
                        }
                    };
                    micros.append_option(parsed.as_ref().map(|parsed| parsed.micros));
                    offsets.append_option(parsed.and_then(|parsed| parsed.offset));
                }
                let (micros, offsets) = (micros.finish(), offsets.finish());
                let nulls = micros.nulls().cloned();
                Arc::new(StructArray::try_new(
                    parsed_fields(),
                    vec![Arc::new(micros), Arc::new(offsets)],
                    nulls,
                )?)
            }
            Op::BytesToString => {
                let bytes = arrow_cast(&value, &DataType::Binary)?;
                let bytes = bytes.as_binary::<i32>();
                let encoding = encoding(&format);
                let mut builder = StringBuilder::new();
                for row in 0..bytes.len() {
                    if bytes.is_null(row) {
                        builder.append_null();
                        continue;
                    }
                    match encoding.clone().and_then(|e| encode(bytes.value(row), e)) {
                        Ok(text) => builder.append_value(text),
                        Err(message) => {
                            fail(message)?;
                            builder.append_null();
                        }
                    }
                }
                Arc::new(builder.finish())
            }
            Op::StringToBytes => {
                let text = arrow_cast(&value, &DataType::Utf8)?;
                let text = text.as_string::<i32>();
                let encoding = encoding(&format);
                let mut builder = BinaryBuilder::new();
                for row in 0..text.len() {
                    if text.is_null(row) {
                        builder.append_null();
                        continue;
                    }
                    match encoding.clone().and_then(|e| decode(text.value(row), e)) {
                        Ok(bytes) => builder.append_value(bytes),
                        Err(message) => {
                            fail(message)?;
                            builder.append_null();
                        }
                    }
                }
                Arc::new(builder.finish())
            }
            Op::ToString => {
                let mut builder = StringBuilder::new();
                for row in 0..value.len() {
                    match value.is_null(row) {
                        true => builder.append_null(),
                        false => builder.append_value(canonical(&value, row, extra_at(row))?),
                    }
                }
                Arc::new(builder.finish())
            }
            Op::NumberToString => {
                // Integers of every width as INT64s, and floats as FLOAT64s, but UINT64s.
                let value = match value.data_type() {
                    DataType::UInt64 | DataType::Int64 | DataType::Float64 | DataType::Float32 => {
                        value
                    }
                    ty if ty.is_integer() => arrow_cast(&value, &DataType::Int64)?,
                    ty if ty.is_floating() => arrow_cast(&value, &DataType::Float64)?,
                    _ => value,
                };
                let model = super::number_format::NumberFormat::parse(&format);
                let mut builder = StringBuilder::new();
                for row in 0..value.len() {
                    if value.is_null(row) {
                        builder.append_null();
                        continue;
                    }
                    let text = model
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|model| super::number_format::format_row(model, &value, row));
                    match text {
                        Ok(text) => builder.append_value(text),
                        Err(message) => {
                            fail(message)?;
                            builder.append_null();
                        }
                    }
                }
                Arc::new(builder.finish())
            }
        };
        match scalar {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(&out, 0)?)),
            false => Ok(ColumnarValue::Array(out)),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The plans of the casts.
// ---------------------------------------------------------------------------------------------

/// The microseconds of the civil date and time of the instant `value` in the time zone `zone`.
fn local_micros(value: Expr, zone: &str) -> Expr {
    let zoned = cast(
        value,
        DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(zone))),
    );
    let civil = cast(
        datafusion::functions::datetime::expr_fn::to_local_time(vec![zoned]),
        types::datetime(),
    );
    cast(civil, DataType::Int64)
}

/// The instant of the civil microseconds `micros` in the time zone `zone`.
fn instant_of_civil(micros: Expr, zone: &str) -> Expr {
    let civil = cast(micros, types::datetime());
    let zoned = cast(
        civil,
        DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(zone))),
    );
    cast(zoned, types::timestamp())
}

/// `CAST(value AS to FORMAT format [AT TIME ZONE zone])`, or `SAFE_CAST` when `safe`, of a value
/// of the type `from`. A `NULL` format gives `NULL`. A `TIMESTAMP` takes the time zone `zone`,
/// and `explicit_zone` says whether the cast names it, which only a `TIMESTAMP` allows.
pub(crate) fn cast_format(
    value: Expr,
    from: &Ty,
    to: &Ty,
    format: Option<&str>,
    zone: &str,
    explicit_zone: bool,
    safe: bool,
) -> Result<Expr, String> {
    let names = |ty: &Ty| crate::types::googlesql_name(&ty.arrow(Default::default()));
    let allowed = matches!(
        (from, to),
        (
            Ty::Date | Ty::Time | Ty::Datetime | Ty::Timestamp,
            Ty::String
        ) | (
            Ty::String,
            Ty::Date | Ty::Time | Ty::Datetime | Ty::Timestamp
        ) | (Ty::Bytes, Ty::String)
            | (Ty::String, Ty::Bytes)
            | (
                Ty::Int64 | Ty::Float64 | Ty::Numeric | Ty::BigNumeric,
                Ty::String
            )
    );
    if !allowed {
        return Err(format!(
            "FORMAT is not allowed for cast from {} to {}",
            names(from),
            names(to)
        ));
    }
    if explicit_zone && !matches!((from, to), (Ty::Timestamp, _) | (_, Ty::Timestamp)) {
        return Err(format!(
            "AT TIME ZONE is not allowed for cast from {} to {}",
            names(from),
            names(to)
        ));
    }
    let target = to.arrow(Default::default());
    let Some(format) = format else {
        return Ok(Expr::Literal(
            ScalarValue::try_from(&target).map_err(|e| e.to_string())?,
            None,
        ));
    };
    let args = |ty: &str, value: Expr, extra: Option<Expr>| {
        let mut args = vec![
            lit_str(format),
            lit_str(if safe { "safe" } else { "" }),
            lit_str(ty),
            value,
        ];
        args.extend(extra);
        args
    };
    Ok(match (from, to) {
        (Ty::Bytes, Ty::String) => BYTES_TO_STRING.call(args("", value, None)),
        (Ty::String, Ty::Bytes) => STRING_TO_BYTES.call(args("", value, None)),
        (_, Ty::String) if Temporal::of(from).is_none() => {
            NUMBER_TO_STRING.call(args("", value, None))
        }
        (_, Ty::String) => {
            let ty = Temporal::of(from).expect("a date or a time");
            let (micros, offset) = match ty {
                Temporal::Date => (
                    cast(cast(value, DataType::Int32), DataType::Int64) * lit_i64(MICROS_PER_DAY),
                    None,
                ),
                Temporal::Time | Temporal::Datetime => (cast(value, DataType::Int64), None),
                Temporal::Timestamp => {
                    let local = local_micros(value.clone(), zone);
                    let utc = cast(cast(value, types::timestamp()), DataType::Int64);
                    let offset = (local.clone() - utc) / lit_i64(MICROS_PER_SECOND);
                    (local, Some(offset))
                }
            };
            FORMAT_TEMPORAL.call(args(ty.name(), micros, offset))
        }
        (Ty::String, _) => {
            let ty = Temporal::of(to).expect("a date or a time");
            let now = local_micros(datafusion::functions::datetime::expr_fn::now(), zone);
            let parsed = PARSE_TEMPORAL.call(args(ty.name(), value, Some(now)));
            let micros = get_field(parsed.clone(), "micros");
            match ty {
                Temporal::Date => cast(
                    cast(micros / lit_i64(MICROS_PER_DAY), DataType::Int32),
                    DataType::Date32,
                ),
                // A time's string has no date, so the date of now drops away.
                Temporal::Time => cast(
                    micros % lit_i64(MICROS_PER_DAY),
                    DataType::Time64(TimeUnit::Microsecond),
                ),
                Temporal::Datetime => cast(micros, types::datetime()),
                Temporal::Timestamp => {
                    let offset = get_field(parsed, "offset");
                    let instant = Expr::Case(datafusion::logical_expr::Case {
                        expr: None,
                        when_then_expr: vec![(
                            Box::new(offset.clone().is_null()),
                            Box::new(instant_of_civil(micros.clone(), zone)),
                        )],
                        else_expr: Some(Box::new(cast(
                            micros - offset * lit_i64(MICROS_PER_SECOND),
                            types::timestamp(),
                        ))),
                    });
                    // An instant out of GoogleSQL's range of timestamps fails, or is `NULL`.
                    let low = days_from_civil(1, 1, 1) * MICROS_PER_DAY;
                    let high = days_from_civil(10_000, 1, 1) * MICROS_PER_DAY;
                    let as_micros = cast(instant.clone(), DataType::Int64);
                    let outside = as_micros
                        .clone()
                        .lt(lit_i64(low))
                        .or(as_micros.gt_eq(lit_i64(high)));
                    match safe {
                        true => super::null_when(outside, instant),
                        false => super::error::error_when(
                            outside,
                            instant,
                            "The parsing result is out of valid time range",
                            Vec::new(),
                        ),
                    }
                }
            }
        }
        _ => unreachable!("checked above"),
    })
}
