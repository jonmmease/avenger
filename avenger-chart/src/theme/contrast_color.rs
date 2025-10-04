//! CSS contrast-color() function support
//!
//! Implements parsing and evaluation of contrast-color() as defined in:
//! https://drafts.csswg.org/css-color-6/
//!
//! # Syntax
//!
//! Basic form:
//! ```css
//! contrast-color(<color>)
//! ```
//!
//! Returns either black or white, whichever provides better WCAG 2.1 contrast
//! against the input color.
//!
//! # Examples
//!
//! ```css
//! /* Dark background gets white text */
//! color: contrast-color(#000);
//!
//! /* Light background gets black text */
//! color: contrast-color(#fff);
//!
//! /* Works with CSS variables */
//! color: contrast-color(var(--bg-color));
//! ```
//!
//! # Future Extensions
//!
//! The CSS spec defines extended syntax that we may implement later:
//! - `contrast-color(<color>, <color-list>)` - Choose from multiple colors
//! - `contrast-color(<color> to <target-contrast>)` - Target specific ratio

use crate::color::contrast::choose_contrast_color;
use crate::color::types::AbsoluteColor;
use crate::theme::{CssRgba, ThemeValue};

/// Parse and evaluate a contrast-color() function
///
/// # Syntax
///
/// `contrast-color(<color>)`
///
/// # Arguments
///
/// * `args` - Parsed function arguments from CSS parser (should contain exactly one color)
///
/// # Returns
///
/// The contrasting color (black or white) as CssRgba, or None if parsing fails
///
/// # Examples
///
/// ```ignore
/// use avenger_chart::theme::contrast_color::parse_contrast_color_function;
/// use avenger_chart::theme::ThemeValue;
///
/// // CSS: contrast-color(blue)
/// let args = vec![ThemeValue::String("blue".to_string())];
/// let result = parse_contrast_color_function(&args);
/// // result => Some(CssRgba { white })
/// ```
pub fn parse_contrast_color_function(args: &[ThemeValue]) -> Option<CssRgba> {
    // Expect exactly one argument
    if args.len() != 1 {
        return None;
    }

    // Parse the base color
    let base_color = parse_color_value(&args[0])?;

    // Choose contrasting color (black or white)
    let contrast_color = choose_contrast_color(&base_color);

    // Convert to CssRgba
    Some(contrast_color.to_css_rgba())
}

/// Resolve contrast-color() function with runtime parameter support
///
/// This is called from `ThemeValue::as_color_with_params()` when a
/// `ThemeValue::Function("contrast-color", ...)` needs to be resolved.
///
/// Unlike `parse_contrast_color_function()`, this function can handle CSS variables
/// by recursively resolving arguments using `as_color_with_params()`.
///
/// # Arguments
///
/// * `args` - Function arguments (may contain variables)
/// * `params` - Runtime parameters for variable resolution
/// * `base_font_size` - Base font size for relative units
///
/// # Returns
///
/// The contrasting color after resolving all variables
///
/// # Examples
///
/// ```ignore
/// // CSS: contrast-color(var(--bg-color))
/// let args = vec![ThemeValue::Variable("--bg-color".to_string())];
/// let mut params = IndexMap::new();
/// params.insert("--bg-color".to_string(), ScalarValue::Utf8(Some("#000".to_string())));
/// let result = resolve_contrast_color_with_params(&args, &params, 16.0);
/// // result => Some(CssRgba { white })
/// ```
pub fn resolve_contrast_color_with_params(
    args: &[ThemeValue],
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    base_font_size: f32,
) -> Option<CssRgba> {
    // Expect exactly one argument
    if args.len() != 1 {
        return None;
    }

    // Recursively resolve the argument (handles variables!)
    let base_color_rgba = args[0].as_color_with_params(params, base_font_size)?;

    // Convert to AbsoluteColor
    let base_color = AbsoluteColor::from_css_rgba(&base_color_rgba);

    // Choose contrasting color
    let contrast_color = choose_contrast_color(&base_color);

    // Convert to CssRgba
    Some(contrast_color.to_css_rgba())
}

/// Parse a ThemeValue into an AbsoluteColor
///
/// Supports:
/// - `ThemeValue::Color` - Direct color values
/// - `ThemeValue::String` - Named colors (e.g., "red", "blue") or hex colors
///
/// # Note
///
/// CSS variables (`var(...)`) should be resolved before calling this function,
/// as we cannot resolve them without runtime context.
fn parse_color_value(value: &ThemeValue) -> Option<AbsoluteColor> {
    match value {
        // Direct color value
        ThemeValue::Color(rgba) => Some(AbsoluteColor::from_css_rgba(rgba)),

        // String - try to parse as a color
        ThemeValue::String(s) => {
            use crate::theme::value::parse_color_string;
            parse_color_string(s).map(|rgba| AbsoluteColor::from_css_rgba(&rgba))
        }

        // Other types are not supported
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_contrast_color_with_hex() {
        // contrast-color(#000000) should return white
        let args = vec![ThemeValue::String("#000000".to_string())];
        let result = parse_contrast_color_function(&args).unwrap();

        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
        assert_eq!(result.alpha, 255);
    }

    #[test]
    fn test_parse_contrast_color_with_named_color() {
        // contrast-color(blue) should return white
        let args = vec![ThemeValue::String("blue".to_string())];
        let result = parse_contrast_color_function(&args).unwrap();

        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_parse_contrast_color_with_white() {
        // contrast-color(white) should return black
        let args = vec![ThemeValue::String("white".to_string())];
        let result = parse_contrast_color_function(&args).unwrap();

        assert_eq!(result.red, 0);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);
    }

    #[test]
    fn test_parse_contrast_color_with_color_value() {
        // contrast-color with direct Color value
        let black = CssRgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 255,
        };

        let args = vec![ThemeValue::Color(black)];
        let result = parse_contrast_color_function(&args).unwrap();

        // Should return white
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_parse_contrast_color_invalid_args() {
        // Too many arguments
        let args = vec![
            ThemeValue::String("red".to_string()),
            ThemeValue::String("blue".to_string()),
        ];
        assert!(parse_contrast_color_function(&args).is_none());

        // No arguments
        let args: Vec<ThemeValue> = vec![];
        assert!(parse_contrast_color_function(&args).is_none());

        // Invalid argument type
        let args = vec![ThemeValue::Number(5.0)];
        assert!(parse_contrast_color_function(&args).is_none());
    }

    #[test]
    fn test_parse_contrast_color_yellow() {
        // Yellow is very light, should get black
        let args = vec![ThemeValue::String("#ffff00".to_string())];
        let result = parse_contrast_color_function(&args).unwrap();

        assert_eq!(result.red, 0);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);
    }

    #[test]
    fn test_parse_contrast_color_purple() {
        // Purple/navy is dark, should get white
        let args = vec![ThemeValue::String("#000080".to_string())]; // navy
        let result = parse_contrast_color_function(&args).unwrap();

        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    // Tests for resolve_contrast_color_with_params (runtime resolution with variables)

    #[test]
    fn test_resolve_contrast_color_with_variable() {
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        // Create function args with a variable
        let args = vec![ThemeValue::Variable("--bg-color".to_string())];

        // Create params with the variable value
        let mut params = IndexMap::new();
        params.insert(
            "--bg-color".to_string(),
            ScalarValue::Utf8(Some("#000000".to_string())),
        );

        // Resolve with runtime params
        let result = resolve_contrast_color_with_params(&args, &params, 16.0).unwrap();

        // Black background should get white text
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_resolve_contrast_color_with_static_color() {
        use indexmap::IndexMap;

        // Create function args with a static color string
        let args = vec![ThemeValue::String("#ffffff".to_string())];

        let params = IndexMap::new();

        // Resolve (should work even without params)
        let result = resolve_contrast_color_with_params(&args, &params, 16.0).unwrap();

        // White background should get black text
        assert_eq!(result.red, 0);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);
    }

    #[test]
    fn test_resolve_contrast_color_with_undefined_variable() {
        use indexmap::IndexMap;

        // Create function args with a variable
        let args = vec![ThemeValue::Variable("--undefined".to_string())];

        // Empty params (variable not defined)
        let params = IndexMap::new();

        // Should return None for undefined variable
        let result = resolve_contrast_color_with_params(&args, &params, 16.0);
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_contrast_color_different_runtime_params() {
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let args = vec![ThemeValue::Variable("--user-bg".to_string())];

        // Test with dark background
        let mut params_dark = IndexMap::new();
        params_dark.insert(
            "--user-bg".to_string(),
            ScalarValue::Utf8(Some("#222222".to_string())),
        );
        let result_dark = resolve_contrast_color_with_params(&args, &params_dark, 16.0).unwrap();
        assert_eq!(result_dark.red, 255); // White for dark bg

        // Test with light background
        let mut params_light = IndexMap::new();
        params_light.insert(
            "--user-bg".to_string(),
            ScalarValue::Utf8(Some("#eeeeee".to_string())),
        );
        let result_light = resolve_contrast_color_with_params(&args, &params_light, 16.0).unwrap();
        assert_eq!(result_light.red, 0); // Black for light bg
    }
}
