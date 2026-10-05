use crate::notation::Notation;
use crate::{
    locale::{Affix, LocaleData},
    precision::{Digits, Precision},
    skeleton::{Skeleton, Unit},
};
use avenger_format::{
    FormattedNumber, NumberTypesetting, PreparedNumberFormatter, TickSpacing, TickStep,
};
use fixed_decimal::{Decimal, Sign};
use std::fmt::{self, Write};
use writeable::{Part, PartsWrite, Writeable};

#[derive(Debug)]
pub(crate) struct Prepared {
    pub skeleton: Skeleton,
    pub locale: LocaleData,
}

/// A compact unit shared by tick labels: its notation exponent, and the magnitude whose
/// pattern supplies the unit.
#[derive(Debug, Clone, Copy)]
struct CompactUnit {
    exponent: i16,
    magnitude: i16,
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
    /// Larger mixed components display integers but share the smallest component's plural precision.
    pub(crate) fn format_mixed_integer(
        &self,
        number: Decimal,
        precision: &crate::precision::Precision,
    ) -> FormattedNumber {
        let mut plural = number.clone();
        precision.apply(&mut plural, self.skeleton.rounding);
        let sign = self
            .skeleton
            .sign
            .display(number.sign == Sign::Negative, number.is_zero());
        let body = self.body(&number, &self.skeleton.precision);
        FormattedNumber::plain(self.affix(body, sign, Some(&plural), 0))
    }

    /// `precision` is the rounding that produced `value`.
    fn body(&self, value: &Decimal, precision: &Precision) -> String {
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
            && !(display.is_zero() && precision.keeps_zero_integer())
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
        self.render(original, &self.skeleton.precision, None)
    }

    /// Round with `precision`. A shared compact unit replaces the exponent chosen per value.
    fn render(
        &self,
        original: Decimal,
        precision: &Precision,
        unit: Option<CompactUnit>,
    ) -> FormattedNumber {
        let s = &self.skeleton;
        let round = |exponent: i16| {
            let mut number = original.clone();
            number.multiply_pow10(-exponent);
            precision.apply(&mut number, s.rounding);
            number
        };
        let (exponent, number) = match unit {
            Some(unit) => (unit.exponent, round(unit.exponent)),
            None => {
                let mut exponent = if original.is_zero() {
                    0
                } else {
                    self.exponent(original.nonzero_magnitude_start())
                };
                let mut number = round(exponent);
                if !number.is_zero() {
                    let after = self.exponent(number.nonzero_magnitude_start() + exponent);
                    if after != exponent {
                        exponent = after;
                        number = round(exponent);
                    }
                }
                (exponent, number)
            }
        };
        let sign = s
            .sign
            .display(number.sign == Sign::Negative, number.is_zero());
        let body = self.body(&number, precision);
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
                let magnitude = unit.map_or(number.nonzero_magnitude_start() + exponent, |unit| {
                    unit.magnitude
                });
                let body = compact.render(&number, magnitude, &body);
                FormattedNumber::plain(self.affix(body, sign, Some(&number), exponent))
            }
            Notation::Simple => {
                FormattedNumber::plain(self.affix(body, sign, Some(&number), exponent))
            }
        }
    }
}

impl Prepared {
    fn format_value(
        &self,
        value: f64,
        precision: &Precision,
        unit: Option<CompactUnit>,
    ) -> FormattedNumber {
        if value.is_finite() {
            let mut number = crate::arithmetic::from_float(value);
            self.skeleton.scale.scale(&mut number);
            self.render(number, precision, unit)
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

    /// One precision for every tick, and for compact notation one unit, chosen from the
    /// ticks' resolution and largest magnitude after scaling.
    fn uniform(&self, ticks: &TickStep) -> (Precision, Option<CompactUnit>) {
        let s = &self.skeleton;
        let resolution = ticks.resolution + i32::from(s.scale.exponent());
        let mut largest = crate::arithmetic::from_float(ticks.magnitude);
        s.scale.scale(&mut largest);
        let magnitude = if largest.is_zero() {
            0
        } else {
            largest.nonzero_magnitude_start()
        };
        let fixed = |places: i16| {
            Precision::fraction(Digits {
                min: places,
                max: Some(places),
            })
        };
        match s.notation {
            Notation::Simple => (fixed(fraction_digits(resolution)), None),
            Notation::Scientific { .. } => {
                let digits = (i32::from(magnitude) - resolution + 1).clamp(1, 999) as i16;
                let digits = Digits {
                    min: digits,
                    max: Some(digits),
                };
                (Precision::significant(digits), None)
            }
            Notation::CompactShort | Notation::CompactLong => {
                let exponent = self.exponent(magnitude);
                let places = fraction_digits(resolution - i32::from(exponent));
                (
                    fixed(places),
                    Some(CompactUnit {
                        exponent,
                        magnitude,
                    }),
                )
            }
        }
    }
}

/// Fraction digits that show a decimal place, from none to ICU's limit of 999.
fn fraction_digits(resolution: i32) -> i16 {
    (-resolution).clamp(0, 999) as i16
}

impl PreparedNumberFormatter for Prepared {
    fn format(&self, value: f64) -> FormattedNumber {
        self.format_value(value, &self.skeleton.precision, None)
    }

    /// Without a precision stem, uniform ticks share one precision and compact unit, and
    /// varying ticks in simple notation each keep their own exact digits.
    fn format_ticks(&self, values: &[f64], spacing: TickSpacing) -> Vec<FormattedNumber> {
        let s = &self.skeleton;
        let ticks = (!s.explicit_precision)
            .then(|| TickStep::infer(values))
            .flatten();
        match (spacing, ticks) {
            (TickSpacing::Uniform, Some(ticks)) => {
                let (precision, unit) = self.uniform(&ticks);
                values
                    .iter()
                    .map(|&value| self.format_value(value, &precision, unit))
                    .collect()
            }
            (TickSpacing::Varying, Some(_)) if s.notation == Notation::Simple => values
                .iter()
                .map(|&value| match TickStep::infer(&[value]) {
                    Some(tick) => {
                        let resolution = tick.resolution + i32::from(s.scale.exponent());
                        let digits = Digits {
                            min: 0,
                            max: Some(fraction_digits(resolution)),
                        };
                        self.format_value(value, &Precision::fraction(digits), None)
                    }
                    None => self.format(value),
                })
                .collect(),
            _ => values.iter().map(|&value| self.format(value)).collect(),
        }
    }
}
