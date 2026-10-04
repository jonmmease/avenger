use crate::data::Context;
use crate::skeleton::{invalid, Sign};
use avenger_format::NumberFormatError;
use fixed_decimal::Decimal;
use icu_decimal::provider::*;
use icu_pattern::{Pattern, SinglePlaceholder};
use icu_plurals::{PluralOperands, PluralRules};
use icu_provider::prelude::*;
use writeable::Writeable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notation {
    Simple,
    Scientific {
        engineering: bool,
        digits: i16,
        sign: Sign,
    },
    CompactShort,
    CompactLong,
}

impl Notation {
    pub fn is_compact(self) -> bool {
        matches!(self, Self::CompactShort | Self::CompactLong)
    }

    pub fn parse(stem: &str, options: &[&str], position: usize) -> Result<Self, NumberFormatError> {
        let mut digits = 1;
        let mut sign = Sign::Auto;
        let mut has_digits = false;
        let mut has_sign = false;
        for option in options {
            if !matches!(stem, "scientific" | "engineering") {
                return Err(invalid("unexpected notation option", position));
            }
            if option.starts_with(['*', '+']) && !has_digits {
                let body = &option[1..];
                if body.is_empty() || body.len() > 999 || !body.bytes().all(|b| b == b'e') {
                    return Err(invalid("invalid exponent width", position));
                }
                digits = body.len() as i16;
                has_digits = true;
            } else if !has_sign {
                sign = match *option {
                    "sign-auto" => Sign::Auto,
                    "sign-always" => Sign::Always,
                    "sign-never" => Sign::Never,
                    "sign-except-zero" => Sign::ExceptZero,
                    "sign-negative" => Sign::Negative,
                    _ => return Err(invalid("invalid exponent sign option", position)),
                };
                has_sign = true;
            } else {
                return Err(invalid("duplicate notation option", position));
            }
        }
        Ok(match stem {
            "scientific" | "engineering" => Self::Scientific {
                engineering: stem == "engineering",
                digits,
                sign,
            },
            "compact-short" | "K" => Self::CompactShort,
            "compact-long" | "KK" => Self::CompactLong,
            _ => Self::Simple,
        })
    }

    pub fn concise(stem: &str, position: usize) -> Result<Self, NumberFormatError> {
        let engineering = stem.starts_with("EE");
        let mut rest = &stem[if engineering { 2 } else { 1 }..];
        let sign = if let Some(tail) = rest.strip_prefix("+!") {
            rest = tail;
            Sign::Always
        } else if let Some(tail) = rest.strip_prefix("+?") {
            rest = tail;
            Sign::ExceptZero
        } else {
            Sign::Auto
        };
        if rest.is_empty() || rest.len() > 999 || !rest.bytes().all(|b| b == b'0') {
            return Err(invalid("invalid scientific blueprint", position));
        }
        Ok(Self::Scientific {
            engineering,
            digits: rest.len() as i16,
            sign,
        })
    }
}

/// ICU compact patterns accept our already-rounded numeric body, including display adjustments.
#[derive(Debug)]
pub(crate) struct Compact {
    data: DataPayload<DecimalCompactShortV1>,
    plurals: PluralRules,
}
impl Compact {
    pub fn new(context: &Context, notation: Notation) -> Result<Self, DataError> {
        let data = if notation == Notation::CompactLong {
            context.load::<DecimalCompactLongV1, _>(&Baked)?.cast()
        } else {
            context.load::<DecimalCompactShortV1, _>(&Baked)?
        };
        Ok(Self {
            data,
            plurals: context.plural_rules()?,
        })
    }
    pub fn exponent(&self, magnitude: i16) -> i16 {
        self.data
            .get()
            .0
            .iter()
            .filter(|p| i16::from(p.sized) <= magnitude)
            .last()
            .map_or(0, |p| i16::from(p.sized - p.variable.get_default().0.get()))
    }
    /// Apply the pattern for `magnitude`, choosing its plural form from the displayed number.
    pub fn render(&self, number: &Decimal, magnitude: i16, body: &str) -> String {
        // ICU uses explicit "=1" patterns for a positive mantissa equal to one, such as 1.0, but
        // plural categories from the displayed digits; in CLDR 48 only French data has such
        // patterns. ICU4X matches "=1" from operands exactly equal to one. A positive one with the
        // same category uses those operands, and a negative one uses "1.0", which keeps its
        // category without matching.
        let operands = PluralOperands::from(&number.absolute);
        let one = number.nonzero_magnitude_start() == 0
            && number.nonzero_magnitude_end() == 0
            && number.digit_at(0) == 1;
        let category = self.plurals.category_for(operands);
        let operands = if !one {
            operands
        } else if number.sign == fixed_decimal::Sign::Negative {
            let decimal: Decimal = "1.0".parse().expect("valid decimal");
            Some(PluralOperands::from(&decimal.absolute))
                .filter(|o| self.plurals.category_for(*o) == category)
                .unwrap_or(operands)
        } else if self.plurals.category_for(1u32) == category {
            PluralOperands::from(1u32)
        } else {
            operands
        };
        let pattern = self
            .data
            .get()
            .0
            .iter()
            .filter(|p| i16::from(p.sized) <= magnitude)
            .last()
            .map(|p| p.variable.get(operands, &self.plurals).1)
            .unwrap_or(Pattern::<SinglePlaceholder>::PASS_THROUGH);
        pattern.interpolate([body]).write_to_string().into_owned()
    }
}

/// CLDR 48 distinguishes exponent operands 0–5 from all other values.
/// Rescale the significand to preserve the quantity when ICU4X's byte-sized exponent cannot hold it.
pub(crate) fn plural_operands(
    number: &fixed_decimal::Decimal,
    exponent: i16,
) -> icu_plurals::PluralOperands {
    if let Ok(exponent) = u8::try_from(exponent) {
        return icu_plurals::PluralOperands::from_significand_and_exponent(
            &number.absolute,
            exponent,
        );
    }
    let mut number = number.clone();
    number.multiply_pow10(exponent - 6);
    icu_plurals::PluralOperands::from_significand_and_exponent(&number.absolute, 6)
}
