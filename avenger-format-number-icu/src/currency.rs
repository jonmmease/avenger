use crate::{
    arithmetic::Literal,
    data::{data_error, Context},
    locale::Signs,
    skeleton::{unsupported, Skeleton, Unit},
};
use avenger_format::NumberFormatError;
use fixed_decimal::{Decimal, Sign};
use icu_experimental::{
    dimension::{
        currency::CurrencyType,
        provider::currency::{
            essentials::CurrencyEssentialsV1,
            extended::CurrencyExtendedDataV1,
            fractions::{CurrencyFractionsV1, Rounding},
            no_currency::CurrencyPatternsNoCurrencyV1,
            patterns::CurrencyPatternsDataV1,
            symbols::CurrencySymbolsV1,
        },
    },
    provider::Baked,
};
use icu_pattern::{DoublePlaceholderKey, DoublePlaceholderPattern, PatternItem};
use icu_plurals::PluralRules;
use icu_provider::prelude::*;
use writeable::Writeable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Width {
    Short,
    Narrow,
    FullName,
    Code,
    Hidden,
    Formal,
    Variant,
}
impl Width {
    pub fn parse(stem: &str) -> Self {
        match stem {
            "unit-width-narrow" => Self::Narrow,
            "unit-width-full-name" => Self::FullName,
            "unit-width-iso-code" => Self::Code,
            "unit-width-hidden" => Self::Hidden,
            "unit-width-formal" => Self::Formal,
            "unit-width-variant" => Self::Variant,
            _ => Self::Short,
        }
    }
}

pub(crate) fn resolve_precision(skeleton: &mut Skeleton) -> Result<(), NumberFormatError> {
    if matches!(skeleton.unit, Unit::Currency(_)) {
        if skeleton.notation.is_compact() {
            return Err(unsupported(
                "notation",
                "compact currency formatting is unsupported",
            ));
        }
        if matches!(skeleton.width, Width::Formal | Width::Variant) {
            return Err(unsupported(
                "unit-width",
                "formal and variant currency symbols are unsupported",
            ));
        }
    }
    let needed = skeleton.currency_precision.is_some()
        || (!skeleton.explicit_precision
            && !skeleton.notation.is_compact()
            && matches!(skeleton.unit, Unit::Currency(_)));
    if !needed {
        return Ok(());
    }
    let code = match skeleton.unit {
        Unit::Currency(code) => code,
        _ => "XXX".parse::<CurrencyType>().expect("valid currency code"),
    };
    let data = DataProvider::<CurrencyFractionsV1>::load(&Baked, Default::default())
        .map_err(data_error)?;
    let info = data.payload.get().resolve(code);
    let cash = skeleton.currency_precision.unwrap_or(false);
    let digits = if cash {
        info.cash_digits.unwrap_or(info.digits)
    } else {
        info.digits
    };
    let rounding = if cash {
        info.cash_rounding.unwrap_or(info.rounding)
    } else {
        info.rounding
    };
    let coefficient = match rounding {
        Rounding::R5 => 5,
        Rounding::R20 => 20,
        Rounding::R50 => 50,
        _ => 1,
    };
    skeleton.precision = skeleton
        .precision
        .currency(Literal::parse(&format!("{coefficient}e-{digits}"), 0)?);
    Ok(())
}

#[derive(Debug)]
enum Presentation {
    Symbol {
        patterns: DataPayload<CurrencyEssentialsV1>,
        text: String,
        starts: bool,
        ends: bool,
    },
    Name {
        names: Option<DataPayload<CurrencyExtendedDataV1>>,
        patterns: DataPayload<CurrencyPatternsDataV1>,
        code: String,
    },
    Hidden(DataPayload<CurrencyPatternsNoCurrencyV1>),
}
/// Upstream currency patterns applied to numeric text prepared from the skeleton.
#[derive(Debug)]
pub(crate) struct Currency {
    presentation: Presentation,
    plurals: PluralRules,
    accounting: bool,
}
impl Currency {
    pub fn new(context: &Context, s: &Skeleton, code: CurrencyType) -> Result<Self, DataError> {
        let code_string = code.iso_code().to_string();
        let presentation = match s.width {
            Width::FullName => Presentation::Name {
                names: context
                    .load_attributes::<CurrencyExtendedDataV1, _>(
                        &Baked,
                        DataMarkerAttributes::from_str_or_panic(&code_string),
                    )?
                    .map(|r| r.payload),
                patterns: context.load::<CurrencyPatternsDataV1, _>(&Baked)?,
                code: code_string,
            },
            Width::Hidden => {
                Presentation::Hidden(context.load::<CurrencyPatternsNoCurrencyV1, _>(&Baked)?)
            }
            _ => {
                let width = if s.width == Width::Narrow { "n" } else { "s" };
                let key = format!("{width}/{code_string}");
                let symbol = context
                    .load_attributes::<CurrencySymbolsV1, _>(
                        &Baked,
                        DataMarkerAttributes::from_str_or_panic(&key),
                    )?
                    .map(|r| r.payload);
                let text = if s.width == Width::Code {
                    code_string.clone()
                } else {
                    symbol
                        .as_ref()
                        .map_or(&*code_string, |s| s.get().as_str())
                        .into()
                };
                let (starts, ends) = if matches!(s.width, Width::Short | Width::Narrow) {
                    symbol.as_ref().map_or((true, true), |s| {
                        (s.get().starts_with_letter(), s.get().ends_with_letter())
                    })
                } else {
                    (
                        text.starts_with(char::is_alphabetic),
                        text.ends_with(char::is_alphabetic),
                    )
                };
                Presentation::Symbol {
                    patterns: context.load::<CurrencyEssentialsV1, _>(&Baked)?,
                    text,
                    starts,
                    ends,
                }
            }
        };
        Ok(Self {
            presentation,
            plurals: context.plural_rules()?,
            accounting: s.accounting,
        })
    }

    pub fn render(
        &self,
        body: &str,
        sign: Sign,
        number: Option<&Decimal>,
        exponent: i16,
        signs: &Signs,
    ) -> String {
        let (positive, negative, currency) = match &self.presentation {
            Presentation::Name {
                names,
                patterns,
                code,
            } => {
                // Non-finite values have no plural operands; ICU uses the "other" form.
                let operands = number.map(|n| crate::notation::plural_operands(n, exponent));
                let name = names.as_ref().map_or(&**code, |n| match operands {
                    Some(operands) => n.get().get(operands, &self.plurals),
                    None => n.get().elements.get_default().1,
                });
                let pattern = match operands {
                    Some(operands) => patterns.get().get(operands, &self.plurals),
                    None => patterns.get().elements.get_default().1,
                };
                return signs.apply(&pattern.interpolate((body, name)).write_to_string(), sign);
            }
            Presentation::Hidden(patterns) => {
                let p = patterns.get();
                if self.accounting {
                    (p.get_positive_accounting(), p.get_negative_accounting(), "")
                } else {
                    (p.get_positive(), p.get_negative(), "")
                }
            }
            Presentation::Symbol {
                patterns,
                text,
                starts,
                ends,
            } => {
                let p = patterns.get();
                if self.accounting {
                    (
                        p.get_positive_accounting(*starts, *ends),
                        p.get_negative_accounting(*starts, *ends),
                        &**text,
                    )
                } else {
                    (
                        p.get_positive(*starts, *ends),
                        p.get_negative(*starts, *ends),
                        &**text,
                    )
                }
            }
        };
        if let Some(negative) = negative {
            if sign == Sign::Negative {
                return negative
                    .interpolate((body, currency))
                    .write_to_string()
                    .into_owned();
            }
            let minus = signs.symbol(Sign::Negative);
            if sign == Sign::Positive
                && negative
                    .iter()
                    .any(|item| matches!(item, PatternItem::Literal(l) if l.contains(&minus)))
            {
                return positive_sign(negative, body, currency, &minus, &signs.symbol(sign));
            }
        }
        signs.apply(
            &positive.interpolate((body, currency)).write_to_string(),
            sign,
        )
    }
}

/// Replace a sign in pattern literals without inspecting the formatted number or currency name.
fn positive_sign(
    pattern: &DoublePlaceholderPattern,
    body: &str,
    currency: &str,
    minus: &str,
    plus: &str,
) -> String {
    let mut out = String::new();
    for item in pattern.iter() {
        match item {
            PatternItem::Literal(s) => out.push_str(&s.replace(minus, plus)),
            PatternItem::Placeholder(DoublePlaceholderKey::Place0) => out.push_str(body),
            PatternItem::Placeholder(DoublePlaceholderKey::Place1) => out.push_str(currency),
        }
    }
    out
}
