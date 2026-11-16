//! The units: truncating units with a count, and collapsing units, and their text, which
//! names the calendar of a unit that depends on one: the first day of the weeks of `week(2)`,
//! Monday, and the first month of the fiscal years of `fiscal_quarter(10)`, October.

use datafusion::common::{Result, plan_err};

use super::WeekStart;

/// A truncating unit: values go to the unit that contains them on the calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Trunc {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
    FiscalQuarter,
    FiscalYear,
}

pub(super) const TRUNCS: [(Trunc, &str); 11] = [
    (Trunc::Millisecond, "millisecond"),
    (Trunc::Second, "second"),
    (Trunc::Minute, "minute"),
    (Trunc::Hour, "hour"),
    (Trunc::Day, "day"),
    (Trunc::Week, "week"),
    (Trunc::Month, "month"),
    (Trunc::Quarter, "quarter"),
    (Trunc::Year, "year"),
    (Trunc::FiscalQuarter, "fiscal_quarter"),
    (Trunc::FiscalYear, "fiscal_year"),
];

/// A collapsing unit: the values of every period go to one reference calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Collapse {
    QuarterOfYear,
    FiscalQuarterOfYear,
    MonthOfYear,
    MonthDay,
    DayOfYear,
    WeekOfYear,
    DayOfMonth,
    DayOfWeek,
    HourOfDay,
    MinuteOfDay,
    SecondOfDay,
    TimeOfDay,
    MinuteOfHour,
    SecondOfMinute,
}

pub(super) const COLLAPSES: [(Collapse, &str); 14] = [
    (Collapse::QuarterOfYear, "quarter_of_year"),
    (Collapse::FiscalQuarterOfYear, "fiscal_quarter_of_year"),
    (Collapse::MonthOfYear, "month_of_year"),
    (Collapse::MonthDay, "month_day"),
    (Collapse::DayOfYear, "day_of_year"),
    (Collapse::WeekOfYear, "week_of_year"),
    (Collapse::DayOfMonth, "day_of_month"),
    (Collapse::DayOfWeek, "day_of_week"),
    (Collapse::HourOfDay, "hour_of_day"),
    (Collapse::MinuteOfDay, "minute_of_day"),
    (Collapse::SecondOfDay, "second_of_day"),
    (Collapse::TimeOfDay, "time_of_day"),
    (Collapse::MinuteOfHour, "minute_of_hour"),
    (Collapse::SecondOfMinute, "second_of_minute"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Trunc(Trunc, u32),
    Collapse(Collapse),
}

/// The calendar that the text of a unit names, in the parentheses after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitCalendar {
    /// The first day of the weeks of `week`, `week_of_year`, and `day_of_week`.
    WeekStart(WeekStart),
    /// The first month, from 1 to 12, of the fiscal years of the fiscal units.
    FiscalYearStart(i64),
}

impl Unit {
    /// The unit of `text`, such as `'month'`, `'3 hours'`, or `'2 weeks(2)'`, and the calendar
    /// that it names. A unit of weeks names the first day of its weeks as GoogleSQL's
    /// `DAYOFWEEK` numbers it, from 1 for Sunday to 7 for Saturday, and a fiscal unit the first
    /// month of its fiscal years, from 1 to 12.
    pub fn parse(text: &str) -> Result<(Unit, Option<UnitCalendar>)> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let Some(last) = words.last() else {
            return plan_err!("unknown time unit '{text}'");
        };
        let (name, parameter) = match last.strip_suffix(')').and_then(|rest| rest.split_once('(')) {
            Some((name, parameter)) => match parameter.trim().parse::<i64>() {
                Ok(n) => (name, Some(n)),
                Err(_) => return plan_err!("the parameter of the unit '{text}' is a number"),
            },
            None => (*last, None),
        };
        let unit = Self::parse_name(text, &words[..words.len() - 1], name)?;
        let calendar = match (parameter, unit.follows_weeks(), unit.is_fiscal()) {
            (None, ..) => None,
            (Some(n), true, _) => match WeekStart::from_number(n) {
                Some(day) => Some(UnitCalendar::WeekStart(day)),
                None => {
                    return plan_err!(
                        "the first day of the weeks of '{text}' is a number from 1 for Sunday to 7 for Saturday, not {n}"
                    );
                }
            },
            (Some(n), _, true) if (1..=12).contains(&n) => Some(UnitCalendar::FiscalYearStart(n)),
            (Some(n), _, true) => {
                return plan_err!(
                    "the first month of the fiscal years of '{text}' is a number from 1 to 12, not {n}"
                );
            }
            (Some(_), false, false) => {
                return plan_err!(
                    "the unit '{}' takes no parameter: only the units of weeks and the fiscal units name a calendar",
                    unit.text()
                );
            }
        };
        Ok((unit, calendar))
    }

    /// The unit of the name `name`, after the words `count` of its text `text`.
    fn parse_name(text: &str, count: &[&str], name: &str) -> Result<Unit> {
        let words: Vec<&str> = count.iter().copied().chain([name]).collect();
        let (count, name) = match words[..] {
            [name] => (1, name),
            [count, name] => match count.parse::<u32>() {
                Ok(c) if c > 0 => (c, name),
                _ => return plan_err!("the count of the unit '{text}' must be a positive integer"),
            },
            _ => return plan_err!("unknown time unit '{text}'"),
        };
        let name = name.to_ascii_lowercase();
        if let Some((c, _)) = COLLAPSES.iter().find(|(_, n)| *n == name) {
            if words.len() > 1 {
                return plan_err!("the collapsing unit '{name}' takes no count");
            }
            return Ok(Unit::Collapse(*c));
        }
        let singular = name.strip_suffix('s').unwrap_or(&name);
        let found = TRUNCS.iter().find(|(_, n)| *n == name || *n == singular);
        match found {
            Some((t, _)) => Ok(Unit::Trunc(*t, count)),
            None => {
                let truncs: Vec<&str> = TRUNCS.iter().map(|(_, n)| *n).collect();
                let collapses: Vec<&str> = COLLAPSES.iter().map(|(_, n)| *n).collect();
                plan_err!(
                    "unknown time unit '{text}': the truncating units are {}, with an optional count, and the collapsing units are {}",
                    truncs.join(", "),
                    collapses.join(", ")
                )
            }
        }
    }

    /// The text of the unit and of its calendar, which names the first day of the weeks of a
    /// unit of weeks and the first month of the fiscal years of a fiscal unit, such as
    /// `'week(1)'` or `'fiscal_quarter(10)'`.
    pub fn text_in(&self, week_start: WeekStart, fiscal_year_start: Option<i64>) -> String {
        let text = self.text();
        match (self.follows_weeks(), self.is_fiscal(), fiscal_year_start) {
            (true, ..) => format!("{text}({})", week_start.number()),
            (_, true, Some(month)) => format!("{text}({month})"),
            _ => text,
        }
    }

    /// The text of the unit without its calendar, such as `'2 weeks'`.
    pub fn text(&self) -> String {
        match self {
            Unit::Trunc(t, k) => {
                let name = TRUNCS.iter().find(|(u, _)| u == t).unwrap().1;
                if *k == 1 {
                    name.to_string()
                } else {
                    format!("{k} {name}s")
                }
            }
            Unit::Collapse(c) => COLLAPSES
                .iter()
                .find(|(u, _)| u == c)
                .unwrap()
                .1
                .to_string(),
        }
    }

    /// Shorter than one day.
    pub fn is_subday(&self) -> bool {
        match self {
            Unit::Trunc(t, _) => *t <= Trunc::Hour,
            Unit::Collapse(c) => matches!(
                c,
                Collapse::HourOfDay
                    | Collapse::MinuteOfDay
                    | Collapse::SecondOfDay
                    | Collapse::TimeOfDay
                    | Collapse::MinuteOfHour
                    | Collapse::SecondOfMinute
            ),
        }
    }

    /// Whether the first day of a week moves the bounds of the unit: `week`, `week_of_year`,
    /// and `day_of_week`.
    pub fn follows_weeks(&self) -> bool {
        matches!(
            self,
            Unit::Trunc(Trunc::Week, _)
                | Unit::Collapse(Collapse::WeekOfYear | Collapse::DayOfWeek)
        )
    }

    pub fn is_fiscal(&self) -> bool {
        matches!(
            self,
            Unit::Trunc(Trunc::FiscalQuarter | Trunc::FiscalYear, _)
                | Unit::Collapse(Collapse::FiscalQuarterOfYear)
        )
    }

    /// The approximate length in milliseconds: a month is 30 days and a year 365.
    pub fn approx_ms(&self) -> f64 {
        let Unit::Trunc(t, k) = self else {
            return f64::INFINITY;
        };
        let one = match t {
            Trunc::Millisecond => 1.0,
            Trunc::Second => 1e3,
            Trunc::Minute => 6e4,
            Trunc::Hour => 3.6e6,
            Trunc::Day => 8.64e7,
            Trunc::Week => 6.048e8,
            Trunc::Month => 2.592e9,
            Trunc::Quarter | Trunc::FiscalQuarter => 7.776e9,
            Trunc::Year | Trunc::FiscalYear => 3.1536e10,
        };
        one * *k as f64
    }

    /// The exact length in milliseconds of a unit of fixed length (up to weeks).
    pub(super) fn exact_ms(&self) -> Option<i64> {
        match self {
            Unit::Trunc(t, _) if *t <= Trunc::Week => Some(self.approx_ms() as i64),
            _ => None,
        }
    }
}
