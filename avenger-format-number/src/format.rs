use crate::{
    compact::CompactTier,
    currency::{currency_display_text, currency_metadata, validate_currency_code},
    digits::substitute_digits,
    error::FormatError,
    locale::{CurrencyDisplay, DecimalPattern, ResolvedNumberLocale},
    parser::parse_number_spec,
    registry::NumberLocaleRegistry,
    spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol},
    typesetting::{ExponentMarker, FormattedNumber, NumberTypesetting},
};

pub(crate) const SI_PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "\u{00b5}", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompatibilityPolicy {
    #[default]
    D3,
}

#[derive(Debug, Clone, Copy)]
pub struct NumberFormatContext<'a> {
    pub locale: &'a ResolvedNumberLocale,
    pub registry: Option<&'a NumberLocaleRegistry>,
    pub compatibility: CompatibilityPolicy,
}

impl<'a> NumberFormatContext<'a> {
    pub fn new(locale: &'a ResolvedNumberLocale) -> Self {
        Self {
            locale,
            registry: None,
            compatibility: CompatibilityPolicy::D3,
        }
    }

    pub fn with_registry(mut self, registry: &'a NumberLocaleRegistry) -> Self {
        self.registry = Some(registry);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NumberFormatOverrides {
    pub format_type: Option<FormatType>,
    pub digit_spec: Option<DigitSpec>,
    pub group: Option<bool>,
    pub trim: Option<bool>,
    pub sign: Option<SignPolicy>,
    pub symbol: Option<Option<Symbol>>,
    pub width: Option<Option<usize>>,
    pub fill: Option<Option<char>>,
    pub align: Option<Option<Align>>,
    pub zero: Option<bool>,
    pub currency: Option<String>,
    pub currency_display: Option<CurrencyDisplay>,
}

impl NumberFormatOverrides {
    pub fn with_precision(mut self, precision: u8) -> Self {
        self.digit_spec = Some(DigitSpec::Precision(precision));
        self
    }

    pub fn with_fraction_digits(mut self, fraction_digits: u8) -> Self {
        self.digit_spec = Some(DigitSpec::Fraction(fraction_digits));
        self
    }

    pub fn with_significant_digits(mut self, significant_digits: u8) -> Self {
        self.digit_spec = Some(DigitSpec::Significant(significant_digits));
        self
    }

    pub fn with_currency(mut self, currency: impl Into<String>) -> Self {
        self.currency = Some(currency.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNumberFormat {
    pub fill: char,
    pub align: Align,
    pub sign: SignPolicy,
    pub symbol: Option<Symbol>,
    pub width: Option<usize>,
    pub group: bool,
    pub digit_spec: DigitSpec,
    pub trim: bool,
    pub format_type: Option<FormatType>,
    pub currency: Option<String>,
    pub currency_display: CurrencyDisplay,
}

pub fn format_number(
    value: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError> {
    let parsed = parse_number_spec(spec.unwrap_or(""))?;
    let resolved = resolve_number_format(parsed, overrides)?;
    format_resolved_number(value, &resolved, context)
}

pub fn resolve_number_format(
    spec: NumberFormatSpec,
    overrides: NumberFormatOverrides,
) -> Result<ResolvedNumberFormat, FormatError> {
    let mut fill = spec.fill.unwrap_or(' ');
    let mut align = spec.align.unwrap_or(Align::Right);
    let sign = overrides.sign.or(spec.sign).unwrap_or(SignPolicy::Minus);
    let symbol = match overrides.symbol {
        Some(symbol) => symbol,
        None => spec.symbol,
    };
    let width = match overrides.width {
        Some(width) => width,
        None => spec.width,
    };
    let group = overrides.group.or(spec.group).unwrap_or(matches!(
        overrides.format_type.or(spec.format_type),
        Some(FormatType::LocaleDefault)
    ));
    let trim = overrides.trim.or(spec.trim).unwrap_or(false);
    let zero = overrides.zero.unwrap_or(spec.zero);
    let format_type = overrides.format_type.or(spec.format_type);
    let currency = overrides.currency.or(spec.currency);
    let currency_display = overrides.currency_display.unwrap_or_default();

    if let Some(fill_override) = overrides.fill {
        fill = fill_override.unwrap_or(' ');
    }
    if let Some(align_override) = overrides.align {
        align = align_override.unwrap_or(Align::Right);
    }

    if zero {
        fill = '0';
        align = Align::AfterSign;
    }

    let digit_spec = overrides
        .digit_spec
        .unwrap_or_else(|| spec.precision.map(DigitSpec::Precision).unwrap_or_default());

    let resolved = ResolvedNumberFormat {
        fill,
        align,
        sign,
        symbol,
        width,
        group,
        digit_spec,
        trim,
        format_type,
        currency,
        currency_display,
    };

    validate_resolved_format(&resolved)?;
    Ok(resolved)
}

fn validate_resolved_format(format: &ResolvedNumberFormat) -> Result<(), FormatError> {
    if format.currency.is_some() && format.format_type != Some(FormatType::Currency) {
        return Err(FormatError::InvalidFormat(
            "currency override is only valid with currency type `C`".to_string(),
        ));
    }

    if matches!(format.format_type, Some(FormatType::Currency)) {
        if format.currency.is_none() {
            return Err(FormatError::MissingCurrencyCode);
        }
        if let Some(symbol) = format.symbol {
            return Err(FormatError::InvalidFormat(format!(
                "symbol `{}` cannot be combined with currency type `C`",
                match symbol {
                    Symbol::CurrencyCompat => "$",
                    Symbol::Alternate => "#",
                }
            )));
        }
        validate_currency_code(format.currency.as_ref().unwrap())?;
    }

    if matches!(
        format.format_type,
        Some(FormatType::CompactShort | FormatType::CompactLong)
    ) && format.symbol == Some(Symbol::Alternate)
    {
        return Err(FormatError::InvalidFormat(
            "`#` cannot be combined with compact extension types".to_string(),
        ));
    }

    Ok(())
}

pub(crate) fn format_resolved_number(
    value: f64,
    format: &ResolvedNumberFormat,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError> {
    if value.is_nan() {
        return Ok(FormattedNumber::plain(context.locale.nan.clone()));
    }
    if value.is_infinite() {
        let is_negative = value.is_sign_negative();
        let body = context.locale.infinity.clone();
        let text = apply_sign_and_padding(is_negative, &body, "", "", format, context.locale);
        return Ok(FormattedNumber::plain(text));
    }

    let effective_type = effective_format_type(format);
    if effective_type == FormatType::Currency {
        return format_currency_number(value, format, context);
    }

    let mut scaled_value = value.abs();
    let mut suffix = String::new();
    let mut prefix = String::new();
    let mut typesetting = NumberTypesetting::Plain;

    if matches!(
        effective_type,
        FormatType::Percent | FormatType::PercentRounded
    ) {
        scaled_value *= 100.0;
    }

    let mut raw_body = match effective_type {
        FormatType::Exponent => {
            let precision = precision_or_default(format, 6);
            let (mantissa, exponent) = format_exponent_parts(scaled_value, precision)?;
            let text = format!("{mantissa}e{exponent:+}");
            typesetting = NumberTypesetting::Exponent {
                mantissa: mantissa.clone(),
                exponent,
                marker: ExponentMarker::LowerE,
            };
            text
        }
        FormatType::Fixed | FormatType::Percent => {
            let precision = fraction_digits(format, 6);
            format_fixed(scaled_value, precision)
        }
        FormatType::General | FormatType::LocaleDefault => {
            let precision =
                significant_digits(format, if format.format_type.is_none() { 12 } else { 6 });
            let mut text = format_general(scaled_value, precision);
            if format.trim || format.format_type.is_none() {
                text = trim_number_text(&text);
            }
            if let Some((mantissa, exponent)) = split_exponent(&text) {
                typesetting = NumberTypesetting::Exponent {
                    mantissa,
                    exponent,
                    marker: ExponentMarker::LowerE,
                };
            }
            text
        }
        FormatType::Rounded | FormatType::PercentRounded => {
            let precision = significant_digits(format, 6);
            let mut text = format_significant_fixed(scaled_value, precision);
            if format.trim {
                text = trim_number_text(&text);
            }
            text
        }
        FormatType::Si => {
            let precision = significant_digits(format, 6);
            let (text, si_prefix) = format_si(scaled_value, precision);
            suffix.push_str(si_prefix);
            text
        }
        FormatType::CompactShort | FormatType::CompactLong => {
            let precision = significant_digits(format, 6);
            let (text, compact_prefix, compact_suffix) = format_compact(
                scaled_value,
                effective_type,
                precision,
                format,
                context.locale,
            )?;
            prefix.push_str(&compact_prefix);
            suffix.push_str(&compact_suffix);
            text
        }
        FormatType::Binary => format_integer_radix(scaled_value, 2, false),
        FormatType::Octal => format_integer_radix(scaled_value, 8, false),
        FormatType::DecimalInteger => format!("{:.0}", scaled_value),
        FormatType::HexLower => format_integer_radix(scaled_value, 16, false),
        FormatType::HexUpper => format_integer_radix(scaled_value, 16, true),
        FormatType::Character => format_character(scaled_value)?,
        FormatType::Currency => unreachable!("currency returns before scalar formatting"),
    };

    if format.trim
        && !matches!(
            effective_type,
            FormatType::General | FormatType::LocaleDefault
        )
    {
        raw_body = trim_number_text(&raw_body);
    }

    let body = localize_number_body(&raw_body, format, context.locale);

    if let Some(symbol) = format.symbol {
        match symbol {
            Symbol::Alternate => match effective_type {
                FormatType::Binary => prefix.push_str("0b"),
                FormatType::Octal => prefix.push_str("0o"),
                FormatType::HexLower | FormatType::HexUpper => prefix.push_str("0x"),
                _ => {}
            },
            Symbol::CurrencyCompat => prefix.push('$'),
        }
    }

    let is_negative = value.is_sign_negative() && !rounds_to_zero(&body, context.locale);
    let pattern = if matches!(
        effective_type,
        FormatType::Percent | FormatType::PercentRounded
    ) {
        &context.locale.percent_pattern
    } else {
        &context.locale.decimal_pattern
    };
    let text = apply_pattern_affixes_and_padding(
        is_negative,
        &body,
        &prefix,
        &suffix,
        format,
        context.locale,
        pattern,
    );
    let text = substitute_digits(&text, context.locale);

    if let NumberTypesetting::Exponent {
        mantissa,
        exponent,
        marker,
    } = &typesetting
    {
        let mantissa_text = if value.is_sign_negative() {
            format!("{}{}", context.locale.minus, mantissa)
        } else {
            mantissa.clone()
        };
        typesetting = NumberTypesetting::Exponent {
            mantissa: substitute_digits(&mantissa_text, context.locale),
            exponent: *exponent,
            marker: *marker,
        };
    }

    Ok(FormattedNumber { text, typesetting })
}

fn format_currency_number(
    value: f64,
    format: &ResolvedNumberFormat,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError> {
    let currency = format
        .currency
        .as_ref()
        .ok_or(FormatError::MissingCurrencyCode)?;
    let metadata = currency_metadata(currency)
        .ok_or_else(|| FormatError::InvalidCurrencyCode(currency.clone()))?;
    let precision = match format.digit_spec {
        DigitSpec::Auto => metadata.default_fraction_digits as usize,
        DigitSpec::Precision(value) | DigitSpec::Fraction(value) => value as usize,
        DigitSpec::Significant(value) => value as usize,
    };
    let mut raw_body = format_fixed(value.abs(), precision);
    if format.trim {
        raw_body = trim_number_text(&raw_body);
    }
    let body = localize_number_body(&raw_body, format, context.locale);
    let currency_display = currency_display_text(context.locale, currency, format.currency_display);
    let is_negative = value.is_sign_negative() && !rounds_to_zero(&body, context.locale);
    let pattern = if is_negative && format.sign == SignPolicy::Parentheses {
        &context.locale.currency.accounting
    } else {
        &context.locale.currency.standard
    };
    let (pattern_prefix, pattern_suffix) = if is_negative {
        (&pattern.negative_prefix, &pattern.negative_suffix)
    } else {
        (&pattern.positive_prefix, &pattern.positive_suffix)
    };
    let mut prefix_template = pattern_prefix.clone();
    if !is_negative {
        match format.sign {
            SignPolicy::Plus => {
                prefix_template = format!("{}{}", context.locale.plus, prefix_template)
            }
            SignPolicy::Space => prefix_template = format!(" {prefix_template}"),
            _ => {}
        }
    }

    let prefix = localize_minus_affix(&prefix_template, context.locale)
        .replace('\u{00a4}', &currency_display);
    let suffix = pattern_suffix.replace('\u{00a4}', &currency_display);
    let text = format!("{prefix}{body}{suffix}");
    let text = apply_padding_to_content(text, format);
    Ok(FormattedNumber::plain(substitute_digits(
        &text,
        context.locale,
    )))
}

fn format_compact(
    value: f64,
    format_type: FormatType,
    precision: usize,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
) -> Result<(String, String, String), FormatError> {
    if value == 0.0 {
        let mut text = format_significant_fixed(value, precision);
        if format.trim {
            text = trim_number_text(&text);
        }
        return Ok((text, String::new(), String::new()));
    }

    let tiers = match format_type {
        FormatType::CompactShort => &locale.compact_short,
        FormatType::CompactLong => &locale.compact_long,
        _ => unreachable!("compact formatter called with non-compact type"),
    };
    let tier = select_compact_tier(value, tiers).ok_or_else(|| {
        FormatError::UnsupportedExtension(format!(
            "{} requires compact locale data",
            format_type.as_char()
        ))
    })?;

    let scaled = value / 10_f64.powi(tier.exponent);
    let mut text = format_significant_fixed(scaled, precision);
    if format.trim {
        text = trim_number_text(&text);
    }
    let pattern = tier.pattern_for_value(&text);
    let (prefix, suffix) = split_compact_pattern(pattern)?;
    Ok((text, prefix.to_string(), suffix.to_string()))
}

pub(crate) fn select_compact_tier(value: f64, tiers: &[CompactTier]) -> Option<&CompactTier> {
    let exponent = value.abs().log10().floor() as i32;
    tiers
        .iter()
        .filter(|tier| tier.exponent <= exponent)
        .max_by_key(|tier| tier.exponent)
}

pub(crate) fn split_compact_pattern(pattern: &str) -> Result<(&str, &str), FormatError> {
    let Some((prefix, suffix)) = pattern.split_once("{0}") else {
        return Err(FormatError::InvalidLocaleData(format!(
            "compact pattern `{pattern}` must contain `{{0}}`"
        )));
    };
    Ok((prefix, suffix))
}

pub(crate) fn format_fixed_scaled_with_affixes(
    value: f64,
    scale_exponent: i32,
    precision: usize,
    unit_prefix: &str,
    unit_suffix: &str,
    format: &ResolvedNumberFormat,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError> {
    if !value.is_finite() {
        return format_resolved_number(value, format, context);
    }

    let scaled_value = value.abs() / 10_f64.powi(scale_exponent);
    let mut raw_body = format_fixed(scaled_value, precision);
    if format.trim {
        raw_body = trim_number_text(&raw_body);
    }
    let body = localize_number_body(&raw_body, format, context.locale);

    let mut prefix = unit_prefix.to_string();
    if format.symbol == Some(Symbol::CurrencyCompat) {
        prefix.push('$');
    }

    let is_negative = value.is_sign_negative() && !rounds_to_zero(&body, context.locale);
    let text = apply_sign_and_padding(
        is_negative,
        &body,
        &prefix,
        unit_suffix,
        format,
        context.locale,
    );
    Ok(FormattedNumber::plain(substitute_digits(
        &text,
        context.locale,
    )))
}

fn apply_padding_to_content(content: String, format: &ResolvedNumberFormat) -> String {
    let Some(width) = format.width else {
        return content;
    };
    let len = content.chars().count();
    if len >= width {
        return content;
    }

    let padding_len = width - len;
    let padding: String = std::iter::repeat_n(format.fill, padding_len).collect();
    match format.align {
        Align::Left => format!("{content}{padding}"),
        Align::Center => {
            let left = padding_len / 2;
            let right = padding_len - left;
            let left_padding: String = std::iter::repeat_n(format.fill, left).collect();
            let right_padding: String = std::iter::repeat_n(format.fill, right).collect();
            format!("{left_padding}{content}{right_padding}")
        }
        Align::AfterSign | Align::Right => format!("{padding}{content}"),
    }
}

fn apply_pattern_affixes_and_padding(
    is_negative: bool,
    body: &str,
    prefix: &str,
    suffix: &str,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
    pattern: &DecimalPattern,
) -> String {
    let (pattern_prefix, pattern_suffix) = if is_negative {
        if format.sign == SignPolicy::Parentheses {
            (
                format!("({}", pattern.positive_prefix),
                format!("{})", pattern.positive_suffix),
            )
        } else {
            (
                localize_minus_affix(&pattern.negative_prefix, locale),
                pattern.negative_suffix.clone(),
            )
        }
    } else {
        let mut positive_prefix = pattern.positive_prefix.clone();
        match format.sign {
            SignPolicy::Plus => positive_prefix = format!("{}{}", locale.plus, positive_prefix),
            SignPolicy::Space => positive_prefix = format!(" {positive_prefix}"),
            _ => {}
        }
        (positive_prefix, pattern.positive_suffix.clone())
    };

    apply_affixes_and_padding(
        &pattern_prefix,
        prefix,
        body,
        suffix,
        &pattern_suffix,
        format,
    )
}

fn localize_minus_affix(affix: &str, locale: &ResolvedNumberLocale) -> String {
    affix
        .strip_prefix('-')
        .map(|rest| format!("{}{}", locale.minus, rest))
        .unwrap_or_else(|| affix.to_string())
}

fn effective_format_type(format: &ResolvedNumberFormat) -> FormatType {
    if format.format_type == Some(FormatType::LocaleDefault) {
        FormatType::General
    } else {
        format.format_type.unwrap_or(FormatType::General)
    }
}

fn precision_or_default(format: &ResolvedNumberFormat, default: u8) -> usize {
    match format.digit_spec {
        DigitSpec::Auto => default as usize,
        DigitSpec::Precision(value)
        | DigitSpec::Fraction(value)
        | DigitSpec::Significant(value) => value as usize,
    }
}

fn fraction_digits(format: &ResolvedNumberFormat, default: u8) -> usize {
    match format.digit_spec {
        DigitSpec::Fraction(value) | DigitSpec::Precision(value) => value as usize,
        DigitSpec::Significant(value) => value as usize,
        DigitSpec::Auto => default as usize,
    }
}

fn significant_digits(format: &ResolvedNumberFormat, default: u8) -> usize {
    match format.digit_spec {
        DigitSpec::Significant(value) | DigitSpec::Precision(value) => value.max(1) as usize,
        DigitSpec::Fraction(value) => value.max(1) as usize,
        DigitSpec::Auto => default as usize,
    }
}

fn format_fixed(value: f64, precision: usize) -> String {
    format!("{value:.precision$}")
}

fn format_exponent_parts(value: f64, precision: usize) -> Result<(String, i32), FormatError> {
    let text = format!("{value:.precision$e}");
    let (mantissa, exponent) = split_exponent(&text).ok_or_else(|| {
        FormatError::InvalidFormat(format!("could not split exponent output `{text}`"))
    })?;
    Ok((mantissa, exponent))
}

fn split_exponent(text: &str) -> Option<(String, i32)> {
    let (mantissa, exponent) = text.split_once('e')?;
    let exponent = exponent.parse::<i32>().ok()?;
    Some((mantissa.to_string(), exponent))
}

fn format_general(value: f64, precision: usize) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    let abs = value.abs();
    let exponent = abs.log10().floor() as i32;
    if exponent < -4 || exponent >= precision as i32 {
        let frac = precision.saturating_sub(1);
        normalize_exponent_text(&format!("{value:.frac$e}"))
    } else {
        let frac = (precision as i32 - exponent - 1).max(0) as usize;
        format!("{value:.frac$}")
    }
}

fn format_significant_fixed(value: f64, precision: usize) -> String {
    if value == 0.0 {
        let frac = precision.saturating_sub(1);
        return format!("{value:.frac$}");
    }
    let exponent = value.abs().log10().floor() as i32;
    let frac = precision as i32 - exponent - 1;
    if frac >= 0 {
        let frac = frac as usize;
        format!("{value:.frac$}")
    } else {
        let factor = 10_f64.powi(-frac);
        let rounded = (value / factor).round() * factor;
        format!("{rounded:.0}")
    }
}

fn normalize_exponent_text(text: &str) -> String {
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text.to_string();
    };
    let sign = if exponent.starts_with('-') { '-' } else { '+' };
    let exp_digits = exponent.trim_start_matches(['+', '-']);
    let exp = exp_digits.parse::<i32>().unwrap_or(0);
    format!("{mantissa}e{sign}{exp}")
}

fn trim_number_text(text: &str) -> String {
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return trim_decimal(text);
    };
    format!("{}e{}", trim_decimal(mantissa), exponent)
}

fn trim_decimal(text: &str) -> String {
    if let Some(dot) = text.find('.') {
        let mut end = text.len();
        while end > dot && text.as_bytes()[end - 1] == b'0' {
            end -= 1;
        }
        if end > dot && text.as_bytes()[end - 1] == b'.' {
            end -= 1;
        }
        text[..end].to_string()
    } else {
        text.to_string()
    }
}

fn format_integer_radix(value: f64, radix: u32, upper: bool) -> String {
    let integer = value.round() as i64;
    match (radix, upper) {
        (2, _) => format!("{integer:b}"),
        (8, _) => format!("{integer:o}"),
        (16, false) => format!("{integer:x}"),
        (16, true) => format!("{integer:X}"),
        _ => integer.to_string(),
    }
}

fn format_character(value: f64) -> Result<String, FormatError> {
    let code_point = value.round();
    if !(0.0..=char::MAX as u32 as f64).contains(&code_point) {
        return Err(FormatError::InvalidFormat(format!(
            "character format code point `{code_point}` is outside the Unicode scalar range",
        )));
    }
    let code_point = code_point as u32;
    let ch = char::from_u32(code_point).ok_or_else(|| {
        FormatError::InvalidFormat(format!(
            "character format code point `{code_point}` is not a Unicode scalar value",
        ))
    })?;
    Ok(ch.to_string())
}

fn format_si(value: f64, precision: usize) -> (String, &'static str) {
    if value == 0.0 {
        let frac = precision.saturating_sub(1);
        return (format!("{value:.frac$}"), "");
    }

    let (coefficient, exponent) = decompose_to_coefficient_and_exponent(value, precision);
    let prefix_exponent = (exponent.div_euclid(3)).clamp(-8, 8);
    let decimal_index = exponent - prefix_exponent * 3 + 1;
    let coefficient_len = coefficient.len() as i32;
    let text = if decimal_index == coefficient_len {
        coefficient
    } else if decimal_index > coefficient_len {
        format!(
            "{}{}",
            coefficient,
            "0".repeat((decimal_index - coefficient_len) as usize)
        )
    } else if decimal_index > 0 {
        let decimal_index = decimal_index as usize;
        format!(
            "{}.{}",
            &coefficient[..decimal_index],
            &coefficient[decimal_index..]
        )
    } else {
        let zeros = "0".repeat((-decimal_index) as usize);
        format!("0.{zeros}{coefficient}")
    };
    (text, SI_PREFIXES[(prefix_exponent + 8) as usize])
}

fn decompose_to_coefficient_and_exponent(value: f64, precision: usize) -> (String, i32) {
    let frac = precision.saturating_sub(1);
    let formatted = format!("{value:.frac$e}");
    let Some((mantissa, exponent)) = split_exponent(&formatted) else {
        return (formatted, 0);
    };
    (mantissa.replace('.', ""), exponent)
}

fn localize_number_body(
    raw_body: &str,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
) -> String {
    let (mantissa, exponent) = raw_body
        .split_once('e')
        .map(|(m, e)| (m, Some(e)))
        .unwrap_or((raw_body, None));
    let (integer, fraction) = mantissa
        .split_once('.')
        .map(|(i, f)| (i, Some(f)))
        .unwrap_or((mantissa, None));
    let mut output = if format.group {
        group_integer(integer, locale)
    } else {
        integer.to_string()
    };
    if let Some(fraction) = fraction {
        output.push_str(&locale.decimal);
        output.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        output.push('e');
        output.push_str(exponent);
    }
    output
}

fn group_integer(integer: &str, locale: &ResolvedNumberLocale) -> String {
    let chars: Vec<char> = integer.chars().collect();
    if chars.len() < locale.grouping.min_grouping_digits + locale.grouping.primary {
        return integer.to_string();
    }

    let mut groups = Vec::new();
    let mut end = chars.len();
    let mut group_size = locale.grouping.primary;
    while end > 0 {
        let start = end.saturating_sub(group_size);
        groups.push(chars[start..end].iter().collect::<String>());
        end = start;
        group_size = locale.grouping.secondary.unwrap_or(locale.grouping.primary);
    }
    groups.reverse();
    groups.join(&locale.group)
}

fn apply_sign_and_padding(
    is_negative: bool,
    body: &str,
    prefix: &str,
    suffix: &str,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
) -> String {
    let sign_prefix = if is_negative {
        if format.sign == SignPolicy::Parentheses {
            "("
        } else {
            &locale.minus
        }
    } else {
        match format.sign {
            SignPolicy::Plus => &locale.plus,
            SignPolicy::Space => " ",
            _ => "",
        }
    };
    let sign_suffix = if is_negative && format.sign == SignPolicy::Parentheses {
        ")"
    } else {
        ""
    };

    apply_affixes_and_padding(sign_prefix, prefix, body, suffix, sign_suffix, format)
}

fn apply_affixes_and_padding(
    sign_prefix: &str,
    prefix: &str,
    body: &str,
    suffix: &str,
    sign_suffix: &str,
    format: &ResolvedNumberFormat,
) -> String {
    let content = format!("{sign_prefix}{prefix}{body}{suffix}{sign_suffix}");
    let Some(width) = format.width else {
        return content;
    };
    let len = content.chars().count();
    if len >= width {
        return content;
    }
    let padding_len = width - len;
    let padding: String = std::iter::repeat_n(format.fill, padding_len).collect();
    match format.align {
        Align::Left => format!("{content}{padding}"),
        Align::AfterSign => format!("{sign_prefix}{prefix}{padding}{body}{suffix}{sign_suffix}"),
        Align::Center => {
            let left = padding_len / 2;
            let right = padding_len - left;
            let left_padding: String = std::iter::repeat_n(format.fill, left).collect();
            let right_padding: String = std::iter::repeat_n(format.fill, right).collect();
            format!("{left_padding}{content}{right_padding}")
        }
        Align::Right => format!("{padding}{content}"),
    }
}

fn rounds_to_zero(body: &str, locale: &ResolvedNumberLocale) -> bool {
    let ascii = body
        .replace(&locale.group, "")
        .replace(&locale.decimal, ".");
    ascii
        .parse::<f64>()
        .map(|value| value == 0.0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{format_number, NumberFormatContext, NumberFormatOverrides};
    use crate::{
        registry::NumberLocaleRegistry,
        spec::{Align, DigitSpec, FormatType},
        typesetting::{ExponentMarker, NumberTypesetting},
    };

    fn en_us() -> crate::locale::ResolvedNumberLocale {
        NumberLocaleRegistry::with_builtins()
            .resolve("en-US")
            .unwrap()
    }

    #[test]
    fn formats_fixed_with_grouping() {
        let locale = en_us();
        let out = format_number(
            1234.5,
            Some(",.1f"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "1,234.5");
        assert_eq!(out.typesetting, NumberTypesetting::Plain);
    }

    #[test]
    fn returns_parse_errors() {
        let locale = en_us();
        assert!(format_number(
            1.0,
            Some("C[]"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .is_err());
    }

    #[test]
    fn overrides_precision() {
        let locale = en_us();
        let out = format_number(
            1.234,
            Some(".1f"),
            NumberFormatOverrides {
                digit_spec: Some(DigitSpec::Precision(2)),
                ..Default::default()
            },
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "1.23");
    }

    #[test]
    fn exponent_reports_typesetting() {
        let locale = en_us();
        let out = format_number(
            1200.0,
            Some(".1e"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "1.2e+3");
        assert_eq!(
            out.typesetting,
            NumberTypesetting::Exponent {
                mantissa: "1.2".to_string(),
                exponent: 3,
                marker: ExponentMarker::LowerE,
            }
        );
    }

    #[test]
    fn applies_width_and_alignment() {
        let locale = en_us();
        let out = format_number(
            42.0,
            Some(".>6.0f"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "....42");

        let out = format_number(
            42.0,
            Some(".<6.0f"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "42....");
    }

    #[test]
    fn formats_basic_d3_types() {
        let locale = en_us();
        let context = NumberFormatContext::new(&locale);
        let cases = [
            ("d", 42.0, "42"),
            ("b", 3.0, "11"),
            ("#b", 3.0, "0b11"),
            ("o", 8.0, "10"),
            ("x", 255.0, "ff"),
            ("X", 255.0, "FF"),
            (".0%", 0.123, "12%"),
            (".3s", 42e6, "42.0M"),
            (".0s", 1e-6, "1\u{00b5}"),
            (".3s", 999.5, "1.00k"),
            ("n", 1234.5, "1,234.50"),
            ("#.0f", 10.1, "10"),
            ("c", 65.0, "A"),
            (".2g", 1234.5, "1.2e+3"),
            (".2r", 1234.5, "1200"),
            ("p", 0.1234, "12.3400%"),
            (".1p", 0.1234, "10%"),
        ];
        for (spec, value, expected) in cases {
            let out = format_number(value, Some(spec), NumberFormatOverrides::default(), context)
                .unwrap();
            assert_eq!(out.text, expected, "{spec}");
        }
    }

    #[test]
    fn character_type_rejects_invalid_code_points() {
        let locale = en_us();
        let err = format_number(
            0x11_0000 as f64,
            Some("c"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap_err();
        assert!(matches!(err, crate::error::FormatError::InvalidFormat(_)));
    }

    #[test]
    fn custom_locale_changes_symbols() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "test",
                r#"{
                    "base": "en-US",
                    "decimal": ",",
                    "group": ".",
                    "minus": "\u2212",
                    "digits": ["0","1","2","3","4","5","6","7","8","9"]
                }"#,
            )
            .unwrap();
        let locale = registry.resolve("test").unwrap();
        let out = format_number(
            -1234.5,
            Some(",.1f"),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "\u{2212}1.234,5");
    }

    #[test]
    fn applies_normalized_decimal_and_percent_patterns() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "patterns",
                r##"{
                    "base": "en-US",
                    "decimal_pattern": "'~'#,##0 'items';('~'#,##0 'items')",
                    "percent_pattern": "#,##0 percent;minus #,##0 percent"
                }"##,
            )
            .unwrap();
        let locale = registry.resolve("patterns").unwrap();
        let context = NumberFormatContext::new(&locale);

        let out =
            format_number(12.0, Some(".0f"), NumberFormatOverrides::default(), context).unwrap();
        assert_eq!(out.text, "~12 items");

        let out = format_number(
            -12.0,
            Some(".0f"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "(~12 items)");

        let out =
            format_number(0.12, Some(".0%"), NumberFormatOverrides::default(), context).unwrap();
        assert_eq!(out.text, "12 percent");

        let out = format_number(
            -0.12,
            Some(".0%"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "minus 12 percent");
    }

    #[test]
    fn resolves_override_fields() {
        let locale = en_us();
        let out = format_number(
            42.0,
            Some(".1f"),
            NumberFormatOverrides {
                format_type: Some(FormatType::Fixed),
                width: Some(Some(6)),
                align: Some(Some(Align::Left)),
                fill: Some(Some('.')),
                ..Default::default()
            },
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(out.text, "42.0..");
    }

    #[test]
    fn formats_currency_extensions() {
        let locale = en_us();
        let context = NumberFormatContext::new(&locale);

        let out = format_number(
            1234.5,
            Some(",C[USD]"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "$1,234.50");

        let out = format_number(
            1234.5,
            Some(",C[JPY]"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "\u{00a5}1,234");

        let out = format_number(
            1234.5678,
            Some(",C[BHD]"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "BHD1,234.568");

        let out = format_number(
            -1234.5,
            Some("(,.1C[USD]"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "($1,234.5)");

        let out = format_number(
            1234.5,
            Some(",.2C[EUR]"),
            NumberFormatOverrides {
                currency_display: Some(crate::locale::CurrencyDisplay::Code),
                ..Default::default()
            },
            context,
        )
        .unwrap();
        assert_eq!(out.text, "EUR1,234.50");

        let out = format_number(
            1234.5,
            Some(",C"),
            NumberFormatOverrides {
                currency: Some("USD".to_string()),
                ..Default::default()
            },
            context,
        )
        .unwrap();
        assert_eq!(out.text, "$1,234.50");

        let out = format_number(
            1234.5,
            Some(",C[EUR]"),
            NumberFormatOverrides {
                currency: Some("USD".to_string()),
                ..Default::default()
            },
            context,
        )
        .unwrap();
        assert_eq!(out.text, "$1,234.50");

        let out = format_number(
            1234.0,
            Some(",.2~C[USD]"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "$1,234");
    }

    #[test]
    fn formats_compact_extensions() {
        let locale = en_us();
        let context = NumberFormatContext::new(&locale);

        let out = format_number(
            1_200_000.0,
            Some(".2S"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "1.2M");

        let out = format_number(
            1_200_000.0,
            Some(".2L"),
            NumberFormatOverrides::default(),
            context,
        )
        .unwrap();
        assert_eq!(out.text, "1.2 million");
    }

    #[test]
    fn rejects_invalid_extension_combinations() {
        let locale = en_us();
        let context = NumberFormatContext::new(&locale);

        assert!(format_number(1.0, Some("C"), NumberFormatOverrides::default(), context,).is_err());
        assert!(format_number(
            1.0,
            Some("$C[USD]"),
            NumberFormatOverrides::default(),
            context,
        )
        .is_err());
        assert!(
            format_number(1.0, Some("#S"), NumberFormatOverrides::default(), context,).is_err()
        );
    }
}
