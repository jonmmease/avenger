//! The units: truncating units with a count, and collapsing units.

use datafusion::common::{Result, plan_err};

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

impl Unit {
    pub fn parse(text: &str) -> Result<Unit> {
        let words: Vec<&str> = text.split_whitespace().collect();
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
