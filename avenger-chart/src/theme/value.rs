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

    /// Color value (parsed from CSS)
    Color(CssRgba),

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
