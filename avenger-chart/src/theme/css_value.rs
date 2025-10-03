//! CSS value parsing utilities

use crate::color::convert::hsl_to_rgb;
use crate::theme::{CssRgba, ThemeValue};

/// Parse an rgb() or rgba() function from parsed CSS arguments
pub fn parse_rgb_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    let red = match &args[0] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let green = match &args[1] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let blue = match &args[2] {
        ThemeValue::Number(n) => (*n as u8).min(255),
        _ => return None,
    };

    let alpha = if args.len() > 3 {
        match &args[3] {
            ThemeValue::Number(n) => ((*n * 255.0) as u8).min(255),
            _ => 255,
        }
    } else {
        255
    };

    Some(CssRgba {
        red,
        green,
        blue,
        alpha,
    })
}

/// Extract a number or angle value from a ThemeValue, converting to degrees if needed
fn extract_number_or_angle(value: &ThemeValue) -> Option<f32> {
    value.as_angle_degrees().map(|deg| deg as f32)
}

/// Extract a percentage value from a ThemeValue
fn extract_percentage(value: &ThemeValue) -> Option<f32> {
    match value {
        ThemeValue::Percentage(p) => Some(*p as f32),
        // Also accept numbers as percentages (0-100)
        ThemeValue::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// Extract an alpha value from a ThemeValue (number 0-1 or percentage 0-100)
fn extract_alpha(value: &ThemeValue) -> Option<f32> {
    match value {
        ThemeValue::Number(n) => Some((*n as f32).clamp(0.0, 1.0)),
        ThemeValue::Percentage(p) => Some(((*p as f32) / 100.0).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// Parse an hsl() or hsla() function from parsed CSS arguments
///
/// # Arguments
///
/// * `args` - Parsed CSS function arguments
///
/// # CSS Syntax
///
/// - `hsl(hue, saturation, lightness)`
/// - `hsla(hue, saturation, lightness, alpha)`
///
/// Where:
/// - hue: number (degrees) or angle
/// - saturation: percentage 0-100%
/// - lightness: percentage 0-100%
/// - alpha: number 0-1 or percentage 0-100%
///
/// # Examples
///
/// ```css
/// hsl(0, 100%, 50%)        /* red */
/// hsl(120, 100%, 50%)      /* green */
/// hsl(240, 100%, 50%)      /* blue */
/// hsla(0, 100%, 50%, 0.5)  /* semi-transparent red */
/// ```
pub fn parse_hsl_function(args: &[ThemeValue]) -> Option<CssRgba> {
    if args.len() < 3 {
        return None;
    }

    // Extract hue (in degrees)
    let hue = extract_number_or_angle(&args[0])?;

    // Extract saturation and lightness (percentages)
    let saturation = extract_percentage(&args[1])?;
    let lightness = extract_percentage(&args[2])?;

    // Extract optional alpha
    let alpha = if args.len() > 3 {
        extract_alpha(&args[3])?
    } else {
        1.0
    };

    // Convert HSL to RGB (hsl_to_rgb returns values in 0.0-1.0 range)
    let (r, g, b) = hsl_to_rgb(hue, saturation, lightness);

    // Convert to CssRgba (0-255 range)
    Some(CssRgba {
        red: (r * 255.0) as u8,
        green: (g * 255.0) as u8,
        blue: (b * 255.0) as u8,
        alpha: (alpha * 255.0) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hsl_red() {
        let args = vec![
            ThemeValue::Number(0.0),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
        assert_eq!(color.alpha, 255);
    }

    #[test]
    fn test_parse_hsl_green() {
        let args = vec![
            ThemeValue::Number(120.0),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 0);
        assert_eq!(color.alpha, 255);
    }

    #[test]
    fn test_parse_hsl_blue() {
        let args = vec![
            ThemeValue::Number(240.0),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255);
        assert_eq!(color.alpha, 255);
    }

    #[test]
    fn test_parse_hsla_with_alpha() {
        let args = vec![
            ThemeValue::Number(0.0),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
            ThemeValue::Number(0.5),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
        assert_eq!(color.alpha, 127); // 0.5 * 255 ≈ 127
    }

    #[test]
    fn test_parse_hsla_with_percentage_alpha() {
        let args = vec![
            ThemeValue::Number(120.0),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
            ThemeValue::Percentage(75.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 0);
        assert_eq!(color.alpha, 191); // 0.75 * 255 ≈ 191
    }

    #[test]
    fn test_parse_hsl_gray() {
        let args = vec![
            ThemeValue::Number(0.0),
            ThemeValue::Percentage(0.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 127);
        assert_eq!(color.green, 127);
        assert_eq!(color.blue, 127);
    }

    #[test]
    fn test_parse_hsl_insufficient_args() {
        let args = vec![ThemeValue::Number(0.0), ThemeValue::Percentage(100.0)];
        assert!(parse_hsl_function(&args).is_none());
    }

    #[test]
    fn test_parse_hsl_invalid_arg_type() {
        let args = vec![
            ThemeValue::String("invalid".to_string()),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        assert!(parse_hsl_function(&args).is_none());
    }

    #[test]
    fn test_parse_hsl_with_deg_angle() {
        use crate::theme::AngleUnit;
        let args = vec![
            ThemeValue::Angle(120.0, AngleUnit::Deg),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn test_parse_hsl_with_turn_angle() {
        use crate::theme::AngleUnit;
        // 0.5 turn = 180 degrees = cyan
        let args = vec![
            ThemeValue::Angle(0.5, AngleUnit::Turn),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 255);
    }

    #[test]
    fn test_parse_hsl_with_rad_angle() {
        use crate::theme::AngleUnit;
        use std::f64::consts::PI;
        // 2π/3 radians ≈ 120 degrees = green
        let args = vec![
            ThemeValue::Angle(2.0 * PI / 3.0, AngleUnit::Rad),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn test_parse_hsl_with_grad_angle() {
        use crate::theme::AngleUnit;
        // 400grad = 360 degrees = 0 degrees = red (normalized)
        let args = vec![
            ThemeValue::Angle(400.0, AngleUnit::Grad),
            ThemeValue::Percentage(100.0),
            ThemeValue::Percentage(50.0),
        ];
        let color = parse_hsl_function(&args).unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);
    }

    #[test]
    fn test_hsl_angle_units_via_css() {
        use crate::theme::Theme;

        // Test that angle units work through the full CSS parsing pipeline
        let css = r#"
            mark {
                fill: hsl(120deg, 100%, 50%);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Query the fill color for a mark
        let ctx = crate::theme::ThemeContext::new("mark");
        let color = theme.fill_color(&ctx);

        assert!(color.is_some(), "Should have parsed fill color");
        let [r, g, b, a] = color.unwrap();

        // Should be green: hsl(120deg, 100%, 50%) = rgb(0, 255, 0)
        assert!((r - 0.0).abs() < 0.01, "Red should be ~0");
        assert!((g - 1.0).abs() < 0.01, "Green should be ~1");
        assert!((b - 0.0).abs() < 0.01, "Blue should be ~0");
        assert!((a - 1.0).abs() < 0.01, "Alpha should be ~1");
    }
}
