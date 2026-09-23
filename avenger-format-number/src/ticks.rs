use crate::{
    decimal,
    format::{resolve_number_format, PreparedNumberFormat, SI_PREFIXES},
    parse_number_spec, DigitSpec, FormatError, FormatType, NumberFormatContext,
    NumberFormatOverrides,
};

/// A prepared number formatter with a scale shared by an entire tick set.
pub type PreparedNumberTickFormat = PreparedNumberFormat;

/// Prepare labels from an existing tick set using its extent and interval count.
pub fn prepare_number_tick_format(
    values: &[f64],
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberTickFormat, FormatError> {
    let mut values: Vec<_> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    values.sort_by(f64::total_cmp);
    values.dedup();
    let start = values.first().copied().unwrap_or(0.0);
    let stop = values.last().copied().unwrap_or(start);
    prepare_number_span_format(
        start,
        stop,
        values.len().saturating_sub(1) as f64,
        spec,
        overrides,
        context,
    )
}

/// Prepare Vega's formatSpan behavior from the scale domain and requested tick count.
pub fn prepare_number_span_format(
    start: f64,
    stop: f64,
    count: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberFormat, FormatError> {
    let mut prepared = PreparedNumberFormat::new(Some(spec.unwrap_or(",f")), overrides, context)?;
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
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberFormat, FormatError> {
    let mut parsed = parse_number_spec(spec)?;
    parsed.format_type = Some(FormatType::Fixed);
    let resolved = resolve_number_format(parsed, NumberFormatOverrides::default())?;
    let exponent = decimal::exponent(value)
        .unwrap_or(0)
        .div_euclid(3)
        .clamp(-8, 8)
        * 3;
    let mut prepared =
        PreparedNumberFormat::new(Some("f"), NumberFormatOverrides::default(), context)?;
    prepared.resolved = resolved;
    prepared.scale = 10_f64.powi(-exponent);
    prepared.suffix = SI_PREFIXES[(exponent / 3 + 8) as usize].into();
    Ok(prepared)
}

/// Prepare Vega's formatFloat, retaining explicit precision when supplied.
pub fn prepare_number_float_format(
    spec: Option<&str>,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberFormat, FormatError> {
    let spec = spec.filter(|value| !value.is_empty()).unwrap_or(",");
    let mut prepared =
        PreparedNumberFormat::new(Some(spec), NumberFormatOverrides::default(), context)?;
    if prepared.resolved.digit_spec == DigitSpec::Auto {
        prepared.resolved.digit_spec = DigitSpec::Precision(match prepared.resolved.format_type {
            Some(FormatType::Percent) => 10,
            Some(FormatType::Exponent) => 11,
            _ => 12,
        });
        prepared.trim_float =
            PreparedNumberFormat::new(Some(".1f"), NumberFormatOverrides::default(), context)?
                .format(1.0)
                .text
                .encode_utf16()
                .nth(1);
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
    let (first, last) = if increment < 0.0 {
        ((start * -increment).ceil(), (stop * -increment).floor())
    } else {
        ((start / increment).ceil(), (stop / increment).floor())
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
