use std::collections::{BTreeMap, BTreeSet};

use crate::{
    cldr_pattern::normalize_number_pattern,
    compact::CompactTier,
    currency::validate_currency_code,
    error::FormatError,
    format::split_compact_pattern,
    locale::{
        CldrCurrencyFormatSpec, CurrencyDisplayNames, CurrencyFormat, CurrencyFormatSpec,
        CurrencyPattern, DecimalPattern, DecimalPatternSpec, GroupingSpec, LocaleId,
        NumberLocaleSpec, ResolvedNumberLocale,
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberLocaleRegistry {
    builtins: BTreeMap<LocaleId, NumberLocaleSpec>,
    custom: BTreeMap<LocaleId, NumberLocaleSpec>,
}

impl NumberLocaleRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry
            .builtins
            .insert(LocaleId::new("en-US"), builtin_en_us_spec());
        registry
            .builtins
            .insert(LocaleId::new("de-DE"), builtin_de_de_spec());
        registry
            .builtins
            .insert(LocaleId::new("fr-FR"), builtin_fr_fr_spec());
        registry
            .builtins
            .insert(LocaleId::new("ja-JP"), builtin_ja_jp_spec());
        registry
    }

    pub fn register_custom_locale(
        &mut self,
        id: impl Into<String>,
        spec: NumberLocaleSpec,
    ) -> Result<(), FormatError> {
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
            locale.decimal_pattern = resolve_decimal_pattern(value)?;
        }
        if let Some(value) = spec.percent_pattern {
            locale.percent_pattern = resolve_decimal_pattern(value)?;
        }
        if let Some(value) = spec.currency {
            locale.currency = resolve_currency_format(value)?;
        }
        if let Some(value) = spec.currency_names {
            validate_currency_names(&value)?;
            locale.currency_names.extend(value);
        }
        if let Some(value) = spec.compact_short {
            let value = resolve_compact_tiers(value)?;
            locale.compact_short = value;
        }
        if let Some(value) = spec.compact_long {
            let value = resolve_compact_tiers(value)?;
            locale.compact_long = value;
        }

        Ok(locale)
    }
}

fn builtin_en_us_spec() -> NumberLocaleSpec {
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
    }
}

fn builtin_de_de_spec() -> NumberLocaleSpec {
    NumberLocaleSpec {
        decimal: Some(",".to_string()),
        group: Some(".".to_string()),
        grouping: Some(GroupingSpec::default()),
        minus: Some("-".to_string()),
        plus: Some("+".to_string()),
        percent: Some("%".to_string()),
        permille: Some("\u{2030}".to_string()),
        nan: Some("NaN".to_string()),
        infinity: Some("\u{221e}".to_string()),
        currency: Some(currency_suffix_format("\u{00a0}", false).into()),
        currency_names: Some(currency_names([
            ("USD", "$", "$", "US-Dollar"),
            ("EUR", "\u{20ac}", "\u{20ac}", "Euro"),
            ("JPY", "\u{00a5}", "\u{00a5}", "Japanischer Yen"),
        ])),
        compact_short: Some(vec![
            compact_tier(3, "{0}"),
            compact_tier(6, "{0}\u{00a0}Mio."),
            compact_tier(9, "{0}\u{00a0}Mrd."),
            compact_tier(12, "{0}\u{00a0}Bio."),
        ]),
        compact_long: Some(vec![
            CompactTier {
                exponent: 3,
                one: Some("{0} Tausend".to_string()),
                other: "{0} Tausend".to_string(),
            },
            CompactTier {
                exponent: 6,
                one: Some("{0} Million".to_string()),
                other: "{0} Millionen".to_string(),
            },
            CompactTier {
                exponent: 9,
                one: Some("{0} Milliarde".to_string()),
                other: "{0} Milliarden".to_string(),
            },
            CompactTier {
                exponent: 12,
                one: Some("{0} Billion".to_string()),
                other: "{0} Billionen".to_string(),
            },
        ]),
        ..Default::default()
    }
}

fn builtin_fr_fr_spec() -> NumberLocaleSpec {
    NumberLocaleSpec {
        decimal: Some(",".to_string()),
        group: Some("\u{202f}".to_string()),
        grouping: Some(GroupingSpec::default()),
        minus: Some("-".to_string()),
        plus: Some("+".to_string()),
        percent: Some("%".to_string()),
        permille: Some("\u{2030}".to_string()),
        nan: Some("NaN".to_string()),
        infinity: Some("\u{221e}".to_string()),
        currency: Some(currency_suffix_format("\u{00a0}", true).into()),
        currency_names: Some(currency_names([
            ("USD", "$US", "$", "dollars des \u{00c9}tats-Unis"),
            ("EUR", "\u{20ac}", "\u{20ac}", "euros"),
            ("JPY", "JPY", "\u{00a5}", "yens japonais"),
        ])),
        compact_short: Some(vec![
            compact_tier(3, "{0}\u{00a0}k"),
            compact_tier(6, "{0}\u{00a0}M"),
            compact_tier(9, "{0}\u{00a0}Md"),
            compact_tier(12, "{0}\u{00a0}Bn"),
        ]),
        compact_long: Some(vec![
            CompactTier {
                exponent: 3,
                one: Some("{0} millier".to_string()),
                other: "{0} mille".to_string(),
            },
            CompactTier {
                exponent: 6,
                one: Some("{0} million".to_string()),
                other: "{0} millions".to_string(),
            },
            CompactTier {
                exponent: 9,
                one: Some("{0} milliard".to_string()),
                other: "{0} milliards".to_string(),
            },
            CompactTier {
                exponent: 12,
                one: Some("{0} billion".to_string()),
                other: "{0} billions".to_string(),
            },
        ]),
        ..Default::default()
    }
}

fn builtin_ja_jp_spec() -> NumberLocaleSpec {
    NumberLocaleSpec {
        decimal: Some(".".to_string()),
        group: Some(",".to_string()),
        grouping: Some(GroupingSpec::default()),
        minus: Some("-".to_string()),
        plus: Some("+".to_string()),
        percent: Some("%".to_string()),
        permille: Some("\u{2030}".to_string()),
        nan: Some("NaN".to_string()),
        infinity: Some("\u{221e}".to_string()),
        currency: Some(currency_prefix_format().into()),
        currency_names: Some(currency_names([
            ("USD", "$", "$", "\u{7c73}\u{30c9}\u{30eb}"),
            ("EUR", "\u{20ac}", "\u{20ac}", "\u{30e6}\u{30fc}\u{30ed}"),
            ("JPY", "\u{ffe5}", "\u{ffe5}", "\u{65e5}\u{672c}\u{5186}"),
        ])),
        compact_short: Some(vec![
            compact_tier(4, "{0}\u{4e07}"),
            compact_tier(8, "{0}\u{5104}"),
            compact_tier(12, "{0}\u{5146}"),
            compact_tier(16, "{0}\u{4eac}"),
        ]),
        compact_long: Some(vec![
            compact_tier(4, "{0}\u{4e07}"),
            compact_tier(8, "{0}\u{5104}"),
            compact_tier(12, "{0}\u{5146}"),
            compact_tier(16, "{0}\u{4eac}"),
        ]),
        ..Default::default()
    }
}

fn currency_prefix_format() -> CurrencyFormat {
    CurrencyFormat {
        standard: CurrencyPattern {
            positive_prefix: "\u{00a4}".to_string(),
            positive_suffix: String::new(),
            negative_prefix: "-\u{00a4}".to_string(),
            negative_suffix: String::new(),
        },
        accounting: CurrencyPattern {
            positive_prefix: "\u{00a4}".to_string(),
            positive_suffix: String::new(),
            negative_prefix: "(\u{00a4}".to_string(),
            negative_suffix: ")".to_string(),
        },
    }
}

fn currency_suffix_format(space: &str, accounting_parentheses: bool) -> CurrencyFormat {
    CurrencyFormat {
        standard: CurrencyPattern {
            positive_prefix: String::new(),
            positive_suffix: format!("{space}\u{00a4}"),
            negative_prefix: "-".to_string(),
            negative_suffix: format!("{space}\u{00a4}"),
        },
        accounting: CurrencyPattern {
            positive_prefix: String::new(),
            positive_suffix: format!("{space}\u{00a4}"),
            negative_prefix: if accounting_parentheses {
                "(".to_string()
            } else {
                "-".to_string()
            },
            negative_suffix: if accounting_parentheses {
                format!("{space}\u{00a4})")
            } else {
                format!("{space}\u{00a4}")
            },
        },
    }
}

fn currency_names<const N: usize>(
    names: [(&'static str, &'static str, &'static str, &'static str); N],
) -> BTreeMap<String, CurrencyDisplayNames> {
    names
        .into_iter()
        .map(|(code, symbol, narrow_symbol, name)| {
            (
                code.to_string(),
                CurrencyDisplayNames {
                    symbol: Some(symbol.to_string()),
                    narrow_symbol: Some(narrow_symbol.to_string()),
                    name: Some(name.to_string()),
                },
            )
        })
        .collect()
}

fn compact_tier(exponent: i32, pattern: &str) -> CompactTier {
    CompactTier {
        exponent,
        one: Some(pattern.to_string()),
        other: pattern.to_string(),
    }
}

fn resolve_decimal_pattern(pattern: DecimalPatternSpec) -> Result<DecimalPattern, FormatError> {
    match pattern {
        DecimalPatternSpec::Normalized(pattern) => {
            validate_decimal_pattern(&pattern)?;
            Ok(pattern)
        }
        DecimalPatternSpec::Cldr(pattern) => {
            let pattern = normalize_number_pattern(&pattern)?;
            Ok(DecimalPattern {
                positive_prefix: pattern.positive_prefix,
                positive_suffix: pattern.positive_suffix,
                negative_prefix: pattern.negative_prefix,
                negative_suffix: pattern.negative_suffix,
            })
        }
    }
}

fn resolve_currency_format(format: CurrencyFormatSpec) -> Result<CurrencyFormat, FormatError> {
    let format = match format {
        CurrencyFormatSpec::Normalized(format) => format,
        CurrencyFormatSpec::Cldr(source) => resolve_cldr_currency_format(source)?,
        CurrencyFormatSpec::CldrSingle(pattern) => {
            resolve_cldr_currency_format(CldrCurrencyFormatSpec {
                standard: pattern,
                accounting: None,
            })?
        }
    };
    validate_currency_format(&format)?;
    Ok(format)
}

fn resolve_cldr_currency_format(
    source: CldrCurrencyFormatSpec,
) -> Result<CurrencyFormat, FormatError> {
    let accounting = source.accounting.as_deref().unwrap_or(&source.standard);
    Ok(CurrencyFormat {
        standard: resolve_cldr_currency_pattern(&source.standard)?,
        accounting: resolve_cldr_currency_pattern(accounting)?,
    })
}

fn resolve_cldr_currency_pattern(pattern: &str) -> Result<CurrencyPattern, FormatError> {
    let pattern = normalize_number_pattern(pattern)?;
    Ok(CurrencyPattern {
        positive_prefix: normalize_currency_sign_runs(&pattern.positive_prefix),
        positive_suffix: normalize_currency_sign_runs(&pattern.positive_suffix),
        negative_prefix: normalize_currency_sign_runs(&pattern.negative_prefix),
        negative_suffix: normalize_currency_sign_runs(&pattern.negative_suffix),
    })
}

fn normalize_currency_sign_runs(value: &str) -> String {
    let mut output = String::new();
    let mut in_currency_run = false;

    for ch in value.chars() {
        if ch == '\u{00a4}' {
            if !in_currency_run {
                output.push(ch);
            }
            in_currency_run = true;
        } else {
            output.push(ch);
            in_currency_run = false;
        }
    }

    output
}

fn validate_decimal_pattern(pattern: &DecimalPattern) -> Result<(), FormatError> {
    validate_affix("positive decimal prefix", &pattern.positive_prefix)?;
    validate_affix("positive decimal suffix", &pattern.positive_suffix)?;
    validate_affix("negative decimal prefix", &pattern.negative_prefix)?;
    validate_affix("negative decimal suffix", &pattern.negative_suffix)
}

fn validate_currency_format(format: &CurrencyFormat) -> Result<(), FormatError> {
    validate_currency_pattern("standard positive", &format.standard)?;
    validate_currency_pattern("accounting positive", &format.accounting)
}

fn validate_currency_pattern(name: &str, pattern: &CurrencyPattern) -> Result<(), FormatError> {
    validate_affix(
        &format!("{name} currency positive prefix"),
        &pattern.positive_prefix,
    )?;
    validate_affix(
        &format!("{name} currency positive suffix"),
        &pattern.positive_suffix,
    )?;
    validate_affix(
        &format!("{name} currency negative prefix"),
        &pattern.negative_prefix,
    )?;
    validate_affix(
        &format!("{name} currency negative suffix"),
        &pattern.negative_suffix,
    )?;
    if !pattern.positive_prefix.contains('\u{00a4}')
        && !pattern.positive_suffix.contains('\u{00a4}')
        && !pattern.negative_prefix.contains('\u{00a4}')
        && !pattern.negative_suffix.contains('\u{00a4}')
    {
        return Err(FormatError::InvalidLocaleData(
            "currency patterns must contain a currency sign".to_string(),
        ));
    }
    Ok(())
}

fn validate_currency_names(
    names: &BTreeMap<String, CurrencyDisplayNames>,
) -> Result<(), FormatError> {
    for (code, display) in names {
        validate_currency_code(code)?;
        if display
            .symbol
            .as_ref()
            .or(display.narrow_symbol.as_ref())
            .or(display.name.as_ref())
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            return Err(FormatError::InvalidLocaleData(format!(
                "currency display names for `{code}` must include a nonempty display string"
            )));
        }
    }
    Ok(())
}

fn resolve_compact_tiers(tiers: Vec<CompactTier>) -> Result<Vec<CompactTier>, FormatError> {
    let tiers = tiers
        .into_iter()
        .map(|tier| {
            Ok(CompactTier {
                exponent: tier.exponent,
                one: tier.one.map(normalize_compact_pattern).transpose()?,
                other: normalize_compact_pattern(tier.other)?,
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    validate_compact_tiers(&tiers)?;
    Ok(tiers)
}

fn normalize_compact_pattern(pattern: String) -> Result<String, FormatError> {
    if split_compact_pattern(&pattern).is_ok() {
        return Ok(pattern);
    }
    let normalized = normalize_number_pattern(&pattern)?;
    Ok(format!(
        "{}{{0}}{}",
        normalized.positive_prefix, normalized.positive_suffix
    ))
}

fn validate_compact_tiers(tiers: &[CompactTier]) -> Result<(), FormatError> {
    let mut seen = BTreeSet::new();
    for tier in tiers {
        if !seen.insert(tier.exponent) {
            return Err(FormatError::InvalidLocaleData(format!(
                "duplicate compact tier exponent `{}`",
                tier.exponent
            )));
        }
        split_compact_pattern(&tier.other)?;
        if let Some(one) = &tier.one {
            split_compact_pattern(one)?;
        }
    }
    Ok(())
}

fn validate_affix(name: &str, value: &str) -> Result<(), FormatError> {
    if value.contains('*') {
        return Err(FormatError::InvalidLocaleData(format!(
            "{name} contains unsupported CLDR padding escape"
        )));
    }
    Ok(())
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
    if override_spec.currency_names.is_some() {
        base.currency_names = override_spec.currency_names;
    }
    if override_spec.compact_short.is_some() {
        base.compact_short = override_spec.compact_short;
    }
    if override_spec.compact_long.is_some() {
        base.compact_long = override_spec.compact_long;
    }
}

#[cfg(test)]
mod tests {
    use super::NumberLocaleRegistry;
    use crate::{
        compact::CompactTier,
        format::{format_number, NumberFormatContext, NumberFormatOverrides},
        locale::{LocaleId, NumberLocaleSpec},
    };

    #[test]
    fn resolves_builtin_en_us() {
        let registry = NumberLocaleRegistry::with_builtins();
        let locale = registry.resolve("en-US").unwrap();
        assert_eq!(locale.decimal, ".");
        assert_eq!(locale.group, ",");
    }

    #[test]
    fn resolves_additional_builtin_locale_symbols() {
        let registry = NumberLocaleRegistry::with_builtins();

        let de = registry.resolve("de-DE").unwrap();
        assert_eq!(de.decimal, ",");
        assert_eq!(de.group, ".");

        let fr = registry.resolve("fr-FR").unwrap();
        assert_eq!(fr.decimal, ",");
        assert_eq!(fr.group, "\u{202f}");

        let ja = registry.resolve("ja-JP").unwrap();
        assert_eq!(ja.currency_names["JPY"].symbol.as_deref(), Some("\u{ffe5}"));
    }

    #[test]
    fn formats_with_additional_builtin_locales() {
        let registry = NumberLocaleRegistry::with_builtins();

        let de = registry.resolve("de-DE").unwrap();
        let de_context = NumberFormatContext::new(&de).with_registry(&registry);
        assert_eq!(
            format_number(
                1234.5,
                Some(",.1f"),
                NumberFormatOverrides::default(),
                de_context,
            )
            .unwrap()
            .text,
            "1.234,5"
        );
        assert_eq!(
            format_number(
                1234.5,
                Some(",.2C[EUR]"),
                NumberFormatOverrides::default(),
                de_context,
            )
            .unwrap()
            .text,
            "1.234,50\u{00a0}\u{20ac}"
        );
        assert_eq!(
            format_number(
                1_200_000.0,
                Some(".2S"),
                NumberFormatOverrides::default(),
                de_context,
            )
            .unwrap()
            .text,
            "1,2\u{00a0}Mio."
        );

        let fr = registry.resolve("fr-FR").unwrap();
        let fr_context = NumberFormatContext::new(&fr).with_registry(&registry);
        assert_eq!(
            format_number(
                1234.5,
                Some(",.2C[EUR]"),
                NumberFormatOverrides::default(),
                fr_context,
            )
            .unwrap()
            .text,
            "1\u{202f}234,50\u{00a0}\u{20ac}"
        );
        assert_eq!(
            format_number(
                1_200_000.0,
                Some(".2S"),
                NumberFormatOverrides::default(),
                fr_context,
            )
            .unwrap()
            .text,
            "1,2\u{00a0}M"
        );

        let ja = registry.resolve("ja-JP").unwrap();
        let ja_context = NumberFormatContext::new(&ja).with_registry(&registry);
        assert_eq!(
            format_number(
                1234.5,
                Some(",C[JPY]"),
                NumberFormatOverrides::default(),
                ja_context,
            )
            .unwrap()
            .text,
            "\u{ffe5}1,234"
        );
        assert_eq!(
            format_number(
                1_200_000.0,
                Some(".3S"),
                NumberFormatOverrides::default(),
                ja_context,
            )
            .unwrap()
            .text,
            "120\u{4e07}"
        );
    }

    #[test]
    fn registers_partial_custom_locale_with_base() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "comma",
                r#"{
                    "base": "en-US",
                    "decimal": ",",
                    "group": ".",
                    "grouping": { "primary": 3, "secondary": 2, "min_grouping_digits": 1 }
                }"#,
            )
            .unwrap();

        let locale = registry.resolve("comma").unwrap();
        let formatted = format_number(
            1234567.5,
            Some(",.1f"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(formatted.text, "12.34.567,5");
    }

    #[test]
    fn normalizes_json_cldr_patterns() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "patterns",
                r##"{
                    "base": "en-US",
                    "decimal_pattern": "'~'#,##0 'items';('~'#,##0 'items')",
                    "percent_pattern": "#,##0 percent"
                }"##,
            )
            .unwrap();

        let locale = registry.resolve("patterns").unwrap();
        assert_eq!(locale.decimal_pattern.positive_prefix, "~");
        assert_eq!(locale.decimal_pattern.positive_suffix, " items");
        assert_eq!(locale.decimal_pattern.negative_prefix, "(~");
        assert_eq!(locale.decimal_pattern.negative_suffix, " items)");
        assert_eq!(locale.percent_pattern.positive_suffix, " percent");
    }

    #[test]
    fn normalizes_json_cldr_currency_patterns() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "currency-patterns",
                r##"{
                    "base": "en-US",
                    "currency": {
                        "standard": "¤¤ #,##0.00;-¤¤ #,##0.00",
                        "accounting": "¤ #,##0.00;(¤ #,##0.00)"
                    }
                }"##,
            )
            .unwrap();

        let locale = registry.resolve("currency-patterns").unwrap();
        assert_eq!(locale.currency.standard.positive_prefix, "\u{00a4} ");
        assert_eq!(locale.currency.standard.negative_prefix, "-\u{00a4} ");
        assert_eq!(locale.currency.accounting.negative_prefix, "(\u{00a4} ");
        assert_eq!(locale.currency.accounting.negative_suffix, ")");

        let context = NumberFormatContext::new(&locale).with_registry(&registry);
        assert_eq!(
            format_number(
                1234.5,
                Some(",.2C[USD]"),
                NumberFormatOverrides::default(),
                context,
            )
            .unwrap()
            .text,
            "$ 1,234.50"
        );
        assert_eq!(
            format_number(
                -1234.5,
                Some("(,.2C[USD]"),
                NumberFormatOverrides::default(),
                context,
            )
            .unwrap()
            .text,
            "($ 1,234.50)"
        );
    }

    #[test]
    fn normalizes_json_cldr_compact_patterns() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "compact-patterns",
                r##"{
                    "base": "en-US",
                    "compact_short": [
                        { "exponent": 3, "one": "0K", "other": "0K" },
                        { "exponent": 6, "one": "0M", "other": "0M" }
                    ],
                    "compact_long": [
                        { "exponent": 3, "one": "0 thousand", "other": "0 thousand" },
                        { "exponent": 6, "one": "0 million", "other": "0 million" }
                    ]
                }"##,
            )
            .unwrap();

        let locale = registry.resolve("compact-patterns").unwrap();
        assert_eq!(locale.compact_short[0].other, "{0}K");
        assert_eq!(locale.compact_long[0].other, "{0} thousand");

        let context = NumberFormatContext::new(&locale).with_registry(&registry);
        assert_eq!(
            format_number(
                1234.5,
                Some(".2S"),
                NumberFormatOverrides::default(),
                context,
            )
            .unwrap()
            .text,
            "1.2K"
        );
        assert_eq!(
            format_number(
                1_234_500.0,
                Some(".2L"),
                NumberFormatOverrides::default(),
                context,
            )
            .unwrap()
            .text,
            "1.2 million"
        );
    }

    #[test]
    fn rejects_cldr_currency_pattern_without_currency_sign() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        let err = registry
            .register_custom_locale_json(
                "bad-currency-pattern",
                r##"{
                    "base": "en-US",
                    "currency": {
                        "standard": "#,##0.00",
                        "accounting": "(#,##0.00)"
                    }
                }"##,
            )
            .unwrap_err();
        assert!(err.to_string().contains("currency sign"));
    }

    #[test]
    fn rejects_missing_base_and_base_cycles() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        let err = registry
            .register_custom_locale(
                "missing",
                NumberLocaleSpec {
                    base: Some(LocaleId::new("does-not-exist")),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));

        registry
            .register_custom_locale(
                "a",
                NumberLocaleSpec {
                    base: Some(LocaleId::new("en-US")),
                    ..Default::default()
                },
            )
            .unwrap();
        registry
            .register_custom_locale(
                "b",
                NumberLocaleSpec {
                    base: Some(LocaleId::new("a")),
                    ..Default::default()
                },
            )
            .unwrap();
        let err = registry
            .register_custom_locale(
                "a",
                NumberLocaleSpec {
                    base: Some(LocaleId::new("b")),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("cycle"));
        assert!(registry.resolve("a").is_ok());
    }

    #[test]
    fn invalid_registration_rolls_back_previous_locale() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json("custom", r#"{ "base": "en-US", "decimal": "," }"#)
            .unwrap();
        assert_eq!(registry.resolve("custom").unwrap().decimal, ",");

        assert!(registry
            .register_custom_locale_json(
                "custom",
                r#"{ "base": "en-US", "grouping": { "primary": 0 } }"#,
            )
            .is_err());
        assert_eq!(registry.resolve("custom").unwrap().decimal, ",");
    }

    #[test]
    fn validates_compact_tier_patterns() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        let err = registry
            .register_custom_locale(
                "bad-compact",
                NumberLocaleSpec {
                    compact_short: Some(vec![CompactTier {
                        exponent: 3,
                        one: None,
                        other: "K".to_string(),
                    }]),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("digit placeholders"));
    }
}
