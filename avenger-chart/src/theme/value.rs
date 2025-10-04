//! Theme value types and color utilities

use serde::{Deserialize, Serialize};

/// Value types that themes can return
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThemeValue {
    /// String value (keywords, identifiers, font families, etc.)
    String(String),

    /// Numeric value (all CSS numbers: sizes, opacities, angles, weights, etc.)
    Number(f64),

    /// Boolean value
    Boolean(bool),

    /// Length with unit
    Length(f64, LengthUnit),

    /// Angle with unit (for hue values in HSL, rotations, etc.)
    Angle(f64, AngleUnit),

    /// Percentage value
    Percentage(f64),

    /// Color value (parsed from CSS)
    Color(CssRgba),

    /// CSS function call
    Function(String, Vec<ThemeValue>),

    /// Multiple values (for padding, margin, etc.)
    List(Vec<ThemeValue>),

    /// CSS variable reference
    Variable(String),

    /// light-dark() function - theme-aware color selection
    /// First value for light mode, second for dark mode
    LightDark(Box<ThemeValue>, Box<ThemeValue>),

    /// Initial value
    Initial,

    /// Inherited value
    Inherit,

    /// No value (property not set)
    None,
}

/// RGBA color representation from CSS (0-255 range)
///
/// This type represents colors as parsed from CSS, using u8 values (0-255).
/// It converts to normalized [f32; 4] arrays (0.0-1.0) for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CssRgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl CssRgba {
    /// Convert to normalized RGBA array [0.0-1.0] for use with ColorOrGradient
    pub fn to_array(&self) -> [f32; 4] {
        [
            self.red as f32 / 255.0,
            self.green as f32 / 255.0,
            self.blue as f32 / 255.0,
            self.alpha as f32 / 255.0,
        ]
    }
}

/// Length units
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthUnit {
    Px,
    Rem,
}

/// Angle units for CSS angle values
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AngleUnit {
    /// Degrees (360deg = full circle)
    Deg,
    /// Radians (2π rad ≈ 6.28318 rad = full circle)
    Rad,
    /// Gradians (400grad = full circle)
    Grad,
    /// Turns (1turn = full circle)
    Turn,
}

impl AngleUnit {
    /// Convert angle value to degrees
    pub fn to_degrees(&self, value: f64) -> f64 {
        match self {
            AngleUnit::Deg => value,
            AngleUnit::Rad => value.to_degrees(),
            AngleUnit::Grad => value * 360.0 / 400.0,
            AngleUnit::Turn => value * 360.0,
        }
    }
}

impl ThemeValue {
    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Extract font-size value with rem support
    ///
    /// Converts font-size values to pixels:
    /// - Number: Returns the raw number (interpreted as px)
    /// - Length(Px): Returns pixel value
    /// - Length(Rem): Converts rem to pixels using base_font_size (rem-based scaling)
    ///
    /// Note: Percentage values are not supported (would require parent element context)
    pub fn as_font_size(&self, base_font_size: f32) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            _ => None,
        }
    }

    /// Try to get as number
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ThemeValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get angle value in degrees
    /// - Angle values are converted to degrees
    /// - Plain numbers are interpreted as degrees
    pub fn as_angle_degrees(&self) -> Option<f64> {
        match self {
            ThemeValue::Number(n) => Some(*n), // Interpret as degrees
            ThemeValue::Angle(value, unit) => Some(unit.to_degrees(*value)),
            _ => None,
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ThemeValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as list
    pub fn as_list(&self) -> Option<&[ThemeValue]> {
        match self {
            ThemeValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// Try to get as CSS color
    pub fn as_color(&self) -> Option<CssRgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),
            _ => None,
        }
    }

    /// Try to get as normalized color array [0.0-1.0] for use with ColorOrGradient
    pub fn as_color_array(&self) -> Option<[f32; 4]> {
        self.as_color().map(|css_rgba| css_rgba.to_array())
    }

    /// Check if value is None
    pub fn is_none(&self) -> bool {
        matches!(self, ThemeValue::None)
    }
}

/// Parse a color string using avenger-scales color parser (internal use only)
/// Supports hex colors, rgb/rgba, hsl/hsla, and all CSS named colors
pub(crate) fn parse_color_string(color_str: &str) -> Option<CssRgba> {
    avenger_scales::color::parse_color_string(color_str).map(|[r, g, b, a]| CssRgba {
        red: (r * 255.0) as u8,
        green: (g * 255.0) as u8,
        blue: (b * 255.0) as u8,
        alpha: (a * 255.0) as u8,
    })
}

/// Parse a length string like "16px", "1.5rem", "10em" into a ThemeValue::Length
pub(crate) fn parse_length_string(length_str: &str) -> Option<ThemeValue> {
    let s = length_str.trim();

    if s.ends_with("px") {
        let num_str = &s[..s.len() - 2];
        if let Ok(n) = num_str.parse::<f64>() {
            return Some(ThemeValue::Length(n, LengthUnit::Px));
        }
    } else if s.ends_with("rem") {
        let num_str = &s[..s.len() - 3];
        if let Ok(n) = num_str.parse::<f64>() {
            return Some(ThemeValue::Length(n, LengthUnit::Rem));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_angle_unit_to_degrees() {
        // Test degrees (identity)
        assert_eq!(AngleUnit::Deg.to_degrees(180.0), 180.0);
        assert_eq!(AngleUnit::Deg.to_degrees(360.0), 360.0);

        // Test radians
        use std::f64::consts::PI;
        assert!((AngleUnit::Rad.to_degrees(PI) - 180.0).abs() < 0.01);
        assert!((AngleUnit::Rad.to_degrees(2.0 * PI) - 360.0).abs() < 0.01);
        assert!((AngleUnit::Rad.to_degrees(PI / 2.0) - 90.0).abs() < 0.01);

        // Test gradians
        assert_eq!(AngleUnit::Grad.to_degrees(200.0), 180.0);
        assert_eq!(AngleUnit::Grad.to_degrees(400.0), 360.0);
        assert_eq!(AngleUnit::Grad.to_degrees(100.0), 90.0);

        // Test turns
        assert_eq!(AngleUnit::Turn.to_degrees(0.5), 180.0);
        assert_eq!(AngleUnit::Turn.to_degrees(1.0), 360.0);
        assert_eq!(AngleUnit::Turn.to_degrees(0.25), 90.0);
    }

    #[test]
    fn test_theme_value_as_angle_degrees() {
        // Test plain number (interpreted as degrees)
        let value = ThemeValue::Number(120.0);
        assert_eq!(value.as_angle_degrees(), Some(120.0));

        // Test angle with degrees
        let value = ThemeValue::Angle(120.0, AngleUnit::Deg);
        assert_eq!(value.as_angle_degrees(), Some(120.0));

        // Test angle with turns
        let value = ThemeValue::Angle(0.5, AngleUnit::Turn);
        assert_eq!(value.as_angle_degrees(), Some(180.0));

        // Test angle with radians
        use std::f64::consts::PI;
        let value = ThemeValue::Angle(PI, AngleUnit::Rad);
        assert!((value.as_angle_degrees().unwrap() - 180.0).abs() < 0.01);

        // Test angle with gradians
        let value = ThemeValue::Angle(200.0, AngleUnit::Grad);
        assert_eq!(value.as_angle_degrees(), Some(180.0));

        // Test non-angle value
        let value = ThemeValue::String("not an angle".to_string());
        assert_eq!(value.as_angle_degrees(), None);
    }
}
