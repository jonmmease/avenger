//! The parse of a pattern: literal text around exactly one numeric conversion.

use avenger_format::NumberFormatError;

/// The conversions that a chart's pattern takes, for the messages of errors.
const CONVERSIONS: &str =
    "one of `%d`, `%i`, `%o`, `%x`, `%X`, `%f`, `%F`, `%e`, `%E`, `%g`, or `%G`";

/// The largest width or precision, which keeps a label from asking for a huge buffer.
const LIMIT: usize = 1_000;

/// The kind of a numeric conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Kind {
    /// `d` and `i`: a decimal integer.
    Decimal,
    /// `o`: an octal integer.
    Octal,
    /// `x` and `X`: a hexadecimal integer.
    Hex,
    /// `f` and `F`: fixed decimals.
    Fixed,
    /// `e` and `E`: scientific notation.
    Exponent,
    /// `g` and `G`: fixed decimals or scientific notation, by the exponent.
    #[default]
    General,
}

impl Kind {
    /// Whether the conversion rounds the value to an integer.
    pub(crate) fn is_integer(self) -> bool {
        matches!(self, Kind::Decimal | Kind::Octal | Kind::Hex)
    }
}

/// A numeric conversion with its flags, width, and precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Conversion {
    pub(crate) kind: Kind,
    /// An uppercase specifier: `X`, `F`, `E`, or `G`.
    pub(crate) upper: bool,
    /// `-`: pad on the right.
    pub(crate) left: bool,
    /// `+`: write `+` before a value that is not negative.
    pub(crate) plus: bool,
    /// Space: write a space before a value that is not negative.
    pub(crate) space: bool,
    /// `#`: the prefixes `0`, `0x`, and `0X`, and a decimal point without decimals.
    pub(crate) alternate: bool,
    /// `0`: pad with zeros after the sign and the prefix.
    pub(crate) zero: bool,
    /// `'`: group the digits.
    pub(crate) group: bool,
    /// The minimum width of the conversion's text.
    pub(crate) width: usize,
    /// The precision, where the pattern writes one.
    pub(crate) precision: Option<usize>,
}

/// A pattern: the text before the conversion, the conversion, and the text after it, with each
/// `%%` read as `%`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pattern {
    pub(crate) before: String,
    pub(crate) conversion: Conversion,
    pub(crate) after: String,
}

fn invalid(message: String, position: usize) -> NumberFormatError {
    NumberFormatError::InvalidPattern {
        message,
        position: Some(position),
    }
}

/// Reads a decimal number at `*index`, if one starts there.
fn number(
    bytes: &[u8],
    index: &mut usize,
    start: usize,
    what: &str,
) -> Result<usize, NumberFormatError> {
    let mut value = 0_usize;
    while let Some(digit) = bytes.get(*index).filter(|byte| byte.is_ascii_digit()) {
        value = value * 10 + usize::from(digit - b'0');
        if value > LIMIT {
            return Err(invalid(
                format!("the {what} of the conversion is over {LIMIT}"),
                start,
            ));
        }
        *index += 1;
    }
    Ok(value)
}

/// Parses `pattern` into literal text and exactly one numeric conversion.
pub(crate) fn parse(pattern: &str) -> Result<Pattern, NumberFormatError> {
    let bytes = pattern.as_bytes();
    let mut literals = [String::new(), String::new()];
    let mut conversion: Option<(Conversion, usize)> = None;
    let mut index = 0;
    let mut literal_start = 0;
    while let Some(offset) = pattern[index..].find('%') {
        let start = index + offset;
        let side = usize::from(conversion.is_some());
        literals[side].push_str(&pattern[literal_start..start]);
        index = start + 1;
        if bytes.get(index) == Some(&b'%') {
            literals[side].push('%');
            index += 1;
            literal_start = index;
            continue;
        }
        let mut spec = Conversion::default();
        while let Some(&flag) = bytes.get(index) {
            match flag {
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alternate = true,
                b'0' => spec.zero = true,
                b'\'' => spec.group = true,
                _ => break,
            }
            index += 1;
        }
        if bytes.get(index) == Some(&b'*') {
            return Err(invalid(
                format!(
                    "a `*` width takes its value from an argument, which a chart's pattern \
                     does not have; write the width as digits, and the conversion as \
                     {CONVERSIONS}"
                ),
                index,
            ));
        }
        spec.width = number(bytes, &mut index, start, "width")?;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            if bytes.get(index) == Some(&b'*') {
                return Err(invalid(
                    format!(
                        "a `*` precision takes its value from an argument, which a chart's \
                         pattern does not have; write the precision as digits, and the \
                         conversion as {CONVERSIONS}"
                    ),
                    index,
                ));
            }
            spec.precision = Some(number(bytes, &mut index, start, "precision")?);
        }
        let Some(specifier) = pattern[index..].chars().next() else {
            return Err(invalid(
                format!(
                    "the pattern ends in a lone `{}`; write `%%` for a percent sign, and a \
                     conversion as {CONVERSIONS}",
                    &pattern[start..]
                ),
                start,
            ));
        };
        let element = &pattern[start..index + specifier.len_utf8()];
        (spec.kind, spec.upper) = match specifier {
            'd' | 'i' => (Kind::Decimal, false),
            'o' => (Kind::Octal, false),
            'x' => (Kind::Hex, false),
            'X' => (Kind::Hex, true),
            'f' => (Kind::Fixed, false),
            'F' => (Kind::Fixed, true),
            'e' => (Kind::Exponent, false),
            'E' => (Kind::Exponent, true),
            'g' => (Kind::General, false),
            'G' => (Kind::General, true),
            _ => {
                return Err(invalid(
                    format!(
                        "`{element}` is not a conversion of a number; a chart's pattern takes \
                         {CONVERSIONS}"
                    ),
                    start,
                ))
            }
        };
        if let Some((_, first)) = conversion {
            return Err(invalid(
                format!(
                    "`{element}` is a second conversion after the one at byte {first}; a \
                     chart's pattern takes exactly one conversion, {CONVERSIONS}"
                ),
                start,
            ));
        }
        conversion = Some((spec, start));
        index += specifier.len_utf8();
        literal_start = index;
    }
    let Some((conversion, _)) = conversion else {
        return Err(NumberFormatError::InvalidPattern {
            message: format!(
                "the pattern has no conversion; a chart's pattern takes exactly one conversion, \
                 {CONVERSIONS}"
            ),
            position: None,
        });
    };
    let [before, mut after] = literals;
    after.push_str(&pattern[literal_start..]);
    Ok(Pattern {
        before,
        conversion,
        after,
    })
}
