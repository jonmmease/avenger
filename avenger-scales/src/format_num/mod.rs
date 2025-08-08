//! Dynamic formatting of numbers into human readable forms.
//!
//! Forked from the unmaintained format_num crate under the Apahce 2.0 and MIT licenses.
//!
//! Did you encounter cases where Rust doesn't represent numbers the way you expect?
//!
//! ```
//! for i in 1..=10 {
//!     println!("{}", 0.1 * i as f64);
//! }
//! ```
//!
//! You get this:
//!
//! ```text
//! 0.1
//! 0.2
//! 0.30000000000000004
//! 0.4
//! 0.5
//! 0.6000000000000001
//! 0.7000000000000001
//! 0.8
//! 0.9
//! 1
//! ```
//!
//! That's actually not a Rust issue, but rather [how floats are represented in binary](https://en.wikipedia.org/wiki/Double-precision_floating-point_format).
//!
//! Yet rounding error is not the only reason to customize number formatting. A table of numbers
//! should be formatted consistently for comparison; above, 1.0 would be better than 1. Large
//! numbers may need to have grouped digits (e.g. 42,000) or be in scientific or metric notation
//! (4.2e+4, 42k). Reported numerical results should be rounded to significant digits (4021 becomes
//! 4000) and so on.
//!
//! The parser is modeled after Python 3's [format specification mini-language](https://docs.python.org/3/library/string.html#format-specification-mini-language)
//! [(PEP3101)](https://www.python.org/dev/peps/pep-3101/) with some minor implementation details changes.
//!
//! The general form of a format specifier is:
//!
//! ```text
//! [[fill]align][sign][symbol][0][width][,][.precision][type]
//! ```
//!
//! The _fill_ can be any character. The presence of a fill character is signaled by the align
//! character following it, which must be one of the following:
//!
//! `>` - Forces the field to be right-aligned within the available space.
//!
//! `<` - Forces the field to be left-aligned within the available space.
//!
//! `^` - Forces the field to be centered within the available space.
//!
//! `=` - like `>`, but with any sign and symbol to the left of any padding.
//!
//! The _sign_ can be:
//!
//! `-` - nothing for zero or positive and a minus sign for negative (default behavior).
//!
//! `+` - a plus sign for zero or positive and a minus sign for negative.
//!
//! ` ` (space) - a space for zero or positive and a minus sign for negative.
//!
//! The _symbol_ can be:
//!
//! The `#` option causes the “alternate form” to be used for the conversion. The alternate
//! form is defined differently for different types. For integers, when binary (`b`), octal
//! (`o` or `O`), or hexadecimal (`x` or `X`) output is used, this option adds the prefix
//! respective "0b", "0o", "0O" or "0x" to the output value. For floats, the alternate form
//! causes the result of the conversion to always contain a decimal-point character,
//! even if no digits follow it.
//!
//! The zero (0) option enables zero-padding; this implicitly sets fill to 0 and align to =.
//!
//! The _width_ defines the minimum field width; if not specified, then the width will be
//! determined by the content.
//!
//! The comma (,) option enables the use of a group separator, such as a comma for thousands.
//!
//! Depending on the _type_, the _precision_ either indicates the number of digits that follow
//! the decimal point (types `f` and `%`), or the number of significant digits (types `e`
//! and `s`). If the precision is not specified, it defaults to 6 for all types. Precision
//! is ignored for integer formats (types `b`, `o`, `d`, `x` and `X`).
//!
//! The available _type_ values are:
//!
//! `e` - exponent notation.
//!
//! `f` - fixed point notation.
//!
//! `s` - decimal notation with an SI prefix, rounded to significant digits.
//!
//! `%` - multiply by 100, and then decimal notation with a percent sign.
//!
//! `b` - binary notation, rounded to integer.
//!
//! `o` - octal notation, rounded to integer.
//!
//! `d` - decimal notation, rounded to integer.
//!
//! `x` - hexadecimal notation, using lower-case letters, rounded to integer.
//!
//! `X` - hexadecimal notation, using upper-case letters, rounded to integer.
//!
//!
//! # Examples
//!
//! ```
//! use avenger_scales::format_num::NumberFormat;
//!
//! let num = NumberFormat::new();
//!
//! assert_eq!(num.format(".1f", 0.06), "0.1");
//! assert_eq!(num.format("#.0f", 10.1), "10."); // float alternate form (always show a decimal point)
//! assert_eq!(num.format("+14d", 2_147_483_647), "   +2147483647");
//! assert_eq!(num.format("#b", 3), "0b11");
//! assert_eq!(num.format("b", 3), "11");
//! assert_eq!(num.format("#X", 48879), "0xBEEF");
//! assert_eq!(num.format(".2s", 42e6), "42M");
//! assert_eq!(num.format(".^20d", 12), ".........12........."); // dot filled and centered
//! assert_eq!(num.format("+10.0f", 255), "      +255");
//! assert_eq!(num.format(".0%", 0.123), "12%");
//! assert_eq!(num.format("+016,.2s", 42e12), "+000,000,000,042T"); // grouped zero-padded with a mandatory sign, SI-prefixed with 2 significant digits
//! ```
//!
//! # Note
//!
//! A current limitation is that the number to be formatted should implement the `Into<f64>`
//! trait. While this covers a broad range of use cases, for big numbers (>u64::MAX) some
//! precision will be lost.
use regex::{Captures, Regex};
use std::cmp::max;

const PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "µ", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

/// A struct that defines the formatting specs and implements the formatting behavior.
///
/// Defines the characters used as a decimal symbol as well as the character used to
/// delimit groups of characters in the integer part of the number.
pub struct NumberFormat {
    decimal: char,
    group_delimiter: char,
}

/// Represents a destructured specification of a provided format pattern string.
#[derive(Debug)]
struct FormatSpec<'a> {
    zero: bool,
    fill: Option<&'a str>,
    align: Option<&'a str>,
    sign: Option<&'a str>,
    symbol: Option<&'a str>,
    width: Option<usize>,
    grouping: Option<&'a str>,
    precision: Option<i32>,
    format_type: Option<&'a str>,
}

impl<'a> From<Captures<'a>> for FormatSpec<'a> {
    /// Create a `FormatSpec` instance from a parsed format pattern string.
    fn from(c: Captures<'a>) -> Self {
        let mut spec = Self {
            fill: c.get(1).map(|m| m.as_str()).or(Some(" ")),
            align: c.get(2).map(|m| m.as_str()),
            sign: c.get(3).map(|m| m.as_str()).or(Some("-")),
            symbol: c.get(4).map(|m| m.as_str()),
            zero: c.get(5).is_some(),
            width: c.get(6).map(|m| m.as_str().parse().unwrap()).or(Some(0)),
            grouping: c.get(7).map(|m| m.as_str()),
            precision: c
                .get(8)
                .map(|m| m.as_str()[1..].parse().unwrap())
                .or(Some(6)),
            format_type: c.get(9).map(|m| m.as_str()),
        };

        // If zero fill is specified, padding goes after sign and before digits.
        if spec.zero
            || (spec.fill.unwrap_or_default() == "0" && spec.align.unwrap_or_default() == "=")
        {
            spec.zero = true;
            spec.fill = Some("0");
            spec.align = Some("=");
        }

        // Ignore precision for decimal notation.
        if spec.format_type.unwrap_or_default() == "d" {
            spec.precision = Some(0);
        };

        spec
    }
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl NumberFormat {
    /// Create a new instance of HumanNumberFormat.
    pub fn new() -> Self {
        Self {
            decimal: '.',
            group_delimiter: ',',
        }
    }

    #[allow(dead_code)]
    fn get_significant_digits(input: &str) -> usize {
        let contains_dot = input.contains(".");
        let mut dot_counted = false;
        let mut insignificant = 0;
        for char in input.chars() {
            match char {
                '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => break,
                '.' => {
                    insignificant += 1;
                    dot_counted = true;
                }
                _ => insignificant += 1,
            }
        }

        if !contains_dot {
            for char in input.chars().rev() {
                match char {
                    '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => break,
                    _ => insignificant += 1,
                }
            }
        }

        input.len() - insignificant - (contains_dot && !dot_counted) as usize
    }

    /// Computes the decimal coefficient and exponent of the specified number `value` with supplied
    /// amount of significant digits. For example, decompose_to_coefficient_and_exponent(1.23, Option<2>)
    /// returns ("12", 0).
    fn decompose_to_coefficient_and_exponent(
        &self,
        value: f64,
        significant_digits: Option<usize>,
    ) -> (String, isize) {
        // Use exponential formatting to get the expected number of significant digits.
        let formatted_value = if significant_digits.is_some() {
            let precision = if significant_digits.unwrap() == 0 {
                0
            } else {
                significant_digits.unwrap() - 1
            };
            format!("{value:.precision$e}")
        } else {
            format!("{value:e}")
        };

        let exp_tokens: Vec<&str> = formatted_value.split('e').collect::<Vec<&str>>();
        let exponent = exp_tokens[1].parse().unwrap();

        // The `formatted_num` can have 2 shapes: `1e2` and `1.2e2`. Remove the decimal character
        // in case it's in the latter form.
        if exp_tokens[0].len() == 1 {
            (exp_tokens[0].to_owned(), exponent)
        } else {
            let dot_idx = exp_tokens[0]
                .chars()
                .position(|c| c == self.decimal)
                .unwrap();
            (
                format!(
                    "{}{}",
                    &exp_tokens[0][..dot_idx],
                    &exp_tokens[0][dot_idx + 1..]
                ),
                exponent,
            )
        }
    }

    /// Compute the [SI prefix](https://en.wikipedia.org/wiki/Metric_prefix) of the number and scale it accordingly.
    fn format_si_prefix(&self, value: f64, precision: Option<i32>) -> (String, isize) {
        let (coefficient, exponent) =
            self.decompose_to_coefficient_and_exponent(value, precision.map(|p| p as usize));
        let prefix_exponent = ((exponent as f32 / 3_f32).floor() as isize).clamp(-8, 8);
        let i: isize = exponent - prefix_exponent * 3 + 1;
        let n: isize = coefficient.len() as isize;

        if i == n {
            (coefficient, prefix_exponent)
        } else if i > n {
            (
                format!(
                    "{}{}",
                    coefficient,
                    std::iter::repeat_n("0", (i - n) as usize).collect::<String>()
                ),
                prefix_exponent,
            )
        } else if i > 0 {
            (
                format!(
                    "{}{}{}",
                    &coefficient[..i as usize],
                    self.decimal,
                    &coefficient[i as usize..]
                ),
                prefix_exponent,
            )
        } else {
            // less than 1 yocto
            (
                format!(
                    "0{}{}{}",
                    self.decimal,
                    std::iter::repeat_n("0", i.unsigned_abs()).collect::<String>(),
                    self.decompose_to_coefficient_and_exponent(
                        value,
                        precision.and(Some(max(
                            0,
                            precision
                                .map(|p| (p - i.abs() as i32 - 1) as usize)
                                .unwrap()
                        )))
                    )
                    .0
                ),
                prefix_exponent,
            )
        }
    }

    /// Parse the formatting pattern and return a format specification based on the pattern.
    ///
    /// The parser is modeled after Python 3's [format specification mini-language](https://docs.python.org/3/library/string.html#format-specification-mini-language)
    /// [(PEP3101)](https://www.python.org/dev/peps/pep-3101/) with some minor implementation
    /// details changes.
    ///
    /// The format spec pattern is the following: [[fill]align][sign][symbol][0][width][,][.precision][type]
    fn parse_pattern<'a>(&self, pattern: &'a str) -> FormatSpec<'a> {
        let re =
            Regex::new(r"^(?:(.)?([<>=^]))?([+\- ])?([$#])?(0)?(\d+)?(,)?(\.\d+)?([A-Za-z%])?$")
                .unwrap();
        FormatSpec::from(re.captures(pattern).unwrap())
    }

    /// Group digits using the `group_delimiter` character.
    ///
    /// A width is going to be specified (>0) only when the formatted value should be filled in
    /// with "0" characters before the number itself (e.g. using a "020f" or "0=12f" pattern).
    ///
    /// If width > 0, the result will fit into the provided width.
    /// In case the width > 0 and the grouped value starts with the grouping character
    /// (e.g. width = 4, value = 0001 -> ,001), it will be formatted as 0,001, since ,001
    /// is not a valid representation.
    ///
    /// If width = 0, the result will group all passed digits without truncating any of them.
    fn group_value(&self, value: &str, width: usize) -> String {
        let mut reversed_chars: Vec<&[char]> = Vec::new();
        let input_chars: Vec<char> = value.chars().rev().collect();
        let separator: [char; 1] = [self.group_delimiter];

        // After the below loop, an input of "1234" is going to be
        // transformed into `vec![['4', '3', '2'], [','], ['1'], [',']]`.
        for group in input_chars.chunks(3) {
            reversed_chars.push(group);
            reversed_chars.push(&separator);
        }
        // pop last grouping character since it is going to become the leading one after reverse.
        reversed_chars.pop();

        // Flatten the reversed_chars vec
        let grouped: Vec<&char> = reversed_chars.into_iter().flatten().collect();

        // Assure the grouped value fits into provided width in case width > 0
        if width > 0 && grouped.len() > width {
            // If the first character is going to be the group delimiter,
            // keep the one preceding the group delimiter.
            let to_skip = if grouped[width - 1] == &separator[0] {
                grouped.len() - width - 1
            } else {
                grouped.len() - width
            };
            grouped.into_iter().rev().skip(to_skip).collect::<String>()
        } else {
            grouped.into_iter().rev().collect::<String>()
        }
    }

    /// Format the number using scientific notation. The exponent is always represented with
    /// the corresponding sign and at least 2 digits (e.g. 1e+01, 2.1e-02, 42.12e+210).
    ///
    /// The `format_type` is either a small "e" or a capital "E". Also, the format spec pattern
    /// might require displaying a decimal point even if the formatted number does not contain
    /// any decimal digits.
    fn get_formatted_exp_value(
        &self,
        format_type: &str,
        value: f64,
        precision: usize,
        include_decimal_point: bool,
    ) -> String {
        let formatted = format!("{value:.precision$e}");
        let tokens = formatted.split(format_type).collect::<Vec<&str>>();

        let exp_suffix = if &tokens[1][0..1] == "-" {
            if tokens[1].len() == 2 {
                format!("-0{}", &tokens[1][1..])
            } else {
                tokens[1].to_owned()
            }
        } else {
            format!("+{:0>2}", &tokens[1])
        };

        let possible_decimal = if include_decimal_point && precision == 0 {
            format_args!("{}", self.decimal).to_string()
        } else {
            "".to_owned()
        };

        format!(
            "{}{}{}{}",
            &tokens[0], possible_decimal, format_type, exp_suffix
        )
    }

    /// Compute the sign prefix to display based on num sign and format spec.
    ///
    /// If the number is negative, always show "-" sign.
    /// Otherwise, if the format spec contains:
    ///   - "+" sign, show a "+" sign for positive numbers
    ///   - " " a blank space, leave a blank space for positive numbers
    ///
    /// If the format_spec does not contain any info regarding the sign, use an empty string.
    fn get_sign_prefix(&self, is_negative: bool, format_spec: &FormatSpec) -> &str {
        if is_negative {
            "-"
        } else if format_spec.sign.unwrap() == "+" {
            "+"
        } else if format_spec.sign.unwrap() == " " {
            " "
        } else {
            ""
        }
    }

    /// Format a number to a specific human readable form defined by the format spec pattern.
    /// The method takes in a string specifier and a number and returns the string representation
    /// of the formatted number.
    pub fn format<T: Into<f64>>(&self, pattern: &str, input: T) -> String {
        let format_spec = self.parse_pattern(pattern);

        let input_f64: f64 = input.into();
        let mut value_is_negative: bool = input_f64.is_sign_negative();

        let mut decimal_part = String::new();
        let mut si_prefix_exponent: &str = "";
        let unit_of_measurement: &str = match format_spec.format_type {
            Some("%") => "%",
            _ => "",
        };

        let mut value = match format_spec.format_type {
            Some("%") => format!(
                "{:.1$}",
                input_f64.abs() * 100_f64,
                format_spec.precision.unwrap() as usize
            ),
            Some("b") => format!("{:#b}", input_f64.abs() as i64)[2..].into(),
            Some("o") | Some("O") => format!("{:#o}", input_f64.abs() as i64)[2..].into(),
            Some("x") => format!("{:#x}", input_f64.abs() as i64)[2..].into(),
            Some("X") => format!("{:#X}", input_f64.abs() as i64)[2..].into(),
            Some("f") if format_spec.symbol.unwrap_or_default() == "#" => {
                let maybe_decimal = if format_spec.precision.unwrap() == 0 {
                    self.decimal.to_string()
                } else {
                    "".to_string()
                };
                format!(
                    "{:.2$}{}",
                    input_f64.abs(),
                    maybe_decimal,
                    format_spec.precision.unwrap() as usize
                )
            }
            Some("e") => self.get_formatted_exp_value(
                "e",
                input_f64.abs(),
                format_spec.precision.unwrap() as usize,
                format_spec.symbol.unwrap_or_default() == "#",
            ),
            Some("E") => self.get_formatted_exp_value(
                "E",
                input_f64.abs(),
                format_spec.precision.unwrap() as usize,
                format_spec.symbol.unwrap_or_default() == "#",
            ),
            Some("s") => {
                let (val, si_prefix) =
                    self.format_si_prefix(input_f64.abs(), format_spec.precision);
                si_prefix_exponent = PREFIXES[(8 + si_prefix) as usize];
                val
            }
            _ => format!(
                "{:.1$}",
                input_f64.abs(),
                format_spec.precision.unwrap() as usize
            ),
        };

        // If a negative value rounds to zero after formatting, and no explicit positive sign is requested, hide the sign.
        if format_spec.format_type != Some("x")
            && format_spec.format_type != Some("X")
            && value_is_negative
            && value.parse::<f64>().unwrap() == 0_f64
            && format_spec.sign.unwrap_or("+") != "+"
        {
            value_is_negative = false;
        }

        let sign_prefix = self.get_sign_prefix(value_is_negative, &format_spec);

        let leading_part = match format_spec.symbol {
            Some("#") => match format_spec.format_type {
                Some("b") => "0b",
                Some("o") => "0o",
                Some("x") => "0x",
                Some("O") => "0O",
                Some("X") => "0x",
                _ => "",
            },
            Some("$") => "$",
            _ => "",
        };

        // Split the integer part of the value for grouping purposes and attach the decimal part as suffix.
        let chars = value.chars().enumerate();
        for (i, c) in chars {
            if "0123456789".find(c).is_none() {
                decimal_part = value[i..].to_owned();
                value = value[..i].to_owned();
                break;
            }
        }

        // Compute the prefix and suffix.
        let prefix = format!("{sign_prefix}{leading_part}");
        let suffix = format!("{decimal_part}{si_prefix_exponent}{unit_of_measurement}");

        // If should group and filling character is different than "0",
        // group digits before applying padding.
        if format_spec.grouping.is_some() && !format_spec.zero {
            value = self.group_value(&value, 0)
        }

        // Compute the padding.
        let length = prefix.len() + value.to_string().len() + suffix.len();
        let mut padding = if length < format_spec.width.unwrap() {
            vec![format_spec.fill.unwrap(); format_spec.width.unwrap() - length].join("")
        } else {
            "".to_owned()
        };

        // If "0" is the filling character, grouping is applied after computing padding.
        if format_spec.grouping.is_some() && format_spec.zero {
            value = self.group_value(
                format!("{}{}", &padding, value).as_str(),
                if !padding.is_empty() {
                    format_spec.width.unwrap() - suffix.len()
                } else {
                    0
                },
            );
            padding = "".to_owned();
        };

        match format_spec.align {
            Some("<") => format!("{prefix}{value}{suffix}{padding}"),
            Some("=") => format!("{prefix}{padding}{value}{suffix}"),
            Some("^") => format!(
                "{}{}{}{}{}",
                &padding[..padding.len() / 2],
                prefix,
                value,
                suffix,
                &padding[padding.len() / 2..]
            ),
            _ => format!("{padding}{prefix}{value}{suffix}"),
        }
    }
}

/// The macro used to format a given number inline, similar to `format!` macro.
///
/// This is a shortcut for [HumanNumberFormat::format()](struct.HumanNumberFormat.html#method.format) method.
///
/// # Examples
///
/// ```
/// use avenger_scales::format_num;
///
/// assert_eq!(format_num!(".0%", 0.123), "12%");
/// assert_eq!(format_num!(".2s", 0.012345), "12m");
/// ```
#[macro_export]
macro_rules! format_num {
    ( $x:expr, $y:expr ) => {{
        $crate::format_num::NumberFormat::new().format($x, $y)
    }};
}

#[cfg(test)]
mod tests {
    use super::NumberFormat;

    #[test]
    fn initialization() {
        let num = NumberFormat::new();
        assert_eq!(num.decimal, '.');
        assert_eq!(num.group_delimiter, ',');
    }

    #[test]
    fn significant_digits() {
        assert_eq!(NumberFormat::get_significant_digits("81"), 2);
        assert_eq!(NumberFormat::get_significant_digits("26.2"), 3);
        assert_eq!(NumberFormat::get_significant_digits("0.004"), 1);
        assert_eq!(NumberFormat::get_significant_digits("5200.38"), 6);
        assert_eq!(NumberFormat::get_significant_digits("380.0"), 4);
        assert_eq!(NumberFormat::get_significant_digits("78800"), 3);
        assert_eq!(NumberFormat::get_significant_digits("78800."), 5);
    }

    #[test]
    fn precision_0_percentage() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".0%", 0), "0%");
        assert_eq!(num.format(".0%", 0.042), "4%");
        assert_eq!(num.format(".0%", 0.42), "42%");
        assert_eq!(num.format(".0%", 4.2), "420%");
        assert_eq!(num.format(".0%", -0.042), "-4%");
        assert_eq!(num.format(".0%", -0.42), "-42%");
        assert_eq!(num.format(".0%", -4.2), "-420%");
    }

    #[test]
    fn precision_gt_0_percentage() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".1%", 0.234), "23.4%");
        assert_eq!(num.format(".1%", 0.23456), "23.5%");
        assert_eq!(num.format(".2%", 0.234), "23.40%");
    }

    #[test]
    fn percentage_forms() {
        let num = NumberFormat::new();
        assert_eq!(num.format("020.0%", 12), "0000000000000001200%");
        assert_eq!(num.format("20.0%", 12), "               1200%");
        assert_eq!(num.format("^21.0%", 0.12), "         12%         ");
        assert_eq!(num.format("^21,.0%", 122), "       12,200%       ");
        assert_eq!(num.format("^21,.0%", -122), "      -12,200%       ");
    }

    #[test]
    fn grouping() {
        let num = NumberFormat::new();
        assert_eq!(num.format("01,.0d", 0), "0");
        assert_eq!(num.format("02,.0d", 0), "00");
        assert_eq!(num.format("03,.0d", 0), "000");
        assert_eq!(num.format("04,.0d", 0), "0,000");
        assert_eq!(num.format("05,.0d", 0), "0,000");
        assert_eq!(num.format("08,.0d", 0), "0,000,000");
        assert_eq!(num.format("013,.0d", 0), "0,000,000,000");
        assert_eq!(num.format("021,.0d", 0), "0,000,000,000,000,000");
        assert_eq!(num.format("013,.8d", -42000000), "-0,042,000,000");
    }

    #[test]
    fn zeroes() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".0f", 0), "0");
        assert_eq!(num.format(".1f", 0), "0.0");
        assert_eq!(num.format(".2f", 0), "0.00");
        assert_eq!(num.format(".3f", 0), "0.000");
        assert_eq!(
            num.format(".50f", 0),
            "0.00000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn precision_0() {
        let num = NumberFormat::new();
        // for precision 0, result should never include a .
        assert_eq!(num.format(".0f", 1.5), "2");
        assert_eq!(num.format(".0f", 2.5), "2");
        assert_eq!(num.format(".0f", 3.5), "4");
        assert_eq!(num.format(".0f", 0.0), "0");
        assert_eq!(num.format(".0f", 0.1), "0");
        assert_eq!(num.format(".0f", 0.001), "0");
        assert_eq!(num.format(".0f", 10.0), "10");
        assert_eq!(num.format(".0f", 10.1), "10");
        assert_eq!(num.format(".0f", 10.01), "10");
        assert_eq!(num.format(".0f", 123.456), "123");
        assert_eq!(num.format(".0f", 1234.56), "1235");
        assert_eq!(
            num.format(".0f", 1e49),
            "9999999999999999464902769475481793196872414789632"
        );
        assert_eq!(
            num.format(".0f", 9.999_999_999_999_999e49),
            "99999999999999986860582406952576489172979654066176"
        );
        assert_eq!(
            num.format(".0f", 1e50),
            "100000000000000007629769841091887003294964970946560"
        );
    }

    #[test]
    fn precision_1() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".1f", 0.0001), "0.0");
        assert_eq!(num.format(".1f", 0.001), "0.0");
        assert_eq!(num.format(".1f", 0.01), "0.0");
        assert_eq!(num.format(".1f", 0.04), "0.0");
        assert_eq!(num.format(".1f", 0.06), "0.1");
        assert_eq!(num.format(".1f", 0.25), "0.2");
        assert_eq!(num.format(".1f", 0.75), "0.8");
        assert_eq!(num.format(".1f", 1.4), "1.4");
        assert_eq!(num.format(".1f", 1.5), "1.5");
        assert_eq!(num.format(".1f", 10.0), "10.0");
        assert_eq!(num.format(".1f", 1000.03), "1000.0");
        assert_eq!(num.format(".1f", 1234.5678), "1234.6");
        assert_eq!(num.format(".1f", 1234.7499), "1234.7");
        assert_eq!(num.format(".1f", 1234.75), "1234.8");
    }

    #[test]
    fn precision_2() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".2f", 0.0001), "0.00");
        assert_eq!(num.format(".2f", 0.001), "0.00");
        assert_eq!(num.format(".2f", 0.004999), "0.00");
        assert_eq!(num.format(".2f", 0.005001), "0.01");
        assert_eq!(num.format(".2f", 0.01), "0.01");
        assert_eq!(num.format(".2f", 0.125), "0.12");
        assert_eq!(num.format(".2f", 0.375), "0.38");
        assert_eq!(num.format(".2f", 1234500), "1234500.00");
        assert_eq!(num.format(".2f", 1234560), "1234560.00");
        assert_eq!(num.format(".2f", 1234567), "1234567.00");
        assert_eq!(num.format(".2f", 1234567.8), "1234567.80");
        assert_eq!(num.format(".2f", 1234567.89), "1234567.89");
        assert_eq!(num.format(".2f", 1234567.891), "1234567.89");
        assert_eq!(num.format(".2f", 1234567.8912), "1234567.89");
    }

    #[test]
    fn decimal_alternate_form() {
        let num = NumberFormat::new();
        // alternate form always includes a decimal point.  This only
        // makes a difference when the precision is 0.
        assert_eq!(num.format("#.0f", 0), "0.");
        assert_eq!(num.format("#.1f", 0), "0.0");
        assert_eq!(num.format("#.0f", 1.5), "2.");
        assert_eq!(num.format("#.0f", 2.5), "2.");
        assert_eq!(num.format("#.0f", 10.1), "10.");
        assert_eq!(num.format("#.0f", 1234.56), "1235.");
        assert_eq!(num.format("#.1f", 1.4), "1.4");
        assert_eq!(num.format("#.2f", 0.375), "0.38");
    }

    #[test]
    fn default_precision() {
        let num = NumberFormat::new();
        assert_eq!(num.format("f", 0), "0.000000");
        assert_eq!(num.format("f", 1230000), "1230000.000000");
        assert_eq!(num.format("f", 1234567), "1234567.000000");
        assert_eq!(num.format("f", 123.4567), "123.456700");
        assert_eq!(num.format("f", 1.23456789), "1.234568");
        assert_eq!(num.format("f", 0.00012), "0.000120");
        assert_eq!(num.format("f", 0.000123), "0.000123");
        assert_eq!(num.format("f", 0.00012345), "0.000123");
        assert_eq!(num.format("f", 0.000001), "0.000001");
        assert_eq!(num.format("f", 0.0000005001), "0.000001");
        assert_eq!(num.format("f", 0.0000004999), "0.000000");
    }

    // 'e' code formatting with explicit precision (>= 0). Output should always
    // have exactly the number of places after the point that were requested.
    #[test]
    fn zeroes_exp() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".0e", 0), "0e+00");
        assert_eq!(num.format(".1e", 0), "0.0e+00");
        assert_eq!(num.format(".2e", 0), "0.00e+00");
        assert_eq!(num.format(".10e", 0), "0.0000000000e+00");
        assert_eq!(
            num.format(".50e", 0),
            "0.00000000000000000000000000000000000000000000000000e+00"
        );
    }

    #[test]
    fn precision_0_exp() {
        let num = NumberFormat::new();
        // no decimal point in the output
        assert_eq!(num.format(".0e", 0.01), "1e-02");
        assert_eq!(num.format(".0e", 0.1), "1e-01");
        assert_eq!(num.format(".0e", 1), "1e+00");
        assert_eq!(num.format(".0e", 10), "1e+01");
        assert_eq!(num.format(".0e", 100), "1e+02");
        assert_eq!(num.format(".0e", 0.012), "1e-02");
        assert_eq!(num.format(".0e", 0.12), "1e-01");
        assert_eq!(num.format(".0e", 1.2), "1e+00");
        assert_eq!(num.format(".0e", 12), "1e+01");
        assert_eq!(num.format(".0e", 120), "1e+02");
        assert_eq!(num.format(".0e", 123.456), "1e+02");
        assert_eq!(num.format(".0e", 0.000123456), "1e-04");
        assert_eq!(num.format(".0e", 123456000), "1e+08");
        assert_eq!(num.format(".0e", 0.5), "5e-01");
        assert_eq!(num.format(".0e", 1.4), "1e+00");
        assert_eq!(num.format(".0e", 1.5), "2e+00");
        assert_eq!(num.format(".0e", 1.6), "2e+00");
        assert_eq!(num.format(".0e", 2.4999999), "2e+00");
        assert_eq!(num.format(".0e", 2.5), "2e+00");
        assert_eq!(num.format(".0e", 2.5000001), "3e+00");
        assert_eq!(num.format(".0e", 3.499999999999), "3e+00");
        assert_eq!(num.format(".0e", 3.5), "4e+00");
        assert_eq!(num.format(".0e", 4.5), "4e+00");
        assert_eq!(num.format(".0e", 5.5), "6e+00");
        assert_eq!(num.format(".0e", 6.5), "6e+00");
        assert_eq!(num.format(".0e", 7.5), "8e+00");
        assert_eq!(num.format(".0e", 8.5), "8e+00");
        assert_eq!(num.format(".0e", 9.4999), "9e+00");
        assert_eq!(num.format(".0e", 9.5), "1e+01");
        assert_eq!(num.format(".0e", 10.5), "1e+01");
        assert_eq!(num.format(".0e", 14.999), "1e+01");
        assert_eq!(num.format(".0e", 15), "2e+01");
    }

    #[test]
    fn precision_1_exp() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".1e", 0.0001), "1.0e-04");
        assert_eq!(num.format(".1e", 0.001), "1.0e-03");
        assert_eq!(num.format(".1e", 0.01), "1.0e-02");
        assert_eq!(num.format(".1e", 0.1), "1.0e-01");
        assert_eq!(num.format(".1e", 1), "1.0e+00");
        assert_eq!(num.format(".1e", 10), "1.0e+01");
        assert_eq!(num.format(".1e", 100), "1.0e+02");
        assert_eq!(num.format(".1e", 120), "1.2e+02");
        assert_eq!(num.format(".1e", 123), "1.2e+02");
        assert_eq!(num.format(".1e", 123.4), "1.2e+02");
    }

    #[test]
    fn precision_2_exp() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".2e", 0.00013), "1.30e-04");
        assert_eq!(num.format(".2e", 0.000135), "1.35e-04");
        assert_eq!(num.format(".2e", 0.0001357), "1.36e-04");
        assert_eq!(num.format(".2e", 0.0001), "1.00e-04");
        assert_eq!(num.format(".2e", 0.001), "1.00e-03");
        assert_eq!(num.format(".2e", 0.01), "1.00e-02");
        assert_eq!(num.format(".2e", 0.1), "1.00e-01");
        assert_eq!(num.format(".2e", 1), "1.00e+00");
        assert_eq!(num.format(".2e", 10), "1.00e+01");
        assert_eq!(num.format(".2e", 100), "1.00e+02");
        assert_eq!(num.format(".2e", 1000), "1.00e+03");
        assert_eq!(num.format(".2e", 1500), "1.50e+03");
        assert_eq!(num.format(".2e", 1590), "1.59e+03");
        assert_eq!(num.format(".2e", 1598), "1.60e+03");
        assert_eq!(num.format(".2e", 1598.7), "1.60e+03");
        assert_eq!(num.format(".2e", 1598.76), "1.60e+03");
        assert_eq!(num.format(".2e", 9999), "1.00e+04");
    }

    #[test]
    fn default_precision_exp() {
        let num = NumberFormat::new();
        assert_eq!(num.format("e", 0), "0.000000e+00");
        assert_eq!(num.format("e", 165), "1.650000e+02");
        assert_eq!(num.format("e", 1234567), "1.234567e+06");
        assert_eq!(num.format("e", 12345678), "1.234568e+07");
        assert_eq!(num.format("e", 1.1), "1.100000e+00");
    }

    #[test]
    fn alternate_form_exp() {
        let num = NumberFormat::new();
        assert_eq!(num.format("#.0e", 0.01), "1.e-02");
        assert_eq!(num.format("#.0e", 0.1), "1.e-01");
        assert_eq!(num.format("#.0e", 1), "1.e+00");
        assert_eq!(num.format("#.0e", 10), "1.e+01");
        assert_eq!(num.format("#.0e", 100), "1.e+02");
        assert_eq!(num.format("#.0e", 0.012), "1.e-02");
        assert_eq!(num.format("#.0e", 0.12), "1.e-01");
        assert_eq!(num.format("#.0e", 1.2), "1.e+00");
        assert_eq!(num.format("#.0e", 12), "1.e+01");
        assert_eq!(num.format("#.0e", 120), "1.e+02");
        assert_eq!(num.format("#.0e", 123.456), "1.e+02");
        assert_eq!(num.format("#.0e", 0.000123456), "1.e-04");
        assert_eq!(num.format("#.0e", 123456000), "1.e+08");
        assert_eq!(num.format("#.0e", 0.5), "5.e-01");
        assert_eq!(num.format("#.0e", 1.4), "1.e+00");
        assert_eq!(num.format("#.0e", 1.5), "2.e+00");
        assert_eq!(num.format("#.0e", 1.6), "2.e+00");
        assert_eq!(num.format("#.0e", 2.4999999), "2.e+00");
        assert_eq!(num.format("#.0e", 2.5), "2.e+00");
        assert_eq!(num.format("#.0e", 2.5000001), "3.e+00");
        assert_eq!(num.format("#.0e", 3.499999999999), "3.e+00");
        assert_eq!(num.format("#.0e", 3.5), "4.e+00");
        assert_eq!(num.format("#.0e", 4.5), "4.e+00");
        assert_eq!(num.format("#.0e", 5.5), "6.e+00");
        assert_eq!(num.format("#.0e", 6.5), "6.e+00");
        assert_eq!(num.format("#.0e", 7.5), "8.e+00");
        assert_eq!(num.format("#.0e", 8.5), "8.e+00");
        assert_eq!(num.format("#.0e", 9.4999), "9.e+00");
        assert_eq!(num.format("#.0e", 9.5), "1.e+01");
        assert_eq!(num.format("#.0e", 10.5), "1.e+01");
        assert_eq!(num.format("#.0e", 14.999), "1.e+01");
        assert_eq!(num.format("#.0e", 15), "2.e+01");
        assert_eq!(num.format("#.1e", 123.4), "1.2e+02");
        assert_eq!(num.format("#.2e", 0.0001357), "1.36e-04");
    }

    #[test]
    fn decimal() {
        let num = NumberFormat::new();
        assert_eq!(num.format("d", 2_147_483_647), "2147483647");
        assert_eq!(num.format("d", -2_147_483_647), "-2147483647");
        assert_eq!(num.format("5d", -2_147_483_647), "-2147483647");
        assert_eq!(num.format("11d", -2_147_483_647), "-2147483647");
        assert_eq!(num.format("12d", -2_147_483_647), " -2147483647");
        assert_eq!(num.format("-12d", -2_147_483_647), " -2147483647");
        assert_eq!(num.format("012d", -2_147_483_647), "-02147483647");
        assert_eq!(num.format("-012d", -2_147_483_647), "-02147483647");
        assert_eq!(num.format("014d", -2_147_483_647), "-0002147483647");
        assert_eq!(num.format("014d", 2_147_483_647), "00002147483647");
        assert_eq!(num.format("0=+14d", 2_147_483_647), "+0002147483647");
        assert_eq!(num.format(">+14d", 2_147_483_647), "   +2147483647");
        assert_eq!(num.format(".^+14d", 2_147_483_647), ".+2147483647..");
        assert_eq!(num.format("+014d", 2_147_483_647), "+0002147483647");
        assert_eq!(num.format("+14d", 2_147_483_647), "   +2147483647");
        assert_eq!(num.format("14d", 2_147_483_647), "    2147483647");
        assert_eq!(num.format(".2d", 2_147_483_647), "2147483647");
        assert_eq!(num.format(".10d", 2_147_483_647), "2147483647");
        assert_eq!(num.format(".11d", 2_147_483_647), "2147483647");
        assert_eq!(num.format("12.11d", 2_147_483_647), "  2147483647");
    }

    #[test]
    fn bin() {
        let num = NumberFormat::new();
        assert_eq!(num.format("#b", 3), "0b11");
        assert_eq!(num.format("b", 3), "11");
        assert_eq!(num.format("+020b", 123), "+0000000000001111011");
        assert_eq!(num.format(" 020b", 123), " 0000000000001111011");
        assert_eq!(num.format("+#020b", 123), "+0b00000000001111011");
    }

    #[test]
    fn hex() {
        let num = NumberFormat::new();
        assert_eq!(num.format("x", 0xf12abcd), "f12abcd");
        assert_eq!(num.format("x", -0xf12abcd), "-f12abcd");
        assert_eq!(num.format("5x", -0xf12abcd), "-f12abcd");
        assert_eq!(num.format("8x", -0xf12abcd), "-f12abcd");
        assert_eq!(num.format("9x", -0xf12abcd), " -f12abcd");
        assert_eq!(num.format("-9x", -0xf12abcd), " -f12abcd");
        assert_eq!(num.format("09x", -0xf12abcd), "-0f12abcd");
        assert_eq!(num.format("-09x", -0xf12abcd), "-0f12abcd");
        assert_eq!(num.format("011x", -0xf12abcd), "-000f12abcd");
        assert_eq!(num.format("011x", 0xf12abcd), "0000f12abcd");
        assert_eq!(num.format("0=+11x", 0xf12abcd), "+000f12abcd");
        assert_eq!(num.format("0>+11x", 0xf12abcd), "000+f12abcd");
        assert_eq!(num.format("+11x", 0xf12abcd), "   +f12abcd");
        assert_eq!(num.format("11x", 0xf12abcd), "    f12abcd");
        assert_eq!(num.format(".2x", 0xf12abcd), "f12abcd");
        assert_eq!(num.format(".7x", 0xf12abcd), "f12abcd");
        assert_eq!(num.format(".8x", 0xf12abcd), "f12abcd");
        assert_eq!(num.format("9.8x", 0xf12abcd), "  f12abcd");
        assert_eq!(num.format("X", 0xf12abcd), "F12ABCD");
        assert_eq!(num.format("#X", 0xf12abcd), "0xF12ABCD");
        assert_eq!(num.format("#x", 0xf12abcd), "0xf12abcd");
        assert_eq!(num.format("#x", -0xf12abcd), "-0xf12abcd");
        assert_eq!(num.format("#13x", 0xf12abcd), "    0xf12abcd");
        assert_eq!(num.format("<#13x", 0xf12abcd), "0xf12abcd    ");
        assert_eq!(num.format("#013x", 0xf12abcd), "0x0000f12abcd");
        assert_eq!(num.format("#.9x", 0xf12abcd), "0xf12abcd");
        assert_eq!(num.format("#.9x", -0xf12abcd), "-0xf12abcd");
        assert_eq!(num.format("#13.9x", 0xf12abcd), "    0xf12abcd");
        assert_eq!(num.format("#013.9x", 0xf12abcd), "0x0000f12abcd");
        assert_eq!(num.format("+#.9x", 0xf12abcd), "+0xf12abcd");
        assert_eq!(num.format(" #.9x", 0xf12abcd), " 0xf12abcd");
        assert_eq!(num.format("+#.9X", 0xf12abcd), "+0xF12ABCD");
    }

    #[test]
    fn oct() {
        let num = NumberFormat::new();
        assert_eq!(num.format("o", 1234567890), "11145401322");
        assert_eq!(num.format("o", -1234567890), "-11145401322");
        assert_eq!(num.format("5o", -1234567890), "-11145401322");
        assert_eq!(num.format("8o", -1234567890), "-11145401322");
        assert_eq!(num.format("13o", -1234567890), " -11145401322");
        assert_eq!(num.format("-13o", -1234567890), " -11145401322");
        assert_eq!(num.format("013o", -1234567890), "-011145401322");
        assert_eq!(num.format("-013o", -1234567890), "-011145401322");
        assert_eq!(num.format("015o", -1234567890), "-00011145401322");
        assert_eq!(num.format("015o", 1234567890), "000011145401322");
        assert_eq!(num.format("0=+15o", 1234567890), "+00011145401322");
        assert_eq!(num.format("0>+15o", 1234567890), "000+11145401322");
        assert_eq!(num.format("+15o", 1234567890), "   +11145401322");
        assert_eq!(num.format("15o", 1234567890), "    11145401322");
        assert_eq!(num.format(".2o", 1234567890), "11145401322");
        assert_eq!(num.format(".7o", 1234567890), "11145401322");
        assert_eq!(num.format(".13o", 1234567890), "11145401322");
        assert_eq!(num.format("13.12o", 1234567890), "  11145401322");
        assert_eq!(num.format("O", 1234567890), "11145401322");
        assert_eq!(num.format("#O", 1234567890), "0O11145401322");
        assert_eq!(num.format("#o", 1234567890), "0o11145401322");
        assert_eq!(num.format("#o", -1234567890), "-0o11145401322");
        assert_eq!(num.format("#17o", 1234567890), "    0o11145401322");
        assert_eq!(num.format("<#17o", 1234567890), "0o11145401322    ");
        assert_eq!(num.format("#017o", 1234567890), "0o000011145401322");
        assert_eq!(num.format("#.13o", 1234567890), "0o11145401322");
        assert_eq!(num.format("#.13o", -1234567890), "-0o11145401322");
        assert_eq!(num.format("#17.13o", 1234567890), "    0o11145401322");
        assert_eq!(num.format("#017.13o", 1234567890), "0o000011145401322");
        assert_eq!(num.format("+#.13o", 1234567890), "+0o11145401322");
        assert_eq!(num.format(" #.13o", 1234567890), " 0o11145401322");
        assert_eq!(num.format("+#.13O", 1234567890), "+0O11145401322");
    }

    #[test]
    fn small_ints() {
        let num = NumberFormat::new();
        assert_eq!(num.format("d", 42), "42");
        assert_eq!(num.format("d", -42), "-42");
        assert_eq!(num.format("d", 42.0), "42");
        assert_eq!(num.format("#x", 1), "0x1");
        assert_eq!(num.format("#X", 1), "0x1");
        assert_eq!(num.format("#o", 1), "0o1");
        assert_eq!(num.format("#o", 0), "0o0");
        assert_eq!(num.format("o", 0), "0");
        assert_eq!(num.format("d", 0), "0");
        assert_eq!(num.format("#x", 0), "0x0");
        assert_eq!(num.format("#X", 0), "0x0");
        assert_eq!(num.format("x", 0x42), "42");
        assert_eq!(num.format("x", -0x42), "-42");
        assert_eq!(num.format("o", 0o42), "42");
        assert_eq!(num.format("o", -0o42), "-42");
    }

    #[test]
    fn si_prefix_default_precision() {
        let num = NumberFormat::new();
        assert_eq!(num.format("s", 0), "0.00000");
        assert_eq!(num.format("s", 1), "1.00000");
        assert_eq!(num.format("s", 10), "10.0000");
        assert_eq!(num.format("s", 100), "100.000");
        assert_eq!(num.format("s", 999.5), "999.500");
        assert_eq!(num.format("s", 999500), "999.500k");
        assert_eq!(num.format("s", 1000), "1.00000k");
        assert_eq!(num.format("s", 100), "100.000");
        assert_eq!(num.format("s", 1400), "1.40000k");
        assert_eq!(num.format("s", 1500.5), "1.50050k");
        assert_eq!(num.format("s", 0.00001), "10.0000µ");
        assert_eq!(num.format("s", 0.000001), "1.00000µ");
    }

    #[test]
    fn si_prefix_custom_precision() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".3s", 0), "0.00");
        assert_eq!(num.format(".3s", 1), "1.00");
        assert_eq!(num.format(".3s", 10), "10.0");
        assert_eq!(num.format(".3s", 100), "100");
        assert_eq!(num.format(".3s", 999.5), "1.00k");
        assert_eq!(num.format(".3s", 999500), "1.00M");
        assert_eq!(num.format(".3s", 1000), "1.00k");
        assert_eq!(num.format(".3s", 1500.5), "1.50k");
        assert_eq!(num.format(".3s", 42e6), "42.0M");
        assert_eq!(num.format(".3s", 145500000), "146M");
        assert_eq!(num.format(".3s", 145_999_999.999_999_34), "146M");
        assert_eq!(num.format(".3s", 1e26), "100Y");
        assert_eq!(num.format(".3s", 0.000001), "1.00µ");
        assert_eq!(num.format(".3s", 0.009995), "10.0m");
        assert_eq!(num.format(".4s", 999.5), "999.5");
        assert_eq!(num.format(".4s", 999500), "999.5k");
        assert_eq!(num.format(".4s", 0.009995), "9.995m");
    }

    #[test]
    fn si_prefix_numbers_smaller_than_one_yocto() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".8s", 1.29e-30), "0.0000013y"); // Note: rounded!
        assert_eq!(num.format(".8s", 1.29e-29), "0.0000129y");
        assert_eq!(num.format(".8s", 1.29e-28), "0.0001290y");
        assert_eq!(num.format(".8s", 1.29e-27), "0.0012900y");
        assert_eq!(num.format(".8s", 1.29e-26), "0.0129000y");
        assert_eq!(num.format(".8s", 1.29e-25), "0.1290000y");
        assert_eq!(num.format(".8s", 1.29e-24), "1.2900000y");
        assert_eq!(num.format(".8s", 1.29e-23), "12.900000y");
        assert_eq!(num.format(".8s", 1.29e-22), "129.00000y");
        assert_eq!(num.format(".8s", 1.29e-21), "1.2900000z");
        assert_eq!(num.format(".8s", -1.29e-30), "-0.0000013y"); // Note: rounded!
        assert_eq!(num.format(".8s", -1.29e-29), "-0.0000129y");
        assert_eq!(num.format(".8s", -1.29e-28), "-0.0001290y");
        assert_eq!(num.format(".8s", -1.29e-27), "-0.0012900y");
        assert_eq!(num.format(".8s", -1.29e-26), "-0.0129000y");
        assert_eq!(num.format(".8s", -1.29e-25), "-0.1290000y");
        assert_eq!(num.format(".8s", -1.29e-24), "-1.2900000y");
        assert_eq!(num.format(".8s", -1.29e-23), "-12.900000y");
        assert_eq!(num.format(".8s", -1.29e-22), "-129.00000y");
        assert_eq!(num.format(".8s", -1.29e-21), "-1.2900000z");
    }

    #[test]
    fn si_prefix_numbers_bigger_than_one_yotta() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".8s", 1.23e+21), "1.2300000Z");
        assert_eq!(num.format(".8s", 1.23e+22), "12.300000Z");
        assert_eq!(num.format(".8s", 1.23e+23), "123.00000Z");
        assert_eq!(num.format(".8s", 1.23e+24), "1.2300000Y");
        assert_eq!(num.format(".8s", 1.23e+25), "12.300000Y");
        assert_eq!(num.format(".8s", 1.23e+26), "123.00000Y");
        assert_eq!(num.format(".8s", 1.23e+27), "1230.0000Y");
        assert_eq!(num.format(".8s", 1.23e+28), "12300.000Y");
        assert_eq!(num.format(".8s", 1.23e+29), "123000.00Y");
        assert_eq!(num.format(".8s", 1.23e+30), "1230000.0Y");
        assert_eq!(num.format(".8s", -1.23e+21), "-1.2300000Z");
        assert_eq!(num.format(".8s", -1.23e+22), "-12.300000Z");
        assert_eq!(num.format(".8s", -1.23e+23), "-123.00000Z");
        assert_eq!(num.format(".8s", -1.23e+24), "-1.2300000Y");
        assert_eq!(num.format(".8s", -1.23e+25), "-12.300000Y");
        assert_eq!(num.format(".8s", -1.23e+26), "-123.00000Y");
        assert_eq!(num.format(".8s", -1.23e+27), "-1230.0000Y");
        assert_eq!(num.format(".8s", -1.23e+28), "-12300.000Y");
        assert_eq!(num.format(".8s", -1.23e+29), "-123000.00Y");
        assert_eq!(num.format(".8s", -1.23e+30), "-1230000.0Y");
    }

    #[test]
    fn si_prefix_consistent_for_small_and_big_numbers() {
        let num = NumberFormat::new();
        assert_eq!(num.format(".0s", 1e-5), "10µ");
        assert_eq!(num.format(".0s", 1e-4), "100µ");
        assert_eq!(num.format(".0s", 1e-3), "1m");
        assert_eq!(num.format(".0s", 1e-2), "10m");
        assert_eq!(num.format(".0s", 1e-1), "100m");
        assert_eq!(num.format(".0s", 1e+0), "1");
        assert_eq!(num.format(".0s", 1e+1), "10");
        assert_eq!(num.format(".0s", 1e+2), "100");
        assert_eq!(num.format(".0s", 1e+3), "1k");
        assert_eq!(num.format(".0s", 1e+4), "10k");
        assert_eq!(num.format(".0s", 1e+5), "100k");
        assert_eq!(num.format(".4s", 1e-5), "10.00µ");
        assert_eq!(num.format(".4s", 1e-4), "100.0µ");
        assert_eq!(num.format(".4s", 1e-3), "1.000m");
        assert_eq!(num.format(".4s", 1e-2), "10.00m");
        assert_eq!(num.format(".4s", 1e-1), "100.0m");
        assert_eq!(num.format(".4s", 1e+0), "1.000");
        assert_eq!(num.format(".4s", 1e+1), "10.00");
        assert_eq!(num.format(".4s", 1e+2), "100.0");
        assert_eq!(num.format(".4s", 1e+3), "1.000k");
        assert_eq!(num.format(".4s", 1e+4), "10.00k");
        assert_eq!(num.format(".4s", 1e+5), "100.0k");
    }

    #[test]
    fn si_prefix_grouping() {
        let num = NumberFormat::new();
        assert_eq!(num.format("020,s", 42), "000,000,000,042.0000");
        assert_eq!(num.format("020,s", 42e12), "00,000,000,042.0000T");
        assert_eq!(num.format(",s", 42e30), "42,000,000Y");
    }

    #[test]
    fn negative_zero_correct_formatting() {
        let num = NumberFormat::new();
        assert_eq!(num.format("f", -1e-12), "0.000000");
        assert_eq!(num.format("+f", -0.0), "-0.000000");
        assert_eq!(num.format("+f", 0), "+0.000000");
        assert_eq!(num.format("+f", -1e-12), "-0.000000");
        assert_eq!(num.format("+f", 1e-12), "+0.000000");
    }
}
