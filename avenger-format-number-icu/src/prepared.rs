use crate::notation::Notation;
use crate::{
    locale::{Affix, LocaleData},
    skeleton::{Skeleton, Unit},
};
use avenger_format::{FormattedNumber, NumberTypesetting, PreparedNumberFormatter};
use fixed_decimal::{Decimal, Sign};
use std::fmt::{self, Write};
use writeable::{Part, PartsWrite, Writeable};

#[derive(Debug)]
pub(crate) struct Prepared {
    pub skeleton: Skeleton,
    pub locale: LocaleData,
}

/// Suppress only the numeric integer part, before signs and unit affixes are added.
struct BodyWriter {
    text: String,
    omit_integer: bool,
    suppress: bool,
}
impl Write for BodyWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if !self.suppress {
            self.text.push_str(s);
        }
        Ok(())
    }
}
impl PartsWrite for BodyWriter {
    type SubPartsWrite = Self;
    fn with_part(&mut self, part: Part, f: impl FnOnce(&mut Self) -> fmt::Result) -> fmt::Result {
        let old = self.suppress;
        self.suppress |= self.omit_integer && part == icu_decimal::parts::INTEGER;
        let result = f(self);
        self.suppress = old;
        result
    }
}

impl Prepared {
    fn body(&self, value: &Decimal) -> String {
        let s = &self.skeleton;
        let mut display = value.clone();
        display.sign = Sign::None;
        // Scaling and truncation can leave leading zeros; integer width alone decides padding.
        display.trim_start();
        if let Some(max) = s.integer_max {
            if !display.is_zero() && display.nonzero_magnitude_start() >= max {
                display.set_max_position(max);
                display.trim_start();
            }
        }
        display.pad_start(s.integer_min);
        let omit_integer = s.integer_min == 0
            && !(display.is_zero() && s.precision.keeps_zero_integer())
            && (display.nonzero_magnitude_start() < 0 || display.is_zero())
            && (*display.magnitude_range().start() < 0 || s.decimal_always);
        let mut writer = BodyWriter {
            text: String::new(),
            omit_integer,
            suppress: false,
        };
        self.locale
            .decimal
            .format(&display)
            .write_to_parts(&mut writer)
            .expect("writing to String cannot fail");
        if s.decimal_always && *display.magnitude_range().start() == 0 {
            writer.text.push_str(&self.locale.separator);
        }
        writer.text
    }

    fn affix(&self, body: String, sign: Sign, number: Option<&Decimal>, exponent: i16) -> String {
        let signs = &self.locale.signs;
        match &self.locale.affix {
            Affix::Plain => signs.apply(&body, sign),
            // Unit patterns can precede the number, so the sign stays with the digits.
            Affix::Unit(units) => units.render(&signs.apply(&body, sign), number, exponent),
            Affix::Percent(percent) => percent.render(&body, sign, signs),
            Affix::Currency(currency) => currency.render(&body, sign, number, exponent, signs),
        }
    }
}

impl Prepared {
    fn exponent(&self, magnitude: i16) -> i16 {
        match self.skeleton.notation {
            Notation::Scientific {
                engineering: true, ..
            } => magnitude.div_euclid(3) * 3,
            Notation::Scientific { .. } => magnitude,
            _ => self
                .locale
                .compact
                .as_ref()
                .map_or(0, |c| c.exponent(magnitude)),
        }
    }

    pub(crate) fn format_decimal(&self, original: Decimal) -> FormattedNumber {
        let s = &self.skeleton;
        let mut exponent = if original.is_zero() {
            0
        } else {
            self.exponent(original.nonzero_magnitude_start())
        };
        let round = |exponent: i16| {
            let mut number = original.clone();
            number.multiply_pow10(-exponent);
            s.precision.apply(&mut number, s.rounding);
            number
        };
        let mut number = round(exponent);
        if !number.is_zero() {
            let after = self.exponent(number.nonzero_magnitude_start() + exponent);
            if after != exponent {
                exponent = after;
                number = round(exponent);
            }
        }
        let sign = s
            .sign
            .display(number.sign == Sign::Negative, number.is_zero());
        let body = self.body(&number);
        match s.notation {
            Notation::Scientific {
                digits,
                sign: exponent_sign,
                ..
            } => {
                let mantissa = self.affix(body.clone(), sign, Some(&number), exponent);
                let mut exponent_number = Decimal::from(exponent);
                // ICU shows an exponent's plus sign only for sign-always.
                exponent_number.sign = match exponent_sign {
                    crate::skeleton::Sign::Never => Sign::None,
                    _ if exponent < 0 => Sign::Negative,
                    crate::skeleton::Sign::Always => Sign::Positive,
                    _ => Sign::None,
                };
                exponent_number.pad_start(digits);
                let suffix = self
                    .locale
                    .exponent_decimal
                    .format(&exponent_number)
                    .write_to_string()
                    .into_owned();
                let text = self.affix(
                    format!("{body}{}{suffix}", self.locale.symbols.exponent),
                    sign,
                    Some(&number),
                    exponent,
                );
                let typesetting = if s.unit == Unit::None
                    && self.locale.latin_digits
                    && s.integer_min <= 1
                    && s.integer_max.is_none()
                    && digits == 1
                    && exponent_sign == crate::skeleton::Sign::Auto
                {
                    NumberTypesetting::Exponent {
                        mantissa,
                        exponent: i32::from(exponent),
                    }
                } else {
                    NumberTypesetting::Plain
                };
                FormattedNumber { text, typesetting }
            }
            Notation::CompactShort | Notation::CompactLong => {
                let compact = self
                    .locale
                    .compact
                    .as_ref()
                    .expect("compact data prepared with notation");
                let body = compact.render(&number, exponent, &body);
                FormattedNumber::plain(self.affix(body, sign, Some(&number), exponent))
            }
            Notation::Simple => {
                FormattedNumber::plain(self.affix(body, sign, Some(&number), exponent))
            }
        }
    }
}

impl PreparedNumberFormatter for Prepared {
    fn format(&self, value: f64) -> FormattedNumber {
        if value.is_finite() {
            let mut number = crate::arithmetic::from_float(value);
            self.skeleton.scale.scale(&mut number);
            self.format_decimal(number)
        } else {
            let body = if value.is_nan() {
                self.locale.symbols.nan
            } else {
                self.locale.symbols.infinity
            };
            let sign = self
                .skeleton
                .sign
                .display(!value.is_nan() && value.is_sign_negative(), value.is_nan());
            FormattedNumber::plain(self.affix(body.into(), sign, None, 0))
        }
    }
}
