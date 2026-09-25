use crate::{
    decimal,
    format::{trim_number_text, ResolvedNumberFormat},
    DigitSpec, FormatError, FormatType, ResolvedNumberLocale,
};
use serde::{Deserialize, Serialize};

/// A power-of-ten divisor and patterns for the resulting coefficient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactTier {
    /// Decimal exponent in `1..=308`. Tiers are sorted during registration.
    pub exponent: i32,
    /// Pattern selected by the locale's compact plural rule.
    #[serde(default)]
    pub one: Option<String>,
    /// Fallback pattern with exactly one `{0}` coefficient placeholder.
    pub other: String,
    /// Override for a coefficient equal to one, such as French `mille`.
    /// May omit `{0}`. Takes precedence over the plural rule.
    #[serde(default)]
    pub exact_one: Option<String>,
}

/// Rules for the `one` compact pattern. All other categories use `other`.
/// Selection uses the rounded coefficient, including visible fractional zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompactPluralRule {
    /// Use `other` for every coefficient, as in Japanese.
    #[default]
    Other,
    /// Use `one` for 1 with no visible fractional digits, as in English and German.
    IntegerOne,
    /// Use `one` when the integer part is 0 or 1, as in French.
    ZeroOrOneIntegerPart,
}

impl CompactTier {
    fn pattern(&self, body: &str, rule: CompactPluralRule) -> &str {
        if body.parse::<f64>().ok() == Some(1.0) {
            if let Some(pattern) = &self.exact_one {
                return pattern;
            }
        }
        let one = match rule {
            CompactPluralRule::Other => false,
            CompactPluralRule::IntegerOne => body == "1",
            CompactPluralRule::ZeroOrOneIntegerPart => {
                matches!(body.split('.').next(), Some("0" | "1"))
            }
        };
        if one {
            self.one.as_deref().unwrap_or(&self.other)
        } else {
            &self.other
        }
    }
}

/// Fixed tier and optional automatic fraction precision for a supplied step.
#[derive(Debug, Clone)]
pub(crate) struct SharedCompact {
    tier: Option<usize>,
    fraction: Option<usize>,
}

pub(crate) fn validate_tiers(tiers: &mut [CompactTier]) -> Result<(), FormatError> {
    tiers.sort_by_key(|tier| tier.exponent);
    if tiers
        .windows(2)
        .any(|pair| pair[0].exponent == pair[1].exponent)
    {
        return Err(FormatError::InvalidLocaleData(
            "duplicate compact tier exponent".into(),
        ));
    }
    for tier in tiers {
        if !(1..=308).contains(&tier.exponent) {
            return Err(FormatError::InvalidLocaleData(
                "compact tier exponents must be in 1..=308".into(),
            ));
        }
        if std::iter::once(&tier.other)
            .chain(tier.one.iter())
            .any(|p| p.matches("{0}").count() != 1)
            || tier
                .exact_one
                .as_ref()
                .is_some_and(|p| p.matches("{0}").count() > 1)
        {
            return Err(FormatError::InvalidLocaleData("compact patterns require one `{0}` placeholder, except exact-one patterns may omit it".into()));
        }
    }
    Ok(())
}

fn tiers<'a>(format: &ResolvedNumberFormat, locale: &'a ResolvedNumberLocale) -> &'a [CompactTier] {
    if format.format_type == Some(FormatType::CompactShort) {
        &locale.extensions.compact_short
    } else {
        &locale.extensions.compact_long
    }
}

fn select_tier(value: f64, tiers: &[CompactTier]) -> Option<usize> {
    value
        .is_finite()
        .then(|| tiers.iter().rposition(|t| value >= 10_f64.powi(t.exponent)))
        .flatten()
}

pub(crate) fn prepare_shared(
    step: f64,
    reference: f64,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
) -> SharedCompact {
    select_rounded(reference, tiers(format, locale), format, Some(step)).0
}

/// Promote through rounded boundaries, including the boundary before the first tier.
fn select_rounded(
    value: f64,
    tiers: &[CompactTier],
    format: &ResolvedNumberFormat,
    step: Option<f64>,
) -> (SharedCompact, String) {
    let mut tier = select_tier(value, tiers);
    loop {
        let exponent = tier.map_or(0, |i| tiers[i].exponent);
        let fraction = if format.digit_spec == DigitSpec::Auto {
            step.and_then(decimal::exponent)
                .map(|s| (exponent - s).clamp(0, 20) as usize)
        } else {
            None
        };
        let raw = body(value / 10_f64.powi(exponent), format, fraction);
        let next = tier.map_or(0, |i| i + 1);
        if value.is_finite()
            && tiers.get(next).is_some_and(|t| {
                raw.parse::<f64>().unwrap_or(0.0) >= 10_f64.powi(t.exponent - exponent)
            })
        {
            tier = Some(next);
        } else {
            return (SharedCompact { tier, fraction }, raw);
        }
    }
}

fn body(value: f64, format: &ResolvedNumberFormat, fraction: Option<usize>) -> String {
    let raw = if let Some(p) = fraction {
        decimal::fixed(value, p)
    } else {
        let p = match format.digit_spec {
            DigitSpec::Auto => 6,
            DigitSpec::Precision(p) => p.clamp(1, 21) as usize,
        };
        decimal::rounded(value, p)
    };
    if format.trim {
        trim_number_text(&raw)
    } else {
        raw
    }
}

/// Round before promoting a tier, then select affixes from the final coefficient.
pub(crate) fn render(
    value: f64,
    format: &ResolvedNumberFormat,
    locale: &ResolvedNumberLocale,
    shared: Option<&SharedCompact>,
) -> (String, String, String) {
    let tiers = tiers(format, locale);
    let (selected, raw) = if let Some(shared) = shared {
        let divisor = shared.tier.map_or(1.0, |i| 10_f64.powi(tiers[i].exponent));
        (
            shared.clone(),
            body(value / divisor, format, shared.fraction),
        )
    } else {
        select_rounded(value, tiers, format, None)
    };
    if let Some(tier) = selected.tier.filter(|_| value.is_finite()) {
        let pattern = tiers[tier].pattern(&raw, locale.extensions.compact_plural_rule);
        if let Some((before, after)) = pattern.split_once("{0}") {
            (raw, before.into(), after.into())
        } else {
            (String::new(), pattern.into(), String::new())
        }
    } else {
        (raw, String::new(), String::new())
    }
}
