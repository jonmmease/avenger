use crate::{
    decimal,
    format::{resolve_number_format, PreparedNumberFormat, SI_PREFIXES},
    parse_number_spec, DigitSpec, FormatError, FormatType, NumberFormatOverrides,
    ResolvedNumberLocale,
};

/// Prepare Vega's formatSpan behavior from the scale domain and requested tick count.
pub fn prepare_number_span_format(
    start: f64,
    stop: f64,
    count: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    let mut prepared = PreparedNumberFormat::new(Some(spec.unwrap_or(",f")), overrides, locale)?;
    let step = tick_step(start, stop, count).abs();
    let value = start.abs().max(stop.abs());
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

/// Prepare D3 formatPrefix with a reference value that fixes the SI unit.
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

/// Prepare automatic floating-point labels, retaining explicit precision when supplied.
/// Trimming precedes localization and padding so custom numerals and field widths are preserved.
pub fn prepare_number_float_format(
    spec: Option<&str>,
    locale: &ResolvedNumberLocale,
) -> Result<PreparedNumberFormat, FormatError> {
    let spec = spec.filter(|value| !value.is_empty()).unwrap_or(",");
    let mut prepared =
        PreparedNumberFormat::new(Some(spec), NumberFormatOverrides::default(), locale)?;
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

fn tick_step(start: f64, stop: f64, count: f64) -> f64 {
    if stop < start {
        return -tick_step(stop, start, count);
    }
    let step = (stop - start) / count.max(0.0);
    let power = step.log10().floor();
    let error = step / 10_f64.powf(power);
    let factor = if error >= 50_f64.sqrt() {
        10.0
    } else if error >= 10_f64.sqrt() {
        5.0
    } else if error >= 2_f64.sqrt() {
        2.0
    } else {
        1.0
    };
    let increment = if power < 0.0 {
        -(10_f64.powf(-power) / factor)
    } else {
        10_f64.powf(power) * factor
    };
    // Compare reconstructed ticks to retain endpoints despite multiplication rounding.
    let (first, last) = if increment < 0.0 {
        let mut first = (start * -increment).round();
        let mut last = (stop * -increment).round();
        if first / -increment < start {
            first += 1.0;
        }
        if last / -increment > stop {
            last -= 1.0;
        }
        (first, last)
    } else {
        let mut first = (start / increment).round();
        let mut last = (stop / increment).round();
        if first * increment < start {
            first += 1.0;
        }
        if last * increment > stop {
            last -= 1.0;
        }
        (first, last)
    };
    if last < first && (0.5..2.0).contains(&count) {
        return tick_step(start, stop, count * 2.0);
    }
    if increment < 0.0 {
        1.0 / -increment
    } else {
        increment
    }
}
