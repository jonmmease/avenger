use std::collections::{BTreeMap, BTreeSet};

use crate::{
    error::FormatError,
    locale::{LocaleId, NumberLocaleSpec, ResolvedNumberLocale},
};

#[derive(Debug, Clone, Default)]
pub struct NumberLocaleRegistry {
    builtins: BTreeMap<LocaleId, NumberLocaleSpec>,
    custom: BTreeMap<LocaleId, NumberLocaleSpec>,
}

impl NumberLocaleRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry.builtins.insert(
            LocaleId::new("en-US"),
            NumberLocaleSpec {
                decimal: Some(".".to_string()),
                group: Some(",".to_string()),
                grouping: Some(Default::default()),
                minus: Some("-".to_string()),
                plus: Some("+".to_string()),
                percent: Some("%".to_string()),
                permille: Some("\u{2030}".to_string()),
                nan: Some("NaN".to_string()),
                infinity: Some("\u{221e}".to_string()),
                ..Default::default()
            },
        );
        registry
    }

    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: NumberLocaleSpec,
    ) -> Result<(), FormatError> {
        let id = LocaleId::new(id);
        self.custom.insert(id.clone(), spec);
        self.resolve(&id.0)?;
        Ok(())
    }

    pub fn register_custom_locale_json(
        &mut self,
        id: impl Into<String>,
        json: &str,
    ) -> Result<(), FormatError> {
        let spec: NumberLocaleSpec = serde_json::from_str(json)
            .map_err(|err| FormatError::InvalidLocaleData(err.to_string()))?;
        self.register_custom_locale(id, spec)
    }

    pub fn resolve(&self, id: &str) -> Result<ResolvedNumberLocale, FormatError> {
        let id = LocaleId::new(id);
        let spec = self.resolve_spec(&id, &mut BTreeSet::new())?;
        self.finish_resolved(id, spec)
    }

    fn resolve_spec(
        &self,
        id: &LocaleId,
        resolving: &mut BTreeSet<LocaleId>,
    ) -> Result<NumberLocaleSpec, FormatError> {
        if !resolving.insert(id.clone()) {
            return Err(FormatError::InvalidLocaleData(format!(
                "base locale cycle involving `{id}`"
            )));
        }

        let spec = self
            .custom
            .get(id)
            .or_else(|| self.builtins.get(id))
            .ok_or_else(|| FormatError::LocaleNotFound(id.to_string()))?;

        let mut resolved = if let Some(base) = &spec.base {
            self.resolve_spec(base, resolving)?
        } else if id.as_ref() == "en-US" {
            NumberLocaleSpec::default()
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
        spec: NumberLocaleSpec,
    ) -> Result<ResolvedNumberLocale, FormatError> {
        let mut locale = ResolvedNumberLocale::en_us();
        locale.id = id;

        if let Some(value) = spec.decimal {
            locale.decimal = value;
        }
        if let Some(value) = spec.group {
            locale.group = value;
        }
        if let Some(value) = spec.grouping {
            if value.primary == 0 || value.secondary == Some(0) {
                return Err(FormatError::InvalidLocaleData(
                    "grouping sizes must be greater than zero".to_string(),
                ));
            }
            locale.grouping = value;
        }
        if let Some(value) = spec.minus {
            locale.minus = value;
        }
        if let Some(value) = spec.plus {
            locale.plus = value;
        }
        if let Some(value) = spec.percent {
            locale.percent = value;
        }
        if let Some(value) = spec.permille {
            locale.permille = value;
        }
        if let Some(value) = spec.nan {
            locale.nan = value;
        }
        if let Some(value) = spec.infinity {
            locale.infinity = value;
        }
        if let Some(value) = spec.digits {
            if value.iter().any(|digit| digit.is_empty()) {
                return Err(FormatError::InvalidLocaleData(
                    "digit strings must be nonempty".to_string(),
                ));
            }
            locale.digits = Some(value);
        }
        if let Some(value) = spec.decimal_pattern {
            locale.decimal_pattern = value;
        }
        if let Some(value) = spec.percent_pattern {
            locale.percent_pattern = value;
        }
        if let Some(value) = spec.currency {
            locale.currency = value;
        }

        Ok(locale)
    }
}

fn merge_spec(base: &mut NumberLocaleSpec, override_spec: NumberLocaleSpec) {
    if override_spec.decimal.is_some() {
        base.decimal = override_spec.decimal;
    }
    if override_spec.group.is_some() {
        base.group = override_spec.group;
    }
    if override_spec.grouping.is_some() {
        base.grouping = override_spec.grouping;
    }
    if override_spec.minus.is_some() {
        base.minus = override_spec.minus;
    }
    if override_spec.plus.is_some() {
        base.plus = override_spec.plus;
    }
    if override_spec.percent.is_some() {
        base.percent = override_spec.percent;
    }
    if override_spec.permille.is_some() {
        base.permille = override_spec.permille;
    }
    if override_spec.nan.is_some() {
        base.nan = override_spec.nan;
    }
    if override_spec.infinity.is_some() {
        base.infinity = override_spec.infinity;
    }
    if override_spec.digits.is_some() {
        base.digits = override_spec.digits;
    }
    if override_spec.decimal_pattern.is_some() {
        base.decimal_pattern = override_spec.decimal_pattern;
    }
    if override_spec.percent_pattern.is_some() {
        base.percent_pattern = override_spec.percent_pattern;
    }
    if override_spec.currency.is_some() {
        base.currency = override_spec.currency;
    }
}
