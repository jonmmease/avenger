//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts, including colors, fonts, shapes, and other styling properties.

use serde::{Deserialize, Serialize};

// Core theme modules
mod context;
mod value;

// CSS theme system
pub mod css;

// Re-export core types
pub use context::{ContextBuilder, ThemeContext};
pub use value::{
    LengthUnit, Rgba, ThemeValue, parse_color_string, parse_hex_color, parse_named_color,
};

/// Main theme trait that can be implemented by different backends
#[typetag::serde(tag = "type")]
pub trait Theme: Send + Sync {
    /// Query a theme property for a given context
    fn query(&self, context: &ThemeContext, property: &str) -> ThemeValue;

    /// Query with fallback value if property is not found
    fn query_or(&self, context: &ThemeContext, property: &str, fallback: ThemeValue) -> ThemeValue {
        let value = self.query(context, property);
        match value {
            ThemeValue::None => fallback,
            _ => value,
        }
    }

    /// Get the base font size in pixels (used for rem unit conversion)
    fn base_font_size(&self) -> f32 {
        12.0
    }

    /// Get font family for a context
    fn font_family(&self, context: &ThemeContext) -> String {
        let font_family_value = self.query(context, "font-family");

        // Get the list of fonts from the theme value
        let fonts = match font_family_value {
            ThemeValue::List(values) => {
                // It's already a list, extract the font names
                values
                    .into_iter()
                    .filter_map(|v| v.as_string().map(|s| s.to_string()))
                    .collect()
            }
            ThemeValue::String(s) => {
                // Single font
                vec![s]
            }
            _ => vec!["sans-serif".to_string()],
        };

        // Return the first available font from the list
        select_available_font(fonts)
    }

    /// Get font size for a context
    fn font_size(&self, context: &ThemeContext) -> f32 {
        self.query(context, "font-size")
            .as_pixels(self.base_font_size())
            .unwrap_or(12.0)
    }

    /// Get font weight for a context
    fn font_weight(&self, context: &ThemeContext) -> f32 {
        self.query(context, "font-weight")
            .as_pixels(self.base_font_size())
            .unwrap_or(400.0)
    }

    /// Get color for a context
    fn color(&self, context: &ThemeContext) -> String {
        self.query(context, "color")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get fill color for a context
    fn fill_color(&self, context: &ThemeContext) -> String {
        self.query(context, "fill")
            .to_string_value()
            .unwrap_or_else(|| "#4682b4".to_string())
    }

    /// Get stroke color for a context
    fn stroke_color(&self, context: &ThemeContext) -> String {
        self.query(context, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get stroke width for a context
    fn stroke_width(&self, context: &ThemeContext) -> f32 {
        self.query(context, "stroke-width")
            .as_pixels(self.base_font_size())
            .unwrap_or(1.0)
    }

    /// Get opacity for a context
    fn opacity(&self, context: &ThemeContext) -> f32 {
        self.query(context, "opacity")
            .as_pixels(self.base_font_size())
            .unwrap_or(1.0)
    }

    /// Clone the theme into a boxed trait object
    fn clone_box(&self) -> Box<dyn Theme>;

    // Additional methods for accessing specific theme properties

    /// Get canvas background color
    fn canvas_background(&self) -> Option<String> {
        let ctx = ThemeContext::new("canvas");
        match self.query(&ctx, "background-color") {
            ThemeValue::String(s) => Some(s),
            ThemeValue::Color(rgba) => {
                // Convert Color to hex string
                Some(format!(
                    "#{:02x}{:02x}{:02x}",
                    rgba.red, rgba.green, rgba.blue
                ))
            }
            _ => None,
        }
    }

    /// Get mark defaults as a cloned IndexMap
    fn mark_defaults_map(
        &self,
    ) -> indexmap::IndexMap<String, indexmap::IndexMap<String, datafusion_common::ScalarValue>>
    {
        // This is a placeholder - actual implementation would query mark defaults
        indexmap::IndexMap::new()
    }

    /// Get mark default with computed fonts
    fn mark_default_with_computed_fonts(
        &self,
        mark_type: &str,
        channel: &str,
        computed_font_size: f32,
        base_font_family: &str,
    ) -> Option<datafusion_common::ScalarValue> {
        eprintln!(
            "DEBUG trait default mark_default_with_computed_fonts: mark_type={}, channel={}",
            mark_type, channel
        );
        use datafusion_common::ScalarValue;

        if mark_type == "text" {
            match channel {
                "font_size" => return Some(ScalarValue::Float32(Some(computed_font_size))),
                "font" => {
                    // Use the base font family for text marks by default
                    return Some(ScalarValue::Utf8(Some(base_font_family.to_string())));
                }
                _ => {}
            }
        }

        // For other marks/channels, return None (actual implementation would query defaults)
        None
    }

    /// Get base font family
    fn base_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("base"))
    }

    /// Get text mark font size
    fn text_mark_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("mark").with_mark("text"))
    }

    // Legend-specific methods

    /// Get legend background fill
    fn legend_background_fill(&self) -> Option<String> {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        match self.query(&ctx, "fill") {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background stroke
    fn legend_background_stroke(&self) -> Option<String> {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        match self.query(&ctx, "stroke") {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background padding
    fn legend_background_padding(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "padding")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get legend background corner radius
    fn legend_background_corner_radius(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        self.query(&ctx, "corner-radius")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get legend title color
    fn legend_title_color(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.color(&legend_ctx.child("title"))
    }

    /// Get legend label color
    fn legend_label_color(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.color(&legend_ctx.child("label"))
    }

    /// Get legend tick color
    fn legend_tick_color(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.color(&legend_ctx.child("tick"))
    }

    /// Get legend title font family
    fn legend_title_font_family(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.font_family(&legend_ctx.child("title"))
    }

    /// Get legend label font family
    fn legend_label_font_family(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.font_family(&legend_ctx.child("label"))
    }

    /// Get legend tick font family
    fn legend_tick_font_family(&self) -> String {
        let legend_ctx = ThemeContext::new("legend");
        self.font_family(&legend_ctx.child("tick"))
    }

    /// Get legend title font size
    fn legend_title_font_size(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_size(&legend_ctx.child("title"))
    }

    /// Get legend label font size
    fn legend_label_font_size(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_size(&legend_ctx.child("label"))
    }

    /// Get legend tick font size
    fn legend_tick_font_size(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_size(&legend_ctx.child("tick"))
    }

    /// Get legend title font weight
    fn legend_title_font_weight(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_weight(&legend_ctx.child("title"))
    }

    /// Get legend label font weight
    fn legend_label_font_weight(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_weight(&legend_ctx.child("label"))
    }

    /// Get legend tick font weight
    fn legend_tick_font_weight(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        self.font_weight(&legend_ctx.child("tick"))
    }

    // Title-specific methods

    /// Get title color
    fn title_color(&self) -> String {
        self.color(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle color
    fn subtitle_color(&self) -> String {
        self.color(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font family
    fn title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font family
    fn subtitle_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font size
    fn title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font size
    fn subtitle_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font weight
    fn title_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font weight
    fn subtitle_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("chart-subtitle"))
    }

    // Axis-specific methods

    /// Get axis domain color
    fn axis_domain_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("domain");
        self.query(&ctx, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get axis tick color
    fn axis_tick_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, "stroke")
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get axis grid color
    fn axis_grid_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("grid");
        self.stroke_color(&ctx)
    }

    /// Get axis grid opacity
    fn axis_grid_opacity(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("grid");
        self.query(&ctx, "opacity")
            .as_pixels(self.base_font_size())
            .unwrap_or(0.5)
    }

    /// Get axis grid width
    fn axis_grid_width(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("grid");
        self.stroke_width(&ctx)
    }

    /// Get axis label color
    fn axis_label_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        self.color(&axis_ctx.child("label"))
    }

    /// Get axis title color
    fn axis_title_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        self.color(&axis_ctx.child("title"))
    }

    /// Get axis tick length
    fn axis_tick_length(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, "size")
            .as_pixels(self.base_font_size())
            .unwrap_or(5.0)
    }

    /// Get axis label font size
    fn axis_label_font_size(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        self.font_size(&axis_ctx.child("label"))
    }

    /// Get axis label font weight
    fn axis_label_font_weight(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        self.font_weight(&axis_ctx.child("label"))
    }

    /// Get axis title font size
    fn axis_title_font_size(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        self.font_size(&axis_ctx.child("title"))
    }

    /// Get axis title font weight
    fn axis_title_font_weight(&self) -> f32 {
        let axis_ctx = ThemeContext::new("axis");
        self.font_weight(&axis_ctx.child("title"))
    }

    /// Get axis label font family
    fn axis_label_font_family(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        self.font_family(&axis_ctx.child("label"))
    }

    /// Get axis title font family
    fn axis_title_font_family(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        self.font_family(&axis_ctx.child("title"))
    }

    // Access to color palettes, shapes, and dashes

    /// Get categorical color palette
    fn categorical_colors(&self) -> Vec<String> {
        // Okabe-Ito colorblind-safe palette
        vec![
            "#0072B2".to_string(), // Blue
            "#E69F00".to_string(), // Orange
            "#009E73".to_string(), // Bluish Green
            "#F0E442".to_string(), // Yellow
            "#D55E00".to_string(), // Vermillion
            "#56B4E9".to_string(), // Sky Blue
            "#CC79A7".to_string(), // Reddish Purple
            "#999999".to_string(), // Grey
        ]
    }

    /// Get shape names for shape channel
    fn shape_names(&self) -> Vec<String> {
        vec![
            "circle".to_string(),
            "cross".to_string(),
            "diamond".to_string(),
            "square".to_string(),
            "star".to_string(),
            "triangle-up".to_string(),
            "wye".to_string(),
            "cushion".to_string(),
        ]
    }

    /// Get dash pattern names
    fn dash_names(&self) -> Vec<String> {
        vec![
            "solid".to_string(),
            "dashed".to_string(),
            "dotted".to_string(),
            "long-dash".to_string(),
            "dash-dot".to_string(),
            "long-short".to_string(),
            "even-short".to_string(),
            "double-dash".to_string(),
        ]
    }

    /// Get mark default for a specific mark type and channel
    fn mark_default(
        &self,
        _mark_type: &str,
        _channel: &str,
    ) -> Option<datafusion_common::ScalarValue> {
        // Default implementation returns None
        // Actual implementation would query mark defaults
        None
    }

    /// Get default shape range for ordinal scales
    fn get_shape_range(&self, _domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let shapes = self.shape_names();
        let scalars: Vec<ScalarValue> = shapes
            .into_iter()
            .map(|s| ScalarValue::Utf8(Some(s)))
            .collect();
        ScaleRange::new_discrete(scalars)
    }

    /// Get default dash pattern range for ordinal scales
    fn get_dash_range(&self, _domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let patterns = self.dash_names();
        let scalars: Vec<ScalarValue> = patterns
            .into_iter()
            .map(|p| ScalarValue::Utf8(Some(p)))
            .collect();
        ScaleRange::new_discrete(scalars)
    }

    /// Get range for a specific channel based on mark type and range kind
    fn get_range_for_channel(
        &self,
        _mark_type: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion_common::ScalarValue;

        // Default implementation delegates to existing methods for compatibility
        match channel {
            "fill" | "stroke" | "color" => match range_kind {
                RangeKind::Discrete => {
                    // Use categorical colors for discrete
                    let colors = self.categorical_colors();
                    let scalars: Vec<ScalarValue> = colors
                        .into_iter()
                        .map(|c| ScalarValue::Utf8(Some(c)))
                        .collect();
                    ScaleRange::new_discrete(scalars)
                }
                RangeKind::Continuous => {
                    // For continuous color scales, use Color variant with Srgba values
                    use palette::Srgba;

                    // Viridis-like gradient: dark purple -> blue -> green -> yellow
                    let colors = vec![
                        Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
                        Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
                        Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
                        Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
                    ];
                    ScaleRange::new_color(colors)
                }
            },
            "shape" => self.get_shape_range(domain_cardinality),
            "stroke_dash" => self.get_dash_range(domain_cardinality),
            "size" => match range_kind {
                RangeKind::Discrete => {
                    // Discrete sizes
                    let sizes = vec![60.0, 120.0, 180.0, 240.0, 300.0];
                    let scalars: Vec<ScalarValue> = sizes
                        .into_iter()
                        .map(|s| ScalarValue::Float32(Some(s as f32)))
                        .collect();
                    ScaleRange::new_discrete(scalars)
                }
                RangeKind::Continuous => {
                    // Size interval
                    ScaleRange::new_interval(
                        datafusion::logical_expr::lit(20.0f32),
                        datafusion::logical_expr::lit(400.0f32),
                    )
                }
            },
            "opacity" | "fill_opacity" | "stroke_opacity" => match range_kind {
                RangeKind::Discrete => {
                    // Discrete opacities
                    let opacities = vec![0.2, 0.4, 0.6, 0.8, 1.0];
                    let scalars: Vec<ScalarValue> = opacities
                        .into_iter()
                        .map(|o| ScalarValue::Float32(Some(o as f32)))
                        .collect();
                    ScaleRange::new_discrete(scalars)
                }
                RangeKind::Continuous => {
                    // Opacity interval
                    ScaleRange::new_interval(
                        datafusion::logical_expr::lit(0.0f32),
                        datafusion::logical_expr::lit(1.0f32),
                    )
                }
            },
            "stroke_width" => match range_kind {
                RangeKind::Discrete => {
                    // Discrete stroke widths
                    let widths = vec![0.5, 1.0, 2.0, 3.0, 5.0];
                    let scalars: Vec<ScalarValue> = widths
                        .into_iter()
                        .map(|w| ScalarValue::Float32(Some(w as f32)))
                        .collect();
                    ScaleRange::new_discrete(scalars)
                }
                RangeKind::Continuous => {
                    // Stroke width interval
                    ScaleRange::new_interval(
                        datafusion::logical_expr::lit(0.5f32),
                        datafusion::logical_expr::lit(5.0f32),
                    )
                }
            },
            _ => {
                // Default ranges for unknown channels
                match range_kind {
                    RangeKind::Discrete => {
                        let defaults = vec!["A", "B", "C", "D", "E"];
                        let scalars: Vec<ScalarValue> = defaults
                            .into_iter()
                            .map(|s| ScalarValue::Utf8(Some(s.to_string())))
                            .collect();
                        ScaleRange::new_discrete(scalars)
                    }
                    RangeKind::Continuous => ScaleRange::new_interval(
                        datafusion::logical_expr::lit(0.0f32),
                        datafusion::logical_expr::lit(1.0f32),
                    ),
                }
            }
        }
    }
}

/// Select the first available font from a list of font families
/// Checks against the fonts available in the system using avenger-text
fn select_available_font(fonts: Vec<String>) -> String {
    use avenger_text::font_resolver::{FontResolver, default_font_resolver};

    let resolver = default_font_resolver();
    resolver.select_available_font(fonts)
}
