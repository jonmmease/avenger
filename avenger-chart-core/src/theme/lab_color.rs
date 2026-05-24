//! CSS Lab/Lch/Oklab/Oklch color function support
//!
//! Implements parsing for perceptually uniform color spaces:
//! - lab(L a b [/ alpha])
//! - lch(L C H [/ alpha])
//! - oklab(L a b [/ alpha])
//! - oklch(L C H [/ alpha])
//!
//! References:
//! - https://drafts.csswg.org/css-color-4/#lab-colors
//! - https://drafts.csswg.org/css-color-4/#lch-colors
//! - https://drafts.csswg.org/css-color-4/#ok-lab
//! - https://drafts.csswg.org/css-color-4/#ok-lch

use crate::color::types::{AbsoluteColor, ColorSpace};
use crate::theme::{AngleUnit, CssRgba, ThemeValue};

/// Parse an oklab() function
///
/// Syntax: oklab(L a b [/ alpha])
/// - L: lightness, 0-1 (number or percentage)
/// - a: green-red axis, -0.4 to 0.4 (number)
/// - b: blue-yellow axis, -0.4 to 0.4 (number)
/// - alpha: optional, 0-1 (number or percentage)
pub fn parse_oklab_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let lightness = extract_number_or_percentage(&args[0], 1.0)?;
    let a = extract_number(&args[1])?;
    let b = extract_number(&args[2])?;

    let alpha = if args.len() > 3 {
        extract_alpha(&args[3])?
    } else {
        1.0
    };

    let color = AbsoluteColor::new(ColorSpace::Oklab, lightness, a, b, alpha);
    Some(color.to_css_rgba())
}

/// Parse an oklch() function
///
/// Syntax: oklch(L C H [/ alpha])
/// - L: lightness, 0-1 (number or percentage)
/// - C: chroma, 0-0.4 (number)
/// - H: hue angle in degrees (number or angle)
/// - alpha: optional, 0-1 (number or percentage)
pub fn parse_oklch_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let lightness = extract_number_or_percentage(&args[0], 1.0)?;
    let chroma = extract_number(&args[1])?;
    let hue = extract_number_or_angle(&args[2])?;

    let alpha = if args.len() > 3 {
        extract_alpha(&args[3])?
    } else {
        1.0
    };

    let color = AbsoluteColor::new(ColorSpace::Oklch, lightness, chroma, hue, alpha);
    Some(color.to_css_rgba())
}

/// Parse a lab() function
///
/// Syntax: lab(L a b [/ alpha])
/// - L: lightness, 0-100 (number or percentage)
/// - a: green-red axis, -125 to 125 (number)
/// - b: blue-yellow axis, -125 to 125 (number)
/// - alpha: optional, 0-1 (number or percentage)
pub fn parse_lab_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let lightness = extract_number_or_percentage(&args[0], 100.0)?;
    let a = extract_number(&args[1])?;
    let b = extract_number(&args[2])?;

    let alpha = if args.len() > 3 {
        extract_alpha(&args[3])?
    } else {
        1.0
    };

    let color = AbsoluteColor::new(ColorSpace::Lab, lightness, a, b, alpha);
    Some(color.to_css_rgba())
}

/// Parse an lch() function
///
/// Syntax: lch(L C H [/ alpha])
/// - L: lightness, 0-100 (number or percentage)
/// - C: chroma, 0-150 (number)
/// - H: hue angle in degrees (number or angle)
/// - alpha: optional, 0-1 (number or percentage)
pub fn parse_lch_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let lightness = extract_number_or_percentage(&args[0], 100.0)?;
    let chroma = extract_number(&args[1])?;
    let hue = extract_number_or_angle(&args[2])?;

    let alpha = if args.len() > 3 {
        extract_alpha(&args[3])?
    } else {
        1.0
    };

    let color = AbsoluteColor::new(ColorSpace::Lch, lightness, chroma, hue, alpha);
    Some(color.to_css_rgba())
}

/// Extract a number from a ThemeValue
fn extract_number(value: &ThemeValue) -> Option<f32> {
    match value {
        ThemeValue::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// Extract a number or percentage from a ThemeValue
/// If it's a percentage, scale by max_value (e.g., 100 for Lab lightness, 1.0 for Oklab)
fn extract_number_or_percentage(value: &ThemeValue, max_value: f32) -> Option<f32> {
    match value {
        ThemeValue::Number(n) => Some(*n as f32),
        ThemeValue::Percentage(p) => Some((*p as f32 / 100.0) * max_value),
        _ => None,
    }
}

/// Extract a number or angle from a ThemeValue
/// Angles are converted to degrees
fn extract_number_or_angle(value: &ThemeValue) -> Option<f32> {
    match value {
        ThemeValue::Number(n) => Some(*n as f32),
        ThemeValue::Angle(val, unit) => {
            let degrees = match unit {
                AngleUnit::Deg => *val as f32,
                AngleUnit::Rad => (*val as f32).to_degrees(),
                AngleUnit::Grad => (*val as f32) * 0.9, // 400 grad = 360 deg
                AngleUnit::Turn => (*val as f32) * 360.0,
            };
            Some(degrees)
        }
        _ => None,
    }
}

/// Extract alpha value from a ThemeValue
/// Handles both number (0-1) and percentage (0-100%)
fn extract_alpha(value: &ThemeValue) -> Option<f32> {
    match value {
        ThemeValue::Number(n) => Some((*n as f32).clamp(0.0, 1.0)),
        ThemeValue::Percentage(p) => Some(((*p as f32) / 100.0).clamp(0.0, 1.0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_oklab() {
        // oklab(0.5 0.1 -0.1)
        let args = vec![
            ThemeValue::Number(0.5),
            ThemeValue::Number(0.1),
            ThemeValue::Number(-0.1),
        ];

        let result = parse_oklab_function(&args);
        assert!(result.is_some(), "oklab should parse successfully");
    }

    #[test]
    fn test_parse_oklch() {
        // oklch(0.5 0.2 180)
        let args = vec![
            ThemeValue::Number(0.5),
            ThemeValue::Number(0.2),
            ThemeValue::Number(180.0),
        ];

        let result = parse_oklch_function(&args);
        assert!(result.is_some(), "oklch should parse successfully");
    }

    #[test]
    fn test_parse_oklch_with_percentage() {
        // oklch(50% 0.2 180deg)
        use crate::theme::AngleUnit;

        let args = vec![
            ThemeValue::Percentage(50.0),
            ThemeValue::Number(0.2),
            ThemeValue::Angle(180.0, AngleUnit::Deg),
        ];

        let result = parse_oklch_function(&args);
        assert!(
            result.is_some(),
            "oklch with percentage should parse successfully"
        );
    }

    #[test]
    fn test_parse_lab() {
        // lab(50 25 -25)
        let args = vec![
            ThemeValue::Number(50.0),
            ThemeValue::Number(25.0),
            ThemeValue::Number(-25.0),
        ];

        let result = parse_lab_function(&args);
        assert!(result.is_some(), "lab should parse successfully");
    }

    #[test]
    fn test_parse_lch() {
        // lch(50 50 180)
        let args = vec![
            ThemeValue::Number(50.0),
            ThemeValue::Number(50.0),
            ThemeValue::Number(180.0),
        ];

        let result = parse_lch_function(&args);
        assert!(result.is_some(), "lch should parse successfully");
    }

    #[test]
    fn test_parse_oklch_with_alpha() {
        // oklch(0.5 0.2 180 / 0.5)
        let args = vec![
            ThemeValue::Number(0.5),
            ThemeValue::Number(0.2),
            ThemeValue::Number(180.0),
            ThemeValue::Number(0.5),
        ];

        let result = parse_oklch_function(&args);
        assert!(
            result.is_some(),
            "oklch with alpha should parse successfully"
        );

        let color = result.unwrap();
        assert_eq!(color.alpha, 127, "Alpha should be ~0.5 * 255");
    }
}
