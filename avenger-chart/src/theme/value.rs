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

    /// Percentage value
    Percentage(f64),

    /// Color value
    Color(Rgba),

    /// CSS function call
    Function(String, Vec<ThemeValue>),

    /// Multiple values (for padding, margin, etc.)
    List(Vec<ThemeValue>),

    /// CSS variable reference
    Variable(String),

    /// Initial value
    Initial,

    /// Inherited value
    Inherit,

    /// No value (property not set)
    None,
}

/// RGBA color representation
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

/// Length units
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthUnit {
    Px,
    Rem,
}

impl ThemeValue {
    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert value to pixels for rendering
    /// For Rem units, converts using the provided base_font_size
    pub fn as_pixels(&self, base_font_size: f32) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            ThemeValue::Percentage(p) => Some(*p as f32),
            _ => None,
        }
    }

    /// Try to get as f64
    pub fn as_double(&self) -> Option<f64> {
        match self {
            ThemeValue::Number(n) => Some(*n),
            ThemeValue::Length(n, _) => Some(*n),
            ThemeValue::Percentage(p) => Some(*p),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i32> {
        match self {
            ThemeValue::Number(n) => Some(*n as i32),
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

    /// Try to get as color
    pub fn as_color(&self) -> Option<Rgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),
            ThemeValue::String(s) => {
                // Use css-color-parser for comprehensive color parsing
                parse_color_string(s)
            }
            _ => None,
        }
    }

    /// Convert to string value (for display/serialization)
    pub fn to_string_value(&self) -> Option<String> {
        match self {
            ThemeValue::String(s) => Some(s.clone()),
            ThemeValue::Color(rgba) => {
                if rgba.alpha < 255 {
                    Some(format!(
                        "#{:02x}{:02x}{:02x}{:02x}",
                        rgba.red, rgba.green, rgba.blue, rgba.alpha
                    ))
                } else {
                    Some(format!(
                        "#{:02x}{:02x}{:02x}",
                        rgba.red, rgba.green, rgba.blue
                    ))
                }
            }
            _ => None,
        }
    }

    /// Check if value is None
    pub fn is_none(&self) -> bool {
        matches!(self, ThemeValue::None)
    }
}

/// Parse a color string using avenger-scales color parser
/// Supports hex colors, rgb/rgba, hsl/hsla, and all CSS named colors
pub fn parse_color_string(color_str: &str) -> Option<Rgba> {
    avenger_scales::color::parse_color_string(color_str).map(|[r, g, b, a]| Rgba {
        red: (r * 255.0) as u8,
        green: (g * 255.0) as u8,
        blue: (b * 255.0) as u8,
        alpha: (a * 255.0) as u8,
    })
}

/// Parse a hex color (kept for backward compatibility)
pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
    parse_color_string(hex)
}

/// Parse a named color (kept for backward compatibility)
pub fn parse_named_color(name: &str) -> Option<Rgba> {
    parse_color_string(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_string_hex() {
        // Test 3-digit hex
        let color = parse_color_string("#f0a").unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 170);
        assert_eq!(color.alpha, 255);

        // Test 6-digit hex
        let color = parse_color_string("#ff00aa").unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 170);
        assert_eq!(color.alpha, 255);
    }

    #[test]
    fn test_parse_color_string_rgb() {
        // Test rgb function
        let color = parse_color_string("rgb(255, 128, 64)").unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 128);
        assert_eq!(color.blue, 64);
        assert_eq!(color.alpha, 255);

        // Test rgba function
        let color = parse_color_string("rgba(255, 128, 64, 0.5)").unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 128);
        assert_eq!(color.blue, 64);
        assert_eq!(color.alpha, 127); // 0.5 * 255
    }

    #[test]
    fn test_parse_color_string_hsl() {
        // Test hsl function
        let color = parse_color_string("hsl(120, 100%, 50%)").unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 255);
        assert_eq!(color.blue, 0);
        assert_eq!(color.alpha, 255);

        // Test hsla function with opacity
        let color = parse_color_string("hsla(240, 100%, 50%, 0.8)").unwrap();
        assert_eq!(color.red, 0);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 255);
        assert_eq!(color.alpha, 204); // 0.8 * 255
    }

    #[test]
    fn test_parse_color_string_named() {
        // Test common named colors
        let color = parse_color_string("red").unwrap();
        assert_eq!(color.red, 255);
        assert_eq!(color.green, 0);
        assert_eq!(color.blue, 0);

        let color = parse_color_string("steelblue").unwrap();
        assert_eq!(color.red, 70);
        assert_eq!(color.green, 130);
        assert_eq!(color.blue, 180);

        // Test CSS named color that wasn't in our original list
        let color = parse_color_string("rebeccapurple").unwrap();
        assert_eq!(color.red, 102);
        assert_eq!(color.green, 51);
        assert_eq!(color.blue, 153);
    }

    #[test]
    fn test_parse_invalid_color() {
        assert!(parse_color_string("notacolor").is_none());
        assert!(parse_color_string("rgb(256, 300, 400)").is_some()); // css-color-parser clamps values
        assert!(parse_color_string("#gg00ff").is_none());
    }
}
