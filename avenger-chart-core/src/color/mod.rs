//! Color space support for CSS color functions
//!
//! This module provides color space conversions and color mixing functionality
//! needed for advanced CSS color functions like `color-mix()`, `oklch()`, etc.
//!
//! ## Architecture
//!
//! - `types.rs`: Core color types (AbsoluteColor, ColorSpace)
//! - `convert.rs`: Color space conversion functions
//! - `mix.rs`: Color mixing/interpolation
//! - `contrast.rs`: WCAG 2.1 contrast ratio calculations

use avenger_common::types::ColorOrGradient;
use avenger_scales::scales::coerce::Coercer;
use datafusion::scalar::ScalarValue;

use crate::AvengerChartError;

pub mod contrast;
pub mod convert;
pub mod mix;
pub mod types;

pub use self::{
    contrast::{
        choose_best_contrast, choose_contrast_color, contrast_ratio, relative_luminance_srgb,
    },
    convert::{normalize_hue, orthogonal_to_polar, polar_to_orthogonal},
    mix::{HueInterpolationMethod, mix_colors},
    types::{AbsoluteColor, ColorSpace},
};

/// Strict color parser that returns an error if the color string cannot be parsed.
pub fn parse_color_string_strict(color_str: &str) -> Result<ColorOrGradient, AvengerChartError> {
    let coercer = Coercer::default();
    let array = ScalarValue::iter_to_array(
        [ScalarValue::Utf8(Some(color_str.to_string()))]
            .iter()
            .cloned(),
    )
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to create array for color: {}", e))
    })?;

    let colors = coercer.to_color(&array, None).map_err(|e| {
        AvengerChartError::InternalError(format!("Invalid color string '{}': {}", color_str, e))
    })?;

    colors.as_vec(1, None).first().cloned().ok_or_else(|| {
        AvengerChartError::InternalError(format!("Failed to extract color from '{}'", color_str))
    })
}

/// Helper to parse color from string using the color coercer.
///
/// Returns `None` if parsing fails.
pub fn parse_color_string(color_str: &str) -> Option<ColorOrGradient> {
    parse_color_string_strict(color_str).ok()
}

/// Parse a color string to an RGBA array.
///
/// Returns an error if the color string cannot be parsed or parses to a
/// gradient instead of a solid color.
pub fn parse_color_to_array_strict(color_str: &str) -> Result<[f32; 4], AvengerChartError> {
    let color = parse_color_string_strict(color_str)?;
    match color {
        ColorOrGradient::Color(rgba) => Ok(rgba),
        _ => Err(AvengerChartError::InternalError(format!(
            "Color string '{}' parsed to gradient, expected solid color",
            color_str
        ))),
    }
}

/// Helper to parse a color string to an RGBA array.
///
/// Returns black if parsing fails.
pub fn parse_color_to_array(color_str: &str) -> [f32; 4] {
    parse_color_to_array_strict(color_str).unwrap_or([0.0, 0.0, 0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_white_color_to_rgba_array() {
        let white = parse_color_to_array("#FFFFFF");
        assert!(
            white[0] > 0.99 && white[0] <= 1.0,
            "Red should be ~1.0, got {}",
            white[0]
        );
        assert!(
            white[1] > 0.99 && white[1] <= 1.0,
            "Green should be ~1.0, got {}",
            white[1]
        );
        assert!(
            white[2] > 0.99 && white[2] <= 1.0,
            "Blue should be ~1.0, got {}",
            white[2]
        );
        assert_eq!(white[3], 1.0, "Alpha should be 1.0");
    }

    #[test]
    fn parses_black_color_to_rgba_array() {
        let black = parse_color_to_array("#000000");
        assert!(black[0] < 0.01, "Red should be ~0.0, got {}", black[0]);
        assert!(black[1] < 0.01, "Green should be ~0.0, got {}", black[1]);
        assert!(black[2] < 0.01, "Blue should be ~0.0, got {}", black[2]);
        assert_eq!(black[3], 1.0, "Alpha should be 1.0");
    }
}
