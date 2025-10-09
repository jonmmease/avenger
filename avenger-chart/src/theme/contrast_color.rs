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
/// Basic form (choose between black and white):
/// ```css
/// contrast-color(<color>)
/// ```
///
/// Extended form (choose from candidate list):
/// ```css
/// contrast-color(<color>, <color-list>)
/// ```
///
/// # Arguments
///
/// * `args` - Parsed function arguments from CSS parser
///   - Single argument: base color (returns black or white)
///   - Multiple arguments: base color + candidate colors to choose from
///
/// # Returns
///
/// The contrasting color as CssRgba, or None if parsing fails
///
/// Resolve contrast-color() function with runtime parameter support
///
/// This is called from `ThemeValue::as_color_with_params()` when a
/// `ThemeValue::Function("contrast-color", ...")` needs to be resolved.
///
/// Supports CSS variables by recursively resolving arguments using `as_color_with_params()`.
///
/// # Syntax
///
/// Basic form (choose between black and white):
/// ```css
/// contrast-color(var(--bg))
/// ```
///
/// Extended form (choose from candidate list):
/// ```css
/// contrast-color(var(--bg), #222, #eee)
/// contrast-color(var(--bg), var(--text1), var(--text2))
/// ```
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
/// // Basic: contrast-color(var(--bg-color))
/// let args = vec![ThemeValue::Variable("--bg-color".to_string())];
/// let mut params = IndexMap::new();
/// params.insert("--bg-color".to_string(), ScalarValue::Utf8(Some("#000".to_string())));
/// let result = resolve_contrast_color_with_params(&args, &params, 16.0);
/// // result => Some(CssRgba { white })
///
/// // Extended: contrast-color(var(--bg), #222, #eee)
/// let args = vec![
///     ThemeValue::Variable("--bg".to_string()),
///     ThemeValue::String("#222".to_string()),
///     ThemeValue::String("#eee".to_string()),
/// ];
/// let result = resolve_contrast_color_with_params(&args, &params, 16.0);
/// // result => Some(CssRgba { best candidate })
/// ```
pub fn resolve_contrast_color_with_params(
    args: &[ThemeValue],
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    base_font_size: f32,
) -> Option<CssRgba> {
    // Need at least one argument
    if args.is_empty() {
        return None;
    }

    // Recursively resolve the base color argument (handles variables!)
    let base_color_rgba = args[0].as_color_with_params(params, base_font_size)?;
    let base_color = AbsoluteColor::from_css_rgba(&base_color_rgba);

    if args.len() == 1 {
        // Basic form: choose black or white
        let contrast_color = choose_contrast_color(&base_color);
        Some(contrast_color.to_css_rgba())
    } else {
        // Extended form: resolve all candidate colors (handles variables!)
        let candidates: Vec<AbsoluteColor> = args[1..]
            .iter()
            .filter_map(|arg| {
                arg.as_color_with_params(params, base_font_size)
                    .map(|rgba| AbsoluteColor::from_css_rgba(&rgba))
            })
            .collect();

        // If no valid candidates were parsed, fall back to black/white
        if candidates.is_empty() {
            let contrast_color = choose_contrast_color(&base_color);
            return Some(contrast_color.to_css_rgba());
        }

        // Use WCAG AA threshold (4.5:1) as default
        use crate::color::contrast::choose_best_contrast;
        let contrast_color = choose_best_contrast(&base_color, &candidates, 4.5);
        Some(contrast_color.to_css_rgba())
    }
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_contrast_color_with_hex() {
        // contrast-color(#000000) should return white
        let args = vec![ThemeValue::String("#000000".to_string())];
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
        assert_eq!(result.alpha, 255);
    }

    #[test]
    fn test_parse_contrast_color_with_named_color() {
        // contrast-color(blue) should return white
        let args = vec![ThemeValue::String("blue".to_string())];
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_parse_contrast_color_with_white() {
        // contrast-color(white) should return black
        let args = vec![ThemeValue::String("white".to_string())];
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

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
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // Should return white
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_parse_contrast_color_invalid_args() {
        // No arguments
        let args: Vec<ThemeValue> = vec![];
        assert!(resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).is_none());

        // Invalid base color
        let args = vec![ThemeValue::Number(5.0)];
        assert!(resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).is_none());
    }

    #[test]
    fn test_parse_contrast_color_yellow() {
        // Yellow is very light, should get black
        let args = vec![ThemeValue::String("#ffff00".to_string())];
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        assert_eq!(result.red, 0);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);
    }

    #[test]
    fn test_parse_contrast_color_purple() {
        // Purple/navy is dark, should get white
        let args = vec![ThemeValue::String("#000080".to_string())]; // navy
        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

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

    // Tests for extended syntax with candidate lists

    #[test]
    fn test_parse_contrast_color_with_candidates() {
        // contrast-color with dark background and light candidates
        let args = vec![
            ThemeValue::String("#333333".to_string()), // Dark gray background
            ThemeValue::String("#aaaaaa".to_string()), // Light gray
            ThemeValue::String("#eeeeee".to_string()), // Lighter gray
        ];

        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // #eee should be chosen (better contrast)
        assert_eq!(result.red, 0xee);
        assert_eq!(result.green, 0xee);
        assert_eq!(result.blue, 0xee);
    }

    #[test]
    fn test_parse_contrast_color_with_brand_palette() {
        // Light blue background with brand colors
        let args = vec![
            ThemeValue::String("lightblue".to_string()),
            ThemeValue::String("navy".to_string()),
            ThemeValue::String("maroon".to_string()),
            ThemeValue::String("purple".to_string()),
        ];

        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // Navy should provide best contrast
        // Navy is rgb(0, 0, 128)
        assert_eq!(result.blue, 128);
    }

    #[test]
    fn test_parse_contrast_color_empty_args() {
        let args: Vec<ThemeValue> = vec![];
        assert!(resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).is_none());
    }

    #[test]
    fn test_parse_contrast_color_invalid_candidates() {
        // Base color valid, but candidates are invalid
        let args = vec![
            ThemeValue::String("blue".to_string()),
            ThemeValue::Number(5.0),  // Invalid
            ThemeValue::Number(10.0), // Invalid
        ];

        let result = resolve_contrast_color_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // Should fall back to black/white since no valid candidates
        // Blue is dark, should get white
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 255);
        assert_eq!(result.blue, 255);
    }

    #[test]
    fn test_resolve_contrast_color_with_candidate_variables() {
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        // contrast-color(var(--bg), var(--text1), var(--text2))
        let args = vec![
            ThemeValue::Variable("--bg".to_string()),
            ThemeValue::Variable("--text1".to_string()),
            ThemeValue::Variable("--text2".to_string()),
        ];

        let mut params = IndexMap::new();
        params.insert(
            "--bg".to_string(),
            ScalarValue::Utf8(Some("#333333".to_string())), // Dark gray bg
        );
        params.insert(
            "--text1".to_string(),
            ScalarValue::Utf8(Some("#aaaaaa".to_string())), // Light gray
        );
        params.insert(
            "--text2".to_string(),
            ScalarValue::Utf8(Some("#eeeeee".to_string())), // Lighter gray
        );

        let result = resolve_contrast_color_with_params(&args, &params, 16.0).unwrap();

        // #eee should be chosen (better contrast)
        assert_eq!(result.red, 0xee);
        assert_eq!(result.green, 0xee);
        assert_eq!(result.blue, 0xee);
    }

    #[test]
    fn test_resolve_contrast_color_mixed_static_and_variables() {
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        // contrast-color(var(--bg), #aaa, #eee)
        let args = vec![
            ThemeValue::Variable("--bg".to_string()),
            ThemeValue::String("#aaaaaa".to_string()),
            ThemeValue::String("#eeeeee".to_string()),
        ];

        let mut params = IndexMap::new();
        params.insert(
            "--bg".to_string(),
            ScalarValue::Utf8(Some("#333333".to_string())), // Dark gray
        );

        let result = resolve_contrast_color_with_params(&args, &params, 16.0).unwrap();

        // #eee should be chosen
        assert_eq!(result.red, 0xee);
    }
}
