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
    match value {
        ThemeValue::Number(n) => Some(*n as f32),
        // Angles are currently not explicitly parsed as a separate type,
        // so we expect numbers in degrees
        _ => None,
    }
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
}
