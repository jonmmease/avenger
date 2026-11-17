//! The symbols that a pattern writes numbers with: GoogleSQL's, or a d3 number locale's.

use avenger_format::NumberFormatError;
use avenger_format_locales_d3::{bundled_number, NumberLocaleSpec};

/// The symbols of numbers: GoogleSQL's by default, or a d3 number locale's.
///
/// A locale's `decimal` replaces the point of `f F e E g G`, its `thousands` and `grouping` are
/// what `'` inserts in decimal digits, its `minus` replaces the sign `-`, its `nan` replaces
/// `nan` and `NAN` as written, and its `numerals` replace the decimal digits and the zeros of
/// padding. Hexadecimal and octal digits and their groups, `inf`, and the exponent's sign keep
/// GoogleSQL's text, and the locale's `currency` and `percent` do not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    pub(crate) decimal: String,
    pub(crate) thousands: String,
    /// Group sizes, cycled from the right with the last size repeated.
    pub(crate) grouping: Vec<usize>,
    pub(crate) minus: String,
    /// The text of NaN, or `None` for GoogleSQL's `nan` and `NAN`.
    pub(crate) nan: Option<String>,
    pub(crate) numerals: Option<[String; 10]>,
}

impl Default for Locale {
    /// GoogleSQL's symbols, which its `FORMAT` always writes: `.`, `,` in threes, `-`, and
    /// ASCII digits.
    fn default() -> Self {
        Self {
            decimal: ".".into(),
            thousands: ",".into(),
            grouping: vec![3],
            minus: "-".into(),
            nan: None,
            numerals: None,
        }
    }
}

impl Locale {
    /// The symbols of the d3 number locale `spec`. A group size of zero is an error.
    pub fn new(spec: &NumberLocaleSpec) -> Result<Locale, NumberFormatError> {
        if spec.grouping.contains(&0) {
            return Err(NumberFormatError::InvalidLocaleData {
                message: "field `grouping` has a size of 0; group sizes must be positive".into(),
            });
        }
        Ok(Locale {
            decimal: spec.decimal.clone(),
            thousands: spec.thousands.clone(),
            grouping: spec.grouping.clone(),
            minus: spec.minus.clone(),
            nan: Some(spec.nan.clone()),
            numerals: spec.numerals.clone(),
        })
    }

    /// The bundled locale named `id`, with hyphens or underscores, such as `de-DE`: U.S.
    /// English, or all of d3's number locales with the `all-locales` feature.
    pub fn bundled(id: &str) -> Result<Locale, NumberFormatError> {
        let spec = bundled_number(id).ok_or_else(|| NumberFormatError::LocaleUnavailable {
            locale: id.into(),
            message: "no bundled D3 definition is available in this build".into(),
        })?;
        Locale::new(&spec)
    }

    /// The ASCII digits of `digits` in the locale's numerals.
    pub(crate) fn digits(&self, digits: &str) -> String {
        match &self.numerals {
            None => digits.to_string(),
            Some(numerals) => {
                let mut localized = String::with_capacity(digits.len() * 2);
                for c in digits.chars() {
                    match c.to_digit(10) {
                        Some(digit) => localized.push_str(&numerals[digit as usize]),
                        None => localized.push(c),
                    }
                }
                localized
            }
        }
    }

    /// The zero of padding.
    pub(crate) fn zero(&self) -> &str {
        match &self.numerals {
            None => "0",
            Some(numerals) => &numerals[0],
        }
    }

    /// The ASCII digits `digits` in groups of the locale's sizes, from the right, separated by
    /// `thousands`. An empty `thousands` or `grouping` inserts nothing.
    pub(crate) fn group(&self, digits: &str) -> String {
        if self.thousands.is_empty() || self.grouping.is_empty() {
            return self.digits(digits);
        }
        let mut groups = Vec::new();
        let mut end = digits.len();
        let mut sizes = self.grouping.iter();
        let mut size = self.grouping[0];
        while end > 0 {
            size = sizes.next().copied().unwrap_or(size);
            let start = end.saturating_sub(size);
            groups.push(self.digits(&digits[start..end]));
            end = start;
        }
        groups.reverse();
        groups.join(&self.thousands)
    }

    /// The ASCII decimal text `text`, such as `1234.50`, with the locale's point and numerals, and
    /// its integer digits grouped where `grouped` says.
    pub(crate) fn decimal_text(&self, text: &str, grouped: bool) -> String {
        let (integer, fraction) = match text.split_once('.') {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (text, None),
        };
        let mut localized = match grouped {
            true => self.group(integer),
            false => self.digits(integer),
        };
        if let Some(fraction) = fraction {
            localized.push_str(&self.decimal);
            localized.push_str(&self.digits(fraction));
        }
        localized
    }

    /// The ASCII scientific text `text`, such as `1.5e+06`, with the locale's point and numerals.
    /// The exponent's `e` and sign stay as written.
    pub(crate) fn scientific_text(&self, text: &str) -> String {
        match text.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => {
                let e = &text[mantissa.len()..mantissa.len() + 1];
                let (sign, digits) = exponent.split_at(1);
                format!(
                    "{}{e}{sign}{}",
                    self.decimal_text(mantissa, false),
                    self.digits(digits)
                )
            }
            None => self.decimal_text(text, false),
        }
    }
}
