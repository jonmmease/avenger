//! The parse of a pattern of GoogleSQL's `FORMAT` into literal text and elements, and the
//! pattern of a chart: literal text around exactly one numeric conversion.

use avenger_format::NumberFormatError;

/// The conversions that a chart's pattern takes, for the messages of errors.
const CONVERSIONS: &str =
    "one of `%d`, `%i`, `%o`, `%x`, `%X`, `%f`, `%F`, `%e`, `%E`, `%g`, or `%G`";

/// The largest width or precision of a chart's pattern, which keeps a label from asking for a
/// huge buffer.
const LIMIT: u64 = 1_000;

/// The kind of a conversion, as it prints a value.
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
    /// Whether the conversion prints an integer.
    pub(crate) fn is_integer(self) -> bool {
        matches!(self, Kind::Decimal | Kind::Octal | Kind::Hex)
    }

    /// The kind of a numeric specifier, and whether it is uppercase.
    fn of(specifier: char) -> Option<(Kind, bool)> {
        Some(match specifier {
            'd' | 'i' | 'u' => (Kind::Decimal, false),
            'o' => (Kind::Octal, false),
            'x' => (Kind::Hex, false),
            'X' => (Kind::Hex, true),
            'f' => (Kind::Fixed, false),
            'F' => (Kind::Fixed, true),
            'e' => (Kind::Exponent, false),
            'E' => (Kind::Exponent, true),
            'g' => (Kind::General, false),
            'G' => (Kind::General, true),
            _ => return None,
        })
    }
}

/// A conversion with its flags, its width, and its precision, as it prints a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Conversion {
    pub(crate) kind: Kind,
    /// An uppercase specifier: `X`, `F`, `E`, or `G`.
    pub(crate) upper: bool,
    pub(crate) flags: Flags,
    /// The minimum width of the conversion's text.
    pub(crate) width: usize,
    /// The precision, where the pattern writes one.
    pub(crate) precision: Option<usize>,
}

/// The flags of an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Flags {
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
}

/// A width or a precision of an element: written as digits, or `*`, which takes it from the
/// argument before the element's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// A count written as digits, or `u64::MAX` for digits beyond it.
    Written(u64),
    /// `*`.
    Argument,
}

/// An element of a pattern of `FORMAT`: `%`, flags, a width, a precision, and a specifier, which
/// is any character after them, so a caller decides which specifiers it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    /// The specifier, such as `d`, `s`, or `T`, or `%` after flags, a width, or a precision.
    pub specifier: char,
    pub width: Option<Count>,
    pub precision: Option<Count>,
    /// The byte range of the element in the pattern.
    pub start: usize,
    pub end: usize,
    pub(crate) flags: Flags,
}

/// A piece of a pattern of `FORMAT`: literal text, where `%%` is `%`, or an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Element(Element),
}

/// A pattern that ends inside an element, as `abc%` and `%5` do, at the byte where the element
/// starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoneElement {
    pub start: usize,
}

/// Reads the digits at `*index` as a count, saturating at `u64::MAX`.
fn digits(bytes: &[u8], index: &mut usize) -> u64 {
    let mut value = 0_u64;
    while let Some(digit) = bytes.get(*index).filter(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'));
        *index += 1;
    }
    value
}

/// The pieces of a pattern of GoogleSQL's `FORMAT`, which reads an element as `%`, the flags
/// `- + space # 0 '` in any order, a width of digits or `*`, a precision of `.` and digits or `*`,
/// and any character as its specifier. `%%` is literal `%`.
pub fn pieces(pattern: &str) -> Result<Vec<Piece>, LoneElement> {
    let bytes = pattern.as_bytes();
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    while let Some(offset) = pattern[index..].find('%') {
        let start = index + offset;
        text.push_str(&pattern[index..start]);
        index = start + 1;
        if bytes.get(index) == Some(&b'%') {
            text.push('%');
            index += 1;
            continue;
        }
        let mut flags = Flags::default();
        while let Some(&flag) = bytes.get(index) {
            match flag {
                b'-' => flags.left = true,
                b'+' => flags.plus = true,
                b' ' => flags.space = true,
                b'#' => flags.alternate = true,
                b'0' => flags.zero = true,
                b'\'' => flags.group = true,
                _ => break,
            }
            index += 1;
        }
        let count = |index: &mut usize| match bytes.get(*index) {
            Some(b'*') => {
                *index += 1;
                Some(Count::Argument)
            }
            Some(byte) if byte.is_ascii_digit() => Some(Count::Written(digits(bytes, index))),
            _ => None,
        };
        let width = count(&mut index);
        let precision = match bytes.get(index) {
            Some(b'.') => {
                index += 1;
                Some(count(&mut index).unwrap_or(Count::Written(0)))
            }
            _ => None,
        };
        let Some(specifier) = pattern[index..].chars().next() else {
            return Err(LoneElement { start });
        };
        index += specifier.len_utf8();
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        pieces.push(Piece::Element(Element {
            specifier,
            width,
            precision,
            start,
            end: index,
            flags,
        }));
    }
    text.push_str(&pattern[index..]);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

impl Element {
    /// The numeric conversion of the element, with the width and the precision of its `*`
    /// arguments, `width` and `precision`, where it takes them: a negative width pads on the
    /// right, and a negative precision is none. A text specifier, such as `s`, prints a value as
    /// `%g` would, so callers print text with [`Element::format_text`].
    pub(crate) fn conversion(&self, width: Option<i64>, precision: Option<i64>) -> Conversion {
        let mut flags = self.flags;
        let width = match self.width {
            Some(Count::Written(width)) => usize::try_from(width).unwrap_or(usize::MAX),
            Some(Count::Argument) => {
                let width = width.unwrap_or(0);
                flags.left |= width < 0;
                usize::try_from(width.unsigned_abs()).unwrap_or(usize::MAX)
            }
            None => 0,
        };
        let precision = match self.precision {
            Some(Count::Written(precision)) => {
                Some(usize::try_from(precision).unwrap_or(usize::MAX))
            }
            Some(Count::Argument) => precision
                .filter(|&precision| precision >= 0)
                .map(|precision| usize::try_from(precision).unwrap_or(usize::MAX)),
            None => None,
        };
        let (kind, upper) = Kind::of(self.specifier).unwrap_or_default();
        Conversion {
            kind,
            upper,
            flags,
            width,
            precision,
        }
    }
}

/// A pattern of a chart: the text before the conversion, the conversion, and the text after it,
/// with each `%%` read as `%`.
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

/// Parses a chart's `pattern` into literal text and exactly one numeric conversion, with a
/// written width and precision of at most 1,000.
pub(crate) fn parse(pattern: &str) -> Result<Pattern, NumberFormatError> {
    let pieces = pieces(pattern).map_err(|lone| {
        invalid(
            format!(
                "the pattern ends in a lone `{}`; write `%%` for a percent sign, and a \
                 conversion as {CONVERSIONS}",
                &pattern[lone.start..]
            ),
            lone.start,
        )
    })?;
    let mut literals = [String::new(), String::new()];
    let mut conversion: Option<(Conversion, usize)> = None;
    for piece in pieces {
        let element = match piece {
            Piece::Text(text) => {
                literals[usize::from(conversion.is_some())].push_str(&text);
                continue;
            }
            Piece::Element(element) => element,
        };
        let start = element.start;
        let written = &pattern[start..element.end];
        for (count, what) in [(element.width, "width"), (element.precision, "precision")] {
            match count {
                Some(Count::Argument) => {
                    let at = start + written.find('*').expect("a `*`");
                    return Err(invalid(
                        format!(
                            "a `*` {what} takes its value from an argument, which a chart's \
                             pattern does not have; write the {what} as digits, and the \
                             conversion as {CONVERSIONS}"
                        ),
                        at,
                    ));
                }
                Some(Count::Written(value)) if value > LIMIT => {
                    return Err(invalid(
                        format!("the {what} of the conversion is over {LIMIT}"),
                        start,
                    ));
                }
                _ => {}
            }
        }
        if element.specifier == 'u' || Kind::of(element.specifier).is_none() {
            return Err(invalid(
                format!(
                    "`{written}` is not a conversion of a number; a chart's pattern takes \
                     {CONVERSIONS}"
                ),
                start,
            ));
        }
        if let Some((_, first)) = conversion {
            return Err(invalid(
                format!(
                    "`{written}` is a second conversion after the one at byte {first}; a \
                     chart's pattern takes exactly one conversion, {CONVERSIONS}"
                ),
                start,
            ));
        }
        conversion = Some((element.conversion(None, None), start));
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
    let [before, after] = literals;
    Ok(Pattern {
        before,
        conversion,
        after,
    })
}
