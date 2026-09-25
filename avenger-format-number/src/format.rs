use crate::{
    compact::{self, SharedCompact},
    currency::{self, CurrencyDisplay},
    decimal,
    digits::substitute_digits,
    error::FormatError,
    formatted_number::{FormattedNumber, NumberTypesetting},
    locale::ResolvedNumberLocale,
    parser::parse_number_spec,
    spec::{Align, DigitSpec, FormatType, NumberFormatSpec, SignPolicy, Symbol},
};

/// Prefixes for powers of ten from -24 to 24 in steps of three.
pub(crate) const SI_PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "µ", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

/// A parsed number format and locale reusable across values.
/// The retained locale is unaffected by later registry updates.
#[derive(Debug, Clone)]
pub struct PreparedNumberFormat {
    pub(crate) resolved: ResolvedNumberFormat,
    locale: ResolvedNumberLocale,
    /// Multiplier applied before formatting to fix an SI unit across values.
    pub(crate) scale: f64,
    /// SI prefix appended to labels by the step and prefix adapters.
    pub(crate) suffix: String,
    pub(crate) compact: Option<SharedCompact>,
}
impl PreparedNumberFormat {
    /// Parse and resolve a number format before formatting a batch.
    /// `None` is equivalent to an empty specifier, which starts with D3's `.12~g` defaults.
    pub fn new(
        spec: Option<&str>,
        overrides: NumberFormatOverrides,
        locale: &ResolvedNumberLocale,
    ) -> Result<Self, FormatError> {
        Ok(Self::from_resolved(
            resolve_number_format(parse_number_spec(spec.unwrap_or(""))?, overrides)?,
            locale,
        ))
    }

    /// Build an unscaled formatter from resolved fields.
    pub(crate) fn from_resolved(
        resolved: ResolvedNumberFormat,
        locale: &ResolvedNumberLocale,
    ) -> Self {
        Self {
            resolved,
            locale: locale.clone(),
            scale: 1.0,
            suffix: String::new(),
            compact: None,
        }
    }

    /// Format a binary64 value with the prepared locale and specifier.
    pub fn format(&self, value: f64) -> FormattedNumber {
        render_number(
            value * self.scale,
            &self.resolved,
            &self.locale,
            &self.suffix,
            self.compact.as_ref(),
        )
    }
}

/// Field overrides applied to the parsed specifier before defaults and zero padding.
/// `None` preserves a parsed field. For nullable fields, `Some(None)` clears it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NumberFormatOverrides {
    pub format_type: Option<FormatType>,
    /// Replace parsed precision, or use `Some(DigitSpec::Auto)` to restore automatic selection.
    pub digit_spec: Option<DigitSpec>,
    /// Enable or disable locale grouping, including the grouping implied by `n`.
    pub group: Option<bool>,
    /// Enable or disable removal of trailing fractional zeros before localization.
    pub trim: Option<bool>,
    pub sign: Option<SignPolicy>,
    /// `Some(None)` removes currency affixes or a radix prefix requested by the specifier.
    pub symbol: Option<Option<Symbol>>,
    /// Minimum UTF-16 field width before numeral substitution, or `Some(None)` for no padding.
    pub width: Option<Option<usize>>,
    /// `Some(None)` restores a space, subject to the final zero-padding flag.
    pub fill: Option<Option<char>>,
    /// `Some(None)` restores right alignment, subject to the final zero-padding flag.
    pub align: Option<Option<Align>>,
    /// Override the zero-padding flag. An explicit `0=` fill and alignment still take effect.
    pub zero: Option<bool>,
    /// Uppercase currency code for type `C`.
    pub currency: Option<String>,
    /// Symbol or code presentation for type `C`.
    pub currency_display: Option<CurrencyDisplay>,
}

impl NumberFormatOverrides {
    /// Set the currency code for type `C`.
    pub fn with_currency(mut self, currency: impl Into<String>) -> Self {
        self.currency = Some(currency.into());
        self
    }

    /// Set D3 precision, whose meaning depends on the format type.
    pub fn with_precision(mut self, precision: u8) -> Self {
        self.digit_spec = Some(DigitSpec::Precision(precision));
        self
    }
}

/// Merged format fields with defaults applied, retaining automatic precision for the adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedNumberFormat {
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

/// Format one value without retaining a prepared formatter.
pub fn format_number(
    value: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    locale: &ResolvedNumberLocale,
) -> Result<FormattedNumber, FormatError> {
    Ok(PreparedNumberFormat::new(spec, overrides, locale)?.format(value))
}

/// Merge overrides and defaults, then apply the zero-padding flag to fill and alignment.
pub(crate) fn resolve_number_format(
    spec: NumberFormatSpec,
    overrides: NumberFormatOverrides,
) -> Result<ResolvedNumberFormat, FormatError> {
    let mut fill = overrides.fill.unwrap_or(spec.fill).unwrap_or(' ');
    let mut align = overrides
        .align
        .unwrap_or(spec.align)
        .unwrap_or(Align::Right);
    let format_type = overrides.format_type.or(spec.format_type);
    let sign = overrides.sign.or(spec.sign).unwrap_or(SignPolicy::Minus);
    let symbol = overrides.symbol.unwrap_or(spec.symbol);
    let width = overrides.width.unwrap_or(spec.width);
    let group = overrides
        .group
        .or(spec.group)
        .unwrap_or(format_type == Some(FormatType::LocaleDefault));
    let trim = overrides
        .trim
        .or(spec.trim)
        .unwrap_or(format_type.is_none());
    let zero = overrides.zero.unwrap_or(spec.zero);

    if zero {
        fill = '0';
        align = Align::AfterSign;
    }

    let mut digit_spec = overrides
        .digit_spec
        .unwrap_or_else(|| spec.precision.map(DigitSpec::Precision).unwrap_or_default());

    if matches!(
        format_type,
        Some(FormatType::CompactShort | FormatType::CompactLong)
    ) && symbol == Some(Symbol::Alternate)
    {
        return Err(FormatError::InvalidFormat(
            "`#` cannot be combined with compact formats".into(),
        ));
    }
    let currency = overrides.currency.or(spec.currency);
    let currency_display = overrides.currency_display.unwrap_or_default();
    if format_type == Some(FormatType::Currency) {
        let code = currency
            .as_deref()
            .ok_or(FormatError::MissingCurrencyCode)?;
        let digits = currency::fraction_digits(code)
            .ok_or_else(|| FormatError::InvalidCurrencyCode(code.into()))?;
        if symbol.is_some() {
            return Err(FormatError::InvalidFormat(
                "symbols cannot be combined with currency type `C`".into(),
            ));
        }
        if digit_spec == DigitSpec::Auto {
            digit_spec = DigitSpec::Precision(digits);
        }
    } else if currency.is_some() || overrides.currency_display.is_some() {
        return Err(FormatError::InvalidFormat(
            "currency options require type `C`".into(),
        ));
    }
    Ok(ResolvedNumberFormat {
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
    })
}

/// Derive localized text and exponent parts from the same rounded numeric body.
fn render_number(
    value: f64,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
    extra_suffix: &str,
    shared_compact: Option<&SharedCompact>,
) -> FormattedNumber {
    let kind = format.format_type.unwrap_or(FormatType::General);
    let default_precision = if format.format_type.is_none() { 12 } else { 6 };
    let precision = match format.digit_spec {
        DigitSpec::Auto => default_precision,
        DigitSpec::Precision(p) => p as usize,
    };
    let significant = precision.clamp(1, 21);
    let fraction = precision.min(20);
    let mut prefix = String::new();
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
                let (body, before, after) =
                    compact::render(magnitude, format, locale, shared_compact);
                prefix.push_str(&before);
                suffix.insert_str(0, &after);
                body
            }
            FormatType::Currency => decimal::fixed(magnitude, fraction),
            FormatType::Character => unreachable!(),
        }
    };
    if format.trim && !matches!(kind, FormatType::CompactShort | FormatType::CompactLong) {
        raw = trim_number_text(&raw);
    }
    let negative = value.is_sign_negative()
        && !value.is_nan()
        && (raw.parse::<f64>().ok() != Some(0.0) || format.sign == SignPolicy::Plus);
    let parentheses =
        negative && format.sign == SignPolicy::Parentheses && kind != FormatType::Currency;
    if kind == FormatType::Currency {
        let (before, after) = currency::affixes(format, locale, negative, &raw);
        prefix.push_str(&before);
        suffix.insert_str(0, &after);
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
        if parentheses {
            suffix.push(')');
        }
    }
    let typesetting = if matches!(
        kind,
        FormatType::Exponent | FormatType::Fixed | FormatType::General | FormatType::LocaleDefault
    ) && value.is_finite()
        && locale.numerals.is_none()
        && extra_suffix.is_empty()
        && format.symbol.is_none()
        && suffix.is_empty()
        && format.width.is_none()
        && !parentheses
    {
        if let Some((mantissa, exponent)) = raw
            .split_once('e')
            .and_then(|(m, e)| e.parse::<i32>().ok().map(|e| (m, e)))
        {
            NumberTypesetting::Exponent {
                mantissa: format!("{}{}", prefix, mantissa.replace('.', &locale.decimal)),
                exponent,
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
            | FormatType::CompactShort
            | FormatType::CompactLong
            | FormatType::Currency
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

/// Group and pad the numeric parts before substituting locale numerals.
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

/// Apply cyclic group sizes from right to left.
/// A width budget limits zero-padded grouping, counting each separator as one unit as D3 does.
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

/// Round to significant digits and select an SI prefix from the rounded exponent.
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

/// Remove trailing fractional zeros and an empty decimal point from unlocalized text.
/// An exponent suffix is preserved.
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
