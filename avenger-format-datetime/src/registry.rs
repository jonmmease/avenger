use std::collections::{BTreeMap, BTreeSet};

use crate::{
    error::DateTimeFormatError,
    locale::{
        DateTimeLocaleSpec, DayPeriods, DayPeriodsSpec, Lengths, LengthsSpec, LocaleId,
        ResolvedDateTimeLocale, Widths12, Widths12Spec, Widths2, Widths2Spec, Widths4, Widths4Spec,
        Widths7, Widths7Spec,
    },
    parser::parse_datetime_spec,
    style::validate_datetime_glue_pattern,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeLocaleRegistry {
    builtins: BTreeMap<LocaleId, DateTimeLocaleSpec>,
    custom: BTreeMap<LocaleId, DateTimeLocaleSpec>,
}

impl DateTimeLocaleRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry
            .builtins
            .insert(LocaleId::new("en-US"), builtin_en_us_spec());
        registry
    }

    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: DateTimeLocaleSpec,
    ) -> Result<(), DateTimeFormatError> {
        let id = LocaleId::new(id);
        let previous = self.custom.insert(id.clone(), spec);
        match self.resolve(&id.0) {
            Ok(_) => Ok(()),
            Err(err) => {
                if let Some(previous) = previous {
                    self.custom.insert(id, previous);
                } else {
                    self.custom.remove(&id);
                }
                Err(err)
            }
        }
    }

    pub fn register_custom_locale_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), DateTimeFormatError> {
        let spec: DateTimeLocaleSpec = serde_json::from_str(json)
            .map_err(|err| DateTimeFormatError::InvalidLocaleData(err.to_string()))?;
        self.register_custom_locale(id, spec)
    }

    pub fn resolve(&self, id: &str) -> Result<ResolvedDateTimeLocale, DateTimeFormatError> {
        let id = LocaleId::new(id);
        let spec = self.resolve_spec(&id, &mut BTreeSet::new())?;
        self.finish_resolved(id, spec)
    }

    fn resolve_spec(
        &self,
        id: &LocaleId,
        resolving: &mut BTreeSet<LocaleId>,
    ) -> Result<DateTimeLocaleSpec, DateTimeFormatError> {
        if !resolving.insert(id.clone()) {
            return Err(DateTimeFormatError::InvalidLocaleData(format!(
                "base locale cycle involving `{id}`"
            )));
        }

        let spec = self
            .custom
            .get(id)
            .or_else(|| self.builtins.get(id))
            .ok_or_else(|| DateTimeFormatError::LocaleNotFound(id.to_string()))?;

        let mut resolved = if let Some(base) = &spec.base {
            self.resolve_spec(base, resolving)?
        } else if id.as_ref() == "en-US" {
            DateTimeLocaleSpec::default()
        } else {
            self.resolve_spec(&LocaleId::new("en-US"), resolving)?
        };

        merge_spec(&mut resolved, spec.clone());
        resolving.remove(id);
        Ok(resolved)
    }

    fn finish_resolved(
        &self,
        id: LocaleId,
        spec: DateTimeLocaleSpec,
    ) -> Result<ResolvedDateTimeLocale, DateTimeFormatError> {
        let mut locale = ResolvedDateTimeLocale::en_us();
        locale.id = id;

        if let Some(value) = spec.months {
            locale.months = merge_widths12(locale.months, value);
        }
        if let Some(value) = spec.months_standalone {
            locale.months_standalone = merge_widths12(locale.months_standalone, value);
        }
        if let Some(value) = spec.weekdays {
            locale.weekdays = merge_widths7(locale.weekdays, value);
        }
        if let Some(value) = spec.weekdays_standalone {
            locale.weekdays_standalone = merge_widths7(locale.weekdays_standalone, value);
        }
        if let Some(value) = spec.quarters {
            locale.quarters = merge_widths4(locale.quarters, value);
        }
        if let Some(value) = spec.quarters_standalone {
            locale.quarters_standalone = merge_widths4(locale.quarters_standalone, value);
        }
        if let Some(value) = spec.eras {
            locale.eras = merge_widths2(locale.eras, value);
        }
        if let Some(value) = spec.day_periods {
            locale.day_periods = merge_day_periods(locale.day_periods, value);
        }
        if let Some(value) = spec.date_patterns {
            locale.date_patterns = merge_lengths(locale.date_patterns, value);
        }
        if let Some(value) = spec.time_patterns {
            locale.time_patterns = merge_lengths(locale.time_patterns, value);
        }
        if let Some(value) = spec.datetime_glue {
            locale.datetime_glue = merge_lengths(locale.datetime_glue, value);
        }
        if let Some(value) = spec.first_day_of_week {
            locale.first_day_of_week = value;
        }
        if let Some(value) = spec.digits {
            if value.iter().any(|digit| digit.is_empty()) {
                return Err(DateTimeFormatError::InvalidLocaleData(
                    "digit strings must be nonempty".to_string(),
                ));
            }
            locale.digits = Some(value);
        }

        validate_lengths("date pattern", &locale.date_patterns)?;
        validate_lengths("time pattern", &locale.time_patterns)?;
        validate_glue_lengths(&locale.datetime_glue)?;
        Ok(locale)
    }
}

fn builtin_en_us_spec() -> DateTimeLocaleSpec {
    DateTimeLocaleSpec::default()
}

fn merge_spec(target: &mut DateTimeLocaleSpec, source: DateTimeLocaleSpec) {
    if source.base.is_some() {
        target.base = source.base;
    }
    if let Some(source) = source.months {
        target.months = Some(merge_widths12_spec(
            target.months.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.months_standalone {
        target.months_standalone = Some(merge_widths12_spec(
            target.months_standalone.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.weekdays {
        target.weekdays = Some(merge_widths7_spec(
            target.weekdays.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.weekdays_standalone {
        target.weekdays_standalone = Some(merge_widths7_spec(
            target.weekdays_standalone.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.quarters {
        target.quarters = Some(merge_widths4_spec(
            target.quarters.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.quarters_standalone {
        target.quarters_standalone = Some(merge_widths4_spec(
            target.quarters_standalone.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.eras {
        target.eras = Some(merge_widths2_spec(
            target.eras.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.day_periods {
        target.day_periods = Some(merge_day_periods_spec(
            target.day_periods.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.date_patterns {
        target.date_patterns = Some(merge_lengths_spec(
            target.date_patterns.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.time_patterns {
        target.time_patterns = Some(merge_lengths_spec(
            target.time_patterns.take().unwrap_or_default(),
            source,
        ));
    }
    if let Some(source) = source.datetime_glue {
        target.datetime_glue = Some(merge_lengths_spec(
            target.datetime_glue.take().unwrap_or_default(),
            source,
        ));
    }
    if source.first_day_of_week.is_some() {
        target.first_day_of_week = source.first_day_of_week;
    }
    if source.digits.is_some() {
        target.digits = source.digits;
    }
}

fn merge_widths12_spec(mut target: Widths12Spec, source: Widths12Spec) -> Widths12Spec {
    if source.narrow.is_some() {
        target.narrow = source.narrow;
    }
    if source.abbrev.is_some() {
        target.abbrev = source.abbrev;
    }
    if source.wide.is_some() {
        target.wide = source.wide;
    }
    target
}

fn merge_widths7_spec(mut target: Widths7Spec, source: Widths7Spec) -> Widths7Spec {
    if source.narrow.is_some() {
        target.narrow = source.narrow;
    }
    if source.abbrev.is_some() {
        target.abbrev = source.abbrev;
    }
    if source.wide.is_some() {
        target.wide = source.wide;
    }
    target
}

fn merge_widths4_spec(mut target: Widths4Spec, source: Widths4Spec) -> Widths4Spec {
    if source.narrow.is_some() {
        target.narrow = source.narrow;
    }
    if source.abbrev.is_some() {
        target.abbrev = source.abbrev;
    }
    if source.wide.is_some() {
        target.wide = source.wide;
    }
    target
}

fn merge_widths2_spec(mut target: Widths2Spec, source: Widths2Spec) -> Widths2Spec {
    if source.narrow.is_some() {
        target.narrow = source.narrow;
    }
    if source.abbrev.is_some() {
        target.abbrev = source.abbrev;
    }
    if source.wide.is_some() {
        target.wide = source.wide;
    }
    target
}

fn merge_day_periods_spec(mut target: DayPeriodsSpec, source: DayPeriodsSpec) -> DayPeriodsSpec {
    if source.am.is_some() {
        target.am = source.am;
    }
    if source.pm.is_some() {
        target.pm = source.pm;
    }
    target
}

fn merge_lengths_spec(mut target: LengthsSpec, source: LengthsSpec) -> LengthsSpec {
    if source.short.is_some() {
        target.short = source.short;
    }
    if source.medium.is_some() {
        target.medium = source.medium;
    }
    if source.long.is_some() {
        target.long = source.long;
    }
    if source.full.is_some() {
        target.full = source.full;
    }
    target
}

fn merge_widths12(mut target: Widths12, source: Widths12Spec) -> Widths12 {
    if let Some(value) = source.narrow {
        target.narrow = value;
    }
    if let Some(value) = source.abbrev {
        target.abbrev = value;
    }
    if let Some(value) = source.wide {
        target.wide = value;
    }
    target
}

fn merge_widths7(mut target: Widths7, source: Widths7Spec) -> Widths7 {
    if let Some(value) = source.narrow {
        target.narrow = value;
    }
    if let Some(value) = source.abbrev {
        target.abbrev = value;
    }
    if let Some(value) = source.wide {
        target.wide = value;
    }
    target
}

fn merge_widths4(mut target: Widths4, source: Widths4Spec) -> Widths4 {
    if let Some(value) = source.narrow {
        target.narrow = value;
    }
    if let Some(value) = source.abbrev {
        target.abbrev = value;
    }
    if let Some(value) = source.wide {
        target.wide = value;
    }
    target
}

fn merge_widths2(mut target: Widths2, source: Widths2Spec) -> Widths2 {
    if let Some(value) = source.narrow {
        target.narrow = value;
    }
    if let Some(value) = source.abbrev {
        target.abbrev = value;
    }
    if let Some(value) = source.wide {
        target.wide = value;
    }
    target
}

fn merge_day_periods(mut target: DayPeriods, source: DayPeriodsSpec) -> DayPeriods {
    if let Some(value) = source.am {
        target.am = value;
    }
    if let Some(value) = source.pm {
        target.pm = value;
    }
    target
}

fn merge_lengths(mut target: Lengths, source: LengthsSpec) -> Lengths {
    if let Some(value) = source.short {
        target.short = value;
    }
    if let Some(value) = source.medium {
        target.medium = value;
    }
    if let Some(value) = source.long {
        target.long = value;
    }
    if let Some(value) = source.full {
        target.full = value;
    }
    target
}

fn validate_lengths(name: &str, lengths: &Lengths) -> Result<(), DateTimeFormatError> {
    for (length, pattern) in [
        ("short", &lengths.short),
        ("medium", &lengths.medium),
        ("long", &lengths.long),
        ("full", &lengths.full),
    ] {
        parse_datetime_spec(pattern).map_err(|err| {
            DateTimeFormatError::InvalidLocaleData(format!("{name} `{length}` is invalid: {err}"))
        })?;
    }
    Ok(())
}

fn validate_glue_lengths(lengths: &Lengths) -> Result<(), DateTimeFormatError> {
    for (length, pattern) in [
        ("short", &lengths.short),
        ("medium", &lengths.medium),
        ("long", &lengths.long),
        ("full", &lengths.full),
    ] {
        validate_datetime_glue_pattern(pattern).map_err(|err| {
            DateTimeFormatError::InvalidLocaleData(format!(
                "datetime glue `{length}` is invalid: {err}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::{str_array12, LengthsSpec, Widths12Spec};

    #[test]
    fn resolves_builtin_en_us() {
        let registry = DateTimeLocaleRegistry::with_builtins();
        let locale = registry.resolve("en-US").unwrap();
        assert_eq!(locale.months.abbrev[0], "Jan");
        assert_eq!(locale.date_patterns.medium, "MMM d, y");
    }

    #[test]
    fn registers_partial_custom_locale_with_base() {
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        registry
            .register_custom_locale(
                "test",
                DateTimeLocaleSpec {
                    base: Some(LocaleId::new("en-US")),
                    months: Some(Widths12Spec {
                        abbrev: Some(str_array12([
                            "Ja", "Fe", "Mr", "Ap", "My", "Jn", "Jl", "Au", "Se", "Oc", "No", "De",
                        ])),
                        ..Default::default()
                    }),
                    date_patterns: Some(LengthsSpec {
                        long: Some("d MMMM y".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        let locale = registry.resolve("test").unwrap();
        assert_eq!(locale.months.abbrev[0], "Ja");
        assert_eq!(locale.months.wide[0], "January");
        assert_eq!(locale.date_patterns.long, "d MMMM y");
    }

    #[test]
    fn registers_partial_custom_locale_from_json() {
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "json-test",
                r#"{
                    "base": "en-US",
                    "months": {
                        "abbrev": ["Ja","Fe","Mr","Ap","My","Jn","Jl","Au","Se","Oc","No","De"]
                    },
                    "date_patterns": {
                        "long": "d MMMM y"
                    }
                }"#,
            )
            .unwrap();
        let locale = registry.resolve("json-test").unwrap();
        assert_eq!(locale.months.abbrev[0], "Ja");
        assert_eq!(locale.months.wide[0], "January");
        assert_eq!(locale.date_patterns.long, "d MMMM y");
    }

    #[test]
    fn rejects_missing_base_cycle_and_invalid_patterns() {
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        assert!(registry
            .register_custom_locale(
                "missing-base",
                DateTimeLocaleSpec {
                    base: Some(LocaleId::new("nope")),
                    ..Default::default()
                },
            )
            .is_err());
        assert!(registry
            .register_custom_locale(
                "bad-pattern",
                DateTimeLocaleSpec {
                    date_patterns: Some(LengthsSpec {
                        long: Some("%Y".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .is_err());
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        registry.custom.insert(
            LocaleId::new("a"),
            DateTimeLocaleSpec {
                base: Some(LocaleId::new("b")),
                ..Default::default()
            },
        );
        registry.custom.insert(
            LocaleId::new("b"),
            DateTimeLocaleSpec {
                base: Some(LocaleId::new("a")),
                ..Default::default()
            },
        );
        assert!(registry.resolve("a").is_err());
    }
}
