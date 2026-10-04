use avenger_format::NumberFormatError;
use fixed_decimal::{SignedRoundingMode as R, UnsignedRoundingMode as U};
use std::collections::HashSet;

use crate::arithmetic::Literal;
use crate::notation::Notation;
use crate::precision::{Digits, Precision};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Grouping {
    #[default]
    Auto,
    Off,
    Min2,
    Aligned,
    Thousands,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Sign {
    #[default]
    Auto,
    Always,
    Never,
    ExceptZero,
    Negative,
}

impl Sign {
    pub fn display(self, negative: bool, zero: bool) -> fixed_decimal::Sign {
        use fixed_decimal::Sign as S;
        match self {
            Self::Never => S::None,
            Self::ExceptZero | Self::Negative if zero => S::None,
            _ if negative => S::Negative,
            Self::Always | Self::ExceptZero => S::Positive,
            _ => S::None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Skeleton {
    pub notation: Notation,
    pub precision: Precision,
    pub rounding: R,
    pub grouping: Grouping,
    pub sign: Sign,
    pub integer_min: i16,
    pub integer_max: Option<i16>,
    pub decimal_always: bool,
    pub numbering_system: Option<String>,
    pub scale: Literal,
}

impl Default for Skeleton {
    fn default() -> Self {
        Self {
            notation: Notation::Simple,
            precision: Precision::default(),
            rounding: R::Unsigned(U::HalfEven),
            grouping: Grouping::Auto,
            sign: Sign::Auto,
            integer_min: 1,
            integer_max: None,
            decimal_always: false,
            numbering_system: None,
            scale: Literal::power(0),
        }
    }
}

pub(crate) fn invalid(message: impl Into<String>, position: usize) -> NumberFormatError {
    NumberFormatError::InvalidPattern {
        message: message.into(),
        position: Some(position),
    }
}

pub(crate) fn unsupported(option: &str, message: &str) -> NumberFormatError {
    NumberFormatError::InvalidOption {
        option: option.into(),
        message: message.into(),
    }
}

/// ICU skeletons use Unicode Pattern_White_Space, which excludes nonbreaking spaces.
fn separator(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            ..='\u{000d}' | ' ' | '\u{0085}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
    )
}

impl Skeleton {
    pub fn parse(input: &str) -> Result<Self, NumberFormatError> {
        let mut result = Self::default();
        let mut seen = HashSet::new();
        let mut offset = 0;
        for token in input.split(separator) {
            let start = offset;
            offset += token.len();
            // split() omits the delimiter; its UTF-8 width can exceed one byte.
            if let Some(c) = input[offset..].chars().next() {
                offset += c.len_utf8();
            }
            if token.is_empty() {
                continue;
            }
            let parts: Vec<_> = token.split('/').collect();
            let stem = parts[0];
            let options = &parts[1..];
            let no_options = || {
                if options.is_empty() {
                    Ok(())
                } else {
                    Err(invalid("unexpected option", start + stem.len() + 1))
                }
            };
            let one_option = || {
                if options.len() == 1 && !options[0].is_empty() {
                    Ok(options[0])
                } else {
                    Err(invalid("expected one option", start))
                }
            };
            let category = match stem {
                "notation-simple" | "scientific" | "engineering" | "compact-short"
                | "compact-long" | "K" | "KK" => {
                    result.notation = Notation::parse(stem, options, start)?;
                    "notation"
                }
                _ if stem.starts_with('E') => {
                    no_options()?;
                    result.notation = Notation::concise(stem, start)?;
                    "notation"
                }
                "precision-integer" | "precision-unlimited" => {
                    if stem == "precision-unlimited" && options.iter().any(|o| *o != "w") {
                        return Err(invalid("precision-unlimited accepts only /w", start));
                    }
                    result.precision = Precision::fraction(Digits {
                        min: 0,
                        max: (stem == "precision-integer").then_some(0),
                    });
                    result.precision.options(options, start)?;
                    "precision"
                }
                _ if stem.starts_with('.') || stem.starts_with('@') => {
                    let significant = stem.starts_with('@');
                    let digits = parse_digits(
                        if significant { stem } else { &stem[1..] },
                        if significant { '@' } else { '0' },
                        start,
                    )?;
                    result.precision = if significant {
                        Precision::significant(digits)
                    } else {
                        Precision::fraction(digits)
                    };
                    result.precision.options(options, start)?;
                    "precision"
                }
                "rounding-mode-ceiling"
                | "rounding-mode-floor"
                | "rounding-mode-down"
                | "rounding-mode-up"
                | "rounding-mode-half-even"
                | "rounding-mode-half-down"
                | "rounding-mode-half-up" => {
                    no_options()?;
                    result.rounding = match stem {
                        "rounding-mode-ceiling" => R::Ceil,
                        "rounding-mode-floor" => R::Floor,
                        "rounding-mode-down" => R::Unsigned(U::Trunc),
                        "rounding-mode-up" => R::Unsigned(U::Expand),
                        "rounding-mode-half-down" => R::Unsigned(U::HalfTrunc),
                        "rounding-mode-half-up" => R::Unsigned(U::HalfExpand),
                        _ => R::Unsigned(U::HalfEven),
                    };
                    "rounding"
                }
                "rounding-mode-unnecessary" => {
                    return Err(unsupported(
                        stem,
                        "formatting cannot report value-dependent rounding errors",
                    ))
                }
                "group-off" | ",_" | "group-auto" | "group-min2" | ",?" | "group-on-aligned"
                | ",!" | "group-thousands" => {
                    no_options()?;
                    result.grouping = match stem {
                        "group-off" | ",_" => Grouping::Off,
                        "group-min2" | ",?" => Grouping::Min2,
                        "group-on-aligned" | ",!" => Grouping::Aligned,
                        "group-thousands" => Grouping::Thousands,
                        _ => Grouping::Auto,
                    };
                    "grouping"
                }
                "sign-auto"
                | "sign-always"
                | "+!"
                | "sign-never"
                | "+_"
                | "sign-except-zero"
                | "+?"
                | "sign-negative"
                | "+-"
                | "sign-accounting"
                | "()"
                | "sign-accounting-always"
                | "()!"
                | "sign-accounting-except-zero"
                | "()?"
                | "sign-accounting-negative"
                | "()-" => {
                    no_options()?;
                    result.sign = match stem {
                        "sign-always" | "+!" | "sign-accounting-always" | "()!" => Sign::Always,
                        "sign-never" | "+_" => Sign::Never,
                        "sign-except-zero" | "+?" | "sign-accounting-except-zero" | "()?" => {
                            Sign::ExceptZero
                        }
                        "sign-negative" | "+-" | "sign-accounting-negative" | "()-" => {
                            Sign::Negative
                        }
                        _ => Sign::Auto,
                    };
                    "sign"
                }
                "integer-width" => {
                    let option = one_option()?;
                    let unlimited = option.starts_with(['*', '+']);
                    let body = if unlimited { &option[1..] } else { option };
                    let hashes = body.bytes().take_while(|b| *b == b'#').count();
                    let zeros = &body[hashes..];
                    if (unlimited && hashes != 0)
                        || !zeros.bytes().all(|b| b == b'0')
                        || body.len() > 999
                    {
                        return Err(invalid("invalid integer width", start));
                    }
                    result.integer_min = zeros.len() as i16;
                    result.integer_max = (!unlimited).then_some(body.len() as i16);
                    "integer-width"
                }
                "integer-width-trunc" => {
                    no_options()?;
                    result.integer_min = 0;
                    result.integer_max = Some(0);
                    "integer-width"
                }
                _ if stem.starts_with('0') => {
                    no_options()?;
                    if !stem.bytes().all(|b| b == b'0') || stem.len() > 999 {
                        return Err(invalid("invalid integer width", start));
                    }
                    result.integer_min = stem.len() as i16;
                    "integer-width"
                }
                "decimal-auto" | "decimal-always" => {
                    no_options()?;
                    result.decimal_always = stem == "decimal-always";
                    "decimal"
                }
                "latin" => {
                    no_options()?;
                    result.numbering_system = Some("latn".into());
                    "symbols"
                }
                "numbering-system" => {
                    let name = one_option()?;
                    // ICU names numbering systems with one lowercase Unicode type subtag.
                    if !(3..=8).contains(&name.len())
                        || !name
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    {
                        return Err(invalid("invalid numbering system", start));
                    }
                    result.numbering_system = Some(name.into());
                    "symbols"
                }
                "scale" => {
                    result.scale = Literal::parse(one_option()?, start)?;
                    "scale"
                }
                "precision-increment" => {
                    if options.is_empty() || options.len() > 2 {
                        return Err(invalid("expected increment and optional /w", start));
                    }
                    let increment = Literal::parse(options[0], start)?;
                    if !increment.positive() {
                        return Err(invalid("increment must be positive", start));
                    }
                    result.precision = Precision::increment(increment);
                    result.precision.options(&options[1..], start)?;
                    "precision"
                }
                _ => return Err(invalid(format!("unknown skeleton stem: {stem}"), start)),
            };
            if !seen.insert(category) {
                return Err(invalid(format!("duplicate {category}"), start));
            }
        }
        if result.notation.is_compact() {
            if !seen.contains("precision") {
                result.precision = Precision::compact();
            }
            if !seen.contains("grouping") {
                result.grouping = Grouping::Min2;
            }
        }
        Ok(result)
    }
}

pub(crate) fn parse_digits(
    input: &str,
    required: char,
    position: usize,
) -> Result<Digits, NumberFormatError> {
    let min = input.chars().take_while(|c| *c == required).count();
    let tail = &input[min..];
    let max = if matches!(tail, "*" | "+") {
        None
    } else if tail.bytes().all(|b| b == b'#') {
        Some(input.len() as i16)
    } else {
        return Err(invalid("invalid precision blueprint", position));
    };
    if min > 999 || input.len() > 1000 || max.is_some_and(|n| n > 999) {
        return Err(invalid(
            "precision must contain at most 999 digits",
            position,
        ));
    }
    Ok(Digits {
        min: min as i16,
        max,
    })
}
