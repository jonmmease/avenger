//! CSS calc() expression support
//!
//! This module implements CSS calc() expressions following the CSS Values and Units Module Level 4
//! specification. It provides a complete implementation of mathematical expressions in CSS,
//! including runtime parameter substitution for dynamic theming.
//!
//! # Features
//!
//! ## Basic Arithmetic
//! - Addition: `calc(10px + 5px)` → `15px`
//! - Subtraction: `calc(100px - 20px)` → `80px`
//! - Multiplication: `calc(8px * 2)` → `16px`
//! - Division: `calc(80px / 4)` → `20px`
//! - Parentheses: `calc((100px - 20px) / 4)` → `20px`
//!
//! ## Unit System
//! - Length units: `px`, `rem`
//! - Percentage: `%`
//! - Angle units: `deg`, `rad`, `grad`, `turn`
//! - Unit conversion: `rem` values are converted to `px` using the base font size
//!
//! ## Math Functions
//! - `min()`: Returns the smallest value from a list
//! - `max()`: Returns the largest value from a list
//! - `clamp()`: Clamps a value between min and max
//! - `abs()`: Returns the absolute value
//! - `sign()`: Returns the sign of a number (-1, 0, or 1)
//! - `round()`: Rounds to nearest, up, down, or toward zero
//! - `mod()`: Modulo operation
//! - `rem()`: Remainder operation
//! - Trigonometric: `sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`, `atan2()`
//! - Exponential: `pow()`, `sqrt()`, `exp()`, `log()`
//! - `hypot()`: Hypotenuse (Euclidean distance)
//!
//! ## CSS Variables (Runtime Parameters)
//! CSS variables are resolved from runtime parameters:
//! ```ignore
//! // CSS
//! mark {
//!     font-size: calc(var(--base-size) * 2);
//! }
//!
//! // Rust
//! let mut params = IndexMap::new();
//! params.insert("base-size".to_string(), ScalarValue::Float64(Some(12.0)));
//! let ctx = ThemeContext::new("mark", params);
//! let size = theme.font_size(&ctx); // => Some(24.0)
//! ```
//!
//! ## Mathematical Constants
//! - `pi` (π): approximately 3.14159
//! - `e`: Euler's number, approximately 2.71828
//!
//! ## Relative Color Syntax (Phase 9)
//! Channel keywords for deriving colors from existing colors:
//! ```ignore
//! // Darken a color by reducing lightness
//! oklch(from var(--primary) calc(l - 0.2) c h)
//!
//! // Adjust saturation
//! hsl(from blue h calc(s * 0.5) l)
//!
//! // Rotate hue for complementary color
//! oklch(from blue l c calc(h + 180deg))
//! ```
//!
//! # Architecture
//!
//! The implementation consists of three main phases:
//!
//! 1. **Parsing** (`parser.rs`): Converts CSS text into an AST (`CalcNode` tree)
//! 2. **Simplification**: Performs constant folding and algebraic simplification
//! 3. **Resolution**: Substitutes variables and evaluates to a final value
//!
//! # Examples
//!
//! ## Basic Usage
//! ```ignore
//! use avenger_chart::theme::Theme;
//!
//! let css = r#"
//!     mark {
//!         font-size: calc(10px + 5px);
//!     }
//! "#;
//!
//! let theme = Theme::from_css(css).unwrap();
//! let ctx = ThemeContext::new("mark");
//! let size = theme.font_size(&ctx); // => Some(15.0)
//! ```
//!
//! ## With Runtime Parameters
//! ```ignore
//! use avenger_chart::theme::{Theme, ThemeContext};
//! use datafusion::common::ScalarValue;
//! use indexmap::IndexMap;
//!
//! let css = r#"
//!     mark {
//!         font-size: calc(var(--base) + var(--offset));
//!     }
//! "#;
//!
//! let theme = Theme::from_css(css).unwrap();
//!
//! let mut params = IndexMap::new();
//! params.insert("base".to_string(), ScalarValue::Float64(Some(12.0)));
//! params.insert("offset".to_string(), ScalarValue::Float64(Some(4.0)));
//!
//! let ctx = ThemeContext::new("mark", params);
//! let size = theme.font_size(&ctx); // => Some(16.0)
//! ```
//!
//! ## Math Functions
//! ```ignore
//! // Use min() to cap a value
//! font-size: min(calc(var(--base) * 1.5), 24px);
//!
//! // Use clamp() for responsive sizing
//! font-size: clamp(12px, calc(var(--scale) * 1rem), 20px);
//! ```

use super::value::{AngleUnit, LengthUnit};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Calculation node - tree structure for calc expressions
///
/// This is the core AST node for CSS calc() expressions. It supports all
/// CSS calc operations and can be evaluated with runtime parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CalcNode {
    /// Leaf values (numbers, lengths, percentages, angles, variables, channel keywords)
    Leaf(CalcLeaf),

    /// Unary negation: -x
    /// CSS: calc(-10px) or calc(-(100% - 20px))
    Negate(Box<CalcNode>),

    /// Sum of terms: a + b + c
    /// Subtraction is represented as addition of negated term: a - b = a + (-b)
    /// CSS: calc(10px + 5px - 3px) => Sum([10px, 5px, -3px])
    Sum(Vec<CalcNode>),

    /// Product of factors: a * b * c
    /// Division is represented as multiplication by inverted factor: a / b = a * (1/b)
    /// CSS: calc(10px * 2 / 5) => Product([10px, 2, 1/5])
    Product(Vec<CalcNode>),

    /// Division helper: 1/x (used within Product for division)
    /// This is an internal representation, not directly parsed from CSS
    Invert(Box<CalcNode>),

    /// min() function: min(a, b, c, ...)
    /// CSS: min(100px, 50%, 10rem)
    Min(Vec<CalcNode>),

    /// max() function: max(a, b, c, ...)
    /// CSS: max(100px, 50%, 10rem)
    Max(Vec<CalcNode>),

    /// clamp() function: clamp(min, value, max)
    /// CSS: clamp(10px, 5vw, 20px)
    Clamp {
        min: Box<CalcNode>,
        center: Box<CalcNode>,
        max: Box<CalcNode>,
    },

    /// abs() function: abs(x)
    /// CSS: abs(-10px) => 10px
    Abs(Box<CalcNode>),

    /// sign() function: sign(x) returns -1, 0, or 1
    /// CSS: sign(-10px) => -1
    Sign(Box<CalcNode>),

    /// round() function: round(strategy, value, step)
    /// CSS: round(nearest, 23px, 10px) => 20px
    Round {
        strategy: RoundingStrategy,
        value: Box<CalcNode>,
        step: Box<CalcNode>,
    },

    /// mod() function: modulo operation
    /// CSS: mod(18px, 5px) => 3px
    Mod {
        dividend: Box<CalcNode>,
        divisor: Box<CalcNode>,
    },

    /// rem() function: remainder operation
    /// CSS: rem(-18px, 5px) => -3px
    Rem {
        dividend: Box<CalcNode>,
        divisor: Box<CalcNode>,
    },

    /// hypot() function: Euclidean distance
    /// CSS: hypot(3px, 4px) => 5px
    Hypot(Vec<CalcNode>),

    /// sin() function: sine (input in radians or degrees)
    Sin(Box<CalcNode>),

    /// cos() function: cosine (input in radians or degrees)
    Cos(Box<CalcNode>),

    /// tan() function: tangent (input in radians or degrees)
    Tan(Box<CalcNode>),

    /// asin() function: arcsine (returns angle)
    Asin(Box<CalcNode>),

    /// acos() function: arccosine (returns angle)
    Acos(Box<CalcNode>),

    /// atan() function: arctangent (returns angle)
    Atan(Box<CalcNode>),

    /// atan2() function: two-argument arctangent
    Atan2 { y: Box<CalcNode>, x: Box<CalcNode> },

    /// pow() function: exponentiation
    Pow {
        base: Box<CalcNode>,
        exponent: Box<CalcNode>,
    },

    /// sqrt() function: square root
    Sqrt(Box<CalcNode>),

    /// exp() function: e^x
    Exp(Box<CalcNode>),

    /// log() function: logarithm
    Log {
        value: Box<CalcNode>,
        base: Option<Box<CalcNode>>,
    },
}

/// Rounding strategy for round() function
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundingStrategy {
    /// Round to nearest (standard rounding)
    Nearest,
    /// Round up (ceiling)
    Up,
    /// Round down (floor)
    Down,
    /// Round toward zero
    ToZero,
}

/// Leaf values in calc expressions
///
/// These are the terminal nodes in the calc AST. They represent concrete values
/// or references that will be resolved at runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CalcLeaf {
    /// Pure number (unitless)
    /// CSS: calc(5 * 2) => Number(10)
    Number(f64),

    /// Length with unit (px, rem)
    /// CSS: 10px => Length(10.0, Px)
    Length(f64, LengthUnit),

    /// Percentage (0-100 range as stored, e.g., 50.0 for 50%)
    /// CSS: 50% => Percentage(50.0)
    Percentage(f64),

    /// Angle with unit (deg, rad, grad, turn)
    /// CSS: 180deg => Angle(180.0, Deg)
    Angle(f64, AngleUnit),

    /// CSS variable reference (e.g., "--base-size" from var(--base-size))
    /// Resolved at runtime using theme context parameters
    /// CSS: var(--size) => Variable("size".to_string())
    Variable(String),

    /// Channel keyword for relative color syntax (CSS Color Level 5)
    /// CSS: oklch(from blue l c h) => ChannelKeyword(L), ChannelKeyword(C), ChannelKeyword(H)
    ChannelKeyword(ChannelKeyword),
}

/// Channel keywords for relative color syntax (CSS Color Level 5)
///
/// These represent components of the origin color in relative color functions:
/// - oklch(from blue L C H) - L/C/H refer to blue's components in Oklch space
/// - rgb(from red R G B) - R/G/B refer to red's components in sRGB space
///
/// # Examples
///
/// ```ignore
/// // Darken a color by reducing lightness
/// oklch(from var(--primary) calc(l - 0.2) c h)
///
/// // Adjust saturation
/// hsl(from blue h calc(s * 0.5) l)
///
/// // Rotate hue for complementary color
/// oklch(from blue l c calc(h + 180deg))
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKeyword {
    /// Lightness/Luminance (Lab, Lch, Oklab, Oklch, Hsl)
    L,

    /// Red component (RGB, sRGB)
    R,
    /// Green component (RGB, sRGB)
    G,
    /// Blue component (RGB, sRGB)
    B,

    /// Chroma (Oklch, Lch)
    C,

    /// Hue (Hsl, Hwb, Oklch, Lch) - in degrees
    H,

    /// Saturation (Hsl)
    S,

    /// Whiteness (Hwb)
    W,
    /// Blackness (Hwb) - note: 'b' conflicts with blue, so named BlacknessB
    BlacknessB,

    /// A axis (Lab, Oklab)
    A,
    /// B axis (Lab, Oklab) - note: conflicts with blue, so named LabB
    LabB,

    /// X component (XYZ color space)
    X,
    /// Y component (XYZ color space)
    Y,
    /// Z component (XYZ color space)
    Z,

    /// Alpha channel (all color spaces)
    Alpha,
}

/// Unit categories for type checking
///
/// This tracks what unit types are allowed in a calc expression and ensures
/// compatibility across operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcUnits {
    /// No unit (pure number)
    None,

    /// Length unit (px, rem, etc.)
    Length,

    /// Percentage
    Percentage,

    /// Can mix length and percentage (for properties like width, padding)
    /// This is common in CSS where percentages and lengths are additive
    LengthPercentage,

    /// Angle (deg, rad, grad, turn)
    Angle,

    /// Unknown/undetermined (for variables before substitution)
    Unknown,
}

impl ChannelKeyword {
    /// Parse from CSS identifier
    ///
    /// Returns None if the identifier is not a valid channel keyword.
    pub fn from_ident(ident: &str) -> Option<Self> {
        match ident.to_lowercase().as_str() {
            "l" => Some(Self::L),
            "r" => Some(Self::R),
            "g" => Some(Self::G),
            "b" => Some(Self::B),
            "c" => Some(Self::C),
            "h" => Some(Self::H),
            "s" => Some(Self::S),
            "w" => Some(Self::W),
            "a" => Some(Self::A),
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "z" => Some(Self::Z),
            "alpha" => Some(Self::Alpha),
            _ => None,
        }
    }

    /// Parse a channel keyword with color space context
    ///
    /// This disambiguates between:
    /// - 'b' as RGB blue vs Lab/Oklab b-axis
    /// - 'a' as Lab/Oklab a-axis vs alpha (though 'a' defaults to a-axis in Lab/Oklab)
    pub fn from_ident_with_color_space(
        ident: &str,
        color_space: Option<crate::color::types::ColorSpace>,
    ) -> Option<Self> {
        use crate::color::types::ColorSpace;

        let lower = ident.to_lowercase();
        match (lower.as_str(), color_space) {
            // In Lab/Oklab contexts, 'b' refers to the b-axis, not blue
            ("b", Some(ColorSpace::Lab | ColorSpace::Oklab)) => Some(Self::LabB),

            // In HWB context, 'b' refers to blackness, not blue
            ("b", Some(ColorSpace::Hwb)) => Some(Self::BlacknessB),

            // Otherwise use the generic parser
            _ => Self::from_ident(ident),
        }
    }
}

impl CalcLeaf {
    /// Get the unit type of this leaf
    pub fn units(&self) -> CalcUnits {
        match self {
            CalcLeaf::Number(_) => CalcUnits::None,
            CalcLeaf::Length(_, _) => CalcUnits::Length,
            CalcLeaf::Percentage(_) => CalcUnits::Percentage,
            CalcLeaf::Angle(_, _) => CalcUnits::Angle,
            CalcLeaf::Variable(_) => CalcUnits::Unknown,
            CalcLeaf::ChannelKeyword(_) => CalcUnits::Unknown, // Depends on color space context
        }
    }

    /// Negate value (multiply by -1)
    pub fn negate(&self) -> CalcLeaf {
        match self {
            CalcLeaf::Number(n) => CalcLeaf::Number(-n),
            CalcLeaf::Length(n, unit) => CalcLeaf::Length(-n, *unit),
            CalcLeaf::Percentage(n) => CalcLeaf::Percentage(-n),
            CalcLeaf::Angle(n, unit) => CalcLeaf::Angle(-n, *unit),
            CalcLeaf::Variable(_) => {
                unreachable!("Variables should be substituted before negation")
            }
            CalcLeaf::ChannelKeyword(_) => {
                unreachable!("Channel keywords should be substituted before negation")
            }
        }
    }

    /// Get absolute value
    pub fn abs(&self) -> CalcLeaf {
        match self {
            CalcLeaf::Number(n) => CalcLeaf::Number(n.abs()),
            CalcLeaf::Length(n, unit) => CalcLeaf::Length(n.abs(), *unit),
            CalcLeaf::Percentage(n) => CalcLeaf::Percentage(n.abs()),
            CalcLeaf::Angle(n, unit) => CalcLeaf::Angle(n.abs(), *unit),
            CalcLeaf::Variable(_) => unreachable!("Variables should be substituted before abs"),
            CalcLeaf::ChannelKeyword(_) => {
                unreachable!("Channel keywords should be substituted before abs")
            }
        }
    }

    /// Get sign (-1, 0, or 1)
    /// Always returns a number
    pub fn sign(&self) -> CalcLeaf {
        let value = match self {
            CalcLeaf::Number(n) => *n,
            CalcLeaf::Length(n, _) => *n,
            CalcLeaf::Percentage(n) => *n,
            CalcLeaf::Angle(n, _) => *n,
            CalcLeaf::Variable(_) => unreachable!("Variables should be substituted before sign"),
            CalcLeaf::ChannelKeyword(_) => {
                unreachable!("Channel keywords should be substituted before sign")
            }
        };

        CalcLeaf::Number(if value > 0.0 {
            1.0
        } else if value < 0.0 {
            -1.0
        } else {
            0.0
        })
    }

    /// Add two leaf values (checks unit compatibility)
    pub fn add(&self, other: &CalcLeaf) -> Result<CalcLeaf, String> {
        match (self, other) {
            (CalcLeaf::Number(a), CalcLeaf::Number(b)) => Ok(CalcLeaf::Number(a + b)),
            (CalcLeaf::Length(a, unit1), CalcLeaf::Length(b, unit2)) if unit1 == unit2 => {
                Ok(CalcLeaf::Length(a + b, *unit1))
            }
            (CalcLeaf::Percentage(a), CalcLeaf::Percentage(b)) => Ok(CalcLeaf::Percentage(a + b)),
            (CalcLeaf::Angle(a, unit1), CalcLeaf::Angle(b, unit2)) if unit1 == unit2 => {
                Ok(CalcLeaf::Angle(a + b, *unit1))
            }
            // Length + Percentage is allowed in CSS for certain properties
            // This is handled at the property level, but we allow it here
            (CalcLeaf::Length(_, _), CalcLeaf::Percentage(_))
            | (CalcLeaf::Percentage(_), CalcLeaf::Length(_, _)) => {
                // Return as-is, cannot simplify without context
                // In reality, we should return a LengthPercentage type
                Err(format!(
                    "Cannot add {:?} and {:?} without context",
                    self, other
                ))
            }
            _ => Err(format!(
                "Incompatible units for addition: {:?} + {:?}",
                self, other
            )),
        }
    }

    /// Multiply two leaf values (at most one can have units)
    pub fn multiply(&self, other: &CalcLeaf) -> Result<CalcLeaf, String> {
        match (self, other) {
            (CalcLeaf::Number(a), CalcLeaf::Number(b)) => Ok(CalcLeaf::Number(a * b)),
            (CalcLeaf::Number(n), CalcLeaf::Length(l, unit))
            | (CalcLeaf::Length(l, unit), CalcLeaf::Number(n)) => {
                Ok(CalcLeaf::Length(n * l, *unit))
            }
            (CalcLeaf::Number(n), CalcLeaf::Percentage(p))
            | (CalcLeaf::Percentage(p), CalcLeaf::Number(n)) => Ok(CalcLeaf::Percentage(n * p)),
            (CalcLeaf::Number(n), CalcLeaf::Angle(a, unit))
            | (CalcLeaf::Angle(a, unit), CalcLeaf::Number(n)) => Ok(CalcLeaf::Angle(n * a, *unit)),
            _ => Err(format!(
                "Cannot multiply two dimensioned values: {:?} * {:?}",
                self, other
            )),
        }
    }

    /// Divide two leaf values
    pub fn divide(&self, other: &CalcLeaf) -> Result<CalcLeaf, String> {
        match (self, other) {
            (_, CalcLeaf::Number(b)) if *b == 0.0 => Err("Division by zero".to_string()),
            (CalcLeaf::Number(a), CalcLeaf::Number(b)) => Ok(CalcLeaf::Number(a / b)),
            (CalcLeaf::Length(l, unit), CalcLeaf::Number(n)) => Ok(CalcLeaf::Length(l / n, *unit)),
            (CalcLeaf::Percentage(p), CalcLeaf::Number(n)) => Ok(CalcLeaf::Percentage(p / n)),
            (CalcLeaf::Angle(a, unit), CalcLeaf::Number(n)) => Ok(CalcLeaf::Angle(a / n, *unit)),
            // Dividing same units gives a number
            (CalcLeaf::Length(a, unit1), CalcLeaf::Length(b, unit2)) if unit1 == unit2 => {
                Ok(CalcLeaf::Number(a / b))
            }
            (CalcLeaf::Percentage(a), CalcLeaf::Percentage(b)) => Ok(CalcLeaf::Number(a / b)),
            (CalcLeaf::Angle(a, unit1), CalcLeaf::Angle(b, unit2)) if unit1 == unit2 => {
                Ok(CalcLeaf::Number(a / b))
            }
            _ => Err(format!("Invalid division: {:?} / {:?}", self, other)),
        }
    }

    /// Convert to pixels (if possible with given base font size)
    /// Returns None for percentages (which need layout context)
    pub fn as_length_px(&self, base_font_size: f32) -> Option<f32> {
        match self {
            CalcLeaf::Length(n, LengthUnit::Px) => Some(*n as f32),
            CalcLeaf::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            CalcLeaf::Number(n) => Some(*n as f32), // Interpret bare numbers as px
            CalcLeaf::Percentage(_) => None,        // Need context
            CalcLeaf::Angle(_, _) => None,
            CalcLeaf::Variable(_) => None,
            CalcLeaf::ChannelKeyword(_) => None,
        }
    }

    /// Get as number (if unitless)
    pub fn as_number(&self) -> Option<f64> {
        match self {
            CalcLeaf::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get as angle in degrees
    pub fn as_angle_degrees(&self) -> Option<f64> {
        match self {
            CalcLeaf::Angle(value, unit) => Some(unit.to_degrees(*value)),
            CalcLeaf::Number(n) => Some(*n), // Interpret as degrees
            _ => None,
        }
    }
}

impl PartialOrd for CalcLeaf {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // For min/max, we need to compare values
        // This only works if units are compatible
        match (self, other) {
            (CalcLeaf::Number(a), CalcLeaf::Number(b)) => a.partial_cmp(b),
            (CalcLeaf::Length(a, u1), CalcLeaf::Length(b, u2)) if u1 == u2 => a.partial_cmp(b),
            (CalcLeaf::Percentage(a), CalcLeaf::Percentage(b)) => a.partial_cmp(b),
            (CalcLeaf::Angle(a, u1), CalcLeaf::Angle(b, u2)) if u1 == u2 => a.partial_cmp(b),
            _ => None,
        }
    }
}

impl CalcLeaf {
    /// Get minimum of two values (if comparable)
    pub fn min(&self, other: &Self) -> Result<CalcLeaf, String> {
        match self.partial_cmp(other) {
            Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal) => Ok(self.clone()),
            Some(std::cmp::Ordering::Greater) => Ok(other.clone()),
            None => Err(format!("Cannot compare {:?} and {:?}", self, other)),
        }
    }

    /// Get maximum of two values (if comparable)
    pub fn max(&self, other: &Self) -> Result<CalcLeaf, String> {
        match self.partial_cmp(other) {
            Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal) => Ok(self.clone()),
            Some(std::cmp::Ordering::Less) => Ok(other.clone()),
            None => Err(format!("Cannot compare {:?} and {:?}", self, other)),
        }
    }

    /// Clamp value between min and max
    pub fn clamp(&self, min: &Self, max: &Self) -> Result<CalcLeaf, String> {
        // First clamp to min
        let above_min = self.max(min)?;
        // Then clamp to max
        above_min.min(max)
    }
}

impl CalcNode {
    /// Check unit compatibility for this node
    ///
    /// Returns the output unit type or error if units are incompatible.
    pub fn check_units(&self) -> Result<CalcUnits, String> {
        match self {
            CalcNode::Leaf(leaf) => Ok(leaf.units()),
            CalcNode::Negate(node) => node.check_units(),
            CalcNode::Sum(nodes) => {
                if nodes.is_empty() {
                    return Err("Empty sum".to_string());
                }
                let mut result_units = nodes[0].check_units()?;
                for node in &nodes[1..] {
                    let node_units = node.check_units()?;
                    result_units = combine_units_for_sum(result_units, node_units)?;
                }
                Ok(result_units)
            }
            CalcNode::Product(nodes) => {
                if nodes.is_empty() {
                    return Err("Empty product".to_string());
                }
                let mut result_units = nodes[0].check_units()?;
                for node in &nodes[1..] {
                    let node_units = node.check_units()?;
                    result_units = combine_units_for_product(result_units, node_units)?;
                }
                Ok(result_units)
            }
            CalcNode::Invert(node) => {
                let units = node.check_units()?;
                // Can only invert numbers
                if units != CalcUnits::None && units != CalcUnits::Unknown {
                    return Err(format!("Cannot invert value with units: {:?}", units));
                }
                Ok(CalcUnits::None)
            }
            CalcNode::Min(nodes) | CalcNode::Max(nodes) | CalcNode::Hypot(nodes) => {
                if nodes.is_empty() {
                    return Err("Empty min/max/hypot".to_string());
                }
                let mut result_units = nodes[0].check_units()?;
                for node in &nodes[1..] {
                    let node_units = node.check_units()?;
                    result_units = combine_units_for_minmax(result_units, node_units)?;
                }
                Ok(result_units)
            }
            CalcNode::Clamp { min, center, max } => {
                let min_units = min.check_units()?;
                let center_units = center.check_units()?;
                let max_units = max.check_units()?;
                let result = combine_units_for_minmax(min_units, center_units)?;
                combine_units_for_minmax(result, max_units)
            }
            CalcNode::Abs(node) => node.check_units(),
            CalcNode::Sign(_) => Ok(CalcUnits::None), // sign() always returns a number
            CalcNode::Round { value, step, .. } => {
                let value_units = value.check_units()?;
                let step_units = step.check_units()?;
                combine_units_for_minmax(value_units, step_units)
            }
            CalcNode::Mod { dividend, divisor } | CalcNode::Rem { dividend, divisor } => {
                let dividend_units = dividend.check_units()?;
                let divisor_units = divisor.check_units()?;
                combine_units_for_minmax(dividend_units, divisor_units)
            }
            CalcNode::Sin(_) | CalcNode::Cos(_) | CalcNode::Tan(_) => {
                // Trig functions return numbers
                Ok(CalcUnits::None)
            }
            CalcNode::Asin(_) | CalcNode::Acos(_) | CalcNode::Atan(_) => {
                // Inverse trig returns angles
                Ok(CalcUnits::Angle)
            }
            CalcNode::Atan2 { .. } => Ok(CalcUnits::Angle),
            CalcNode::Pow { base, .. } => {
                // Result has same units as base
                base.check_units()
            }
            CalcNode::Sqrt(node) => node.check_units(),
            CalcNode::Exp(_) => Ok(CalcUnits::None),
            CalcNode::Log { .. } => Ok(CalcUnits::None),
        }
    }

    /// Simplify expression (constant folding, term merging, etc.)
    ///
    /// This performs bottom-up simplification of the expression tree:
    /// - Constant folding: calc(10px + 5px) => 15px
    /// - Flatten nested sums/products
    /// - Merge like terms
    ///
    /// Note: Variables and channel keywords are not simplified until substituted
    pub fn simplify(&mut self) {
        self.visit_depth_first(&mut |node| {
            node.simplify_direct_children();
        });
    }

    /// Depth-first traversal for applying a function to each node
    fn visit_depth_first<F>(&mut self, f: &mut F)
    where
        F: FnMut(&mut Self),
    {
        // Visit children first (depth-first)
        match self {
            CalcNode::Leaf(_) => {}
            CalcNode::Negate(node) => node.visit_depth_first(f),
            CalcNode::Sum(nodes)
            | CalcNode::Product(nodes)
            | CalcNode::Min(nodes)
            | CalcNode::Max(nodes)
            | CalcNode::Hypot(nodes) => {
                for node in nodes.iter_mut() {
                    node.visit_depth_first(f);
                }
            }
            CalcNode::Invert(node)
            | CalcNode::Abs(node)
            | CalcNode::Sign(node)
            | CalcNode::Sin(node)
            | CalcNode::Cos(node)
            | CalcNode::Tan(node)
            | CalcNode::Asin(node)
            | CalcNode::Acos(node)
            | CalcNode::Atan(node)
            | CalcNode::Sqrt(node)
            | CalcNode::Exp(node) => {
                node.visit_depth_first(f);
            }
            CalcNode::Clamp { min, center, max } => {
                min.visit_depth_first(f);
                center.visit_depth_first(f);
                max.visit_depth_first(f);
            }
            CalcNode::Round { value, step, .. } => {
                value.visit_depth_first(f);
                step.visit_depth_first(f);
            }
            CalcNode::Mod { dividend, divisor } | CalcNode::Rem { dividend, divisor } => {
                dividend.visit_depth_first(f);
                divisor.visit_depth_first(f);
            }
            CalcNode::Atan2 { y, x } => {
                y.visit_depth_first(f);
                x.visit_depth_first(f);
            }
            CalcNode::Pow { base, exponent } => {
                base.visit_depth_first(f);
                exponent.visit_depth_first(f);
            }
            CalcNode::Log { value, base } => {
                value.visit_depth_first(f);
                if let Some(base) = base {
                    base.visit_depth_first(f);
                }
            }
        }

        // Then apply function to self
        f(self);
    }

    /// Simplify direct children of this node
    fn simplify_direct_children(&mut self) {
        match self {
            CalcNode::Sum(nodes) => {
                // Try to merge and simplify terms
                if let Some(simplified) = try_simplify_sum(nodes) {
                    *self = simplified;
                }
            }
            CalcNode::Product(nodes) => {
                // Try to merge and simplify factors
                if let Some(simplified) = try_simplify_product(nodes) {
                    *self = simplified;
                }
            }
            CalcNode::Min(nodes) => {
                // Try to evaluate if all are constants
                if let Some(min_val) = try_evaluate_min(nodes) {
                    *self = CalcNode::Leaf(min_val);
                }
            }
            CalcNode::Max(nodes) => {
                // Try to evaluate if all are constants
                if let Some(max_val) = try_evaluate_max(nodes) {
                    *self = CalcNode::Leaf(max_val);
                }
            }
            CalcNode::Clamp { min, center, max } => {
                // Try to evaluate if all are constants
                if let (
                    CalcNode::Leaf(min_leaf),
                    CalcNode::Leaf(center_leaf),
                    CalcNode::Leaf(max_leaf),
                ) = (&**min, &**center, &**max)
                {
                    if let Ok(result) = center_leaf.clamp(min_leaf, max_leaf) {
                        *self = CalcNode::Leaf(result);
                    }
                }
            }
            CalcNode::Abs(node) => {
                if let CalcNode::Leaf(leaf) = &**node {
                    *self = CalcNode::Leaf(leaf.abs());
                }
            }
            CalcNode::Sign(node) => {
                if let CalcNode::Leaf(leaf) = &**node {
                    *self = CalcNode::Leaf(leaf.sign());
                }
            }
            CalcNode::Negate(node) => {
                if let CalcNode::Leaf(leaf) = &**node {
                    *self = CalcNode::Leaf(leaf.negate());
                }
            }
            _ => {
                // Other nodes don't have special simplification
            }
        }
    }

    /// Resolve to concrete value with runtime parameter substitution
    ///
    /// This is the PRIMARY ENTRY POINT for calc resolution.
    /// It:
    /// 1. Substitutes CSS variables from runtime parameters
    /// 2. Resolves the expression to a final value
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values (var(--name) => params["name"])
    /// * `base_font_size` - Base font size for rem conversion
    pub fn resolve(
        &self,
        params: &IndexMap<String, f64>,
        base_font_size: f32,
    ) -> Result<CalcLeaf, String> {
        self.resolve_with_params_and_origin(params, base_font_size, None)
    }

    /// Resolve to concrete value with runtime parameter substitution AND channel keyword substitution
    ///
    /// This is the ENHANCED ENTRY POINT for calc resolution in relative color contexts.
    /// It:
    /// 1. Substitutes CSS variables from runtime parameters
    /// 2. Substitutes channel keywords from origin color
    /// 3. Resolves the expression to a final value
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values (var(--name) => params["name"])
    /// * `base_font_size` - Base font size for rem conversion
    /// * `origin_color` - Origin color for channel keyword substitution (optional)
    pub fn resolve_with_params_and_origin(
        &self,
        params: &IndexMap<String, f64>,
        base_font_size: f32,
        origin_color: Option<&crate::color::types::AbsoluteColor>,
    ) -> Result<CalcLeaf, String> {
        // First, substitute variables
        let mut substituted = self.substitute_variables(params)?;

        // Then substitute channel keywords if we have an origin color
        if let Some(origin) = origin_color {
            substituted = substituted.substitute_channel_keywords(Some(origin))?;
        }

        // Simplify after substitution
        substituted.simplify();

        // Then resolve to final value
        substituted.resolve_internal(base_font_size)
    }

    /// Internal resolution after variable substitution
    ///
    /// Evaluates the expression tree to a single leaf value.
    /// All variables must be substituted before calling this.
    fn resolve_internal(&self, base_font_size: f32) -> Result<CalcLeaf, String> {
        match self {
            CalcNode::Leaf(leaf) => {
                // Variables and channel keywords should have been substituted
                match leaf {
                    CalcLeaf::Variable(name) => Err(format!("Unsubstituted variable: {}", name)),
                    CalcLeaf::ChannelKeyword(keyword) => Err(format!(
                        "Channel keyword {:?} requires color context (relative color syntax)",
                        keyword
                    )),
                    // Convert rem to px for consistency
                    CalcLeaf::Length(n, LengthUnit::Rem) => Ok(CalcLeaf::Length(
                        (*n as f32 * base_font_size) as f64,
                        LengthUnit::Px,
                    )),
                    _ => Ok(leaf.clone()),
                }
            }
            CalcNode::Negate(node) => {
                let value = node.resolve_internal(base_font_size)?;
                Ok(value.negate())
            }
            CalcNode::Sum(nodes) => {
                let mut result = nodes[0].resolve_internal(base_font_size)?;
                for node in &nodes[1..] {
                    let value = node.resolve_internal(base_font_size)?;
                    result = result.add(&value)?;
                }
                Ok(result)
            }
            CalcNode::Product(nodes) => {
                let mut result = nodes[0].resolve_internal(base_font_size)?;
                for node in &nodes[1..] {
                    let value = node.resolve_internal(base_font_size)?;
                    result = result.multiply(&value)?;
                }
                Ok(result)
            }
            CalcNode::Invert(node) => {
                let value = node.resolve_internal(base_font_size)?;
                // 1 / value
                CalcLeaf::Number(1.0).divide(&value)
            }
            CalcNode::Min(nodes) => {
                let mut result = nodes[0].resolve_internal(base_font_size)?;
                for node in &nodes[1..] {
                    let value = node.resolve_internal(base_font_size)?;
                    result = result.min(&value)?;
                }
                Ok(result)
            }
            CalcNode::Max(nodes) => {
                let mut result = nodes[0].resolve_internal(base_font_size)?;
                for node in &nodes[1..] {
                    let value = node.resolve_internal(base_font_size)?;
                    result = result.max(&value)?;
                }
                Ok(result)
            }
            CalcNode::Clamp { min, center, max } => {
                let min_val = min.resolve_internal(base_font_size)?;
                let center_val = center.resolve_internal(base_font_size)?;
                let max_val = max.resolve_internal(base_font_size)?;
                center_val.clamp(&min_val, &max_val)
            }
            CalcNode::Abs(node) => {
                let value = node.resolve_internal(base_font_size)?;
                Ok(value.abs())
            }
            CalcNode::Sign(node) => {
                let value = node.resolve_internal(base_font_size)?;
                Ok(value.sign())
            }
            CalcNode::Round {
                strategy,
                value,
                step,
            } => {
                let value_num = value
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Round value must be a number")?;
                let step_num = step
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Round step must be a number")?;

                let result = match strategy {
                    RoundingStrategy::Nearest => (value_num / step_num).round() * step_num,
                    RoundingStrategy::Up => (value_num / step_num).ceil() * step_num,
                    RoundingStrategy::Down => (value_num / step_num).floor() * step_num,
                    RoundingStrategy::ToZero => (value_num / step_num).trunc() * step_num,
                };
                Ok(CalcLeaf::Number(result))
            }
            CalcNode::Mod { dividend, divisor } => {
                let a = dividend
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Mod dividend must be a number")?;
                let b = divisor
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Mod divisor must be a number")?;
                Ok(CalcLeaf::Number(a % b))
            }
            CalcNode::Rem { dividend, divisor } => {
                let a = dividend
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Rem dividend must be a number")?;
                let b = divisor
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Rem divisor must be a number")?;
                Ok(CalcLeaf::Number(a.rem_euclid(b)))
            }
            CalcNode::Hypot(nodes) => {
                let mut sum_squares = 0.0;
                for node in nodes {
                    let value = node
                        .resolve_internal(base_font_size)?
                        .as_number()
                        .ok_or("Hypot values must be numbers")?;
                    sum_squares += value * value;
                }
                Ok(CalcLeaf::Number(sum_squares.sqrt()))
            }
            CalcNode::Sin(node) => {
                let angle = node
                    .resolve_internal(base_font_size)?
                    .as_angle_degrees()
                    .ok_or("Sin input must be an angle or number")?;
                Ok(CalcLeaf::Number(angle.to_radians().sin()))
            }
            CalcNode::Cos(node) => {
                let angle = node
                    .resolve_internal(base_font_size)?
                    .as_angle_degrees()
                    .ok_or("Cos input must be an angle or number")?;
                Ok(CalcLeaf::Number(angle.to_radians().cos()))
            }
            CalcNode::Tan(node) => {
                let angle = node
                    .resolve_internal(base_font_size)?
                    .as_angle_degrees()
                    .ok_or("Tan input must be an angle or number")?;
                Ok(CalcLeaf::Number(angle.to_radians().tan()))
            }
            CalcNode::Asin(node) => {
                let value = node
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Asin input must be a number")?;
                Ok(CalcLeaf::Angle(value.asin().to_degrees(), AngleUnit::Deg))
            }
            CalcNode::Acos(node) => {
                let value = node
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Acos input must be a number")?;
                Ok(CalcLeaf::Angle(value.acos().to_degrees(), AngleUnit::Deg))
            }
            CalcNode::Atan(node) => {
                let value = node
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Atan input must be a number")?;
                Ok(CalcLeaf::Angle(value.atan().to_degrees(), AngleUnit::Deg))
            }
            CalcNode::Atan2 { y, x } => {
                let y_val = y
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Atan2 y must be a number")?;
                let x_val = x
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Atan2 x must be a number")?;
                Ok(CalcLeaf::Angle(
                    y_val.atan2(x_val).to_degrees(),
                    AngleUnit::Deg,
                ))
            }
            CalcNode::Pow { base, exponent } => {
                let base_val = base
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Pow base must be a number")?;
                let exp_val = exponent
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Pow exponent must be a number")?;
                Ok(CalcLeaf::Number(base_val.powf(exp_val)))
            }
            CalcNode::Sqrt(node) => {
                let value = node
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Sqrt input must be a number")?;
                Ok(CalcLeaf::Number(value.sqrt()))
            }
            CalcNode::Exp(node) => {
                let value = node
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Exp input must be a number")?;
                Ok(CalcLeaf::Number(value.exp()))
            }
            CalcNode::Log { value, base } => {
                let value_num = value
                    .resolve_internal(base_font_size)?
                    .as_number()
                    .ok_or("Log value must be a number")?;

                if let Some(base_node) = base {
                    let base_num = base_node
                        .resolve_internal(base_font_size)?
                        .as_number()
                        .ok_or("Log base must be a number")?;
                    Ok(CalcLeaf::Number(value_num.log(base_num)))
                } else {
                    // Natural log
                    Ok(CalcLeaf::Number(value_num.ln()))
                }
            }
        }
    }

    /// Substitute CSS variables with values from runtime parameters
    ///
    /// This replaces Variable leaves with Number leaves from the params map.
    /// Variables not found in params will cause an error.
    fn substitute_variables(&self, params: &IndexMap<String, f64>) -> Result<CalcNode, String> {
        match self {
            CalcNode::Leaf(CalcLeaf::Variable(name)) => {
                // Look up variable with full name (including -- prefix)
                if let Some(value) = params.get(name) {
                    Ok(CalcNode::Leaf(CalcLeaf::Number(*value)))
                } else {
                    Err(format!(
                        "CSS variable '{}' not found in runtime parameters",
                        name
                    ))
                }
            }
            CalcNode::Leaf(leaf) => Ok(CalcNode::Leaf(leaf.clone())),
            CalcNode::Negate(node) => Ok(CalcNode::Negate(Box::new(
                node.substitute_variables(params)?,
            ))),
            CalcNode::Sum(nodes) => Ok(CalcNode::Sum(
                nodes
                    .iter()
                    .map(|n| n.substitute_variables(params))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            CalcNode::Product(nodes) => Ok(CalcNode::Product(
                nodes
                    .iter()
                    .map(|n| n.substitute_variables(params))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            CalcNode::Invert(node) => Ok(CalcNode::Invert(Box::new(
                node.substitute_variables(params)?,
            ))),
            CalcNode::Min(nodes) => Ok(CalcNode::Min(
                nodes
                    .iter()
                    .map(|n| n.substitute_variables(params))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            CalcNode::Max(nodes) => Ok(CalcNode::Max(
                nodes
                    .iter()
                    .map(|n| n.substitute_variables(params))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            CalcNode::Clamp { min, center, max } => Ok(CalcNode::Clamp {
                min: Box::new(min.substitute_variables(params)?),
                center: Box::new(center.substitute_variables(params)?),
                max: Box::new(max.substitute_variables(params)?),
            }),
            CalcNode::Abs(node) => Ok(CalcNode::Abs(Box::new(node.substitute_variables(params)?))),
            CalcNode::Sign(node) => {
                Ok(CalcNode::Sign(Box::new(node.substitute_variables(params)?)))
            }
            CalcNode::Round {
                strategy,
                value,
                step,
            } => Ok(CalcNode::Round {
                strategy: *strategy,
                value: Box::new(value.substitute_variables(params)?),
                step: Box::new(step.substitute_variables(params)?),
            }),
            CalcNode::Mod { dividend, divisor } => Ok(CalcNode::Mod {
                dividend: Box::new(dividend.substitute_variables(params)?),
                divisor: Box::new(divisor.substitute_variables(params)?),
            }),
            CalcNode::Rem { dividend, divisor } => Ok(CalcNode::Rem {
                dividend: Box::new(dividend.substitute_variables(params)?),
                divisor: Box::new(divisor.substitute_variables(params)?),
            }),
            CalcNode::Hypot(nodes) => Ok(CalcNode::Hypot(
                nodes
                    .iter()
                    .map(|n| n.substitute_variables(params))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            CalcNode::Sin(node) => Ok(CalcNode::Sin(Box::new(node.substitute_variables(params)?))),
            CalcNode::Cos(node) => Ok(CalcNode::Cos(Box::new(node.substitute_variables(params)?))),
            CalcNode::Tan(node) => Ok(CalcNode::Tan(Box::new(node.substitute_variables(params)?))),
            CalcNode::Asin(node) => {
                Ok(CalcNode::Asin(Box::new(node.substitute_variables(params)?)))
            }
            CalcNode::Acos(node) => {
                Ok(CalcNode::Acos(Box::new(node.substitute_variables(params)?)))
            }
            CalcNode::Atan(node) => {
                Ok(CalcNode::Atan(Box::new(node.substitute_variables(params)?)))
            }
            CalcNode::Atan2 { y, x } => Ok(CalcNode::Atan2 {
                y: Box::new(y.substitute_variables(params)?),
                x: Box::new(x.substitute_variables(params)?),
            }),
            CalcNode::Pow { base, exponent } => Ok(CalcNode::Pow {
                base: Box::new(base.substitute_variables(params)?),
                exponent: Box::new(exponent.substitute_variables(params)?),
            }),
            CalcNode::Sqrt(node) => {
                Ok(CalcNode::Sqrt(Box::new(node.substitute_variables(params)?)))
            }
            CalcNode::Exp(node) => Ok(CalcNode::Exp(Box::new(node.substitute_variables(params)?))),
            CalcNode::Log { value, base } => Ok(CalcNode::Log {
                value: Box::new(value.substitute_variables(params)?),
                base: base
                    .as_ref()
                    .map(|b| b.substitute_variables(params))
                    .transpose()?
                    .map(Box::new),
            }),
        }
    }

    /// Substitute channel keywords with values from origin color
    ///
    /// This is used during relative color syntax resolution to replace
    /// channel keywords (l, c, h, r, g, b, etc.) with actual component values
    /// from the origin color.
    ///
    /// # Arguments
    /// * `origin_color` - Optional origin color for channel keyword resolution
    ///
    /// # Returns
    /// A new CalcNode with channel keywords replaced by concrete values
    pub fn substitute_channel_keywords(
        &self,
        origin_color: Option<&crate::color::types::AbsoluteColor>,
    ) -> Result<CalcNode, String> {
        match self {
            CalcNode::Leaf(CalcLeaf::ChannelKeyword(keyword)) => {
                let origin = origin_color.ok_or_else(|| {
                    format!(
                        "Channel keyword {:?} requires origin color context",
                        keyword
                    )
                })?;

                let value = origin.get_component_by_channel_keyword(*keyword)?;

                // All channel keywords become unitless numbers in relative color context
                // Even hue values are represented as numbers (in degrees)
                let leaf = CalcLeaf::Number(value as f64);

                Ok(CalcNode::Leaf(leaf))
            }

            // Pass through other leaves unchanged
            CalcNode::Leaf(leaf) => Ok(CalcNode::Leaf(leaf.clone())),

            // Recursively process tree
            CalcNode::Negate(node) => Ok(CalcNode::Negate(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Sum(nodes) => Ok(CalcNode::Sum(
                nodes
                    .iter()
                    .map(|n| n.substitute_channel_keywords(origin_color))
                    .collect::<Result<Vec<_>, _>>()?,
            )),

            CalcNode::Product(nodes) => Ok(CalcNode::Product(
                nodes
                    .iter()
                    .map(|n| n.substitute_channel_keywords(origin_color))
                    .collect::<Result<Vec<_>, _>>()?,
            )),

            CalcNode::Invert(node) => Ok(CalcNode::Invert(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Min(nodes) => Ok(CalcNode::Min(
                nodes
                    .iter()
                    .map(|n| n.substitute_channel_keywords(origin_color))
                    .collect::<Result<Vec<_>, _>>()?,
            )),

            CalcNode::Max(nodes) => Ok(CalcNode::Max(
                nodes
                    .iter()
                    .map(|n| n.substitute_channel_keywords(origin_color))
                    .collect::<Result<Vec<_>, _>>()?,
            )),

            CalcNode::Clamp { min, center, max } => Ok(CalcNode::Clamp {
                min: Box::new(min.substitute_channel_keywords(origin_color)?),
                center: Box::new(center.substitute_channel_keywords(origin_color)?),
                max: Box::new(max.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Abs(node) => Ok(CalcNode::Abs(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Sign(node) => Ok(CalcNode::Sign(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Round {
                strategy,
                value,
                step,
            } => Ok(CalcNode::Round {
                strategy: *strategy,
                value: Box::new(value.substitute_channel_keywords(origin_color)?),
                step: Box::new(step.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Mod { dividend, divisor } => Ok(CalcNode::Mod {
                dividend: Box::new(dividend.substitute_channel_keywords(origin_color)?),
                divisor: Box::new(divisor.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Rem { dividend, divisor } => Ok(CalcNode::Rem {
                dividend: Box::new(dividend.substitute_channel_keywords(origin_color)?),
                divisor: Box::new(divisor.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Hypot(nodes) => Ok(CalcNode::Hypot(
                nodes
                    .iter()
                    .map(|n| n.substitute_channel_keywords(origin_color))
                    .collect::<Result<Vec<_>, _>>()?,
            )),

            CalcNode::Sin(node) => Ok(CalcNode::Sin(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Cos(node) => Ok(CalcNode::Cos(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Tan(node) => Ok(CalcNode::Tan(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Asin(node) => Ok(CalcNode::Asin(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Acos(node) => Ok(CalcNode::Acos(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Atan(node) => Ok(CalcNode::Atan(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Atan2 { y, x } => Ok(CalcNode::Atan2 {
                y: Box::new(y.substitute_channel_keywords(origin_color)?),
                x: Box::new(x.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Pow { base, exponent } => Ok(CalcNode::Pow {
                base: Box::new(base.substitute_channel_keywords(origin_color)?),
                exponent: Box::new(exponent.substitute_channel_keywords(origin_color)?),
            }),

            CalcNode::Sqrt(node) => Ok(CalcNode::Sqrt(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Exp(node) => Ok(CalcNode::Exp(Box::new(
                node.substitute_channel_keywords(origin_color)?,
            ))),

            CalcNode::Log { value, base } => Ok(CalcNode::Log {
                value: Box::new(value.substitute_channel_keywords(origin_color)?),
                base: base
                    .as_ref()
                    .map(|b| b.substitute_channel_keywords(origin_color))
                    .transpose()?
                    .map(Box::new),
            }),
        }
    }
}

/// Try to simplify a sum by merging like terms and constant folding
fn try_simplify_sum(nodes: &[CalcNode]) -> Option<CalcNode> {
    // If only one term, return it
    if nodes.len() == 1 {
        return Some(nodes[0].clone());
    }

    // Try to fold all leaf values
    let mut leaves: Vec<CalcLeaf> = Vec::new();
    let mut non_leaves: Vec<CalcNode> = Vec::new();

    for node in nodes {
        match node {
            CalcNode::Leaf(leaf) => leaves.push(leaf.clone()),
            _ => non_leaves.push(node.clone()),
        }
    }

    // Try to merge leaves of same type
    if !leaves.is_empty() {
        // Try to merge compatible leaves
        let mut merged_leaves: Vec<CalcLeaf> = Vec::new();
        for leaf in leaves.iter() {
            if let Some(last) = merged_leaves.last_mut() {
                if let Ok(merged) = last.add(leaf) {
                    *last = merged;
                    continue;
                }
            }
            // Couldn't merge with previous, add as new leaf
            merged_leaves.push(leaf.clone());
        }

        // If we reduced the number of leaves, we simplified something
        if merged_leaves.len() < leaves.len() || !non_leaves.is_empty() {
            // Add merged leaves back to non_leaves
            for leaf in merged_leaves {
                non_leaves.push(CalcNode::Leaf(leaf));
            }
        } else if non_leaves.is_empty() && merged_leaves.len() == 1 {
            // Single leaf and no non-leaves
            return Some(CalcNode::Leaf(merged_leaves.into_iter().next().unwrap()));
        } else {
            // No simplification possible
            return None;
        }
    }

    // If we simplified anything, return new sum, otherwise None
    if non_leaves.len() < nodes.len() {
        if non_leaves.len() == 1 {
            Some(non_leaves.into_iter().next().unwrap())
        } else {
            Some(CalcNode::Sum(non_leaves))
        }
    } else {
        None
    }
}

/// Try to simplify a product by merging factors and constant folding
fn try_simplify_product(nodes: &[CalcNode]) -> Option<CalcNode> {
    // If only one factor, return it
    if nodes.len() == 1 {
        return Some(nodes[0].clone());
    }

    // Try to fold all leaf values
    let mut leaves: Vec<CalcLeaf> = Vec::new();
    let mut non_leaves: Vec<CalcNode> = Vec::new();

    for node in nodes {
        match node {
            CalcNode::Leaf(leaf) => leaves.push(leaf.clone()),
            _ => non_leaves.push(node.clone()),
        }
    }

    // Try to merge leaves
    if !leaves.is_empty() {
        let mut result_leaf: Option<CalcLeaf> = None;
        for leaf in leaves.iter() {
            if let Some(ref current) = result_leaf {
                if let Ok(merged) = current.multiply(leaf) {
                    result_leaf = Some(merged);
                } else {
                    // Can't merge these - give up simplification
                    return None;
                }
            } else {
                result_leaf = Some(leaf.clone());
            }
        }

        // If we merged all leaves and there are no non-leaves, return the result
        if non_leaves.is_empty() {
            return result_leaf.map(CalcNode::Leaf);
        }

        // Otherwise, add the merged leaf to non-leaves
        if let Some(leaf) = result_leaf {
            non_leaves.push(CalcNode::Leaf(leaf));
        }
    }

    // If we simplified anything, return new product, otherwise None
    if non_leaves.len() < nodes.len() {
        if non_leaves.len() == 1 {
            Some(non_leaves.into_iter().next().unwrap())
        } else {
            Some(CalcNode::Product(non_leaves))
        }
    } else {
        None
    }
}

/// Try to evaluate min() if all arguments are comparable leaves
fn try_evaluate_min(nodes: &[CalcNode]) -> Option<CalcLeaf> {
    let mut leaves: Vec<CalcLeaf> = Vec::new();
    for node in nodes {
        if let CalcNode::Leaf(leaf) = node {
            leaves.push(leaf.clone());
        } else {
            return None; // Can't evaluate with non-leaves
        }
    }

    if leaves.is_empty() {
        return None;
    }

    let mut result = leaves[0].clone();
    for leaf in &leaves[1..] {
        result = result.min(leaf).ok()?;
    }
    Some(result)
}

/// Try to evaluate max() if all arguments are comparable leaves
fn try_evaluate_max(nodes: &[CalcNode]) -> Option<CalcLeaf> {
    let mut leaves: Vec<CalcLeaf> = Vec::new();
    for node in nodes {
        if let CalcNode::Leaf(leaf) = node {
            leaves.push(leaf.clone());
        } else {
            return None; // Can't evaluate with non-leaves
        }
    }

    if leaves.is_empty() {
        return None;
    }

    let mut result = leaves[0].clone();
    for leaf in &leaves[1..] {
        result = result.max(leaf).ok()?;
    }
    Some(result)
}

/// Combine units for sum/difference operations
fn combine_units_for_sum(a: CalcUnits, b: CalcUnits) -> Result<CalcUnits, String> {
    use CalcUnits::*;
    match (a, b) {
        (Unknown, other) | (other, Unknown) => Ok(other),
        (None, None) => Ok(None),
        (Length, Length) => Ok(Length),
        (Percentage, Percentage) => Ok(Percentage),
        (Angle, Angle) => Ok(Angle),
        (Length, Percentage) | (Percentage, Length) => Ok(LengthPercentage),
        (LengthPercentage, Length) | (Length, LengthPercentage) => Ok(LengthPercentage),
        (LengthPercentage, Percentage) | (Percentage, LengthPercentage) => Ok(LengthPercentage),
        (LengthPercentage, LengthPercentage) => Ok(LengthPercentage),
        _ => Err(format!(
            "Incompatible units for addition: {:?} + {:?}",
            a, b
        )),
    }
}

/// Combine units for multiplication
fn combine_units_for_product(a: CalcUnits, b: CalcUnits) -> Result<CalcUnits, String> {
    use CalcUnits::*;
    match (a, b) {
        (Unknown, other) | (other, Unknown) => Ok(other),
        (None, other) | (other, None) => Ok(other),
        _ => Err(format!(
            "Cannot multiply two dimensioned values: {:?} * {:?}",
            a, b
        )),
    }
}

/// Combine units for min/max/clamp operations
fn combine_units_for_minmax(a: CalcUnits, b: CalcUnits) -> Result<CalcUnits, String> {
    use CalcUnits::*;
    match (a, b) {
        (Unknown, other) | (other, Unknown) => Ok(other),
        (same_a, same_b) if same_a == same_b => Ok(same_a),
        (Length, Percentage) | (Percentage, Length) => Ok(LengthPercentage),
        (LengthPercentage, Length) | (Length, LengthPercentage) => Ok(LengthPercentage),
        (LengthPercentage, Percentage) | (Percentage, LengthPercentage) => Ok(LengthPercentage),
        _ => Err(format!("Incompatible units for min/max: {:?}, {:?}", a, b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_leaf_units() {
        assert_eq!(CalcLeaf::Number(5.0).units(), CalcUnits::None);
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px).units(),
            CalcUnits::Length
        );
        assert_eq!(CalcLeaf::Percentage(50.0).units(), CalcUnits::Percentage);
        assert_eq!(
            CalcLeaf::Angle(180.0, AngleUnit::Deg).units(),
            CalcUnits::Angle
        );
        assert_eq!(
            CalcLeaf::Variable("size".to_string()).units(),
            CalcUnits::Unknown
        );
    }

    #[test]
    fn test_calc_leaf_negate() {
        assert_eq!(CalcLeaf::Number(5.0).negate(), CalcLeaf::Number(-5.0));
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px).negate(),
            CalcLeaf::Length(-10.0, LengthUnit::Px)
        );
        assert_eq!(
            CalcLeaf::Percentage(50.0).negate(),
            CalcLeaf::Percentage(-50.0)
        );
    }

    #[test]
    fn test_calc_leaf_abs() {
        assert_eq!(CalcLeaf::Number(-5.0).abs(), CalcLeaf::Number(5.0));
        assert_eq!(
            CalcLeaf::Length(-10.0, LengthUnit::Px).abs(),
            CalcLeaf::Length(10.0, LengthUnit::Px)
        );
    }

    #[test]
    fn test_calc_leaf_sign() {
        assert_eq!(CalcLeaf::Number(5.0).sign(), CalcLeaf::Number(1.0));
        assert_eq!(CalcLeaf::Number(-5.0).sign(), CalcLeaf::Number(-1.0));
        assert_eq!(CalcLeaf::Number(0.0).sign(), CalcLeaf::Number(0.0));
        assert_eq!(
            CalcLeaf::Length(-10.0, LengthUnit::Px).sign(),
            CalcLeaf::Number(-1.0)
        );
    }

    #[test]
    fn test_calc_leaf_add() {
        // Same units
        assert_eq!(
            CalcLeaf::Number(5.0).add(&CalcLeaf::Number(3.0)).unwrap(),
            CalcLeaf::Number(8.0)
        );
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px)
                .add(&CalcLeaf::Length(5.0, LengthUnit::Px))
                .unwrap(),
            CalcLeaf::Length(15.0, LengthUnit::Px)
        );

        // Incompatible units
        assert!(
            CalcLeaf::Number(5.0)
                .add(&CalcLeaf::Length(10.0, LengthUnit::Px))
                .is_err()
        );
    }

    #[test]
    fn test_calc_leaf_multiply() {
        // Number * Number
        assert_eq!(
            CalcLeaf::Number(5.0)
                .multiply(&CalcLeaf::Number(3.0))
                .unwrap(),
            CalcLeaf::Number(15.0)
        );

        // Number * Length
        assert_eq!(
            CalcLeaf::Number(2.0)
                .multiply(&CalcLeaf::Length(10.0, LengthUnit::Px))
                .unwrap(),
            CalcLeaf::Length(20.0, LengthUnit::Px)
        );

        // Length * Number
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px)
                .multiply(&CalcLeaf::Number(2.0))
                .unwrap(),
            CalcLeaf::Length(20.0, LengthUnit::Px)
        );

        // Cannot multiply two dimensioned values
        assert!(
            CalcLeaf::Length(10.0, LengthUnit::Px)
                .multiply(&CalcLeaf::Length(5.0, LengthUnit::Px))
                .is_err()
        );
    }

    #[test]
    fn test_calc_leaf_divide() {
        // Number / Number
        assert_eq!(
            CalcLeaf::Number(10.0)
                .divide(&CalcLeaf::Number(2.0))
                .unwrap(),
            CalcLeaf::Number(5.0)
        );

        // Length / Number
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px)
                .divide(&CalcLeaf::Number(2.0))
                .unwrap(),
            CalcLeaf::Length(5.0, LengthUnit::Px)
        );

        // Same units (returns number)
        assert_eq!(
            CalcLeaf::Length(10.0, LengthUnit::Px)
                .divide(&CalcLeaf::Length(2.0, LengthUnit::Px))
                .unwrap(),
            CalcLeaf::Number(5.0)
        );

        // Division by zero
        assert!(
            CalcLeaf::Number(10.0)
                .divide(&CalcLeaf::Number(0.0))
                .is_err()
        );
    }

    #[test]
    fn test_channel_keyword_from_ident() {
        assert_eq!(ChannelKeyword::from_ident("l"), Some(ChannelKeyword::L));
        assert_eq!(ChannelKeyword::from_ident("L"), Some(ChannelKeyword::L));
        assert_eq!(ChannelKeyword::from_ident("r"), Some(ChannelKeyword::R));
        assert_eq!(
            ChannelKeyword::from_ident("alpha"),
            Some(ChannelKeyword::Alpha)
        );
        assert_eq!(ChannelKeyword::from_ident("invalid"), None);
    }

    #[test]
    fn test_division_resolution() {
        // Test: 80px / 4 should equal 20px
        let product = CalcNode::Product(vec![
            CalcNode::Leaf(CalcLeaf::Length(80.0, LengthUnit::Px)),
            CalcNode::Invert(Box::new(CalcNode::Leaf(CalcLeaf::Number(4.0)))),
        ]);

        let result = product.resolve(&IndexMap::new(), 16.0).unwrap();
        assert_eq!(result, CalcLeaf::Length(20.0, LengthUnit::Px));
    }
}
