//! Specified values and the grammars that parse them.

mod calc;
mod color;

use std::str::FromStr;

use avenger_common::pattern::PatternFill;
use cssparser::{ParseError, Parser, Token};

pub(crate) use calc::parse_numeric;
pub use calc::{Calc, Category, Unit, UnitSizes};
pub(crate) use color::parse_color;
pub use color::{Color, SystemColor};

/// A specified value of a property, as a stylesheet or an inline style writes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A number, a length, an angle, or a percentage, alone or in `calc()`.
    Numeric(Calc),
    Integer(i64),
    Color(Color),
    /// A keyword of the property's grammar.
    Keyword(&'static str),
    Boolean(bool),
    /// A name or a quoted string, such as a font family, a palette name, or a pattern.
    String(String),
    /// `<number> / <number>`.
    Ratio(f32, f32),
    /// Values separated by commas.
    List(Vec<Value>),
    /// Values separated by spaces.
    Sequence(Vec<Value>),
}

/// The CSS-wide keywords, which every property takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wide {
    Initial,
    Inherit,
    Unset,
}

impl Wide {
    pub(crate) fn parse(input: &mut Parser<'_>) -> Option<Wide> {
        input
            .try_parse(|input| {
                let wide = match input.expect_ident()?.to_ascii_lowercase().as_str() {
                    "initial" => Wide::Initial,
                    "inherit" => Wide::Inherit,
                    "unset" => Wide::Unset,
                    _ => return Err(ParseError::<()>::custom(())),
                };
                input.expect_exhausted()?;
                Ok(wide)
            })
            .ok()
    }
}

/// The grammar of a property's value.
#[derive(Clone, Copy, Debug)]
pub enum Syntax {
    /// A length, where a number without a unit is a number of pixels.
    Length {
        min: Option<f32>,
    },
    Number {
        min: Option<f32>,
        max: Option<f32>,
    },
    Integer {
        min: i64,
    },
    /// An angle: degrees without a unit, or a dimension in `deg`, `grad`, `rad`, or `turn`.
    Angle,
    Percentage,
    Color,
    /// `true` or `false`.
    Boolean,
    Keywords(&'static [&'static str]),
    /// One or more keywords separated by spaces.
    KeywordList(&'static [&'static str]),
    /// A quoted string.
    String,
    /// Font families separated by commas, each a quoted string or a sequence of names.
    FontFamilies,
    /// `normal`, `bold`, or a weight from 1 to 1000.
    FontWeight,
    /// `<number> [/ <number>]`.
    Ratio,
    /// `<name> [<count>] [reverse]`.
    Scheme,
    /// A symbol name or a quoted SVG path.
    Shape,
    /// A quoted pattern in the text grammar of `avenger_common::pattern`.
    Pattern,
    /// Output values of a scale separated by commas, each one or more components.
    OutputList,
    /// One output value of a scale.
    OutputItem,
    /// A dash pattern: lengths separated by spaces.
    Dash,
    /// A width, a style, and a color in any order, each at most once.
    Line,
    /// `count` values of the grammar separated by spaces, from `min` to `max`.
    Repeat(&'static Syntax, u8, u8),
    /// The first alternative that parses.
    Any(&'static [Syntax]),
}

const LINE_STYLES: &[&str] = &["none", "solid", "dashed", "dotted"];

/// Parses a value of `syntax` from `input`, without the end of the input.
pub(crate) fn parse(syntax: &Syntax, input: &mut Parser<'_>) -> Result<Value, ()> {
    match syntax {
        Syntax::Length { min } => numeric(input, &[Category::Length, Category::Number], *min, None),
        Syntax::Number { min, max } => numeric(input, &[Category::Number], *min, *max),
        Syntax::Percentage => numeric(input, &[Category::Percentage], None, None),
        Syntax::Angle => numeric(input, &[Category::Angle, Category::Number], None, None),
        Syntax::Integer { min } => match input.next().map_err(|_| ())? {
            Token::Number {
                int_value: Some(value),
                ..
            } if i64::from(*value) >= *min => Ok(Value::Integer(i64::from(*value))),
            _ => Err(()),
        },
        Syntax::Color => parse_color(input).map(Value::Color),
        Syntax::Boolean => match input.expect_ident().map_err(|_| ())?.as_ref() {
            "true" => Ok(Value::Boolean(true)),
            "false" => Ok(Value::Boolean(false)),
            _ => Err(()),
        },
        Syntax::Keywords(keywords) => keyword(input, keywords),
        Syntax::KeywordList(keywords) => {
            let mut values = vec![keyword(input, keywords)?];
            while let Ok(value) = input.try_parse(|input| keyword(input, keywords)) {
                values.push(value);
            }
            Ok(Value::Sequence(values))
        }
        Syntax::String => Ok(Value::String(
            input.expect_string().map_err(|_| ())?.to_string(),
        )),
        Syntax::FontFamilies => {
            let families = input
                .parse_comma_separated(|input| family(input).map_err(|()| ParseError::custom(())))
                .map_err(|_: cssparser::ParseError<()>| ())?;
            Ok(Value::List(families))
        }
        Syntax::FontWeight => {
            if let Ok(value) = input.try_parse(|input| keyword(input, &["normal", "bold"])) {
                return Ok(value);
            }
            numeric(input, &[Category::Number], Some(1.0), Some(1000.0))
        }
        Syntax::Ratio => {
            let first = positive_number(input)?;
            let second = input
                .try_parse(|input| {
                    input.expect_delim('/')?;
                    positive_number(input).map_err(|()| ParseError::<()>::custom(()))
                })
                .unwrap_or(1.0);
            Ok(Value::Ratio(first, second))
        }
        Syntax::Scheme => {
            let name = input.expect_ident().map_err(|_| ())?.to_string();
            if name.eq_ignore_ascii_case("auto") {
                return Ok(Value::Keyword("auto"));
            }
            let mut parts = vec![Value::String(name)];
            if let Ok(count) = input.try_parse(|input| parse(&Syntax::Integer { min: 1 }, input)) {
                parts.push(count);
            }
            if input
                .try_parse(|input| input.expect_ident_matching("reverse"))
                .is_ok()
            {
                parts.push(Value::Keyword("reverse"));
            }
            Ok(Value::Sequence(parts))
        }
        Syntax::Shape => match input.next().map_err(|_| ())? {
            Token::Ident(name) => Ok(Value::String(name.to_string())),
            Token::QuotedString(path) => Ok(Value::String(path.to_string())),
            _ => Err(()),
        },
        Syntax::Pattern => {
            if input
                .try_parse(|input| input.expect_ident_matching("none"))
                .is_ok()
            {
                return Ok(Value::Keyword("none"));
            }
            let text = input.expect_string().map_err(|_| ())?.to_string();
            PatternFill::from_str(&text).map_err(|_| ())?;
            Ok(Value::String(text))
        }
        Syntax::OutputList => {
            let items = input
                .parse_comma_separated(|input| {
                    output_item(input).map_err(|()| ParseError::custom(()))
                })
                .map_err(|_: cssparser::ParseError<()>| ())?;
            Ok(Value::List(items))
        }
        Syntax::OutputItem => output_item(input),
        Syntax::Dash => {
            let mut lengths = vec![numeric(
                input,
                &[Category::Length, Category::Number],
                Some(0.0),
                None,
            )?];
            while let Ok(length) = input.try_parse(|input| {
                numeric(
                    input,
                    &[Category::Length, Category::Number],
                    Some(0.0),
                    None,
                )
            }) {
                lengths.push(length);
            }
            Ok(Value::Sequence(lengths))
        }
        Syntax::Line => {
            let mut parts: [Option<Value>; 3] = [None, None, None];
            while !input.is_exhausted() {
                if parts[0].is_none() {
                    if let Ok(width) = input.try_parse(|input| {
                        numeric(
                            input,
                            &[Category::Length, Category::Number],
                            Some(0.0),
                            None,
                        )
                    }) {
                        parts[0] = Some(width);
                        continue;
                    }
                }
                if parts[1].is_none() {
                    if let Ok(style) = input.try_parse(|input| keyword(input, LINE_STYLES)) {
                        parts[1] = Some(style);
                        continue;
                    }
                }
                if parts[2].is_none() {
                    if let Ok(color) = input.try_parse(parse_color) {
                        parts[2] = Some(Value::Color(color));
                        continue;
                    }
                }
                return Err(());
            }
            if parts.iter().all(Option::is_none) {
                return Err(());
            }
            Ok(Value::Sequence(parts.into_iter().flatten().collect()))
        }
        Syntax::Repeat(item, min, max) => {
            let mut values = Vec::new();
            while values.len() < usize::from(*max) {
                match input.try_parse(|input| parse(item, input)) {
                    Ok(value) => values.push(value),
                    Err(()) => break,
                }
            }
            if values.len() < usize::from(*min) {
                return Err(());
            }
            Ok(Value::Sequence(values))
        }
        Syntax::Any(alternatives) => {
            for alternative in alternatives.iter() {
                if let Ok(value) = input.try_parse(|input| {
                    let value = parse(alternative, input)?;
                    match input.is_exhausted() {
                        true => Ok(value),
                        false => Err(()),
                    }
                }) {
                    return Ok(value);
                }
            }
            Err(())
        }
    }
}

/// Parses a whole value of `syntax` from `input`.
pub(crate) fn parse_all(syntax: &Syntax, input: &mut Parser<'_>) -> Result<Value, ()> {
    let value = parse(syntax, input)?;
    input.expect_exhausted().map_err(|_| ())?;
    Ok(value)
}

fn numeric(
    input: &mut Parser<'_>,
    categories: &[Category],
    min: Option<f32>,
    max: Option<f32>,
) -> Result<Value, ()> {
    let state = input.state();
    let is_calc =
        matches!(input.next(), Ok(Token::Function(name)) if name.eq_ignore_ascii_case("calc"));
    input.reset(&state);
    let calc = parse_numeric(input).map_err(|_| ())?;
    let category = calc.category().ok_or(())?;
    if !categories.contains(&category) {
        return Err(());
    }
    // A range applies to a value without `calc()`, and the cascade clamps the value of a
    // `calc()`, as CSS does.
    if let (Calc::Leaf(value, _), false) = (&calc, is_calc) {
        if min.is_some_and(|min| *value < min) || max.is_some_and(|max| *value > max) {
            return Err(());
        }
    }
    Ok(Value::Numeric(calc))
}

fn positive_number(input: &mut Parser<'_>) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Number { value, .. } if *value > 0.0 => Ok(*value),
        _ => Err(()),
    }
}

fn keyword(input: &mut Parser<'_>, keywords: &'static [&'static str]) -> Result<Value, ()> {
    let ident = input.expect_ident().map_err(|_| ())?;
    keywords
        .iter()
        .find(|keyword| ident.eq_ignore_ascii_case(keyword))
        .map(|keyword| Value::Keyword(keyword))
        .ok_or(())
}

fn family(input: &mut Parser<'_>) -> Result<Value, ()> {
    if let Ok(name) = input.try_parse(|input| input.expect_string().map(|name| name.to_string())) {
        return Ok(Value::String(name));
    }
    let mut words = vec![input.expect_ident().map_err(|_| ())?.to_string()];
    while let Ok(word) = input.try_parse(|input| input.expect_ident().map(|word| word.to_string()))
    {
        words.push(word);
    }
    Ok(Value::String(words.join(" ")))
}

/// One output value of a scale: components separated by spaces, each a number, a length, a
/// color, a name, or a quoted string, such as `#e69f00`, `circle`, or the dash pattern `4 2`.
fn output_item(input: &mut Parser<'_>) -> Result<Value, ()> {
    let mut parts = Vec::new();
    loop {
        let state = input.state();
        if input.is_exhausted() {
            break;
        }
        if let Ok(value) = input.try_parse(|input| {
            numeric(
                input,
                &[
                    Category::Number,
                    Category::Length,
                    Category::Angle,
                    Category::Percentage,
                ],
                None,
                None,
            )
        }) {
            parts.push(value);
            continue;
        }
        if let Ok(color) = input.try_parse(parse_color) {
            parts.push(Value::Color(color));
            continue;
        }
        match input.next() {
            Ok(Token::Ident(name)) => parts.push(Value::String(name.to_string())),
            Ok(Token::QuotedString(text)) => parts.push(Value::String(text.to_string())),
            _ => {
                input.reset(&state);
                break;
            }
        }
    }
    match parts.len() {
        0 => Err(()),
        1 => Ok(parts.pop().expect("one part")),
        _ => Ok(Value::Sequence(parts)),
    }
}
