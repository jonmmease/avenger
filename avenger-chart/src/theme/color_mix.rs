//! CSS color-mix() function support
//!
//! Implements parsing and evaluation of color-mix() as defined in:
//! https://drafts.csswg.org/css-color-5/#color-mix

use crate::color::mix::{HueInterpolationMethod, mix_colors};
use crate::color::types::{AbsoluteColor, ColorSpace};
use crate::theme::{CssRgba, ThemeValue};

/// Parse a color space name
fn parse_color_space(value: &ThemeValue) -> Option<ColorSpace> {
    if let ThemeValue::String(s) = value {
        match s.as_str() {
            "srgb" => Some(ColorSpace::Srgb),
            "hsl" => Some(ColorSpace::Hsl),
            "hwb" => Some(ColorSpace::Hwb),
            "lab" => Some(ColorSpace::Lab),
            "lch" => Some(ColorSpace::Lch),
            "oklab" => Some(ColorSpace::Oklab),
            "oklch" => Some(ColorSpace::Oklch),
            _ => None,
        }
    } else {
        None
    }
}

/// Parse a hue interpolation method
fn parse_hue_method(s: &str) -> Option<HueInterpolationMethod> {
    match s {
        "shorter" => Some(HueInterpolationMethod::Shorter),
        "longer" => Some(HueInterpolationMethod::Longer),
        "increasing" => Some(HueInterpolationMethod::Increasing),
        "decreasing" => Some(HueInterpolationMethod::Decreasing),
        "specified" => Some(HueInterpolationMethod::Specified),
        _ => None,
    }
}

/// Resolve color-mix() function with runtime parameters
///
/// This variant handles CSS variables in both colors and arguments.
/// Example: color-mix(in srgb, var(--bg) 70%, contrast-color(var(--bg)) 30%)
///
/// # Arguments
/// * `args` - Function arguments (may contain variables or nested functions)
/// * `params` - Runtime parameters for variable resolution
/// * `base_font_size` - Base font size for length calculations
///
/// # Returns
/// The mixed color as CssRgba, or None if resolution fails
pub fn resolve_color_mix_with_params(
    args: &[ThemeValue],
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    base_font_size: f32,
) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let mut i = 0;

    // First argument should be "in" keyword
    if let ThemeValue::String(s) = &args[i] {
        if s != "in" {
            return None;
        }
        i += 1;
    } else {
        return None;
    }

    // Next is the color space
    if i >= args.len() {
        return None;
    }
    let color_space = parse_color_space(&args[i])?;
    i += 1;

    // Optionally parse hue interpolation method
    let mut hue_method = HueInterpolationMethod::Shorter;
    if i < args.len() {
        if let ThemeValue::String(s) = &args[i] {
            if let Some(method) = parse_hue_method(s) {
                hue_method = method;
                i += 1;
                // Should be followed by "hue" keyword
                if i < args.len() {
                    if let ThemeValue::String(s) = &args[i] {
                        if s == "hue" {
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    // Parse first color and optional percentage (with runtime resolution)
    if i >= args.len() {
        return None;
    }
    let (color1, pct1) =
        parse_color_and_percentage_with_params(&args[i..], params, base_font_size)?;
    i += if pct1.is_some() { 2 } else { 1 };

    // Parse second color and optional percentage (with runtime resolution)
    if i >= args.len() {
        return None;
    }
    let (color2, pct2) =
        parse_color_and_percentage_with_params(&args[i..], params, base_font_size)?;

    // Calculate weights
    let (w1, w2) = normalize_percentages(pct1, pct2);

    // Mix colors
    let mixed = mix_colors(color_space, &color1, w1, &color2, w2, hue_method);

    // Convert to CssRgba
    Some(mixed.to_css_rgba())
}

/// Parse a color and optional percentage from argument list with runtime parameter resolution
///
/// Returns (color, optional_percentage) and consumes 1 or 2 args
fn parse_color_and_percentage_with_params(
    args: &[ThemeValue],
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    base_font_size: f32,
) -> Option<(AbsoluteColor, Option<f32>)> {
    if args.is_empty() {
        return None;
    }

    // Parse color with runtime resolution
    let color = parse_color_value_with_params(&args[0], params, base_font_size)?;

    // Check for percentage
    let percentage = if args.len() > 1 {
        match &args[1] {
            ThemeValue::Percentage(p) => Some(*p as f32),
            ThemeValue::Number(n) => Some(*n as f32),
            _ => None,
        }
    } else {
        None
    };

    Some((color, percentage))
}

/// Parse a ThemeValue into an AbsoluteColor with runtime parameter resolution
fn parse_color_value_with_params(
    value: &ThemeValue,
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    base_font_size: f32,
) -> Option<AbsoluteColor> {
    // Use as_color_with_params to recursively resolve colors, variables, and nested functions
    value
        .as_color_with_params(params, base_font_size)
        .map(|rgba| AbsoluteColor::from_css_rgba(&rgba))
}

/// Normalize percentages to weights that sum to 1.0
///
/// If both percentages are provided, normalize them.
/// If only one is provided, the other is (100 - p1).
/// If neither is provided, use 50/50.
fn normalize_percentages(p1: Option<f32>, p2: Option<f32>) -> (f32, f32) {
    match (p1, p2) {
        (Some(pct1), Some(pct2)) => {
            let total = pct1 + pct2;
            if total == 0.0 {
                (0.5, 0.5)
            } else {
                (pct1 / total, pct2 / total)
            }
        }
        (Some(pct1), None) => {
            let pct1 = pct1 / 100.0;
            (pct1, 1.0 - pct1)
        }
        (None, Some(pct2)) => {
            let pct2 = pct2 / 100.0;
            (1.0 - pct2, pct2)
        }
        (None, None) => (0.5, 0.5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_space() {
        assert_eq!(
            parse_color_space(&ThemeValue::String("srgb".to_string())),
            Some(ColorSpace::Srgb)
        );
        assert_eq!(
            parse_color_space(&ThemeValue::String("oklab".to_string())),
            Some(ColorSpace::Oklab)
        );
        assert_eq!(
            parse_color_space(&ThemeValue::String("invalid".to_string())),
            None
        );
    }

    #[test]
    fn test_normalize_percentages() {
        assert_eq!(normalize_percentages(None, None), (0.5, 0.5));
        assert_eq!(normalize_percentages(Some(30.0), None), (0.3, 0.7));
        assert_eq!(normalize_percentages(None, Some(70.0)), (0.3, 0.7));
        assert_eq!(normalize_percentages(Some(30.0), Some(70.0)), (0.3, 0.7));
    }

    #[test]
    fn test_parse_color_mix_simple() {
        // color-mix(in srgb, red, blue)
        let args = vec![
            ThemeValue::String("in".to_string()),
            ThemeValue::String("srgb".to_string()),
            ThemeValue::Color(CssRgba {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255,
            }),
            ThemeValue::Color(CssRgba {
                red: 0,
                green: 0,
                blue: 255,
                alpha: 255,
            }),
        ];

        let result =
            resolve_color_mix_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // Should be purple (roughly 127, 0, 127)
        assert!(result.red > 120 && result.red < 135);
        assert!(result.green < 5);
        assert!(result.blue > 120 && result.blue < 135);
        assert_eq!(result.alpha, 255);
    }

    #[test]
    fn test_parse_color_mix_with_percentages() {
        // color-mix(in srgb, red 75%, blue 25%)
        let args = vec![
            ThemeValue::String("in".to_string()),
            ThemeValue::String("srgb".to_string()),
            ThemeValue::Color(CssRgba {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255,
            }),
            ThemeValue::Percentage(75.0),
            ThemeValue::Color(CssRgba {
                red: 0,
                green: 0,
                blue: 255,
                alpha: 255,
            }),
            ThemeValue::Percentage(25.0),
        ];

        let result =
            resolve_color_mix_with_params(&args, &indexmap::IndexMap::new(), 16.0).unwrap();

        // Should be more red than blue (roughly 191, 0, 64)
        assert!(result.red > 185);
        assert!(result.green < 5);
        assert!(result.blue > 55 && result.blue < 70);
    }

    #[test]
    fn test_parse_color_mix_named_colors() {
        use crate::theme::value::parse_color_string;

        // Simulate what the parser does - convert "red" and "blue" to Color values
        let red = parse_color_string("red").expect("red should parse");
        let blue = parse_color_string("blue").expect("blue should parse");

        // color-mix(in srgb, red, blue)
        let args = vec![
            ThemeValue::String("in".to_string()),
            ThemeValue::String("srgb".to_string()),
            ThemeValue::Color(red),
            ThemeValue::Color(blue),
        ];

        let result = resolve_color_mix_with_params(&args, &indexmap::IndexMap::new(), 16.0);
        assert!(result.is_some(), "color-mix should parse successfully");

        let color = result.unwrap();
        // Should be purple (roughly 127, 0, 127)
        println!(
            "Result: r={}, g={}, b={}",
            color.red, color.green, color.blue
        );
        assert!(color.red > 120 && color.red < 135);
        assert!(color.green < 5);
        assert!(color.blue > 120 && color.blue < 135);
    }

    #[test]
    fn test_parse_color_mix_string_colors() {
        // Test with string colors (as they would be parsed from CSS)
        // color-mix(in srgb, red, blue)
        let args = vec![
            ThemeValue::String("in".to_string()),
            ThemeValue::String("srgb".to_string()),
            ThemeValue::String("red".to_string()),
            ThemeValue::String("blue".to_string()),
        ];

        let result = resolve_color_mix_with_params(&args, &indexmap::IndexMap::new(), 16.0);
        println!("Result from string colors: {:?}", result);
        assert!(
            result.is_some(),
            "color-mix with string colors should parse successfully"
        );

        let color = result.unwrap();
        // Should be purple (roughly 127, 0, 127)
        println!(
            "Result: r={}, g={}, b={}",
            color.red, color.green, color.blue
        );
        assert!(color.red > 120 && color.red < 135);
        assert!(color.green < 5);
        assert!(color.blue > 120 && color.blue < 135);
    }
}
