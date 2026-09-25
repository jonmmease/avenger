use crate::{
    decimal,
    format::{resolve_number_format, PreparedNumberFormat, SI_PREFIXES},
    parse_number_spec, DigitSpec, FormatError, FormatType, NumberFormatOverrides,
    ResolvedNumberLocale,
};

/// Select precision from a supplied step's decimal order using D3's rules.
/// `reference_value` is typically the value with the largest magnitude to format.
/// Both arguments use absolute values. `None` uses `,f`. Explicit precision is preserved.
/// Automatic `s` formatting selects one SI unit from the reference value.
/// A zero or non-finite step leaves precision at the format default.
pub fn prepare_number_step_format(
    step: f64,
    reference_value: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    let mut prepared = PreparedNumberFormat::new(Some(spec.unwrap_or(",f")), overrides, locale)?;
    let step = step.abs();
    let value = reference_value.abs();
    if prepared.resolved.digit_spec == DigitSpec::Auto {
        let kind = prepared.resolved.format_type;
        let precision = match kind {
            Some(FormatType::Si) => {
                let e = decimal::exponent(value)
                    .unwrap_or(0)
                    .div_euclid(3)
                    .clamp(-8, 8)
                    * 3;
                let p = decimal::exponent(step).map(|step| (e - step).max(0));
                prepared.scale = 10_f64.powi(-e);
                prepared.suffix = SI_PREFIXES[(e / 3 + 8) as usize].into();
                prepared.resolved.format_type = Some(FormatType::Fixed);
                p
            }
            Some(FormatType::Fixed | FormatType::Percent) => {
                decimal::exponent(step).map(|exponent| {
                    (-exponent).max(0)
                        - if kind == Some(FormatType::Percent) {
                            2
                        } else {
                            0
                        }
                })
            }
            None
            | Some(
                FormatType::Exponent
                | FormatType::General
                | FormatType::Rounded
                | FormatType::PercentRounded,
            ) => decimal::exponent(step).and_then(|step_exponent| {
                decimal::exponent(value - step).map(|value_exponent| {
                    (value_exponent - step_exponent).max(0) + 1
                        - i32::from(kind == Some(FormatType::Exponent))
                })
            }),
            _ => None,
        };
        if let Some(precision) = precision {
            prepared.resolved.digit_spec =
                DigitSpec::Precision(precision.clamp(0, u8::MAX as i32) as u8);
        }
    }
    Ok(prepared)
}

/// Fix an SI unit from a reference value using D3's formatPrefix rules.
/// The specifier's type is replaced with `f`, so precision counts fraction digits.
/// A zero or non-finite reference selects no prefix.
pub fn prepare_number_prefix_format(
    spec: &str,
    value: f64,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    let mut parsed = parse_number_spec(spec)?;
    parsed.format_type = Some(FormatType::Fixed);
    let resolved = resolve_number_format(parsed, NumberFormatOverrides::default());
    let exponent = decimal::exponent(value)
        .unwrap_or(0)
        .div_euclid(3)
        .clamp(-8, 8)
        * 3;
    let mut prepared = PreparedNumberFormat::from_resolved(resolved, locale);
    prepared.scale = 10_f64.powi(-exponent);
    prepared.suffix = SI_PREFIXES[(exponent / 3 + 8) as usize].into();
    Ok(prepared)
}

/// Prepare labels with Vega's automatic precision rules. `None` or an empty specifier uses `,`.
/// Explicit precision disables automatic trimming. Otherwise, trimming precedes localization
/// and padding so custom numerals and field widths are preserved.
pub fn prepare_number_float_format(
    spec: Option<&str>,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    prepare_number_float_format_with_overrides(spec, NumberFormatOverrides::default(), locale)
}

pub(crate) fn prepare_number_float_format_with_overrides(
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    let spec = spec.filter(|value| !value.is_empty()).unwrap_or(",");
    let mut prepared = PreparedNumberFormat::new(Some(spec), overrides, locale)?;
    if prepared.resolved.digit_spec == DigitSpec::Auto {
        prepared.resolved.digit_spec = DigitSpec::Precision(match prepared.resolved.format_type {
            Some(FormatType::Percent) => 10,
            Some(FormatType::Exponent) => 11,
            _ => 12,
        });
        prepared.resolved.trim = true;
    }
    Ok(prepared)
}
