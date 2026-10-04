use crate::{locale::LocaleData, skeleton::Skeleton};
use avenger_format::{FormattedNumber, PreparedNumberFormatter};
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

    fn affix(&self, body: String, sign: Sign) -> String {
        self.locale.signs.apply(&body, sign)
    }
}

impl Prepared {
    pub(crate) fn format_decimal(&self, mut number: Decimal) -> FormattedNumber {
        let s = &self.skeleton;
        s.precision.apply(&mut number, s.rounding);
        let sign = s
            .sign
            .display(number.sign == Sign::Negative, number.is_zero());
        FormattedNumber::plain(self.affix(self.body(&number), sign))
    }
}

impl PreparedNumberFormatter for Prepared {
    fn format(&self, value: f64) -> FormattedNumber {
        if value.is_finite() {
            self.format_decimal(crate::arithmetic::from_float(value))
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
            FormattedNumber::plain(self.affix(body.into(), sign))
        }
    }
}
