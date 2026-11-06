//! Numbers, lengths, angles, and percentages, alone or in `calc()`.

use cssparser::{ParseError, Parser, Token};

/// The unit of a numeric value. A number without a unit is a length in pixels where a length is
/// expected, and an angle in degrees where an angle is expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    Number,
    Px,
    Rem,
    Em,
    Deg,
    Grad,
    Rad,
    Turn,
    Percent,
}

impl Unit {
    fn from_dimension(unit: &str) -> Option<Unit> {
        Some(match unit.to_ascii_lowercase().as_str() {
            "px" => Unit::Px,
            "rem" => Unit::Rem,
            "em" => Unit::Em,
            "deg" => Unit::Deg,
            "grad" => Unit::Grad,
            "rad" => Unit::Rad,
            "turn" => Unit::Turn,
            _ => return None,
        })
    }

    fn category(self) -> Category {
        match self {
            Unit::Number => Category::Number,
            Unit::Deg | Unit::Grad | Unit::Rad | Unit::Turn => Category::Angle,
            Unit::Percent => Category::Percentage,
            _ => Category::Length,
        }
    }
}

/// What a numeric value measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Number,
    Length,
    Angle,
    Percentage,
}

/// A numeric value: a number with its unit, or a `calc()` expression over such values, whose units
/// resolve when the cascade computes the value.
#[derive(Clone, Debug, PartialEq)]
pub enum Calc {
    Leaf(f32, Unit),
    Sum(Box<Calc>, Box<Calc>),
    Difference(Box<Calc>, Box<Calc>),
    Product(Box<Calc>, Box<Calc>),
    Quotient(Box<Calc>, Box<Calc>),
}

/// The sizes that the units of a value read, in pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitSizes {
    /// The size of `1rem`.
    pub rem: f32,
    /// The size of `1em`.
    pub em: f32,
}

impl Calc {
    /// What the value measures, or `None` for an expression that combines values of two
    /// categories that do not combine. A number combines with every category, because a number
    /// without a unit is a length in pixels or an angle in degrees.
    pub fn category(&self) -> Option<Category> {
        match self {
            Calc::Leaf(_, unit) => Some(unit.category()),
            Calc::Sum(a, b) | Calc::Difference(a, b) => match (a.category()?, b.category()?) {
                (Category::Number, other) | (other, Category::Number) => Some(other),
                (a, b) if a == b => Some(a),
                _ => None,
            },
            Calc::Product(a, b) => match (a.category()?, b.category()?) {
                (Category::Number, other) | (other, Category::Number) => Some(other),
                _ => None,
            },
            Calc::Quotient(a, b) => match b.category()? {
                Category::Number => a.category(),
                _ => None,
            },
        }
    }

    /// The value in pixels, degrees, a fraction for a percentage, or a number, with the sizes of
    /// `rem` and `em` that `sizes` gives.
    pub fn resolve(&self, sizes: &UnitSizes) -> f32 {
        match self {
            Calc::Leaf(value, unit) => {
                let scale = match unit {
                    Unit::Number | Unit::Px | Unit::Deg => 1.0,
                    Unit::Grad => 0.9,
                    Unit::Rad => 180.0 / std::f32::consts::PI,
                    Unit::Turn => 360.0,
                    Unit::Percent => 0.01,
                    Unit::Rem => sizes.rem,
                    Unit::Em => sizes.em,
                };
                value * scale
            }
            Calc::Sum(a, b) => a.resolve(sizes) + b.resolve(sizes),
            Calc::Difference(a, b) => a.resolve(sizes) - b.resolve(sizes),
            Calc::Product(a, b) => a.resolve(sizes) * b.resolve(sizes),
            Calc::Quotient(a, b) => a.resolve(sizes) / b.resolve(sizes),
        }
    }

    /// The value of an expression of numbers only, such as a component of a color.
    pub fn constant(&self) -> Option<f32> {
        match self.category()? {
            Category::Number | Category::Percentage | Category::Angle => {
                if self.reads_relative() {
                    return None;
                }
                Some(self.resolve(&UnitSizes::default()))
            }
            Category::Length => None,
        }
    }

    fn reads_relative(&self) -> bool {
        match self {
            Calc::Leaf(_, unit) => !matches!(
                unit,
                Unit::Number | Unit::Deg | Unit::Grad | Unit::Rad | Unit::Turn | Unit::Percent
            ),
            Calc::Sum(a, b)
            | Calc::Difference(a, b)
            | Calc::Product(a, b)
            | Calc::Quotient(a, b) => a.reads_relative() || b.reads_relative(),
        }
    }
}

/// Why a numeric value does not parse.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NumericError {
    /// A token that is not a numeric value.
    NotNumeric,
    /// A dimension with a unit that the style language does not have.
    Unit(String),
    /// An expression that combines categories that do not combine.
    Mixed,
}

/// Parses one numeric value: a number, a percentage, a dimension, or `calc()`.
pub(crate) fn parse_numeric(input: &mut Parser<'_>) -> Result<Calc, NumericError> {
    let token = input.next().map_err(|_| NumericError::NotNumeric)?.clone();
    let calc = match token {
        Token::Number { value, .. } => Calc::Leaf(value, Unit::Number),
        Token::Percentage { unit_value, .. } => Calc::Leaf(unit_value * 100.0, Unit::Percent),
        Token::Dimension {
            value, ref unit, ..
        } => match Unit::from_dimension(unit) {
            Some(unit) => Calc::Leaf(value, unit),
            None => return Err(NumericError::Unit(unit.to_string())),
        },
        Token::Function(ref name) if name.eq_ignore_ascii_case("calc") => input
            .parse_nested_block(|input| {
                let sum = sum(input).map_err(ParseError::custom)?;
                input
                    .expect_exhausted()
                    .map_err(|_| ParseError::custom(NumericError::NotNumeric))?;
                Ok(sum)
            })
            .map_err(|error| match error.kind {
                cssparser::ParseErrorKind::Custom(error) => error,
                _ => NumericError::NotNumeric,
            })?,
        _ => return Err(NumericError::NotNumeric),
    };
    if calc.category().is_none() {
        return Err(NumericError::Mixed);
    }
    Ok(calc)
}

fn sum(input: &mut Parser<'_>) -> Result<Calc, NumericError> {
    let mut left = product(input)?;
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Delim('+')) => left = Calc::Sum(Box::new(left), Box::new(product(input)?)),
            Ok(Token::Delim('-')) => {
                left = Calc::Difference(Box::new(left), Box::new(product(input)?))
            }
            _ => {
                input.reset(&state);
                return Ok(left);
            }
        }
    }
}

fn product(input: &mut Parser<'_>) -> Result<Calc, NumericError> {
    let mut left = term(input)?;
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Delim('*')) => left = Calc::Product(Box::new(left), Box::new(term(input)?)),
            Ok(Token::Delim('/')) => left = Calc::Quotient(Box::new(left), Box::new(term(input)?)),
            _ => {
                input.reset(&state);
                return Ok(left);
            }
        }
    }
}

fn term(input: &mut Parser<'_>) -> Result<Calc, NumericError> {
    let state = input.state();
    if let Ok(Token::ParenthesisBlock) = input.next() {
        return input
            .parse_nested_block(|input| {
                let sum = sum(input).map_err(ParseError::custom)?;
                input
                    .expect_exhausted()
                    .map_err(|_| ParseError::custom(NumericError::NotNumeric))?;
                Ok(sum)
            })
            .map_err(|error| match error.kind {
                cssparser::ParseErrorKind::Custom(error) => error,
                _ => NumericError::NotNumeric,
            });
    }
    input.reset(&state);
    parse_numeric(input)
}
