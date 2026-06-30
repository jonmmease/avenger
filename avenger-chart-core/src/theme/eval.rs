//! Type-directed evaluation system for theme values
//!
//! This module provides a structured approach to evaluating `ThemeValue` instances
//! based on their expected type (color, number, length, string). This replaces the
//! variant-based pattern matching in `mark_default()` with type-directed evaluation
//! that properly handles all variants including `Calc`, `RelativeColor`, nested
//! functions, and variables.
//!
//! # Architecture
//!
//! The evaluation system is built around two core concepts:
//!
//! 1. **EvalContext**: Encapsulates evaluation parameters (runtime params, base font size)
//! 2. **Type-directed evaluation**: Methods like `eval_as_color()`, `eval_as_number()`, etc.
//!    that convert any ThemeValue variant to the requested type
//!
//! # Example
//!
//! ```ignore
//! use avenger_chart::theme::eval::EvalContext;
//! use avenger_chart::theme::ThemeValue;
//! use datafusion::common::ScalarValue;
//! use indexmap::IndexMap;
//!
//! let mut params = IndexMap::new();
//! params.insert("--accent".to_string(), ScalarValue::Utf8(Some("#2563eb".into())));
//!
//! let ctx = EvalContext::new(&params, 16.0);
//!
//! // Parse CSS value
//! let theme_value = ThemeValue::Function(
//!     "color-mix".into(),
//!     vec![/* ... */]
//! );
//!
//! // Evaluate as color (handles functions, variables, calc, etc.)
//! let color = theme_value.eval_as_color(&ctx)?;
//! ```

use std::fmt;

use super::value::{CssRgba, ThemeValue};

/// Evaluation context for resolving theme values
///
/// This struct encapsulates the runtime parameters needed to evaluate theme values:
/// - CSS variable values (e.g., `--accent: #2563eb`)
/// - Base font size for rem/em conversions
/// - Color scheme (light/dark) for light-dark() function
/// - Origin color for relative color syntax
///
/// By packaging these into a single context object, we can:
/// - Pass evaluation parameters uniformly across all evaluation methods
/// - Add new context fields without changing method signatures
/// - Make recursive evaluation (e.g., var() in calc()) straightforward
#[derive(Debug, Clone)]
pub struct EvalContext<'a> {
    /// Runtime parameter values (CSS variables, color-scheme, etc.)
    pub params: &'a indexmap::IndexMap<String, datafusion_common::ScalarValue>,

    /// Base font size in pixels (for rem/em conversion)
    pub base_font_size: f32,

    /// Origin color for relative color syntax evaluation (optional)
    /// Example: `oklch(from var(--primary) calc(l - 0.2) c h)` needs origin color
    pub origin_color: Option<CssRgba>,
}

impl<'a> EvalContext<'a> {
    /// Create a new evaluation context
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values (CSS variables from Param instances)
    /// * `base_font_size` - Base font size in pixels (typically 16.0)
    pub fn new(
        params: &'a indexmap::IndexMap<String, datafusion_common::ScalarValue>,
        base_font_size: f32,
    ) -> Self {
        Self {
            params,
            base_font_size,
            origin_color: None,
        }
    }

    /// Create a new context with an origin color for relative color evaluation
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values
    /// * `base_font_size` - Base font size in pixels
    /// * `origin_color` - Origin color for relative color syntax
    pub fn with_origin_color(
        params: &'a indexmap::IndexMap<String, datafusion_common::ScalarValue>,
        base_font_size: f32,
        origin_color: CssRgba,
    ) -> Self {
        Self {
            params,
            base_font_size,
            origin_color: Some(origin_color),
        }
    }

    /// Create a derived context with a different origin color
    pub fn with_new_origin(&self, origin_color: CssRgba) -> Self {
        Self {
            params: self.params,
            base_font_size: self.base_font_size,
            origin_color: Some(origin_color),
        }
    }
}

/// Evaluation error with diagnostic information
///
/// This error type provides detailed diagnostics for why a theme value
/// couldn't be evaluated to the requested type. It includes:
/// - The variant that failed to evaluate
/// - The expected type (color, number, length, string)
/// - Nested errors for compound values (functions, variables, calc)
/// - Human-readable messages for debugging
#[derive(Debug, Clone)]
pub enum EvalError {
    /// Value variant cannot be converted to requested type
    /// Example: Trying to get Number variant as color
    TypeMismatch {
        variant: String,
        expected_type: TargetType,
        value_description: String,
    },

    /// Variable not found in parameters
    /// Example: var(--missing) when --missing not in params
    VariableNotFound { variable_name: String },

    /// Variable exists but has wrong type
    /// Example: var(--size) used as color when --size is a number
    VariableTypeMismatch {
        variable_name: String,
        expected_type: TargetType,
        actual_type: String,
    },

    /// Function evaluation failed
    /// Example: color-mix() with invalid arguments
    FunctionError {
        function_name: String,
        error: String,
    },

    /// Calc expression evaluation failed
    /// Example: calc(100% + 20px) without parent size context
    CalcError { error: String },

    /// Relative color evaluation failed
    /// Example: oklch(from blue ...) with invalid origin
    RelativeColorError { error: String },

    /// Unsupported variant for requested type
    /// Example: Initial/Inherit/None values
    Unsupported {
        variant: String,
        expected_type: TargetType,
    },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::TypeMismatch {
                variant,
                expected_type,
                value_description,
            } => {
                write!(
                    f,
                    "Cannot convert {} variant to {}: {}",
                    variant, expected_type, value_description
                )
            }
            EvalError::VariableNotFound { variable_name } => {
                write!(f, "CSS variable not found: {}", variable_name)
            }
            EvalError::VariableTypeMismatch {
                variable_name,
                expected_type,
                actual_type,
            } => {
                write!(
                    f,
                    "CSS variable {} has type {} but {} expected",
                    variable_name, actual_type, expected_type
                )
            }
            EvalError::FunctionError {
                function_name,
                error,
            } => {
                write!(f, "Function {}() failed: {}", function_name, error)
            }
            EvalError::CalcError { error } => {
                write!(f, "calc() evaluation failed: {}", error)
            }
            EvalError::RelativeColorError { error } => {
                write!(f, "Relative color evaluation failed: {}", error)
            }
            EvalError::Unsupported {
                variant,
                expected_type,
            } => {
                write!(
                    f,
                    "{} variant cannot be evaluated as {}",
                    variant, expected_type
                )
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// Target type for evaluation
///
/// Represents the expected type when evaluating a ThemeValue.
/// Used in error messages and for dispatch to type-specific evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetType {
    /// CSS color (hex, rgb, hsl, named color, etc.)
    Color,
    /// Numeric value (unitless number)
    Number,
    /// Length value in pixels (px, rem, em converted to px)
    Length,
    /// String value (keywords, identifiers, font families)
    String,
    /// Boolean value
    Boolean,
    /// Angle value in degrees
    Angle,
    /// Percentage value
    Percentage,
}

impl fmt::Display for TargetType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetType::Color => write!(f, "color"),
            TargetType::Number => write!(f, "number"),
            TargetType::Length => write!(f, "length"),
            TargetType::String => write!(f, "string"),
            TargetType::Boolean => write!(f, "boolean"),
            TargetType::Angle => write!(f, "angle"),
            TargetType::Percentage => write!(f, "percentage"),
        }
    }
}

/// Helper function to describe a ThemeValue variant for error messages
pub(crate) fn variant_name(value: &ThemeValue) -> &'static str {
    match value {
        ThemeValue::String(_) => "String",
        ThemeValue::Number(_) => "Number",
        ThemeValue::Boolean(_) => "Boolean",
        ThemeValue::Length(_, _) => "Length",
        ThemeValue::Angle(_, _) => "Angle",
        ThemeValue::Percentage(_) => "Percentage",
        ThemeValue::Color(_) => "Color",
        ThemeValue::Function(_, _) => "Function",
        ThemeValue::List(_) => "List",
        ThemeValue::Object(_) => "Object",
        ThemeValue::Array(_) => "Array",
        ThemeValue::Variable(_) => "Variable",
        ThemeValue::LightDark(_, _) => "LightDark",
        ThemeValue::Calc(_) => "Calc",
        ThemeValue::RelativeColor { .. } => "RelativeColor",
        ThemeValue::Initial => "Initial",
        ThemeValue::Inherit => "Inherit",
        ThemeValue::None => "None",
    }
}

/// Get the expected type for a CSS property/channel
///
/// This function maps CSS property names to their expected types, enabling
/// type-directed evaluation in `mark_default()`. It handles both standard CSS
/// properties (e.g., "fill", "stroke-width") and channel-specific properties
/// (e.g., "fill-discrete", "stroke-width-continuous").
///
/// # Property Categories
///
/// ## Color Properties
/// - Direct: `fill`, `stroke`, `color`, `background`, `border-color`
/// - Discrete ranges: `fill-discrete`, `stroke-discrete`, etc.
/// - Continuous ranges: `fill-continuous`, `stroke-continuous`, etc.
///
/// ## Number Properties
/// - Direct: `opacity`, `font-weight`
/// - Discrete ranges: `opacity-discrete`
/// - Continuous ranges: `opacity-continuous`
///
/// ## Length Properties (in pixels)
/// - Direct: `size`, `stroke-width`, `font-size`, `padding`, `margin`,
///   `corner-radius`, `width`, `height`, `min-width`, `max-width`,
///   `min-height`, `max-height`, `gap`, `border-width`
/// - Discrete ranges: `size-discrete`, `stroke-width-discrete`
/// - Continuous ranges: `size-continuous`, `stroke-width-continuous`
///
/// ## String Properties
/// - `font-family`, `text-anchor`, `font-style`, `font-variant`,
///   `text-decoration`, `align`, `baseline`
/// - Discrete ranges: `shape-discrete` (shape names like "circle", "square")
///
/// ## Boolean Properties
/// - `visible`
///
/// ## Angle Properties (in degrees)
/// - `rotation`, `angle`
///
/// # Examples
///
/// ```ignore
/// use avenger_chart::theme::eval::{get_channel_type, TargetType};
///
/// assert_eq!(get_channel_type("fill"), Some(TargetType::Color));
/// assert_eq!(get_channel_type("stroke-width"), Some(TargetType::Length));
/// assert_eq!(get_channel_type("opacity"), Some(TargetType::Number));
/// assert_eq!(get_channel_type("font-family"), Some(TargetType::String));
/// assert_eq!(get_channel_type("visible"), Some(TargetType::Boolean));
/// assert_eq!(get_channel_type("rotation"), Some(TargetType::Angle));
///
/// // Scale ranges inherit type from base property
/// assert_eq!(get_channel_type("fill-discrete"), Some(TargetType::Color));
/// assert_eq!(get_channel_type("size-continuous"), Some(TargetType::Length));
/// ```
pub fn get_channel_type(property: &str) -> Option<TargetType> {
    // Strip -discrete/-continuous/-dash suffixes to get base property
    let base_property = property
        .strip_suffix("-discrete")
        .or_else(|| property.strip_suffix("-continuous"))
        .or_else(|| property.strip_suffix("-dash"))
        .unwrap_or(property);

    match base_property {
        // Color properties
        "fill" | "stroke" | "color" | "background" | "border-color" | "glow-color"
        | "shadow-color" => Some(TargetType::Color),

        // Number properties (0.0-1.0 or unitless)
        "opacity" | "font-weight" => Some(TargetType::Number),

        // Length properties (px, rem → px)
        "size" | "stroke-width" | "font-size" | "padding" | "margin" | "corner-radius"
        | "width" | "height" | "min-width" | "max-width" | "min-height" | "max-height" | "gap"
        | "border-width" | "line-height" | "letter-spacing" | "indent" | "offset"
        | "blur-radius" | "spread-radius" => Some(TargetType::Length),

        // String properties
        "font-family" | "text-anchor" | "font-style" | "font-variant" | "text-decoration"
        | "align" | "baseline" | "shape" | "cursor" | "overflow" | "display" => {
            Some(TargetType::String)
        }

        // Boolean properties
        "visible" => Some(TargetType::Boolean),

        // Angle properties
        "rotation" | "angle" | "skew" => Some(TargetType::Angle),

        // Unknown property
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn test_eval_context_creation() {
        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        assert_eq!(ctx.base_font_size, 16.0);
        assert_eq!(ctx.origin_color, None);
    }

    #[test]
    fn test_eval_context_with_origin() {
        let params = IndexMap::new();
        let origin = CssRgba {
            red: 255,
            green: 0,
            blue: 0,
            alpha: 255,
        };
        let ctx = EvalContext::with_origin_color(&params, 16.0, origin);

        assert_eq!(ctx.origin_color, Some(origin));
    }

    #[test]
    fn test_error_display() {
        let error = EvalError::VariableNotFound {
            variable_name: "--accent".to_string(),
        };
        assert_eq!(error.to_string(), "CSS variable not found: --accent");

        let error = EvalError::TypeMismatch {
            variant: "Number".to_string(),
            expected_type: TargetType::Color,
            value_description: "42".to_string(),
        };
        assert!(error.to_string().contains("Number"));
        assert!(error.to_string().contains("color"));
    }

    #[test]
    fn test_variant_name() {
        use datafusion_common::ScalarValue;

        assert_eq!(variant_name(&ThemeValue::Number(42.0)), "Number");
        assert_eq!(
            variant_name(&ThemeValue::Color(CssRgba {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255
            })),
            "Color"
        );
        assert_eq!(
            variant_name(&ThemeValue::Variable("--accent".to_string())),
            "Variable"
        );

        // Test with ScalarValue to avoid unused import warning
        let _v = ScalarValue::Utf8(Some("test".to_string()));
    }

    #[test]
    fn test_get_channel_type_color() {
        assert_eq!(get_channel_type("fill"), Some(TargetType::Color));
        assert_eq!(get_channel_type("stroke"), Some(TargetType::Color));
        assert_eq!(get_channel_type("color"), Some(TargetType::Color));
        assert_eq!(get_channel_type("background"), Some(TargetType::Color));
        assert_eq!(get_channel_type("border-color"), Some(TargetType::Color));

        // Scale ranges
        assert_eq!(get_channel_type("fill-discrete"), Some(TargetType::Color));
        assert_eq!(
            get_channel_type("stroke-continuous"),
            Some(TargetType::Color)
        );
    }

    #[test]
    fn test_get_channel_type_number() {
        assert_eq!(get_channel_type("opacity"), Some(TargetType::Number));
        assert_eq!(get_channel_type("font-weight"), Some(TargetType::Number));

        // Scale ranges
        assert_eq!(
            get_channel_type("opacity-discrete"),
            Some(TargetType::Number)
        );
        assert_eq!(
            get_channel_type("opacity-continuous"),
            Some(TargetType::Number)
        );
    }

    #[test]
    fn test_get_channel_type_length() {
        assert_eq!(get_channel_type("size"), Some(TargetType::Length));
        assert_eq!(get_channel_type("stroke-width"), Some(TargetType::Length));
        assert_eq!(get_channel_type("font-size"), Some(TargetType::Length));
        assert_eq!(get_channel_type("padding"), Some(TargetType::Length));
        assert_eq!(get_channel_type("margin"), Some(TargetType::Length));
        assert_eq!(get_channel_type("corner-radius"), Some(TargetType::Length));
        assert_eq!(get_channel_type("width"), Some(TargetType::Length));
        assert_eq!(get_channel_type("height"), Some(TargetType::Length));

        // Scale ranges
        assert_eq!(get_channel_type("size-discrete"), Some(TargetType::Length));
        assert_eq!(
            get_channel_type("stroke-width-continuous"),
            Some(TargetType::Length)
        );
    }

    #[test]
    fn test_get_channel_type_string() {
        assert_eq!(get_channel_type("font-family"), Some(TargetType::String));
        assert_eq!(get_channel_type("text-anchor"), Some(TargetType::String));
        assert_eq!(get_channel_type("font-style"), Some(TargetType::String));
        assert_eq!(get_channel_type("shape"), Some(TargetType::String));

        // Scale ranges
        assert_eq!(get_channel_type("shape-discrete"), Some(TargetType::String));
    }

    #[test]
    fn test_get_channel_type_boolean() {
        assert_eq!(get_channel_type("visible"), Some(TargetType::Boolean));
    }

    #[test]
    fn test_get_channel_type_angle() {
        assert_eq!(get_channel_type("rotation"), Some(TargetType::Angle));
        assert_eq!(get_channel_type("angle"), Some(TargetType::Angle));
        assert_eq!(get_channel_type("skew"), Some(TargetType::Angle));
    }

    #[test]
    fn test_get_channel_type_unknown() {
        assert_eq!(get_channel_type("unknown-property"), None);
        assert_eq!(get_channel_type("foo"), None);
        assert_eq!(get_channel_type("bar-discrete"), None);
    }

    #[test]
    fn test_get_channel_type_custom_channels() {
        // Custom channels with standard suffixes should work
        assert_eq!(get_channel_type("glow-color"), Some(TargetType::Color));
        assert_eq!(
            get_channel_type("glow-color-discrete"),
            Some(TargetType::Color)
        );
    }
}
