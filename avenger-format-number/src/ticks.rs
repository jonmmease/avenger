use crate::{
    compact::CompactTier,
    error::FormatError,
    format::{
        format_fixed_scaled_with_affixes, format_resolved_number, resolve_number_format,
        select_compact_tier, split_compact_pattern, NumberFormatContext, NumberFormatOverrides,
        ResolvedNumberFormat, SI_PREFIXES,
    },
    locale::ResolvedNumberLocale,
    parser::parse_number_spec,
    spec::{DigitSpec, FormatType},
    typesetting::FormattedNumber,
};

#[derive(Debug, Clone)]
pub struct PreparedNumberTickFormat {
    pub resolved: ResolvedNumberFormat,
    pub tick_overrides: NumberFormatOverrides,
    locked_scale: Option<LockedScale>,
}

impl PreparedNumberTickFormat {
    pub fn format(
        &self,
        value: f64,
        context: NumberFormatContext<'_>,
    ) -> Result<FormattedNumber, FormatError> {
        match &self.locked_scale {
            Some(LockedScale::Si {
                exponent,
                suffix,
                precision,
            }) => format_fixed_scaled_with_affixes(
                value,
                *exponent,
                *precision,
                "",
                suffix,
                &self.resolved,
                context,
            ),
            Some(LockedScale::Compact { tier, precision }) => {
                format_locked_compact(value, tier, *precision, &self.resolved, context)
            }
            None => format_resolved_number(value, &self.resolved, context),
        }
    }
}

#[derive(Debug, Clone)]
enum LockedScale {
    Si {
        exponent: i32,
        suffix: &'static str,
        precision: usize,
    },
    Compact {
        tier: CompactTier,
        precision: usize,
    },
}

pub fn prepare_number_tick_format(
    values: &[f64],
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberTickFormat, FormatError> {
    let parsed = parse_number_spec(spec.unwrap_or(""))?;
    let force_inferred_digits = overrides.digit_spec == Some(DigitSpec::Auto);
    let has_explicit_digits =
        !force_inferred_digits && (parsed.precision.is_some() || overrides.digit_spec.is_some());
    let mut tick_overrides = overrides;
    let mut resolved = resolve_number_format(parsed, tick_overrides.clone())?;
    let effective_type = effective_format_type(&resolved);

    let finite_values = finite_values(values);
    let tick_step = tick_step(&finite_values);
    let max_abs = finite_values
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);

    let mut locked_scale = None;

    if !has_explicit_digits
        && !matches!(
            effective_type,
            FormatType::Si | FormatType::CompactShort | FormatType::CompactLong
        )
    {
        if let Some(precision) = inferred_tick_precision(effective_type, tick_step, max_abs) {
            tick_overrides.digit_spec = Some(DigitSpec::Precision(precision));
            resolved.digit_spec = DigitSpec::Precision(precision);
        }
    }

    match effective_type {
        FormatType::Si => {
            let exponent = locked_si_exponent(max_abs);
            let inferred = inferred_prefix_precision(tick_step, exponent);
            if !has_explicit_digits {
                if let Some(precision) = inferred {
                    tick_overrides.digit_spec = Some(DigitSpec::Precision(precision));
                    resolved.digit_spec = DigitSpec::Precision(precision);
                }
            }
            let precision = locked_fraction_digits(&resolved, inferred);
            locked_scale = Some(LockedScale::Si {
                exponent,
                suffix: si_prefix_for_exponent(exponent),
                precision,
            });
        }
        FormatType::CompactShort | FormatType::CompactLong => {
            if let Some(tier) = locked_compact_tier(max_abs, effective_type, context.locale)? {
                let inferred = inferred_prefix_precision(tick_step, tier.exponent);
                if !has_explicit_digits {
                    if let Some(precision) = inferred {
                        tick_overrides.digit_spec = Some(DigitSpec::Precision(precision));
                        resolved.digit_spec = DigitSpec::Precision(precision);
                    }
                }
                let precision = locked_fraction_digits(&resolved, inferred);
                locked_scale = Some(LockedScale::Compact { tier, precision });
            }
        }
        _ => {}
    }

    Ok(PreparedNumberTickFormat {
        resolved,
        tick_overrides,
        locked_scale,
    })
}

fn format_locked_compact(
    value: f64,
    tier: &CompactTier,
    precision: usize,
    format: &ResolvedNumberFormat,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError> {
    if !value.is_finite() {
        return format_resolved_number(value, format, context);
    }

    let scaled = value.abs() / 10_f64.powi(tier.exponent);
    let mut probe_format = format.clone();
    probe_format.format_type = Some(FormatType::Fixed);
    probe_format.digit_spec = DigitSpec::Fraction(precision as u8);
    let probe =
        format_fixed_scaled_with_affixes(scaled, 0, precision, "", "", &probe_format, context)?;
    let pattern = tier.pattern_for_value(&probe.text);
    let (prefix, suffix) = split_compact_pattern(pattern)?;
    format_fixed_scaled_with_affixes(
        value,
        tier.exponent,
        precision,
        prefix,
        suffix,
        format,
        context,
    )
}

fn effective_format_type(format: &ResolvedNumberFormat) -> FormatType {
    if format.format_type == Some(FormatType::LocaleDefault) {
        FormatType::General
    } else {
        format.format_type.unwrap_or(FormatType::General)
    }
}

fn finite_values(values: &[f64]) -> Vec<f64> {
    values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect()
}

fn tick_step(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted
        .windows(2)
        .filter_map(|pair| {
            let delta = (pair[1] - pair[0]).abs();
            (delta > 0.0 && delta.is_finite()).then_some(delta)
        })
        .min_by(|a, b| a.total_cmp(b))
}

fn inferred_tick_precision(format_type: FormatType, step: Option<f64>, max_abs: f64) -> Option<u8> {
    let step = step?;
    match format_type {
        FormatType::Fixed => precision_fixed(step),
        FormatType::Percent => precision_fixed(step * 100.0),
        FormatType::Exponent => {
            precision_round(step, max_abs).map(|precision| precision.saturating_sub(1))
        }
        FormatType::General
        | FormatType::LocaleDefault
        | FormatType::Rounded
        | FormatType::PercentRounded => precision_round(step, max_abs),
        FormatType::Si | FormatType::CompactShort | FormatType::CompactLong => None,
        _ => None,
    }
}

fn inferred_prefix_precision(step: Option<f64>, scale_exponent: i32) -> Option<u8> {
    let step = step?;
    decimal_precision_exponent(step)
        .map(|step_exponent| (scale_exponent - step_exponent).clamp(0, u8::MAX as i32) as u8)
}

fn precision_fixed(step: f64) -> Option<u8> {
    decimal_precision_exponent(step).map(|exponent| (-exponent).max(0) as u8)
}

fn precision_round(step: f64, max_abs: f64) -> Option<u8> {
    let step_exponent = decimal_precision_exponent(step)?;
    if max_abs <= step.abs() {
        return Some(precision_fixed(step).unwrap_or(0));
    }
    let range_exponent = decimal_precision_exponent(max_abs - step.abs()).unwrap_or(0);
    Some((range_exponent - step_exponent).max(0) as u8 + 1)
}

fn magnitude_exponent(value: f64) -> Option<i32> {
    if !value.is_finite() || value == 0.0 {
        return None;
    }
    let text = format!("{:e}", value.abs());
    let (_, exponent) = text.split_once('e')?;
    exponent.parse::<i32>().ok()
}

fn decimal_precision_exponent(value: f64) -> Option<i32> {
    if !value.is_finite() || value == 0.0 {
        return None;
    }
    let text = format!("{:e}", value.abs());
    let (mantissa, exponent) = text.split_once('e')?;
    let mut digits = mantissa.replace('.', "");
    while digits.ends_with('0') {
        digits.pop();
    }
    let digits_len = digits.len().max(1) as i32;
    let exponent = exponent.parse::<i32>().ok()?;
    Some(exponent - digits_len + 1)
}

fn locked_si_exponent(max_abs: f64) -> i32 {
    let exponent = magnitude_exponent(max_abs).unwrap_or(0);
    exponent.div_euclid(3).clamp(-8, 8) * 3
}

fn si_prefix_for_exponent(exponent: i32) -> &'static str {
    SI_PREFIXES[(exponent / 3 + 8).clamp(0, 16) as usize]
}

fn locked_compact_tier(
    max_abs: f64,
    format_type: FormatType,
    locale: &ResolvedNumberLocale,
) -> Result<Option<CompactTier>, FormatError> {
    if max_abs == 0.0 {
        return Ok(None);
    }

    let tiers = match format_type {
        FormatType::CompactShort => &locale.compact_short,
        FormatType::CompactLong => &locale.compact_long,
        _ => unreachable!("compact tier requested for non-compact format"),
    };
    Ok(select_compact_tier(max_abs, tiers).cloned())
}

fn locked_fraction_digits(format: &ResolvedNumberFormat, inferred: Option<u8>) -> usize {
    match format.digit_spec {
        DigitSpec::Auto => inferred.unwrap_or(0) as usize,
        DigitSpec::Precision(value)
        | DigitSpec::Fraction(value)
        | DigitSpec::Significant(value) => value as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::prepare_number_tick_format;
    use crate::{
        format::{NumberFormatContext, NumberFormatOverrides},
        locale::ResolvedNumberLocale,
    };

    fn context<'a>(locale: &'a ResolvedNumberLocale) -> NumberFormatContext<'a> {
        NumberFormatContext::new(locale)
    }

    #[test]
    fn infers_fixed_precision_from_tick_step() {
        let locale = ResolvedNumberLocale::en_us();
        let prepared = prepare_number_tick_format(
            &[0.0, 0.25, 0.5, 0.75, 1.0],
            Some("f"),
            NumberFormatOverrides::default(),
            context(&locale),
        )
        .unwrap();

        let labels: Vec<_> = [0.0, 0.25, 1.0]
            .into_iter()
            .map(|value| prepared.format(value, context(&locale)).unwrap().text)
            .collect();
        assert_eq!(labels, ["0.00", "0.25", "1.00"]);
    }

    #[test]
    fn locks_si_prefix_across_tick_set() {
        let locale = ResolvedNumberLocale::en_us();
        let prepared = prepare_number_tick_format(
            &[900_000.0, 1_000_000.0, 1_100_000.0],
            Some("s"),
            NumberFormatOverrides::default(),
            context(&locale),
        )
        .unwrap();

        let labels: Vec<_> = [900_000.0, 1_000_000.0, 1_100_000.0]
            .into_iter()
            .map(|value| prepared.format(value, context(&locale)).unwrap().text)
            .collect();
        assert_eq!(labels, ["0.9M", "1.0M", "1.1M"]);
    }

    #[test]
    fn locks_compact_short_tier_across_tick_set() {
        let locale = ResolvedNumberLocale::en_us();
        let prepared = prepare_number_tick_format(
            &[900_000.0, 1_000_000.0, 1_100_000.0],
            Some("S"),
            NumberFormatOverrides::default(),
            context(&locale),
        )
        .unwrap();

        let labels: Vec<_> = [900_000.0, 1_000_000.0, 1_100_000.0]
            .into_iter()
            .map(|value| prepared.format(value, context(&locale)).unwrap().text)
            .collect();
        assert_eq!(labels, ["0.9M", "1.0M", "1.1M"]);
    }

    #[test]
    fn explicit_precision_overrides_inferred_tick_precision() {
        let locale = ResolvedNumberLocale::en_us();
        let prepared = prepare_number_tick_format(
            &[0.0, 0.25, 0.5],
            Some(".1f"),
            NumberFormatOverrides::default(),
            context(&locale),
        )
        .unwrap();

        assert_eq!(prepared.format(0.25, context(&locale)).unwrap().text, "0.2");
    }

    #[test]
    fn digit_auto_override_requests_inferred_tick_precision() {
        let locale = ResolvedNumberLocale::en_us();
        let overrides = NumberFormatOverrides {
            digit_spec: Some(crate::DigitSpec::Auto),
            ..Default::default()
        };
        let prepared = prepare_number_tick_format(
            &[900_000.0, 1_000_000.0, 1_100_000.0],
            Some(".3s"),
            overrides,
            context(&locale),
        )
        .unwrap();

        assert_eq!(
            prepared.format(900_000.0, context(&locale)).unwrap().text,
            "0.9M"
        );
    }

    #[test]
    fn currency_uses_default_fraction_digits_when_precision_is_omitted() {
        let locale = ResolvedNumberLocale::en_us();
        let prepared = prepare_number_tick_format(
            &[1.0, 2.0],
            Some("C[JPY]"),
            NumberFormatOverrides::default(),
            context(&locale),
        )
        .unwrap();

        assert_eq!(
            prepared.format(1234.5, context(&locale)).unwrap().text,
            "\u{00a5}1234"
        );
    }
}
