//! The options of a call, and the metadata tag of a result.

use super::*;

use crate::OptionError;
use datafusion::arrow::datatypes::{DataType, TimeUnit as ArrowUnit};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

impl WeekStart {
    /// A leap year whose January 1 is the first day of a week: the reference calendar of
    /// the collapsing units, and the origin of the units of a week or shorter.
    pub fn reference_year(self) -> i32 {
        match self {
            WeekStart::Monday => 2024,
            WeekStart::Sunday => 2012,
        }
    }

    /// The day of the name `monday` or `sunday`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "monday" => Some(WeekStart::Monday),
            "sunday" => Some(WeekStart::Sunday),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            WeekStart::Monday => "monday",
            WeekStart::Sunday => "sunday",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct TimeUnitOptions {
    pub unit: Option<Unit>,
    pub maxbins: Option<i64>,
    pub min_unit: Option<Unit>,
    pub week_start: WeekStart,
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

    /// The options that differ from the defaults, as named arguments, such as
    /// `unit => 'month'`.
    pub(super) fn text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(u) = self.unit {
            parts.push(format!("unit => '{}'", u.text()));
        }
        if let Some(n) = self.maxbins {
            parts.push(format!("maxbins => {n}"));
        }
        if let Some(u) = self.min_unit {
            parts.push(format!("min_unit => '{}'", u.text()));
        }
        if self.week_start != WeekStart::default() {
            parts.push(format!("week_start => '{}'", self.week_start.name()));
        }
        if let Some(m) = self.fiscal_year_start {
            parts.push(format!("fiscal_year_start => {m}"));
        }
        parts.join(", ")
    }

    /// The value of the metadata key `avenger.time_unit`.
    pub fn tag(&self) -> String {
        let fiscal = self
            .fiscal_year_start
            .map(|m| format!(",\"fiscal_year_start\":{m}"))
            .unwrap_or_default();
        format!("{{\"week_start\":\"{}\"{fiscal}}}", self.week_start.name())
    }
}
