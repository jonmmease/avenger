//! The options of a call.

use super::*;

use crate::OptionError;
use datafusion::arrow::array::{Array, AsArray, StructArray};
use datafusion::arrow::datatypes::{DataType, Field, Fields, Int64Type, TimeUnit as ArrowUnit};
use datafusion::common::{Result, ScalarValue};
use datafusion::logical_expr::{Expr, lit};

/// The first day of a week, numbered as GoogleSQL's `DAYOFWEEK`, from 1 for Sunday to 7 for
/// Saturday. Weeks start on Sunday by default, as GoogleSQL's `WEEK` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    #[default]
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

const WEEK_STARTS: [WeekStart; 7] = [
    WeekStart::Sunday,
    WeekStart::Monday,
    WeekStart::Tuesday,
    WeekStart::Wednesday,
    WeekStart::Thursday,
    WeekStart::Friday,
    WeekStart::Saturday,
];

impl WeekStart {
    /// The day of the number `n`, from 1 for Sunday to 7 for Saturday.
    pub fn from_number(n: i64) -> Option<Self> {
        let i = usize::try_from(n.checked_sub(1)?).ok()?;
        WEEK_STARTS.get(i).copied()
    }

    /// The number of the day, from 1 for Sunday to 7 for Saturday.
    pub fn number(self) -> i64 {
        self as i64 + 1
    }

    /// A leap year whose January 1 is this day: the reference calendar of `week_of_year` and
    /// `day_of_week`, and the origin of weeks.
    pub fn reference_year(self) -> i32 {
        match self {
            WeekStart::Sunday => 2012,
            WeekStart::Monday => 2024,
            WeekStart::Tuesday => 2008,
            WeekStart::Wednesday => 2020,
            WeekStart::Thursday => 2004,
            WeekStart::Friday => 2016,
            WeekStart::Saturday => 2000,
        }
    }

    /// The index in its week of the day `days` after January 1, 1970, a Thursday: 0 for this
    /// day.
    pub(super) fn index(self, days: i64) -> i64 {
        (days + 5 - self.number()).rem_euclid(7)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct TimeUnitOptions {
    pub unit: Option<Unit>,
    pub maxbins: Option<i64>,
    pub min_unit: Option<Unit>,
    /// The first day of a week, Sunday when unset.
    pub week_start: Option<WeekStart>,
    /// The first month of the fiscal year, 1 to 12.
    pub fiscal_year_start: Option<i64>,
}

impl TimeUnitOptions {
    /// Rejects options that contradict each other or are out of range.
    pub fn check(&self) -> std::result::Result<(), OptionError> {
        if let Some(n) = self.maxbins.filter(|n| !(1..=u32::MAX as i64).contains(n)) {
            return Err(OptionError::new(
                "maxbins",
                format!("The maxbins of timeunit must be a positive integer, not {n}"),
            ));
        }
        if let Some(m) = self.fiscal_year_start.filter(|m| !(1..=12).contains(m)) {
            return Err(OptionError::new(
                "fiscal_year_start",
                format!(
                    "The fiscal_year_start of timeunit must be the number of a month from 1 to 12, such as 10 for October, not {m}"
                ),
            ));
        }
        if let Some(u @ Unit::Collapse(_)) = self.min_unit {
            return Err(OptionError::new(
                "min_unit",
                format!(
                    "The min_unit of timeunit must be a truncating unit, not '{}'",
                    u.text()
                ),
            ));
        }
        if self.unit.is_some() && self.maxbins.is_some() {
            return Err(OptionError::new(
                "maxbins",
                "timeunit takes unit or maxbins, not both: an explicit unit has no automatic choice",
            ));
        }
        if self.unit.is_some() && self.min_unit.is_some() {
            return Err(OptionError::new(
                "min_unit",
                "timeunit takes unit or min_unit, not both: an explicit unit has no automatic choice",
            ));
        }
        for (option, u) in [("unit", self.unit), ("min_unit", self.min_unit)] {
            if let Some(u) = u.filter(|u| u.is_fiscal() && self.fiscal_year_start.is_none()) {
                return Err(OptionError::new(
                    option,
                    format!(
                        "The unit '{}' needs fiscal_year_start; without it, use '{}'",
                        u.text(),
                        u.text().replace("fiscal_", "")
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Rejects units that do not apply to values of the type `dt`.
    pub fn check_type(&self, dt: &DataType) -> std::result::Result<(), OptionError> {
        for (option, u) in [("unit", self.unit), ("min_unit", self.min_unit)] {
            let Some(u) = u else { continue };
            if *dt == DataType::Date32 && u.is_subday() {
                return Err(OptionError::new(
                    option,
                    format!(
                        "The unit '{}' is shorter than one day, so it does not apply to a DATE",
                        u.text()
                    ),
                ));
            }
            if matches!(dt, DataType::Timestamp(ArrowUnit::Second, _))
                && matches!(u, Unit::Trunc(Trunc::Millisecond, _))
            {
                return Err(OptionError::new(
                    option,
                    format!(
                        "The unit '{}' is shorter than one second, so it does not apply to a timestamp in seconds",
                        u.text()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Takes the calendar that the text of the unit of `option` names, such as the Monday of
    /// `'week(2)'`. A calendar that differs from the `week_start` or the `fiscal_year_start` of
    /// the call, or of another unit, is an error of `option`.
    pub fn take_calendar(
        &mut self,
        option: &'static str,
        calendar: Option<UnitCalendar>,
    ) -> std::result::Result<(), OptionError> {
        match calendar {
            None => Ok(()),
            Some(UnitCalendar::WeekStart(day)) => match self.week_start {
                Some(set) if set != day => Err(OptionError::new(
                    option,
                    format!(
                        "The {option} of timeunit starts its weeks on day {}, but the weeks of the call start on day {}",
                        day.number(),
                        set.number()
                    ),
                )),
                _ => {
                    self.week_start = Some(day);
                    Ok(())
                }
            },
            Some(UnitCalendar::FiscalYearStart(month)) => match self.fiscal_year_start {
                Some(set) if set != month => Err(OptionError::new(
                    option,
                    format!(
                        "The {option} of timeunit starts its fiscal years in month {month}, but the fiscal years of the call start in month {set}"
                    ),
                )),
                _ => {
                    self.fiscal_year_start = Some(month);
                    Ok(())
                }
            },
        }
    }
}

impl TimeUnitOptions {
    /// The options as a constant struct of [`options_fields`].
    pub fn expr(&self) -> Expr {
        let text = |u: Option<Unit>| lit(ScalarValue::Utf8(u.map(|u| u.text())));
        let values = [
            text(self.unit),
            lit(ScalarValue::Int64(self.maxbins)),
            text(self.min_unit),
            lit(ScalarValue::Int64(self.week_start.map(WeekStart::number))),
            lit(ScalarValue::Int64(self.fiscal_year_start)),
        ];
        let given = (options_fields().iter().zip(values))
            .map(|(field, value)| (field.name().clone(), value))
            .collect();
        options(given).expect("the options of timeunit")
    }

    /// The options of the constant struct `value` of [`options_fields`], checked.
    pub fn of_scalar(value: &ScalarValue) -> Result<Self> {
        let array = value.to_array()?;
        Ok(Self::of_struct(array.as_struct(), 0)?)
    }

    /// The options of the row `i` of `array`, a struct of [`options_fields`], with the default
    /// of each NULL field, and the calendar that the text of a unit names, checked.
    pub fn of_struct(array: &StructArray, i: usize) -> std::result::Result<Self, OptionError> {
        let integer = |j: usize| {
            let column = array.column(j).as_primitive::<Int64Type>();
            column.is_valid(i).then(|| column.value(i))
        };
        let unit = |option: &'static str, j: usize| {
            let column = array.column(j).as_string::<i32>();
            match column.is_valid(i) {
                true => Unit::parse(column.value(i))
                    .map(Some)
                    .map_err(|error| OptionError::new(option, unit_message(error))),
                false => Ok(None),
            }
        };
        let (unit_of, min_unit) = (unit("unit", 0)?, unit("min_unit", 2)?);
        let week_start = match integer(3) {
            Some(n) => match WeekStart::from_number(n) {
                Some(day) => Some(day),
                None => {
                    return Err(OptionError::new(
                        "week_start",
                        format!(
                            "The week_start of timeunit is a day from 1 for Sunday to 7 for Saturday, not {n}"
                        ),
                    ));
                }
            },
            None => None,
        };
        let mut options = Self {
            unit: unit_of.map(|(unit, _)| unit),
            maxbins: integer(1),
            min_unit: min_unit.map(|(unit, _)| unit),
            week_start,
            fiscal_year_start: integer(4),
        };
        for (option, unit) in [("unit", unit_of), ("min_unit", min_unit)] {
            options.take_calendar(option, unit.and_then(|(_, calendar)| calendar))?;
        }
        options.check()?;
        Ok(options)
    }
}

/// The message of an error of the text of a unit, as a sentence.
fn unit_message(error: datafusion::common::DataFusionError) -> String {
    let message = match error {
        datafusion::common::DataFusionError::Plan(message) => message,
        error => error.to_string(),
    };
    let mut chars = message.chars();
    (chars.next().into_iter())
        .flat_map(char::to_uppercase)
        .chain(chars)
        .collect()
}

/// The names and the types of the options of `timeunit`, the fields of the struct of
/// [`options`]: the units as their text, which names their calendars.
pub fn options_fields() -> Fields {
    Fields::from(vec![
        Field::new("unit", DataType::Utf8, true),
        Field::new("maxbins", DataType::Int64, true),
        Field::new("min_unit", DataType::Utf8, true),
        Field::new("week_start", DataType::Int64, true),
        Field::new("fiscal_year_start", DataType::Int64, true),
    ])
}

/// The struct of the options `given` of a call, by their names, as expressions of the plan,
/// such as literals or placeholders: each cast to the type of its field, and NULL for an option
/// that the call omits, which takes its default.
pub fn options(given: Vec<(String, Expr)>) -> Result<Expr> {
    crate::utils::options_struct("timeunit", &options_fields(), given)
}
