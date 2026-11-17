//! The text of a conversion: GoogleSQL's `FORMAT` of one number.

use avenger_format::NumberFormatError;

use crate::parse::{parse, Conversion, Kind, Pattern};
use crate::Locale;

/// A pattern of GoogleSQL's `FORMAT` that a chart's numbers format with: literal text, `%%`, and
/// exactly one numeric conversion, such as `$%'.2f` or `%.1f%%`, with the symbols of a locale.
///
/// ```rust
/// use avenger_format_number_googlesql::{Format, Locale};
///
/// let format = Format::new("%'.2f km", &Locale::default()).unwrap();
/// assert_eq!(format.format(1234.5), "1,234.50 km");
/// let format = Format::new("%#x", &Locale::default()).unwrap();
/// assert_eq!(format.format(255.0), "0xff");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Format {
    pattern: Pattern,
    locale: Locale,
}

impl Format {
    /// Parses `pattern`, which must hold exactly one conversion of `d i o x X f F e E g G`, with
    /// optional flags, width, and precision, among literal text and `%%`, for the symbols of
    /// `locale`.
    pub fn new(pattern: &str, locale: &Locale) -> Result<Self, NumberFormatError> {
        Ok(Self {
            pattern: parse(pattern)?,
            locale: locale.clone(),
        })
    }

    /// The text of `value` in the pattern.
    pub fn format(&self, value: f64) -> String {
        let Pattern {
            before,
            conversion,
            after,
        } = &self.pattern;
        let text = convert(conversion, &self.locale, value);
        format!("{before}{text}{after}")
    }
}

/// The text of `value` under `conversion` with the symbols of `locale`.
fn convert(conversion: &Conversion, locale: &Locale, value: f64) -> String {
    let c = conversion;
    if !value.is_finite() {
        // Integer conversions print non-finite values as `%f` does, in lowercase. A locale's
        // NaN prints as written.
        let upper = c.upper && !c.kind.is_integer();
        let word = match (value.is_nan(), upper, &locale.nan) {
            (true, _, Some(nan)) => nan.as_str(),
            (true, false, None) => "nan",
            (true, true, None) => "NAN",
            (false, false, _) => "inf",
            (false, true, _) => "INF",
        };
        return pad(c, sign(c, locale, value < 0.0), "", word, None);
    }
    if c.kind.is_integer() {
        let rounded = value.round_ties_even();
        let magnitude = rounded.abs();
        let mut digits = match c.kind {
            Kind::Decimal => format!("{magnitude:.0}"),
            Kind::Octal => radix_digits(magnitude, 3, false),
            _ => radix_digits(magnitude, 4, c.upper),
        };
        match c.precision {
            Some(0) if magnitude == 0.0 => digits.clear(),
            Some(precision) if digits.len() < precision => {
                digits.insert_str(0, &"0".repeat(precision - digits.len()));
            }
            _ => {}
        }
        // Decimal digits take the locale's groups and numerals; octal and hexadecimal digits
        // keep GoogleSQL's groups and ASCII digits.
        let (digits, zero) = match (c.kind, c.group) {
            (Kind::Decimal, true) => (locale.group(&digits), locale.zero()),
            (Kind::Decimal, false) => (locale.digits(&digits), locale.zero()),
            (Kind::Octal, true) => (group(&digits, ',', 4), "0"),
            (_, true) => (group(&digits, ':', 4), "0"),
            (_, false) => (digits, "0"),
        };
        let prefix = match (c.alternate, c.kind, c.upper) {
            (false, _, _) | (true, Kind::Decimal, _) => "",
            (true, Kind::Octal, _) => "0",
            (true, _, false) => "0x",
            (true, _, true) => "0X",
        };
        // As in C, an integer precision turns the `0` flag off.
        let zero = c.precision.is_none().then_some(zero);
        return pad(c, sign(c, locale, rounded < 0.0), prefix, &digits, zero);
    }
    let magnitude = value.abs();
    let decimals = c.precision.unwrap_or(6);
    let body = match c.kind {
        Kind::Fixed => locale.decimal_text(&fixed(magnitude, decimals, c.alternate), c.group),
        Kind::Exponent => {
            locale.scientific_text(&exponent(magnitude, decimals, c.alternate, c.upper))
        }
        _ => match general(magnitude, decimals, c.alternate, c.upper) {
            (text, true) => locale.scientific_text(&text),
            (text, false) => locale.decimal_text(&text, c.group),
        },
    };
    pad(
        c,
        sign(c, locale, value < 0.0),
        "",
        &body,
        Some(locale.zero()),
    )
}

/// The sign of a value: the locale's minus when it is negative, else `+` or a space by the flags.
fn sign<'a>(c: &Conversion, locale: &'a Locale, negative: bool) -> &'a str {
    match (negative, c.plus, c.space) {
        (true, _, _) => &locale.minus,
        (false, true, _) => "+",
        (false, false, true) => " ",
        (false, false, false) => "",
    }
}

/// `magnitude` with `decimals` decimals, rounded exactly, half to even on exact ties, in ASCII.
pub(crate) fn fixed(magnitude: f64, decimals: usize, alternate: bool) -> String {
    let mut text = format!("{magnitude:.decimals$}");
    if alternate && decimals == 0 {
        text.push('.');
    }
    text
}

/// `%e`: one digit, `decimals` decimals, and GoogleSQL's exponent, in ASCII.
pub(crate) fn exponent(magnitude: f64, decimals: usize, alternate: bool, upper: bool) -> String {
    let text = format!("{magnitude:.decimals$e}");
    let (mantissa, exponent) = split_exponent(&text);
    let point = if alternate && decimals == 0 { "." } else { "" };
    format!("{mantissa}{point}{}", exponent_suffix(exponent, upper))
}

/// `%g` in ASCII, and whether it is in scientific notation: the exponent X of `%e` at precision
/// P - 1 picks scientific notation when X < -4 or X >= P, else P - 1 - X decimals. Without `#`,
/// trailing zeros and a trailing point go.
pub(crate) fn general(
    magnitude: f64,
    precision: usize,
    alternate: bool,
    upper: bool,
) -> (String, bool) {
    let precision = precision.max(1);
    let scientific = format!("{magnitude:.*e}", precision - 1);
    let (mantissa, exponent) = split_exponent(&scientific);
    if exponent < -4 || exponent >= precision as i32 {
        let mantissa = trim(mantissa, alternate);
        return (
            format!("{mantissa}{}", exponent_suffix(exponent, upper)),
            true,
        );
    }
    let decimals = (precision as i32 - 1 - exponent) as usize;
    (trim(&format!("{magnitude:.decimals$}"), alternate), false)
}

/// Without `#`, the decimals' trailing zeros and a trailing point removed. With `#`, a point
/// added where there are no decimals.
fn trim(text: &str, alternate: bool) -> String {
    match (alternate, text.contains('.')) {
        (true, true) => text.to_string(),
        (true, false) => format!("{text}."),
        (false, true) => text.trim_end_matches('0').trim_end_matches('.').to_string(),
        (false, false) => text.to_string(),
    }
}

/// The mantissa and the exponent of Rust's scientific notation, such as `1.5e6`.
fn split_exponent(text: &str) -> (&str, i32) {
    let (mantissa, exponent) = text.split_once('e').expect("scientific notation");
    (mantissa, exponent.parse().expect("an integer exponent"))
}

/// GoogleSQL's exponent: `e` or `E`, a sign, and at least two digits, such as `e+06`.
fn exponent_suffix(exponent: i32, upper: bool) -> String {
    let e = if upper { 'E' } else { 'e' };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{e}{sign}{:02}", exponent.unsigned_abs())
}

/// `digits` with `separator` between groups of `size`, counted from the right.
fn group(digits: &str, separator: char, size: usize) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / size);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(size) {
            grouped.push(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// The digits of the integer `magnitude` in base 2^`bits`, exactly, for any size.
fn radix_digits(magnitude: f64, bits: u32, upper: bool) -> String {
    let symbols: &[u8; 16] = match upper {
        true => b"0123456789ABCDEF",
        false => b"0123456789abcdef",
    };
    if magnitude < 18_446_744_073_709_551_616.0 {
        let integer = magnitude as u64;
        return match (bits, upper) {
            (3, _) => format!("{integer:o}"),
            (_, false) => format!("{integer:x}"),
            (_, true) => format!("{integer:X}"),
        };
    }
    // Above 2^64, the value is its 53-bit significand shifted left by its binary exponent.
    let raw = magnitude.to_bits();
    let significand = (raw & ((1 << 52) - 1)) | (1 << 52);
    let shift = ((raw >> 52) & 0x7ff) as u32 - 1075;
    let binary: Vec<u8> = (0..53)
        .rev()
        .map(|bit| ((significand >> bit) & 1) as u8)
        .chain(std::iter::repeat_n(0, shift as usize))
        .collect();
    let bits = bits as usize;
    let lead = binary.len() % bits;
    let mut chunks: Vec<&[u8]> = Vec::new();
    if lead > 0 {
        chunks.push(&binary[..lead]);
    }
    chunks.extend(binary[lead..].chunks(bits));
    let digits: String = chunks
        .iter()
        .map(|chunk| {
            let value = chunk
                .iter()
                .fold(0, |value, &bit| value * 2 + usize::from(bit));
            char::from(symbols[value])
        })
        .collect();
    digits.trim_start_matches('0').to_string()
}

/// The conversion's text padded to its width in characters: spaces on the right with `-`, zeros
/// after the sign and the prefix with `0` where `zero` gives one, and spaces on the left
/// otherwise. The zeros are not grouped.
fn pad(c: &Conversion, sign: &str, prefix: &str, body: &str, zero: Option<&str>) -> String {
    let length = sign.chars().count() + prefix.chars().count() + body.chars().count();
    let fill = c.width.saturating_sub(length);
    match (c.left, c.zero, zero) {
        (true, _, _) => format!("{sign}{prefix}{body}{}", " ".repeat(fill)),
        (false, true, Some(zero)) => format!("{sign}{prefix}{}{body}", zero.repeat(fill)),
        _ => format!("{}{sign}{prefix}{body}", " ".repeat(fill)),
    }
}
