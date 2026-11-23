//! The text of a conversion: GoogleSQL's `FORMAT` of one number, or of one text.

use avenger_format::NumberFormatError;

use crate::parse::{parse, Conversion, Element, Kind, Pattern};
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
        let text = convert(conversion, &self.locale, Value::Float(value));
        format!("{before}{text}{after}")
    }
}

/// A value of an element of `FORMAT`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    /// An `INT64`, which prints exactly at any size.
    Integer(i64),
    /// A `FLOAT64`.
    Float(f64),
    /// A `NUMERIC` or a `BIGNUMERIC`, which prints exactly.
    Decimal(Decimal<'a>),
}

/// An exact decimal: `digits`, ASCII decimal digits, times ten to the power of minus `scale`,
/// negative where `negative` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decimal<'a> {
    pub negative: bool,
    pub digits: &'a str,
    pub scale: usize,
}

impl Element {
    /// The text of the number `value` in the element, with the symbols of `locale` and the width
    /// and the precision of its `*` arguments, `width` and `precision`, where it takes them: a
    /// negative width pads on the right, and a negative precision is none. An integer conversion
    /// of a float rounds it half to even, and a float conversion of an integer prints it as a
    /// float, which GoogleSQL's `FORMAT` rejects before it prints.
    pub fn format_number(
        &self,
        locale: &Locale,
        width: Option<i64>,
        precision: Option<i64>,
        value: Value<'_>,
    ) -> String {
        convert(&self.conversion(width, precision), locale, value)
    }

    /// The text `text` in the element, as `%s`, `%t`, and `%T` print it: its first characters up
    /// to the precision, padded with spaces to the width, on the right with `-`. The `0` flag
    /// pads with spaces too.
    pub fn format_text(&self, width: Option<i64>, precision: Option<i64>, text: &str) -> String {
        let c = self.conversion(width, precision);
        let text = match c.precision {
            Some(precision) => match text.char_indices().nth(precision) {
                Some((end, _)) => &text[..end],
                None => text,
            },
            None => text,
        };
        let fill = " ".repeat(c.width.saturating_sub(text.chars().count()));
        match c.flags.left {
            true => format!("{text}{fill}"),
            false => format!("{fill}{text}"),
        }
    }
}

/// The text of `value` under `conversion` with the symbols of `locale`.
fn convert(conversion: &Conversion, locale: &Locale, value: Value<'_>) -> String {
    let c = conversion;
    match value {
        Value::Integer(value) if c.kind.is_integer() => {
            let magnitude = value.unsigned_abs();
            let digits = match c.kind {
                Kind::Decimal => magnitude.to_string(),
                Kind::Octal => format!("{magnitude:o}"),
                _ if c.upper => format!("{magnitude:X}"),
                _ => format!("{magnitude:x}"),
            };
            integer(c, locale, value < 0, digits)
        }
        Value::Integer(value) => convert(c, locale, Value::Float(value as f64)),
        Value::Float(value) => float(c, locale, value),
        Value::Decimal(decimal) => {
            let exact = Exact::new(decimal.digits, decimal.scale);
            let negative = decimal.negative && !exact.is_zero();
            match c.kind.is_integer() {
                true => {
                    let value: f64 = exact.fixed(17).parse().expect("a decimal");
                    float(c, locale, if negative { -value } else { value })
                }
                false => decimals(c, locale, negative, &exact),
            }
        }
    }
}

/// The text of the float `value` under `conversion`.
fn float(c: &Conversion, locale: &Locale, value: f64) -> String {
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
        let digits = match c.kind {
            Kind::Decimal => format!("{magnitude:.0}"),
            Kind::Octal => radix_digits(magnitude, 3, false),
            _ => radix_digits(magnitude, 4, c.upper),
        };
        return integer(c, locale, rounded < 0.0, digits);
    }
    decimals(c, locale, value < 0.0, &value.abs())
}

/// The text of an integer, from its sign and the digits of its magnitude in the conversion's
/// base.
///
/// GoogleSQL prints `%d` and `%i` without `'` as C does, where a precision of 0 prints 0 as
/// nothing and a precision turns the `0` flag off, and prints `%o`, `%x`, `%X`, and grouped
/// decimals with its own printer, where 0 keeps its digit and the `0` flag pads after a
/// precision too.
fn integer(c: &Conversion, locale: &Locale, negative: bool, mut digits: String) -> String {
    let as_c = c.kind == Kind::Decimal && !c.flags.group;
    let significant = digits.len();
    match c.precision {
        Some(0) if digits == "0" && as_c => digits.clear(),
        Some(precision) if digits.len() < precision => {
            digits.insert_str(0, &"0".repeat(precision - digits.len()));
        }
        _ => {}
    }
    // Decimal digits take the locale's groups and numerals; octal and hexadecimal digits keep
    // GoogleSQL's groups and ASCII digits.
    let (digits, zero) = match (c.kind, c.flags.group) {
        (Kind::Decimal, true) => (locale.group(&digits), locale.zero()),
        (Kind::Decimal, false) => (locale.digits(&digits), locale.zero()),
        (Kind::Octal, true) => (group(&digits, ',', 4), "0"),
        (_, true) => (group(&digits, ':', 4), "0"),
        (_, false) => (digits, "0"),
    };
    // The octal prefix `0` goes where a precision already writes leading zeros.
    let padded = c.precision.is_some_and(|precision| precision > significant);
    let prefix = match (c.flags.alternate, c.kind, c.upper) {
        (false, _, _) | (true, Kind::Decimal, _) => "",
        (true, Kind::Octal, _) if padded => "",
        (true, Kind::Octal, _) => "0",
        (true, _, false) => "0x",
        (true, _, true) => "0X",
    };
    let zero = (!as_c || c.precision.is_none()).then_some(zero);
    pad(c, sign(c, locale, negative), prefix, &digits, zero)
}

/// The text of a magnitude in a float conversion, `f`, `e`, or `g`, with the sign of `negative`.
fn decimals(c: &Conversion, locale: &Locale, negative: bool, magnitude: &dyn Magnitude) -> String {
    let decimals = c.precision.unwrap_or(6);
    let alternate = c.flags.alternate;
    let body = match c.kind {
        Kind::Fixed => {
            locale.decimal_text(&fixed_of(magnitude, decimals, alternate), c.flags.group)
        }
        Kind::Exponent => {
            locale.scientific_text(&exponent_of(magnitude, decimals, alternate, c.upper))
        }
        _ => match general_of(magnitude, decimals, alternate, c.upper) {
            (text, true) => locale.scientific_text(&text),
            (text, false) => locale.decimal_text(&text, c.flags.group),
        },
    };
    pad(c, sign(c, locale, negative), "", &body, Some(locale.zero()))
}

/// A magnitude, a finite value that is not negative, in decimals: an `f64` by its exact binary
/// value, or an exact decimal.
trait Magnitude {
    /// The magnitude with `decimals` decimals, rounded, in ASCII, such as `12.50`.
    fn fixed(&self, decimals: usize) -> String;
    /// The mantissa with `decimals` decimals, rounded, and the exponent of scientific notation,
    /// such as `1.25` and 6.
    fn scientific(&self, decimals: usize) -> (String, i32);
}

/// An `f64` rounds its exact binary value half to even on exact ties.
impl Magnitude for f64 {
    fn fixed(&self, decimals: usize) -> String {
        format!("{self:.decimals$}")
    }

    fn scientific(&self, decimals: usize) -> (String, i32) {
        let text = format!("{self:.decimals$e}");
        let (mantissa, exponent) = split_exponent(&text);
        (mantissa.to_string(), exponent)
    }
}

/// An exact decimal: its digits without a sign, the integer digits and then the fraction's, and
/// the number of integer digits, at least one. It rounds half away from zero, as GoogleSQL's
/// `NUMERIC` does.
struct Exact {
    digits: Vec<u8>,
    point: usize,
}

impl Exact {
    /// The magnitude of `digits` times ten to the power of minus `scale`.
    fn new(digits: &str, scale: usize) -> Exact {
        let trimmed = digits.trim_start_matches('0');
        let mut digits: Vec<u8> = trimmed.bytes().collect();
        if digits.len() <= scale {
            let zeros = scale + 1 - digits.len();
            digits.splice(0..0, std::iter::repeat_n(b'0', zeros));
        }
        let point = digits.len() - scale;
        Exact { digits, point }
    }

    fn is_zero(&self) -> bool {
        self.digits.iter().all(|&digit| digit == b'0')
    }
}

/// Adds one to the ASCII digits `digits`, and whether the sum carries out of the first digit,
/// which leaves the digits all zeros.
fn increment(digits: &mut [u8]) -> bool {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            return false;
        }
    }
    true
}

impl Magnitude for Exact {
    fn fixed(&self, decimals: usize) -> String {
        let keep = self.point + decimals;
        let mut digits: Vec<u8> = self.digits.iter().copied().take(keep).collect();
        digits.resize(keep, b'0');
        let mut point = self.point;
        if self.digits.get(keep).is_some_and(|&digit| digit >= b'5') && increment(&mut digits) {
            digits.insert(0, b'1');
            point += 1;
        }
        let text = String::from_utf8(digits).expect("ASCII digits");
        match decimals {
            0 => text,
            _ => format!("{}.{}", &text[..point], &text[point..]),
        }
    }

    fn scientific(&self, decimals: usize) -> (String, i32) {
        let Some(first) = self.digits.iter().position(|&digit| digit != b'0') else {
            let zeros = "0".repeat(decimals);
            return match decimals {
                0 => ("0".to_string(), 0),
                _ => (format!("0.{zeros}"), 0),
            };
        };
        let mut exponent = self.point as i32 - first as i32 - 1;
        let significant = &self.digits[first..];
        let mut digits: Vec<u8> = significant.iter().copied().take(decimals + 1).collect();
        digits.resize(decimals + 1, b'0');
        let next = significant.get(decimals + 1);
        if next.is_some_and(|&digit| digit >= b'5') && increment(&mut digits) {
            digits[0] = b'1';
            exponent += 1;
        }
        let text = String::from_utf8(digits).expect("ASCII digits");
        let mantissa = match decimals {
            0 => text,
            _ => format!("{}.{}", &text[..1], &text[1..]),
        };
        (mantissa, exponent)
    }
}

/// The sign of a value: the locale's minus when it is negative, else `+` or a space by the flags.
fn sign<'a>(c: &Conversion, locale: &'a Locale, negative: bool) -> &'a str {
    match (negative, c.flags.plus, c.flags.space) {
        (true, _, _) => &locale.minus,
        (false, true, _) => "+",
        (false, false, true) => " ",
        (false, false, false) => "",
    }
}

/// `magnitude` with `decimals` decimals, rounded exactly, half to even on exact ties, in ASCII.
pub(crate) fn fixed(magnitude: f64, decimals: usize, alternate: bool) -> String {
    fixed_of(&magnitude, decimals, alternate)
}

/// `%e`: one digit, `decimals` decimals, and GoogleSQL's exponent, in ASCII.
pub(crate) fn exponent(magnitude: f64, decimals: usize, alternate: bool, upper: bool) -> String {
    exponent_of(&magnitude, decimals, alternate, upper)
}

/// `%g` in ASCII, and whether it is in scientific notation.
pub(crate) fn general(
    magnitude: f64,
    precision: usize,
    alternate: bool,
    upper: bool,
) -> (String, bool) {
    general_of(&magnitude, precision, alternate, upper)
}

/// `%f` of a magnitude: `decimals` decimals, and a point without decimals with `#`.
fn fixed_of(magnitude: &dyn Magnitude, decimals: usize, alternate: bool) -> String {
    let mut text = magnitude.fixed(decimals);
    if alternate && decimals == 0 {
        text.push('.');
    }
    text
}

/// `%e` of a magnitude: one digit, `decimals` decimals, and GoogleSQL's exponent.
fn exponent_of(magnitude: &dyn Magnitude, decimals: usize, alternate: bool, upper: bool) -> String {
    let (mantissa, exponent) = magnitude.scientific(decimals);
    let point = if alternate && decimals == 0 { "." } else { "" };
    format!("{mantissa}{point}{}", exponent_suffix(exponent, upper))
}

/// `%g` of a magnitude, and whether it is in scientific notation: the exponent X of `%e` at
/// precision P - 1 picks scientific notation when X < -4 or X >= P, else P - 1 - X decimals.
/// Without `#`, trailing zeros and a trailing point go.
fn general_of(
    magnitude: &dyn Magnitude,
    precision: usize,
    alternate: bool,
    upper: bool,
) -> (String, bool) {
    let precision = precision.max(1);
    let (mantissa, exponent) = magnitude.scientific(precision - 1);
    if exponent < -4 || exponent >= precision as i32 {
        let mantissa = trim(&mantissa, alternate);
        return (
            format!("{mantissa}{}", exponent_suffix(exponent, upper)),
            true,
        );
    }
    let decimals = (precision as i32 - 1 - exponent) as usize;
    (trim(&magnitude.fixed(decimals), alternate), false)
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
    match (c.flags.left, c.flags.zero, zero) {
        (true, _, _) => format!("{sign}{prefix}{body}{}", " ".repeat(fill)),
        (false, true, Some(zero)) => format!("{sign}{prefix}{}{body}", zero.repeat(fill)),
        _ => format!("{}{sign}{prefix}{body}", " ".repeat(fill)),
    }
}
