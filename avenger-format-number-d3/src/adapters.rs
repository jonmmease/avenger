use crate::{
    decimal,
    format::{resolve_number_format, PreparedNumberFormat, SI_PREFIXES},
    parse_number_spec, DigitSpec, FormatType, NumberFormatError, ResolvedNumberLocale,
};

/// Select precision from a tick step's decimal order using Vega's `formatSpan` rules.
/// `magnitude` is the largest absolute tick, which selects one SI unit for automatic `s`
/// formatting. Explicit precision is preserved.
pub(crate) fn step_format(
    pattern: &PreparedNumberFormat,
    step: f64,
    magnitude: f64,
) -> PreparedNumberFormat {
    let mut prepared = pattern.clone();
    let Some(step_exponent) = decimal::exponent(step) else {
        return prepared;
    };
    if prepared.resolved.digit_spec != DigitSpec::Auto {
        return prepared;
    }
    let kind = prepared.resolved.format_type;
    let precision = match kind {
        Some(FormatType::Si) => fix_si_unit(&mut prepared, magnitude) - step_exponent,
        Some(FormatType::Fixed) => -step_exponent,
        Some(FormatType::Percent) => -step_exponent - 2,
        None
        | Some(
            FormatType::Exponent
            | FormatType::General
            | FormatType::Rounded
            | FormatType::PercentRounded,
        ) => {
            // Vega subtracts the step from the domain's largest magnitude. For ticks
            // [0, step], the largest tick gives zero where the domain gives a value below
            // the step, and neither adds digits.
            let extra = decimal::exponent(magnitude - step)
                .map_or(0, |exponent| (exponent - step_exponent).max(0));
            extra + 1 - i32::from(kind == Some(FormatType::Exponent))
        }
        _ => return prepared,
    };
    prepared.resolved.digit_spec = DigitSpec::Precision(precision.clamp(0, u8::MAX as i32) as u8);
    prepared
}

/// Fix an SI unit from a reference value using D3's formatPrefix rules.
/// The specifier's type is replaced with `f`, so precision counts fraction digits.
/// A zero or non-finite reference selects no prefix.
pub fn prepare_number_prefix_format(
    spec: &str,
    value: f64,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, NumberFormatError> {
    let mut parsed = parse_number_spec(spec)?;
    parsed.format_type = Some(FormatType::Fixed);
    let mut prepared = PreparedNumberFormat::from_resolved(resolve_number_format(parsed), locale);
    fix_si_unit(&mut prepared, value);
    Ok(prepared)
}

/// Fix the SI unit that D3's formatPrefix selects for `value`, with fixed fraction digits.
/// Returns the unit's power of ten. A zero or non-finite value selects no prefix.
fn fix_si_unit(prepared: &mut PreparedNumberFormat, value: f64) -> i32 {
    let exponent = decimal::exponent(value)
        .unwrap_or(0)
        .div_euclid(3)
        .clamp(-8, 8)
        * 3;
    prepared.scale = 10_f64.powi(-exponent);
    prepared.suffix = SI_PREFIXES[(exponent / 3 + 8) as usize].into();
    prepared.resolved.format_type = Some(FormatType::Fixed);
    exponent
}

/// Prepare labels with Vega's automatic precision rules. `None` or an empty specifier uses `,`.
/// Explicit precision disables automatic trimming. Otherwise, trimming precedes localization
/// and padding so custom numerals and field widths are preserved.
pub fn prepare_number_float_format(
    spec: Option<&str>,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, NumberFormatError> {
    let spec = spec.filter(|value| !value.is_empty()).unwrap_or(",");
    let mut prepared = PreparedNumberFormat::new(Some(spec), locale)?;
    apply_float_precision(&mut prepared);
    Ok(prepared)
}

pub(crate) fn apply_float_precision(prepared: &mut PreparedNumberFormat) {
    if prepared.resolved.digit_spec == DigitSpec::Auto {
        prepared.resolved.digit_spec = DigitSpec::Precision(match prepared.resolved.format_type {
            Some(FormatType::Percent) => 10,
            Some(FormatType::Exponent) => 11,
            _ => 12,
        });
        prepared.resolved.trim = true;
    }
}
