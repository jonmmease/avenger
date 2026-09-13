use crate::{
    compact::CompactTier,
    currency::{currency_display_text, currency_metadata, validate_currency_code},
    decimal,
    digits::substitute_digits,
    error::FormatError,
    locale::{CurrencyDisplay, ResolvedNumberLocale},
    parser::parse_number_spec,
    spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol},
    typesetting::{ExponentMarker, FormattedNumber, NumberTypesetting},
};

pub(crate) const SI_PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "µ", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

/// A resolved number locale owned by the caller.
#[derive(Debug, Clone, Copy)]
pub struct NumberFormatContext<'a> {
    pub locale: &'a ResolvedNumberLocale,
}
impl<'a> NumberFormatContext<'a> {
    /// Use a locale already resolved at context construction.
    pub fn new(locale: &'a ResolvedNumberLocale) -> Self {
        Self { locale }
    }
}

/// A parsed number format and locale reusable across values.
#[derive(Debug, Clone)]
pub struct PreparedNumberFormat {
    pub(crate) resolved: ResolvedNumberFormat,
    pub(crate) locale: ResolvedNumberLocale,
    pub(crate) scale: f64,
    pub(crate) prefix: String,
    pub(crate) suffix: String,
    pub(crate) trim_float: Option<u16>,
}
impl PreparedNumberFormat {
    /// Parse and resolve a number format before formatting a batch.
    pub fn new(
        spec: Option<&str>,
        overrides: NumberFormatOverrides,
        context: NumberFormatContext<'_>,
    ) -> Result<Self, FormatError> {
        Ok(Self {
            resolved: resolve_number_format(parse_number_spec(spec.unwrap_or(""))?, overrides)?,
            locale: context.locale.clone(),
            scale: 1.0,
            prefix: String::new(),
            suffix: String::new(),
            trim_float: None,
        })
    }
    /// Format a binary64 value with the prepared locale and specifier.
    pub fn format(&self, value: f64) -> FormattedNumber {
        let mut formatted = render_number(
            value * self.scale,
            &self.resolved,
            &self.locale,
            &self.prefix,
            &self.suffix,
        );
        if let Some(decimal) = self.trim_float {
            formatted.text = trim_float(&formatted.text, decimal);
            if let NumberTypesetting::Exponent { mantissa, .. } = &mut formatted.typesetting {
                if let Some((text, _)) = formatted.text.rsplit_once('e') {
                    *mantissa = text.into();
                } else {
                    formatted.typesetting = NumberTypesetting::Plain;
                }
            }
        }
        formatted
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
    Ok(PreparedNumberFormat::new(spec, overrides, context)?.format(value))
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
    let trim = overrides
        .trim
        .or(spec.trim)
        .unwrap_or(overrides.format_type.or(spec.format_type).is_none());
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

fn render_number(
    value: f64,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
    extra_prefix: &str,
    extra_suffix: &str,
) -> FormattedNumber {
    let kind = format.format_type.unwrap_or(FormatType::General);
    let default_precision = if format.format_type.is_none() { 12 } else { 6 };
    let precision = match format.digit_spec {
        DigitSpec::Auto => default_precision,
        DigitSpec::Precision(p) | DigitSpec::Fraction(p) | DigitSpec::Significant(p) => p as usize,
    };
    let significant = precision.clamp(1, 21);
    let fraction = precision.min(20);
    let mut prefix = extra_prefix.to_owned();
    let mut suffix = String::new();
    match format.symbol {
        Some(Symbol::CurrencyCompat) => {
            prefix.push_str(&locale.currency[0]);
            suffix.push_str(&locale.currency[1]);
        }
        Some(Symbol::Alternate) => match kind {
            FormatType::Binary => prefix.push_str("0b"),
            FormatType::Octal => prefix.push_str("0o"),
            FormatType::HexLower | FormatType::HexUpper => prefix.push_str("0x"),
            _ => {}
        },
        _ => {}
    }
    if format.symbol != Some(Symbol::CurrencyCompat)
        && matches!(kind, FormatType::Percent | FormatType::PercentRounded)
    {
        suffix.push_str(&locale.percent);
    }
    suffix.push_str(extra_suffix);
    if kind == FormatType::Character {
        suffix.insert_str(0, &decimal::shortest(value));
        return FormattedNumber::plain(assemble("", &prefix, &suffix, format, locale));
    }
    let magnitude = value.abs();
    let mut raw = if value.is_nan() {
        locale.nan.clone()
    } else {
        match kind {
            FormatType::Exponent => decimal::exponential(magnitude, fraction),
            FormatType::Fixed => decimal::fixed(magnitude, fraction),
            FormatType::Percent => decimal::fixed(magnitude * 100.0, fraction),
            FormatType::General | FormatType::LocaleDefault => {
                decimal::precision(magnitude, significant)
            }
            FormatType::Rounded => decimal::rounded(magnitude, significant),
            FormatType::PercentRounded => decimal::rounded(magnitude * 100.0, significant),
            FormatType::Si => {
                let (body, si) = format_si(magnitude, significant);
                suffix.insert_str(0, si);
                body
            }
            FormatType::Binary => decimal::integer(magnitude, 2, false),
            FormatType::Octal => decimal::integer(magnitude, 8, false),
            FormatType::DecimalInteger => decimal::integer(magnitude, 10, false),
            FormatType::HexLower => decimal::integer(magnitude, 16, false),
            FormatType::HexUpper => decimal::integer(magnitude, 16, true),
            FormatType::CompactShort | FormatType::CompactLong => {
                let tiers = if kind == FormatType::CompactShort {
                    &locale.extensions.compact_short
                } else {
                    &locale.extensions.compact_long
                };
                let mut tier = select_compact_tier(magnitude, tiers);
                let mut body = if let Some(selected) = tier {
                    compact_body(
                        magnitude / 10_f64.powi(selected.exponent),
                        format,
                        significant,
                    )
                } else {
                    compact_body(magnitude, format, significant)
                };
                if let Some(selected) = tier {
                    if let Some(next) = tiers.iter().find(|next| next.exponent > selected.exponent)
                    {
                        if body.parse::<f64>().unwrap_or(0.0)
                            >= 10_f64.powi(next.exponent - selected.exponent)
                        {
                            tier = Some(next);
                            body = compact_body(
                                magnitude / 10_f64.powi(next.exponent),
                                format,
                                significant,
                            );
                        }
                    }
                }
                if let Some(tier) = tier {
                    let pattern = tier.pattern_for_value(&body);
                    let (before, after) = pattern
                        .split_once("{0}")
                        .expect("validated compact pattern");
                    prefix.push_str(before);
                    suffix.insert_str(0, after);
                }
                body
            }
            FormatType::Currency => {
                let code = format
                    .currency
                    .as_deref()
                    .expect("validated currency format");
                let metadata = currency_metadata(code).expect("validated currency code");
                let precision = if format.digit_spec == DigitSpec::Auto {
                    metadata.default_fraction_digits as usize
                } else {
                    fraction
                };
                decimal::fixed(magnitude, precision)
            }
            FormatType::Character => unreachable!(),
        }
    };
    if format.trim {
        raw = trim_number_text(&raw);
    }
    let negative = value.is_sign_negative()
        && !value.is_nan()
        && (raw.parse::<f64>().ok() != Some(0.0) || format.sign == SignPolicy::Plus);
    let mut parentheses = negative && format.sign == SignPolicy::Parentheses;
    if kind == FormatType::Currency {
        let currency = currency_display_text(
            locale,
            format
                .currency
                .as_deref()
                .expect("validated currency format"),
            format.currency_display,
        );
        let pattern = if parentheses {
            &locale.extensions.currency.accounting
        } else {
            &locale.extensions.currency.standard
        };
        let (before, after) = if negative {
            (&pattern.negative_prefix, &pattern.negative_suffix)
        } else {
            (&pattern.positive_prefix, &pattern.positive_suffix)
        };
        prefix.push_str(&before.replace('¤', &currency).replace('-', &locale.minus));
        suffix.insert_str(0, &after.replace('¤', &currency));
        if !negative {
            match format.sign {
                SignPolicy::Plus => prefix.insert(0, '+'),
                SignPolicy::Space => prefix.insert(0, ' '),
                _ => {}
            }
        }
        parentheses = false;
    } else {
        prefix.insert_str(
            0,
            if negative {
                if parentheses {
                    "("
                } else {
                    &locale.minus
                }
            } else {
                match format.sign {
                    SignPolicy::Plus => "+",
                    SignPolicy::Space => " ",
                    _ => "",
                }
            },
        );
    }
    if parentheses {
        suffix.push(')');
    }
    let typesetting = if value.is_finite()
        && locale.numerals.is_none()
        && extra_prefix.is_empty()
        && extra_suffix.is_empty()
        && format.symbol.is_none()
        && suffix.is_empty()
        && kind != FormatType::Currency
        && format.width.is_none()
        && !parentheses
    {
        if let Some((mantissa, exponent)) = raw
            .split_once('e')
            .and_then(|(m, e)| e.parse::<i32>().ok().map(|e| (m, e)))
        {
            NumberTypesetting::Exponent {
                mantissa: substitute_digits(
                    &format!("{}{}", prefix, mantissa.replace('.', &locale.decimal)),
                    locale,
                ),
                exponent,
                marker: ExponentMarker::LowerE,
            }
        } else {
            NumberTypesetting::Plain
        }
    } else {
        NumberTypesetting::Plain
    };
    let split_suffix = matches!(
        kind,
        FormatType::DecimalInteger
            | FormatType::Exponent
            | FormatType::Fixed
            | FormatType::General
            | FormatType::LocaleDefault
            | FormatType::Rounded
            | FormatType::Percent
            | FormatType::PercentRounded
            | FormatType::Si
            | FormatType::Currency
            | FormatType::CompactShort
            | FormatType::CompactLong
    );
    let integer = if split_suffix {
        let index = raw
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(raw.len());
        let tail = &raw[index..];
        if let Some(fraction) = tail.strip_prefix('.') {
            suffix.insert_str(0, &format!("{}{fraction}", locale.decimal));
        } else {
            suffix.insert_str(0, tail);
        }
        &raw[..index]
    } else {
        &raw
    };
    FormattedNumber {
        text: assemble(integer, &prefix, &suffix, format, locale),
        typesetting,
    }
}

fn compact_body(value: f64, format: &ResolvedNumberFormat, precision: usize) -> String {
    let body = match format.digit_spec {
        DigitSpec::Fraction(p) => decimal::fixed(value, p.min(20) as usize),
        _ => decimal::rounded(value, precision),
    };
    if format.trim {
        trim_number_text(&body)
    } else {
        body
    }
}

fn assemble(
    integer: &str,
    prefix: &str,
    suffix: &str,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
) -> String {
    let zero = format.fill == '0' && format.align == Align::AfterSign;
    let mut value = if format.group && !zero {
        group(integer, None, locale)
    } else {
        integer.to_owned()
    };
    let width = format.width.unwrap_or(0);
    let length = prefix.encode_utf16().count()
        + value.encode_utf16().count()
        + suffix.encode_utf16().count();
    let mut padding = format.fill.to_string().repeat(width.saturating_sub(length));
    if format.group && zero {
        value = group(
            &format!("{padding}{value}"),
            (!padding.is_empty()).then_some(width.saturating_sub(suffix.encode_utf16().count())),
            locale,
        );
        padding.clear();
    }
    let text = match format.align {
        Align::Left => format!("{prefix}{value}{suffix}{padding}"),
        Align::AfterSign => format!("{prefix}{padding}{value}{suffix}"),
        Align::Center => {
            let middle = padding
                .char_indices()
                .nth(padding.chars().count() / 2)
                .map_or(padding.len(), |(index, _)| index);
            format!(
                "{}{prefix}{value}{suffix}{}",
                &padding[..middle],
                &padding[middle..]
            )
        }
        Align::Right => format!("{padding}{prefix}{value}{suffix}"),
    };
    substitute_digits(&text, locale)
}

fn group(value: &str, width: Option<usize>, locale: &ResolvedNumberLocale) -> String {
    if locale.grouping.is_empty() {
        return value.to_owned();
    }
    let chars: Vec<_> = value.chars().collect();
    let mut end = chars.len();
    let mut groups = Vec::new();
    let mut length = 0;
    let mut index = 0;
    while end > 0 {
        let mut size = locale.grouping[index % locale.grouping.len()];
        if let Some(width) = width {
            if length + size + 1 > width {
                size = width.saturating_sub(length).max(1);
            }
        }
        let start = end.saturating_sub(size);
        groups.push(chars[start..end].iter().collect::<String>());
        end = start;
        length += size + 1;
        if width.is_some_and(|width| length > width) {
            break;
        }
        index += 1;
    }
    groups.reverse();
    groups.join(&locale.thousands)
}

fn format_si(value: f64, precision: usize) -> (String, &'static str) {
    if value == 0.0 || !value.is_finite() {
        return (decimal::precision(value, precision), "");
    }
    let (digits, exponent) = decimal::parts(value, precision);
    let prefix = exponent.div_euclid(3).clamp(-8, 8) * 3;
    let index = exponent - prefix + 1;
    let body = if index <= 0 {
        let precision = (precision as i32 + index - 1).max(0) as usize;
        let digits = if precision == 0 {
            decimal::shortest_parts(value).0
        } else {
            decimal::parts(value, precision).0
        };
        format!("0.{}{}", "0".repeat((-index) as usize), digits)
    } else {
        decimal::place_decimal(&digits, index)
    };
    (body, SI_PREFIXES[(prefix / 3 + 8) as usize])
}

pub(crate) fn select_compact_tier(value: f64, tiers: &[CompactTier]) -> Option<&CompactTier> {
    if !value.is_finite() {
        return None;
    }
    tiers
        .iter()
        .filter(|tier| value >= 10_f64.powi(tier.exponent))
        .max_by_key(|tier| tier.exponent)
}

pub(crate) fn trim_number_text(text: &str) -> String {
    let (body, exponent) = text
        .split_once('e')
        .map_or((text, None), |(body, exponent)| (body, Some(exponent)));
    let body = if body.contains('.') {
        body.trim_end_matches('0').trim_end_matches('.')
    } else {
        body
    };
    if let Some(exponent) = exponent {
        format!("{body}e{exponent}")
    } else {
        body.to_owned()
    }
}

fn trim_float(text: &str, decimal: u16) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let Some(start) = units.iter().position(|ch| *ch == decimal) else {
        return text.to_owned();
    };
    let Some(end) = units
        .iter()
        .rposition(|ch| *ch == b'e' as u16)
        .filter(|index| *index > 0)
        .or_else(|| {
            units
                .iter()
                .enumerate()
                .rfind(|(index, ch)| *index > start && (48..=57).contains(*ch))
                .map(|(index, _)| index + 1)
        })
    else {
        return String::new();
    };
    let mut index = end as isize - 1;
    while index > start as isize {
        if units[index as usize] != b'0' as u16 {
            index += 1;
            break;
        }
        index -= 1;
    }
    let index = if index < 0 {
        (units.len() as isize + index).max(0) as usize
    } else {
        index as usize
    };
    let output: Vec<_> = units[..index]
        .iter()
        .chain(&units[end..])
        .copied()
        .collect();
    String::from_utf16_lossy(&output)
}
