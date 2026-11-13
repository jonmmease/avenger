//! GoogleSQL's `CAST(number AS STRING FORMAT '...')`: the number format models and the text they
//! give.
//!
//! The behavior follows GoogleSQL's reference implementation, error messages included. In short:
//!
//! - The digits `0` and `9` print decimal digits, `X` and `x` print hexadecimal digits. Leading
//!   positions print spaces up to the first `0` element and zeros from it on.
//! - Floating-point values round half to even on their exact binary value, as C's `printf` does.
//!   INT64, UINT64, NUMERIC and BIGNUMERIC values round half away from zero. Hexadecimal output
//!   rounds every type half away from zero.
//! - Without a sign element, one position before the number is kept for a `-`.
//! - When the integer part has more digits than the format, every digit prints as `#`.
//! - `TM`, `TM9`, `TME` and `RN` parse, but formatting with them is an error, as in GoogleSQL.

use datafusion::arrow::array::ArrayRef;

/// A number to format: GoogleSQL's INT64, UINT64, FLOAT64 or FLOAT32, or a NUMERIC or BIGNUMERIC
/// given as its exact decimal text.
pub(crate) enum Number<'a> {
    Int64(i64),
    Uint64(u64),
    Float64(f64),
    Float32(f32),
    /// The exact text of a NUMERIC or BIGNUMERIC, such as "-123.450000000" (sign, digits, an
    /// optional fraction), as Arrow's decimal display writes it.
    Decimal(&'a str),
}

/// The largest number of digits after the decimal point (or `V`) that GoogleSQL formats.
const MAX_SCALE: usize = 1074;

/// The largest number of hexadecimal digits in a format.
const MAX_HEX_DIGITS: usize = 16;

/// The case of a letter that an element prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Upper,
    Lower,
}

/// One element of a format string, as the tokenizer reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Element {
    /// `0`: a digit that prints leading zeros.
    Zero,
    /// `9`: a digit that prints leading spaces.
    Nine,
    /// `X` or `x`: a hexadecimal digit.
    Hex(Case),
    /// `.`: the decimal point.
    Point,
    /// `D`: the decimal point of the locale.
    LocalePoint,
    /// `V`: a decimal point that prints nothing.
    Shift,
    /// `,`: the group separator.
    Comma,
    /// `G`: the group separator of the locale.
    LocaleGroup,
    /// `S`: an explicit `+` or `-`.
    Sign,
    /// `MI`: a trailing `-` or space.
    Minus,
    /// `PR`: angle brackets around negative numbers.
    Brackets,
    /// `$`: a dollar sign.
    Dollar,
    /// `C` or `c`: the ISO-4217 currency code of the locale.
    CurrencyCode(Case),
    /// `L`: the currency symbol of the locale.
    CurrencySymbol,
    /// `B`: spaces when the integer part is zero.
    Blank,
    /// `EEEE`: scientific notation, with the case of its first letter.
    Exponent(Case),
    /// `FM`: no spaces and no trailing zeros.
    FillMode,
    /// `TM`, `TM9` and `TME`: text minimal output.
    TextMinimal(&'static str),
    /// `RN`: Roman numerals.
    Roman,
}

impl Element {
    /// The element's name in GoogleSQL's error messages.
    fn name(self) -> &'static str {
        match self {
            Element::Zero => "0",
            Element::Nine => "9",
            Element::Hex(_) => "X",
            Element::Point => ".",
            Element::LocalePoint => "D",
            Element::Shift => "V",
            Element::Comma => ",",
            Element::LocaleGroup => "G",
            Element::Sign => "S",
            Element::Minus => "MI",
            Element::Brackets => "PR",
            Element::Dollar => "$",
            Element::CurrencyCode(_) => "C",
            Element::CurrencySymbol => "L",
            Element::Blank => "B",
            Element::Exponent(_) => "EEEE",
            Element::FillMode => "FM",
            Element::TextMinimal(name) => name,
            Element::Roman => "RN",
        }
    }

    /// Whether the element is a decimal digit or a hexadecimal digit.
    fn is_digit(self) -> bool {
        matches!(self, Element::Zero | Element::Nine | Element::Hex(_))
    }
}

/// The elements of more than one letter, longest first where one starts another.
const LONG_ELEMENTS: [(&str, Element); 8] = [
    ("TME", Element::TextMinimal("TME")),
    ("TM9", Element::TextMinimal("TM9")),
    ("TM", Element::TextMinimal("TM")),
    ("EEEE", Element::Exponent(Case::Upper)),
    ("MI", Element::Minus),
    ("PR", Element::Brackets),
    ("FM", Element::FillMode),
    ("RN", Element::Roman),
];

/// The element that starts `format`, and its length in bytes, or the message for an invalid one.
/// Letters match in any case.
fn next_element(format: &str) -> Result<(Element, usize), String> {
    let bytes = format.as_bytes();
    for (text, element) in LONG_ELEMENTS {
        if bytes.len() >= text.len() && bytes[..text.len()].eq_ignore_ascii_case(text.as_bytes()) {
            let element = match element {
                Element::Exponent(_) if bytes[0] == b'e' => Element::Exponent(Case::Lower),
                element => element,
            };
            return Ok((element, text.len()));
        }
    }
    let element = match bytes[0] {
        b'0' => Element::Zero,
        b'9' => Element::Nine,
        b'X' => Element::Hex(Case::Upper),
        b'x' => Element::Hex(Case::Lower),
        b'.' => Element::Point,
        b'D' | b'd' => Element::LocalePoint,
        b'V' | b'v' => Element::Shift,
        b',' => Element::Comma,
        b'G' | b'g' => Element::LocaleGroup,
        b'S' | b's' => Element::Sign,
        b'$' => Element::Dollar,
        b'C' => Element::CurrencyCode(Case::Upper),
        b'c' => Element::CurrencyCode(Case::Lower),
        b'L' | b'l' => Element::CurrencySymbol,
        b'B' | b'b' => Element::Blank,
        _ => {
            // GoogleSQL names the first byte; a whole character is the closest valid text.
            let invalid = format.chars().next().unwrap_or_default();
            return Err(format_error(&format!("Invalid format element '{invalid}'")));
        }
    };
    Ok((element, 1))
}

/// A message about the format string, with GoogleSQL's prefix.
fn format_error(message: &str) -> String {
    format!("Error in format string: {message}")
}

/// The message for two elements that cannot appear in one format.
fn together(first: &str, second: Element) -> String {
    format_error(&format!(
        "'{first}' cannot appear together with '{}'",
        second.name()
    ))
}

/// The message for a text minimal element with other elements.
const TEXT_MINIMAL_COMBINED: &str =
    "'TM', 'TM9' or 'TME' cannot be combined with other format elements";

/// The part of the format that the parser reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Before a decimal point: digits, group separators and a leading sign.
    Integer,
    /// After a decimal point or `V`: the fraction digits.
    Fraction,
    /// After `EEEE`.
    Exponent,
    /// After a trailing sign.
    End,
}

/// A position of the integer part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A digit. `case` is the case of a hexadecimal digit, which a `0` takes from the next `X`.
    Digit { zero: bool, case: Case },
    /// A group separator.
    Separator,
}

/// Where the sign of the number prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignStyle {
    /// No sign element: one position before the number for `-` or a space.
    Reserved,
    /// `S` before the digits.
    Leading,
    /// `S` after the digits.
    Trailing,
    /// `MI`.
    Minus,
    /// `PR`.
    Brackets,
}

/// What the integer part of a format prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Radix {
    Decimal,
    Hexadecimal,
}

/// The decimal point of a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecimalPoint {
    /// `.` or `D`: prints `.`.
    Visible,
    /// `V`: prints nothing.
    Hidden,
}

/// A parsed number format model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumberFormat {
    radix: Radix,
    /// The integer part, most significant position first.
    integer: Vec<Slot>,
    /// The index in `integer` of the first `0` digit: positions from it on print leading zeros.
    first_zero: Option<usize>,
    decimal_point: Option<DecimalPoint>,
    /// The number of digits after the decimal point.
    scale: usize,
    /// The case of the `E` of scientific notation, when the format has `EEEE`.
    exponent: Option<Case>,
    sign: SignStyle,
    /// The text of the currency element.
    currency: Option<&'static str>,
    /// `B`: spaces when the integer part is zero.
    blank_zero: bool,
    /// `FM`: no spaces and no trailing zeros of the fraction.
    fill_mode: bool,
}

/// The state of the parser between elements.
struct Parser {
    phase: Phase,
    /// Whether a digit, separator, decimal point or leading sign has been read.
    has_elements: bool,
    /// The number of digits in the integer part and the fraction.
    digits: usize,
    has_nine: bool,
    hex: bool,
    separator: Option<Element>,
    decimal_point: Option<Element>,
    sign: Option<(Element, bool)>,
    currency: Option<Element>,
    blank_zero: bool,
    fill_mode: bool,
    text_minimal: bool,
    roman: bool,
    exponent: Option<Case>,
    /// The integer part, with the case of each hexadecimal digit, `0` as `None`.
    integer: Vec<(Slot, Option<Case>)>,
    scale: usize,
}

impl Parser {
    fn new() -> Parser {
        Parser {
            phase: Phase::Integer,
            has_elements: false,
            digits: 0,
            has_nine: false,
            hex: false,
            separator: None,
            decimal_point: None,
            sign: None,
            currency: None,
            blank_zero: false,
            fill_mode: false,
            text_minimal: false,
            roman: false,
            exponent: None,
            integer: Vec::new(),
            scale: 0,
        }
    }

    /// Reads one element.
    fn element(&mut self, element: Element) -> Result<(), String> {
        // `FM`, the currency elements and `B` may appear anywhere, once each.
        match element {
            Element::FillMode => {
                if self.fill_mode {
                    return Err(format_error("'FM' cannot be repeated"));
                }
                self.fill_mode = true;
                return Ok(());
            }
            Element::Dollar | Element::CurrencyCode(_) | Element::CurrencySymbol => {
                if self.currency.is_some() {
                    return Err(format_error("There can be at most one of '$', 'C' or 'L'"));
                }
                self.currency = Some(element);
                return Ok(());
            }
            Element::Blank => {
                if self.blank_zero {
                    return Err(format_error("There can be at most one 'B'"));
                }
                self.blank_zero = true;
                return Ok(());
            }
            _ => {}
        }
        if self.text_minimal {
            return Err(format_error(TEXT_MINIMAL_COMBINED));
        }
        if self.roman {
            return Err(together("RN", element));
        }
        match self.phase {
            Phase::End => self.after_sign(element),
            Phase::Exponent => self.after_exponent(element),
            Phase::Fraction => self.fraction(element),
            Phase::Integer => self.integer(element),
        }
    }

    /// Reads an element after a trailing sign.
    fn after_sign(&mut self, element: Element) -> Result<(), String> {
        if element.is_digit() || matches!(element, Element::Exponent(_)) {
            let message = match self.sign {
                Some((Element::Sign, _)) => {
                    "'S' can only appear before or after all digits and 'EEEE'".to_string()
                }
                Some((sign, _)) => format!(
                    "'{}' can only appear after all digits and 'EEEE'",
                    sign.name()
                ),
                None => unreachable!("the end phase follows a sign"),
            };
            return Err(format_error(&message));
        }
        Err(format_error(&format!(
            "Unexpected format element '{}'",
            element.name()
        )))
    }

    /// Reads an element after `EEEE`.
    fn after_exponent(&mut self, element: Element) -> Result<(), String> {
        match element {
            Element::Comma | Element::LocaleGroup => Err(format_error(
                "',' or 'G' cannot appear together with 'EEEE'",
            )),
            Element::Sign | Element::Minus | Element::Brackets => self.trailing_sign(element),
            _ => Err(format_error(&format!(
                "'{}' cannot appear after 'EEEE'",
                element.name()
            ))),
        }
    }

    /// Reads an element after the decimal point.
    fn fraction(&mut self, element: Element) -> Result<(), String> {
        match element {
            Element::Zero | Element::Nine => {
                self.digits += 1;
                self.scale += 1;
                self.has_elements = true;
                Ok(())
            }
            Element::Hex(_) => Err(together(
                "X",
                self.decimal_point.expect("the fraction follows a point"),
            )),
            Element::Point | Element::LocalePoint | Element::Shift => {
                Err(format_error("There can be at most one of '.', 'D', or 'V'"))
            }
            Element::Comma | Element::LocaleGroup => Err(format_error(
                "',' or 'G' cannot appear after '.', 'D' or 'V'",
            )),
            Element::Exponent(case) => self.exponent(case),
            Element::Sign | Element::Minus | Element::Brackets => self.trailing_sign(element),
            _ => Err(format_error(&format!("Unexpected '{}'", element.name()))),
        }
    }

    /// Reads an element before the decimal point.
    fn integer(&mut self, element: Element) -> Result<(), String> {
        match element {
            Element::Zero | Element::Nine => {
                if self.hex && element == Element::Nine {
                    return Err(together("X", Element::Nine));
                }
                self.has_nine |= element == Element::Nine;
                let zero = element == Element::Zero;
                self.integer.push((
                    Slot::Digit {
                        zero,
                        case: Case::Upper,
                    },
                    None,
                ));
                self.digits += 1;
                self.has_elements = true;
                Ok(())
            }
            Element::Hex(case) => {
                if self.has_nine {
                    return Err(together("X", Element::Nine));
                }
                if self.separator.is_some() {
                    return Err(format_error("'X' cannot appear together with ',' or 'G'"));
                }
                self.hex = true;
                self.integer
                    .push((Slot::Digit { zero: false, case }, Some(case)));
                self.digits += 1;
                self.has_elements = true;
                Ok(())
            }
            Element::Point | Element::LocalePoint | Element::Shift => {
                if self.hex {
                    return Err(together("X", element));
                }
                self.decimal_point = Some(element);
                self.phase = Phase::Fraction;
                self.has_elements = true;
                Ok(())
            }
            Element::Comma | Element::LocaleGroup => {
                if !self.has_elements {
                    return Err(format_error(&format!("Unexpected '{}'", element.name())));
                }
                if self.hex {
                    return Err(together("X", element));
                }
                self.separator.get_or_insert(element);
                self.integer.push((Slot::Separator, None));
                Ok(())
            }
            Element::Sign | Element::Minus | Element::Brackets => {
                if self.sign.is_some() {
                    return Err(format_error(
                        "There can be at most one of 'S', 'MI', or 'PR'",
                    ));
                }
                if self.digits > 0 {
                    return self.trailing_sign(element);
                }
                if element != Element::Sign {
                    return Err(format_error(&format!(
                        "'{}' can only appear after all digits and 'EEEE'",
                        element.name()
                    )));
                }
                self.sign = Some((element, true));
                self.has_elements = true;
                Ok(())
            }
            Element::Exponent(case) => {
                if !self.has_elements {
                    return Err(format_error("Unexpected 'EEEE'"));
                }
                if self.hex {
                    return Err(together("X", element));
                }
                self.exponent(case)
            }
            Element::TextMinimal(_) => {
                if self.hex {
                    return Err(together("X", element));
                }
                if self.has_elements {
                    return Err(format_error(TEXT_MINIMAL_COMBINED));
                }
                self.text_minimal = true;
                Ok(())
            }
            Element::Roman => {
                if self.hex {
                    return Err(together("X", element));
                }
                if self.has_elements {
                    return Err(format_error("Unexpected 'RN'"));
                }
                self.roman = true;
                Ok(())
            }
            Element::FillMode
            | Element::Dollar
            | Element::CurrencyCode(_)
            | Element::CurrencySymbol
            | Element::Blank => unreachable!("read before the phases"),
        }
    }

    /// Reads `EEEE` after the digits.
    fn exponent(&mut self, case: Case) -> Result<(), String> {
        if self.separator.is_some() {
            return Err(format_error(
                "',' or 'G' cannot appear together with 'EEEE'",
            ));
        }
        self.exponent = Some(case);
        self.phase = Phase::Exponent;
        Ok(())
    }

    /// Reads a sign after the digits.
    fn trailing_sign(&mut self, element: Element) -> Result<(), String> {
        if self.sign.is_some() {
            return Err(format_error(
                "There can be at most one of 'S', 'MI', or 'PR'",
            ));
        }
        self.sign = Some((element, false));
        self.phase = Phase::End;
        Ok(())
    }

    /// The format, after the checks that need the whole format string.
    fn finish(self) -> Result<NumberFormat, String> {
        if self.text_minimal {
            if self.currency.is_some() || self.blank_zero || self.fill_mode {
                return Err(format_error(TEXT_MINIMAL_COMBINED));
            }
            return Err("Text minimal output is not supported yet".to_string());
        }
        if self.roman {
            if let Some(currency) = self.currency {
                return Err(together("RN", currency));
            }
            if self.blank_zero {
                return Err(together("RN", Element::Blank));
            }
            return Err("Roman numeral is not supported yet".to_string());
        }
        if self.hex {
            if let Some(currency) = self.currency {
                return Err(together("X", currency));
            }
            if self.blank_zero {
                return Err(together("X", Element::Blank));
            }
            if self.digits > MAX_HEX_DIGITS {
                return Err(format_error("Max number of 'X' is 16"));
            }
        }
        if self.digits == 0 {
            return Err(format_error(
                "Format string must contain at least one of 'X', '0' or '9'",
            ));
        }

        // A `0` prints a hexadecimal letter in the case of the next `X`, or in upper case.
        let mut integer = Vec::with_capacity(self.integer.len());
        let mut next_case = Case::Upper;
        for &(slot, case) in self.integer.iter().rev() {
            let slot = match slot {
                Slot::Digit { zero, .. } => {
                    if let Some(case) = case {
                        next_case = case;
                    }
                    Slot::Digit {
                        zero,
                        case: next_case,
                    }
                }
                Slot::Separator => Slot::Separator,
            };
            integer.push(slot);
        }
        integer.reverse();
        let first_zero = integer
            .iter()
            .position(|slot| matches!(slot, Slot::Digit { zero: true, .. }));
        let sign = match self.sign {
            None => SignStyle::Reserved,
            Some((Element::Sign, true)) => SignStyle::Leading,
            Some((Element::Sign, false)) => SignStyle::Trailing,
            Some((Element::Minus, _)) => SignStyle::Minus,
            Some((_, _)) => SignStyle::Brackets,
        };
        let currency = self.currency.map(|currency| match currency {
            Element::CurrencyCode(Case::Upper) => "USD",
            Element::CurrencyCode(Case::Lower) => "usd",
            _ => "$",
        });
        let decimal_point = self.decimal_point.map(|point| match point {
            Element::Shift => DecimalPoint::Hidden,
            _ => DecimalPoint::Visible,
        });
        Ok(NumberFormat {
            radix: if self.hex {
                Radix::Hexadecimal
            } else {
                Radix::Decimal
            },
            integer,
            first_zero,
            decimal_point,
            scale: self.scale,
            exponent: self.exponent,
            sign,
            currency,
            blank_zero: self.blank_zero,
            fill_mode: self.fill_mode,
        })
    }
}

/// The sign that a value prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signum {
    Positive,
    Negative,
    /// NaN prints no sign in decimal output.
    None,
}

/// The magnitude of a value.
enum Magnitude {
    /// A finite floating-point value; Rust's formatting rounds it as `printf` does.
    Float(f64),
    /// An exact decimal: the digits before and after the decimal point, without a sign.
    Exact {
        integer: String,
        fraction: String,
    },
    Infinite,
    NaN,
}

/// A value to format: its sign and magnitude.
struct Value {
    negative: bool,
    magnitude: Magnitude,
}

impl Value {
    fn new(number: Number) -> Result<Value, String> {
        Ok(match number {
            Number::Int64(value) => Value {
                negative: value < 0,
                magnitude: Magnitude::Exact {
                    integer: value.unsigned_abs().to_string(),
                    fraction: String::new(),
                },
            },
            Number::Uint64(value) => Value {
                negative: false,
                magnitude: Magnitude::Exact {
                    integer: value.to_string(),
                    fraction: String::new(),
                },
            },
            Number::Float64(value) => Value::float(value),
            Number::Float32(value) => Value::float(f64::from(value)),
            Number::Decimal(text) => Value::decimal(text)?,
        })
    }

    fn float(value: f64) -> Value {
        let magnitude = if value.is_nan() {
            Magnitude::NaN
        } else if value.is_infinite() {
            Magnitude::Infinite
        } else {
            Magnitude::Float(value.abs())
        };
        Value {
            negative: value < 0.0,
            magnitude,
        }
    }

    /// The value of a NUMERIC or BIGNUMERIC's text.
    fn decimal(text: &str) -> Result<Value, String> {
        let invalid = || format!("Invalid NUMERIC or BIGNUMERIC text: {text}");
        let (negative, unsigned) = match text.as_bytes().first() {
            Some(b'-') => (true, &text[1..]),
            Some(b'+') => (false, &text[1..]),
            _ => (false, text),
        };
        let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
        if integer.is_empty() && fraction.is_empty()
            || !all_digits(integer)
            || !all_digits(fraction)
        {
            return Err(invalid());
        }
        let integer = integer.trim_start_matches('0');
        let integer = if integer.is_empty() { "0" } else { integer };
        let fraction = fraction.trim_end_matches('0');
        let zero = integer == "0" && fraction.is_empty();
        Ok(Value {
            negative: negative && !zero,
            magnitude: Magnitude::Exact {
                integer: integer.to_string(),
                fraction: fraction.to_string(),
            },
        })
    }

    /// The sign that decimal output prints for the value.
    fn signum(&self) -> Signum {
        match self.magnitude {
            Magnitude::NaN => Signum::None,
            _ if self.negative => Signum::Negative,
            _ => Signum::Positive,
        }
    }
}

/// Adds one to the last place of the decimal digits `digits`; true when that carries out of them.
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

/// An exact magnitude rounded half away from zero to `scale` fraction digits: the integer digits
/// (`"0"` for zero) and exactly `scale` fraction digits.
fn round_fixed(integer: &str, fraction: &str, scale: usize) -> (String, String) {
    let mut digits: Vec<u8> = integer.bytes().collect();
    digits.extend(fraction.bytes().take(scale));
    digits.resize(integer.len() + scale, b'0');
    if fraction
        .as_bytes()
        .get(scale)
        .is_some_and(|&digit| digit >= b'5')
        && increment(&mut digits)
    {
        digits.insert(0, b'1');
    }
    let split = digits.len() - scale;
    let fraction = String::from_utf8(digits.split_off(split)).expect("ASCII digits");
    let integer = String::from_utf8(digits).expect("ASCII digits");
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    (integer.to_string(), fraction)
}

/// An exact magnitude rounded half away from zero to `scale + 1` significant digits: the
/// mantissa's integer digit, its `scale` fraction digits, and the exponent.
fn round_scientific(integer: &str, fraction: &str, scale: usize) -> (u8, String, i64) {
    let integer = integer.trim_start_matches('0');
    let all: Vec<u8> = integer.bytes().chain(fraction.bytes()).collect();
    let Some(first) = all.iter().position(|&digit| digit != b'0') else {
        return (b'0', "0".repeat(scale), 0);
    };
    let mut exponent = integer.len() as i64 - 1 - first as i64;
    let significant = &all[first..];
    let mut digits: Vec<u8> = significant.iter().copied().take(scale + 1).collect();
    digits.resize(scale + 1, b'0');
    if significant
        .get(scale + 1)
        .is_some_and(|&digit| digit >= b'5')
        && increment(&mut digits)
    {
        digits.insert(0, b'1');
        digits.pop();
        exponent += 1;
    }
    let fraction = String::from_utf8(digits[1..].to_vec()).expect("ASCII digits");
    (digits[0], fraction, exponent)
}

/// A finite magnitude rounded half away from zero to an integer, when it is below 2^64.
fn round_integer(magnitude: &Magnitude) -> Option<u64> {
    match magnitude {
        Magnitude::Float(value) => {
            let rounded = value.round();
            (rounded < 18_446_744_073_709_551_616.0).then_some(rounded as u64)
        }
        Magnitude::Exact { integer, fraction } => {
            let (integer, _) = round_fixed(integer, fraction, 0);
            integer.parse().ok()
        }
        Magnitude::Infinite | Magnitude::NaN => None,
    }
}

impl NumberFormat {
    /// The format model `format`, or GoogleSQL's error message for an invalid one.
    pub(crate) fn parse(format: &str) -> Result<NumberFormat, String> {
        let mut parser = Parser::new();
        let mut rest = format;
        while !rest.is_empty() {
            let (element, length) = next_element(rest)?;
            parser.element(element)?;
            rest = &rest[length..];
        }
        parser.finish()
    }

    /// `value` formatted, or GoogleSQL's error message where formatting fails.
    pub(crate) fn format(&self, value: Number) -> Result<String, String> {
        if self.scale > MAX_SCALE {
            return Err(format!("Precision value is too big{MAX_SCALE}"));
        }
        let value = Value::new(value)?;
        let text = match self.radix {
            Radix::Hexadecimal => self.hexadecimal(&value),
            Radix::Decimal if self.exponent.is_some() => self.scientific(&value),
            Radix::Decimal => self.fixed(&value),
        };
        Ok(if self.fill_mode {
            text.replace(' ', "")
        } else {
            text
        })
    }

    /// The number of digits in the integer part.
    fn integer_digits(&self) -> usize {
        self.integer
            .iter()
            .filter(|slot| matches!(slot, Slot::Digit { .. }))
            .count()
    }

    /// The text of the decimal point.
    fn point(&self) -> &'static str {
        match self.decimal_point {
            Some(DecimalPoint::Visible) => ".",
            Some(DecimalPoint::Hidden) | None => "",
        }
    }

    /// The fraction digits, without trailing zeros under `FM`.
    fn fraction<'a>(&self, fraction: &'a str) -> &'a str {
        if self.fill_mode {
            fraction.trim_end_matches('0')
        } else {
            fraction
        }
    }

    /// The integer part with `digits` right-aligned in it. Positions before the digits print
    /// zeros from the first `0` element on and spaces before it; a separator prints only after a
    /// printed digit. `hex` sets the case of hexadecimal letters by position.
    fn integer_part(&self, digits: &[u8], hex: bool) -> Vec<u8> {
        let positions = self.integer_digits();
        let mut out = Vec::with_capacity(self.integer.len());
        let mut digit_index = 0;
        let mut printed = false;
        for (index, slot) in self.integer.iter().enumerate() {
            match *slot {
                Slot::Digit { case, .. } => {
                    let from_end = positions - digit_index;
                    digit_index += 1;
                    let byte = if from_end <= digits.len() {
                        let digit = digits[digits.len() - from_end];
                        if hex && case == Case::Lower {
                            digit.to_ascii_lowercase()
                        } else {
                            digit
                        }
                    } else if self.first_zero.is_some_and(|first| index >= first) {
                        b'0'
                    } else {
                        b' '
                    };
                    printed |= byte != b' ';
                    out.push(byte);
                }
                Slot::Separator => out.push(if printed { b',' } else { b' ' }),
            }
        }
        out
    }

    /// The output from the text of the number, `body`: its leading spaces, then the sign and
    /// currency, then the rest of it and the trailing sign.
    fn assemble(&self, body: &[u8], signum: Signum) -> String {
        let rest_start = body
            .iter()
            .position(|&byte| byte != b' ')
            .unwrap_or(body.len());
        let (lead, rest) = body.split_at(rest_start);
        let mut out = String::with_capacity(body.len() + 8);
        out.push_str(std::str::from_utf8(lead).expect("ASCII"));
        let (prefix, suffix) = match (self.sign, signum) {
            (SignStyle::Reserved, Signum::Negative) => ("-", ""),
            (SignStyle::Reserved, _) => (" ", ""),
            (SignStyle::Leading, Signum::Negative) => ("-", ""),
            (SignStyle::Leading, Signum::Positive) => ("+", ""),
            (SignStyle::Leading, Signum::None) => (" ", ""),
            (SignStyle::Trailing, Signum::Negative) => ("", "-"),
            (SignStyle::Trailing, Signum::Positive) => ("", "+"),
            (SignStyle::Trailing, Signum::None) => ("", " "),
            (SignStyle::Minus, Signum::Negative) => ("", "-"),
            (SignStyle::Minus, _) => ("", " "),
            (SignStyle::Brackets, Signum::Negative) => ("<", ">"),
            (SignStyle::Brackets, _) => (" ", " "),
        };
        out.push_str(prefix);
        out.push_str(self.currency.unwrap_or(""));
        out.push_str(std::str::from_utf8(rest).expect("ASCII"));
        out.push_str(suffix);
        out
    }

    /// The text of a value whose integer part does not fit: `#` for every digit.
    fn overflow(&self, signum: Signum, exponent: bool) -> String {
        let mut body: Vec<u8> = self
            .integer
            .iter()
            .map(|slot| match slot {
                Slot::Digit { .. } => b'#',
                Slot::Separator => b',',
            })
            .collect();
        body.extend_from_slice(self.point().as_bytes());
        body.resize(body.len() + self.scale + if exponent { 4 } else { 0 }, b'#');
        self.assemble(&body, signum)
    }

    /// Decimal output without an exponent.
    fn fixed(&self, value: &Value) -> String {
        let signum = value.signum();
        let (integer, fraction) = match &value.magnitude {
            Magnitude::Infinite | Magnitude::NaN => {
                // INF and NAN fill the right of the integer part, when it is three wide.
                let width = self.integer.len();
                if width < 3 {
                    return self.overflow(signum, false);
                }
                let word: &[u8] = match value.magnitude {
                    Magnitude::NaN => b"NAN",
                    _ => b"INF",
                };
                let mut body = vec![b' '; width - 3];
                body.extend_from_slice(word);
                body.resize(body.len() + self.point().len() + self.scale, b' ');
                return self.assemble(&body, signum);
            }
            Magnitude::Float(magnitude) => {
                let text = format!("{magnitude:.*}", self.scale);
                match text.split_once('.') {
                    Some((integer, fraction)) => (integer.to_string(), fraction.to_string()),
                    None => (text, String::new()),
                }
            }
            Magnitude::Exact { integer, fraction } => round_fixed(integer, fraction, self.scale),
        };
        let zero = integer == "0";
        let fraction = self.fraction(&fraction);
        // A zero integer part prints as one `0` only when no fraction digit prints.
        let digits = if zero && (!fraction.is_empty() || self.integer_digits() == 0) {
            ""
        } else {
            integer.as_str()
        };
        if digits.len() > self.integer_digits() {
            return self.overflow(signum, false);
        }
        let mut body = self.integer_part(digits.as_bytes(), false);
        body.extend_from_slice(self.point().as_bytes());
        body.extend_from_slice(fraction.as_bytes());
        let text = self.assemble(&body, signum);
        // `B` turns the whole output into spaces when the integer part is zero.
        if self.blank_zero && zero {
            return " ".repeat(text.len());
        }
        text
    }

    /// Decimal output in scientific notation, for `EEEE`.
    fn scientific(&self, value: &Value) -> String {
        let signum = value.signum();
        let (digit, fraction, exponent) = match &value.magnitude {
            Magnitude::Infinite | Magnitude::NaN => {
                // INF and NAN fill the right of the mantissa and the exponent; any decimal point
                // counts as one position.
                let word: &[u8] = match value.magnitude {
                    Magnitude::NaN => b"NAN",
                    _ => b"INF",
                };
                let width = self.integer_digits().min(1)
                    + usize::from(self.decimal_point.is_some())
                    + self.scale
                    + 4;
                let mut body = vec![b' '; width.saturating_sub(3)];
                body.extend_from_slice(word);
                return self.assemble(&body, signum);
            }
            Magnitude::Float(magnitude) => {
                let text = format!("{magnitude:.*e}", self.scale);
                let (mantissa, exponent) = text.split_once('e').expect("an exponent");
                let fraction = mantissa.get(2..).unwrap_or("").to_string();
                let exponent: i64 = exponent.parse().expect("an integer exponent");
                (mantissa.as_bytes()[0], fraction, exponent)
            }
            Magnitude::Exact { integer, fraction } => {
                round_scientific(integer, fraction, self.scale)
            }
        };
        let mut body = Vec::new();
        if self.integer_digits() > 0 {
            body.push(digit);
        } else if digit != b'0' {
            return self.overflow(signum, true);
        }
        body.extend_from_slice(self.point().as_bytes());
        body.extend_from_slice(self.fraction(&fraction).as_bytes());
        let letter = match self.exponent {
            Some(Case::Lower) => 'e',
            _ => 'E',
        };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        let exponent = format!("{letter}{exponent_sign}{:02}", exponent.unsigned_abs());
        body.extend_from_slice(exponent.as_bytes());
        let text = self.assemble(&body, signum);
        // `B` turns the whole output into spaces when the mantissa is zero.
        if self.blank_zero && digit == b'0' {
            return " ".repeat(text.len());
        }
        text
    }

    /// Hexadecimal output: the value rounded half away from zero, with its sign.
    fn hexadecimal(&self, value: &Value) -> String {
        // NaN prints as a positive number.
        let signum = if value.negative {
            Signum::Negative
        } else {
            Signum::Positive
        };
        let digits = round_integer(&value.magnitude).map(|integer| format!("{integer:X}"));
        match digits {
            Some(digits) if digits.len() <= self.integer_digits() => {
                let body = self.integer_part(digits.as_bytes(), true);
                self.assemble(&body, signum)
            }
            _ => self.overflow(signum, false),
        }
    }
}

/// The text of the number at `row` of `array`, an integer, a float or a decimal array, in
/// `format`.
pub(crate) fn format_row(
    format: &NumberFormat,
    array: &ArrayRef,
    row: usize,
) -> Result<String, String> {
    use datafusion::arrow::{array::AsArray, datatypes::*};
    let number = match array.data_type() {
        DataType::Int64 => Number::Int64(array.as_primitive::<Int64Type>().value(row)),
        DataType::UInt64 => Number::Uint64(array.as_primitive::<UInt64Type>().value(row)),
        DataType::Float64 => Number::Float64(array.as_primitive::<Float64Type>().value(row)),
        DataType::Float32 => Number::Float32(array.as_primitive::<Float32Type>().value(row)),
        DataType::Decimal128(_, scale) => {
            let text = Decimal128Type::format_decimal(
                array.as_primitive::<Decimal128Type>().value(row),
                38,
                *scale,
            );
            return format.format(Number::Decimal(&text));
        }
        DataType::Decimal256(_, scale) => {
            let text = Decimal256Type::format_decimal(
                array.as_primitive::<Decimal256Type>().value(row),
                76,
                *scale,
            );
            return format.format(Number::Decimal(&text));
        }
        other => return Err(format!("avenger-sql formats numbers, not {other}")),
    };
    format.format(number)
}
