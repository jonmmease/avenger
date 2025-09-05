//! Color palettes for different scale types

use palette::Srgba;

/// Color gradient specification
#[derive(Clone, Debug)]
pub struct ColorGradient {
    /// Colors in the gradient
    pub colors: Vec<Srgba>,
}

impl ColorGradient {
    pub fn new(colors: Vec<Srgba>) -> Self {
        Self { colors }
    }

    /// Create from hex strings
    pub fn from_hex(hex_colors: &[&str]) -> Self {
        let colors = hex_colors
            .iter()
            .filter_map(|&hex| parse_hex_color(hex))
            .collect();
        Self { colors }
    }
}

/// Color palettes for different scale types
#[derive(Clone, Debug)]
pub struct ColorPalettes {
    /// Colors for categorical/ordinal scales
    pub categorical: Vec<String>,

    /// Gradient for sequential continuous scales
    pub sequential: ColorGradient,

    /// Gradient for diverging continuous scales
    pub diverging: ColorGradient,

    /// Colors for quantized scales
    pub quantized: Vec<String>,

    /// Default single color when no scale is applied
    pub default_color: String,
}

impl Default for ColorPalettes {
    fn default() -> Self {
        Self {
            // Okabe-Ito colorblind-safe palette
            categorical: vec![
                "#0072B2".to_string(), // Blue
                "#E69F00".to_string(), // Orange
                "#009E73".to_string(), // Bluish Green
                "#F0E442".to_string(), // Yellow
                "#D55E00".to_string(), // Vermillion
                "#56B4E9".to_string(), // Sky Blue
                "#CC79A7".to_string(), // Reddish Purple
                "#999999".to_string(), // Grey
            ],

            // Viridis-inspired gradient
            sequential: ColorGradient::new(vec![
                Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
                Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
                Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
                Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
            ]),

            // Blue-white-red diverging
            diverging: ColorGradient::new(vec![
                Srgba::new(0.019, 0.188, 0.380, 1.0), // Dark blue
                Srgba::new(0.129, 0.400, 0.675, 1.0), // Blue
                Srgba::new(0.262, 0.576, 0.765, 1.0), // Light blue
                Srgba::new(0.573, 0.773, 0.871, 1.0), // Pale blue
                Srgba::new(0.969, 0.969, 0.969, 1.0), // Near white
                Srgba::new(0.992, 0.859, 0.780, 1.0), // Pale red
                Srgba::new(0.957, 0.647, 0.510, 1.0), // Light red
                Srgba::new(0.843, 0.376, 0.373, 1.0), // Red
                Srgba::new(0.647, 0.000, 0.149, 1.0), // Dark red
            ]),

            // ColorBrewer RdYlBu for quantized scales
            quantized: vec![
                "#a50026".to_string(), // Dark red
                "#d73027".to_string(), // Red
                "#f46d43".to_string(), // Orange red
                "#fdae61".to_string(), // Light orange
                "#fee090".to_string(), // Pale yellow
                "#e0f3f8".to_string(), // Pale blue
                "#abd9e9".to_string(), // Light blue
                "#74add1".to_string(), // Blue
                "#4575b4".to_string(), // Dark blue
                "#313695".to_string(), // Very dark blue
            ],

            // Steel blue default
            default_color: "#4682b4".to_string(),
        }
    }
}

/// Parse a hex color string to Srgba
fn parse_hex_color(hex: &str) -> Option<Srgba> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some(Srgba::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        1.0,
    ))
}

impl ColorPalettes {
    /// Get colors for a specific scale type and optional cardinality
    pub fn for_scale(&self, scale_type: &str, cardinality: Option<usize>) -> Vec<String> {
        match scale_type {
            "ordinal" => match cardinality {
                Some(n) if n <= self.categorical.len() => {
                    self.categorical.iter().take(n).cloned().collect()
                }
                _ => self.categorical.clone(),
            },
            "quantize" | "quantile" | "threshold" => self.quantized.clone(),
            _ => vec![self.default_color.clone()],
        }
    }
}
