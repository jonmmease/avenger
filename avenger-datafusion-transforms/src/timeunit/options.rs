//! The options of a call, and the metadata tag of a result.

use super::*;

use datafusion::arrow::datatypes::{DataType, TimeUnit as ArrowUnit};
use datafusion::common::{Result, ScalarValue, plan_err};

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

    pub(super) fn name(self) -> &'static str {
        match self {
            WeekStart::Monday => "monday",
            WeekStart::Sunday => "sunday",
        }
    }
}

/// The options of `timeunit`.
pub const OPTIONS: &[crate::OptionSpec] = &[
    crate::OptionSpec {
        name: "unit",
        ty: "STRING",
        values: &[],
        doc: "The unit, such as `'month'`, which holds a span of time and takes a count and a plural, as in `'15 minutes'` or `'3 months'`, or a collapsing unit, such as `'month_of_year'`, which maps each value into one reference year. The fiscal units `'fiscal_year'` and `'fiscal_quarter'` need `fiscal_year_start`. When omitted, `timeunit` chooses one unit for each partition.",
    },
    crate::OptionSpec {
        name: "min_unit",
        ty: "STRING",
        values: &[],
        doc: "The shortest unit that `timeunit` can choose. When omitted, the shortest is the resolution of the data, such as `'month'` for values that are all first days of months.",
    },
    crate::OptionSpec {
        name: "maxbins",
        ty: "INT64",
        values: &[],
        doc: "The greatest number of units that a chosen unit gives the partition. When omitted, 20.",
    },
    crate::OptionSpec {
        name: "week_start",
        ty: "STRING",
        values: &["monday", "sunday"],
        doc: "The first day of a week. When omitted, `'monday'`.",
    },
    crate::OptionSpec {
        name: "fiscal_year_start",
        ty: "INT64",
        values: &[],
        doc: "The number of the month, from 1 to 12, in which a fiscal year starts, such as 10 for October.",
    },
];

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TimeUnitOptions {
    pub unit: Option<Unit>,
    pub maxbins: Option<u32>,
    pub min_unit: Option<Unit>,
    pub week_start: WeekStart,
    /// The first month of the fiscal year, 1 to 12.
    pub fiscal_year_start: Option<u32>,
}

impl TimeUnitOptions {
    pub fn parse(pairs: &[ScalarValue]) -> Result<Self> {
        if !pairs.len().is_multiple_of(2) {
            return plan_err!(
                "timeunit options must be pairs of a name and a value, but the last name has no value"
            );
        }
        let mut o = Self::default();
        let mut seen: Vec<&str> = Vec::new();
        for pair in pairs.chunks(2) {
            let Some(Some(name)) = pair[0].try_as_str() else {
                return plan_err!("a timeunit option name must be a string, not {}", pair[0]);
            };
            if seen.contains(&name) {
                return plan_err!("the timeunit option '{name}' is given twice");
            }
            seen.push(name);
            let v = &pair[1];
            let text = || -> Result<&str> {
                match v.try_as_str() {
                    Some(Some(s)) => Ok(s),
                    _ => plan_err!("the timeunit option '{name}' must be a string, not {v}"),
                }
            };
            match name {
                "unit" => o.unit = Some(Unit::parse(text()?)?),
                "min_unit" => {
                    let u = Unit::parse(text()?)?;
                    if matches!(u, Unit::Collapse(_)) {
                        return plan_err!(
                            "'min_unit' must be a truncating unit, not '{}'",
                            u.text()
                        );
                    }
                    o.min_unit = Some(u);
                }
                "maxbins" => {
                    let n = match v.cast_to(&DataType::Float64) {
                        Ok(ScalarValue::Float64(Some(f))) if v.data_type().is_numeric() => f,
                        _ => {
                            return plan_err!(
                                "the timeunit option 'maxbins' must be a number, not {v}"
                            );
                        }
                    };
                    if n < 1.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
                        return plan_err!(
                            "the timeunit option 'maxbins' must be a positive integer, not {v}"
                        );
                    }
                    o.maxbins = Some(n as u32);
                }
                "week_start" => {
                    o.week_start = match text()?.to_ascii_lowercase().as_str() {
                        "monday" => WeekStart::Monday,
                        "sunday" => WeekStart::Sunday,
                        other => {
                            return plan_err!(
                                "'week_start' must be 'monday' or 'sunday', not '{other}'"
                            );
                        }
                    }
                }
                "fiscal_year_start" => {
                    let m = match v.cast_to(&DataType::Float64) {
                        Ok(ScalarValue::Float64(Some(m))) if v.data_type().is_numeric() => m,
                        _ => f64::NAN,
                    };
                    if !(1.0..=12.0).contains(&m) || m.fract() != 0.0 {
                        return plan_err!(
                            "'fiscal_year_start' must be the number of a month from 1 to 12, such as 10 for October, not {v}"
                        );
                    }
                    o.fiscal_year_start = Some(m as u32);
                }
                _ => {
                    return plan_err!(
                        "unknown timeunit option '{name}': the options are unit, maxbins, min_unit, week_start, fiscal_year_start"
                    );
                }
            }
        }
        if o.unit.is_some() && o.maxbins.is_some() {
            return plan_err!(
                "timeunit takes 'unit' or 'maxbins', not both: an explicit unit has no automatic choice"
            );
        }
        if o.unit.is_some() && o.min_unit.is_some() {
            return plan_err!(
                "timeunit takes 'unit' or 'min_unit', not both: an explicit unit has no automatic choice"
            );
        }
        for u in o.unit.iter().chain(o.min_unit.iter()) {
            if u.is_fiscal() && o.fiscal_year_start.is_none() {
                return plan_err!(
                    "the unit '{}' needs 'fiscal_year_start'; without it, use '{}'",
                    u.text(),
                    u.text().replace("fiscal_", "")
                );
            }
        }
        Ok(o)
    }

    /// Checks the options against the type of the value.
    pub fn check(&self, dt: &DataType) -> Result<()> {
        for u in self.unit.iter().chain(self.min_unit.iter()) {
            if *dt == DataType::Date32 && u.is_subday() {
                return plan_err!(
                    "the unit '{}' is shorter than one day, so it does not apply to a DATE",
                    u.text()
                );
            }
            if matches!(dt, DataType::Timestamp(ArrowUnit::Second, _))
                && matches!(u, Unit::Trunc(Trunc::Millisecond, _))
            {
                return plan_err!(
                    "the unit '{}' is shorter than one second, so it does not apply to a timestamp in seconds",
                    u.text()
                );
            }
        }
        Ok(())
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
