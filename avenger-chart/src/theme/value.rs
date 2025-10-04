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

    /// CSS calc() expression
    /// Stores the AST for later evaluation with context (base font size, runtime params)
    /// Example: calc(var(--base-size) * 2), calc(100% - 20px)
    Calc(Box<super::calc::CalcNode>),

    /// Relative color derived from origin using channel keywords
    /// Example: oklch(from blue calc(l - 0.2) c h)
    /// Syntax: <color-function>(from <origin> <components>)
    RelativeColor {
        space: crate::color::types::ColorSpace,
        origin: Box<ThemeValue>,
        lightness: super::color_component::ColorComponent,
        component1: super::color_component::ColorComponent,
        component2: super::color_component::ColorComponent,
        alpha: super::color_component::ColorComponent,
    },

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

    /// Extract font-size value with rem support and calc support
    ///
    /// Converts font-size values to pixels:
    /// - Number: Returns the raw number (interpreted as px)
    /// - Length(Px): Returns pixel value
    /// - Length(Rem): Converts rem to pixels using base_font_size (rem-based scaling)
    /// - Calc: Resolves calc expression with runtime params
    ///
    /// Note: Percentage values are not supported (would require parent element context)
    pub fn as_font_size(&self, base_font_size: f32) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            ThemeValue::Calc(calc_node) => {
                // Resolve calc without params (backward compatibility)
                let resolved = calc_node.resolve(base_font_size).ok()?;
                resolved.as_length_px(base_font_size)
            }
            _ => None,
        }
    }

    /// Extract font-size value with calc and runtime parameter support
    ///
    /// This is the extended version that supports CSS variables in calc expressions.
    /// Converts font-size values to pixels:
    /// - Number: Returns the raw number (interpreted as px)
    /// - Length(Px): Returns pixel value
    /// - Length(Rem): Converts rem to pixels using base_font_size
    /// - Calc: Resolves calc expression with runtime params from ScalarValue map
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values (from ThemeContext)
    /// * `base_font_size` - Base font size for rem conversion
    pub fn as_font_size_with_params(
        &self,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        base_font_size: f32,
    ) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            ThemeValue::Calc(calc_node) => {
                // Convert ScalarValue params to f64
                let f64_params = scalar_value_params_to_f64(params);

                // Resolve calc with params
                let resolved = calc_node
                    .resolve_with_params(&f64_params, base_font_size)
                    .ok()?;
                resolved.as_length_px(base_font_size)
            }
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

    /// Resolve to color with runtime parameters and origin color context
    ///
    /// This handles both absolute colors and relative color syntax.
    /// For relative colors, it resolves the origin color, converts to target space,
    /// evaluates component expressions with channel keywords, and builds the derived color.
    pub fn as_color_with_params(
        &self,
        params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
        base_font_size: f32,
    ) -> Option<CssRgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),

            // Handle CSS variables - look up in params and recursively resolve
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                if let Some(value) = params.get(name) {
                    match value {
                        // String value - could be a color name or hex color
                        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                            parse_color_string(s)
                        }
                        // Already a number - not a color
                        _ => None,
                    }
                } else {
                    None
                }
            }

            // Handle string values that might be color names
            ThemeValue::String(s) => parse_color_string(s),

            ThemeValue::RelativeColor {
                space,
                origin,
                lightness,
                component1,
                component2,
                alpha,
            } => {
                use crate::color::types::AbsoluteColor;

                // 1. Resolve origin color recursively
                let origin_rgba = origin.as_color_with_params(params, base_font_size)?;
                let mut origin_abs = AbsoluteColor::from_css_rgba(&origin_rgba);

                // 2. Convert origin to target color space
                origin_abs = origin_abs.to_color_space(*space);

                // 3. Convert params to f64
                let params_f64 = scalar_value_params_to_f64(params);

                // 4. Resolve each component with origin color context
                let c0 = lightness
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let c1 = component1
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let c2 = component2
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let a = alpha
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;

                // 5. Build derived color in target space
                let derived = AbsoluteColor::new(*space, c0, c1, c2, a);

                // 6. Convert to CssRgba
                Some(derived.to_css_rgba())
            }

            // Handle light-dark() function - resolve based on color-scheme param
            ThemeValue::LightDark(light, dark) => {
                use datafusion_common::ScalarValue;

                // Check color-scheme param to decide which value to use
                let use_dark = params
                    .get("color-scheme")
                    .and_then(|v| match v {
                        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                            Some(s.as_str() == "dark")
                        }
                        _ => None,
                    })
                    .unwrap_or(false); // Default to light mode if no param

                let selected = if use_dark { dark } else { light };

                // Recursively resolve the selected value
                selected.as_color_with_params(params, base_font_size)
            }

            // Handle function calls that return colors (e.g., contrast-color, color-mix)
            ThemeValue::Function(name, args) => match name.as_str() {
                "contrast-color" => {
                    use crate::theme::contrast_color::resolve_contrast_color_with_params;
                    resolve_contrast_color_with_params(args, params, base_font_size)
                }
                "color-mix" => {
                    use crate::theme::color_mix::resolve_color_mix_with_params;
                    resolve_color_mix_with_params(args, params, base_font_size)
                }
                _ => None,
            },

            _ => None,
        }
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

/// Convert ScalarValue parameters to f64 for calc resolution
///
/// This converts DataFusion ScalarValue types to f64 for use in calc expressions.
/// Supports: Int64, UInt64, Float64, Int32, UInt32, Float32
fn scalar_value_params_to_f64(
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
) -> indexmap::IndexMap<String, f64> {
    use datafusion::common::ScalarValue;

    params
        .iter()
        .filter_map(|(key, value)| {
            let f64_value = match value {
                ScalarValue::Int64(Some(v)) => Some(*v as f64),
                ScalarValue::UInt64(Some(v)) => Some(*v as f64),
                ScalarValue::Float64(Some(v)) => Some(*v),
                ScalarValue::Int32(Some(v)) => Some(*v as f64),
                ScalarValue::UInt32(Some(v)) => Some(*v as f64),
                ScalarValue::Float32(Some(v)) => Some(*v as f64),
                _ => None,
            };
            f64_value.map(|v| (key.clone(), v))
        })
        .collect()
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
