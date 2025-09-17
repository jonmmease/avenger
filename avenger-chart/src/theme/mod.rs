//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts, including colors, fonts, shapes, and other styling properties.

mod colors;
mod dashes;
mod marks;
mod presets;
mod shapes;

// New trait-based theme system
mod struct_impl;
mod theme_interface;

// CSS theme systems
pub mod css; // Full CSS parser with cssparser/selectors

pub use colors::ColorPalettes;
pub use dashes::DashPatterns;
pub use marks::MarkDefaults;
pub use shapes::ShapeSequence;

// Export trait and related types
pub use theme_interface::{
    ContextBuilder, LengthUnit, Rgba, Theme, ThemeContext, ThemeProperty, ThemeValue,
};

/// Font scale configuration for proportional font sizing
#[derive(Clone, Debug, Copy)]
pub struct FontScale {
    /// Plot title scale (default: 1.5)
    pub title_scale: f32,
    /// Plot subtitle scale (default: 1.167)
    pub subtitle_scale: f32,
    /// Axis title scale (default: 1.0)
    pub axis_title_scale: f32,
    /// Legend title scale (default: 1.0)
    pub legend_title_scale: f32,
    /// Legend label scale (default: 0.917)
    pub legend_label_scale: f32,
    /// Axis label scale (default: 0.833)
    pub axis_label_scale: f32,
    /// Legend tick scale (default: 0.833)
    pub legend_tick_scale: f32,
    /// Text mark scale (default: 1.0)
    pub text_mark_scale: f32,
}

impl Default for FontScale {
    fn default() -> Self {
        Self {
            title_scale: 1.5,          // 18px @ 12px base
            subtitle_scale: 1.167,     // 14px @ 12px base
            axis_title_scale: 1.0,     // 12px @ 12px base
            legend_title_scale: 1.0,   // 12px @ 12px base
            legend_label_scale: 0.917, // 11px @ 12px base
            axis_label_scale: 0.833,   // 10px @ 12px base
            legend_tick_scale: 0.833,  // 10px @ 12px base
            text_mark_scale: 1.0,      // 12px @ 12px base
        }
    }
}

impl FontScale {
    /// Create a compact font scale with tighter hierarchy
    pub fn compact() -> Self {
        Self {
            title_scale: 1.333,        // 16px @ 12px base
            subtitle_scale: 1.083,     // 13px @ 12px base
            axis_title_scale: 1.0,     // 12px @ 12px base
            legend_title_scale: 1.0,   // 12px @ 12px base
            legend_label_scale: 0.917, // 11px @ 12px base
            axis_label_scale: 0.833,   // 10px @ 12px base
            legend_tick_scale: 0.833,  // 10px @ 12px base
            text_mark_scale: 1.0,      // 12px @ 12px base
        }
    }

    /// Create a dramatic font scale with more contrast
    pub fn dramatic() -> Self {
        Self {
            title_scale: 2.0,          // 24px @ 12px base
            subtitle_scale: 1.333,     // 16px @ 12px base
            axis_title_scale: 1.083,   // 13px @ 12px base
            legend_title_scale: 1.083, // 13px @ 12px base
            legend_label_scale: 0.917, // 11px @ 12px base
            axis_label_scale: 0.75,    // 9px @ 12px base
            legend_tick_scale: 0.75,   // 9px @ 12px base
            text_mark_scale: 1.0,      // 12px @ 12px base
        }
    }
}

/// Complete theme configuration for chart styling (struct implementation)
#[derive(Clone, Debug)]
pub struct StructTheme {
    /// Base font family (all text inherits from this if not specified)
    pub base_font_family: String,

    /// Base font size for all text elements
    pub base_font_size: f32,

    /// Font scale multipliers
    pub font_scale: FontScale,

    /// Title and subtitle theme
    pub title: TitleTheme,

    /// Color palettes for different scale types
    pub colors: ColorPalettes,

    /// Shape sequences for categorical shape encoding
    pub shapes: ShapeSequence,

    /// Dash patterns for line styling
    pub dashes: DashPatterns,

    /// Default values for mark properties
    pub mark_defaults: MarkDefaults,

    /// Layout and spacing configuration
    pub layout: LayoutTheme,

    /// Axis and grid theme
    pub axis: AxisTheme,

    /// Legend theme
    pub legend: LegendTheme,

    /// Background theme
    pub background: BackgroundTheme,
}

/// Layout and spacing configuration
#[derive(Clone, Debug)]
pub struct LayoutTheme {
    /// Default padding around the plot area
    pub padding: Padding,

    /// Spacing between legend items
    pub legend_spacing: f32,

    /// Spacing between faceted plots (row_spacing, col_spacing)
    pub facet_spacing: (f32, f32),
}

/// Padding configuration
#[derive(Clone, Debug, Copy)]
pub struct Padding {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Padding {
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn uniform(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// Axis and grid theme configuration
#[derive(Clone, Debug)]
pub struct AxisTheme {
    /// Grid line color
    pub grid_color: String,

    /// Grid line opacity (0.0 to 1.0)
    pub grid_opacity: f32,

    /// Grid line width
    pub grid_width: f32,

    /// Grid line dash pattern (None for solid)
    pub grid_dash: Option<Vec<f32>>,

    /// Axis domain line color
    pub domain_color: String,

    /// Axis domain line width
    pub domain_width: f32,

    /// Tick mark color
    pub tick_color: String,

    /// Tick mark size
    pub tick_size: f32,

    /// Tick mark length
    pub tick_length: f32,

    /// Label padding from tick marks
    pub label_padding: f32,

    /// Default label angle (0 for horizontal)
    pub label_angle: f32,

    // Axis label (tick label) typography
    /// Label text color
    pub label_color: String,

    /// Label font family (None defaults to base font family)
    pub label_font_family: Option<String>,

    /// Label font weight
    pub label_font_weight: f32,

    // Axis title typography
    /// Title text color
    pub title_color: String,

    /// Title font family (None defaults to base font family)
    pub title_font_family: Option<String>,

    /// Title font weight
    pub title_font_weight: f32,
}

/// Legend theme configuration
#[derive(Clone, Debug)]
pub struct LegendTheme {
    /// Background fill color (None for transparent)
    pub background_fill: Option<String>,

    /// Background stroke color (None for no stroke)
    pub background_stroke: Option<String>,

    /// Padding inside legend background
    pub background_padding: f32,

    /// Corner radius for legend background
    pub background_corner_radius: f32,

    /// Spacing between legend items
    pub item_spacing: f32,

    /// Default symbol size in legends
    pub symbol_size: f32,

    /// Padding between symbol and label
    pub label_padding: f32,

    // Legend title typography
    /// Title text color
    pub title_color: String,

    /// Title font family (None defaults to base font family)
    pub title_font_family: Option<String>,

    /// Title font weight
    pub title_font_weight: f32,

    // Legend label typography (for discrete legends)
    /// Label text color
    pub label_color: String,

    /// Label font family (None defaults to base font family)
    pub label_font_family: Option<String>,

    /// Label font weight
    pub label_font_weight: f32,

    // Legend tick label typography (for continuous/colorbar legends)
    /// Tick label text color
    pub tick_color: String,

    /// Tick label font family (None defaults to base font family)
    pub tick_font_family: Option<String>,

    /// Tick label font weight
    pub tick_font_weight: f32,
}

/// Title theme configuration for plot titles and subtitles
#[derive(Clone, Debug)]
pub struct TitleTheme {
    // Main title typography
    /// Title text color
    pub title_color: String,

    /// Title font family (None defaults to base font family)
    pub title_font_family: Option<String>,

    /// Title font weight
    pub title_font_weight: f32,

    // Subtitle typography
    /// Subtitle text color
    pub subtitle_color: String,

    /// Subtitle font family (None defaults to base font family)
    pub subtitle_font_family: Option<String>,

    /// Subtitle font weight
    pub subtitle_font_weight: f32,
}

/// Background color configuration
#[derive(Clone, Debug, Default)]
pub struct BackgroundTheme {
    /// Plot area background (None for transparent)
    pub plot_background: Option<String>,

    /// Canvas background (None for transparent)
    pub canvas_background: Option<String>,
}

impl StructTheme {
    // Helper methods that use the trait interface for backward compatibility
    // These delegate to the trait implementation for consistency

    pub fn title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("title"))
    }

    pub fn subtitle_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("subtitle"))
    }

    pub fn axis_title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("axis").with_class("title"))
    }

    pub fn axis_label_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("axis").with_class("label"))
    }

    pub fn legend_title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("title"))
    }

    pub fn legend_label_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("label"))
    }

    pub fn legend_tick_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("tick"))
    }

    pub fn text_mark_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("mark").with_mark("text"))
    }

    pub fn title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("title"))
    }

    pub fn subtitle_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("subtitle"))
    }

    pub fn axis_title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("axis").with_class("title"))
    }

    pub fn axis_label_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("axis").with_class("label"))
    }

    pub fn legend_title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("title"))
    }

    pub fn legend_label_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("label"))
    }

    pub fn legend_tick_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("tick"))
    }

    /// Set the font family for all text elements in the theme
    pub fn set_font_family(&mut self, font: &str) {
        self.base_font_family = font.to_string();

        // Clear specific font families to use the new base
        self.title.title_font_family = None;
        self.title.subtitle_font_family = None;

        self.axis.label_font_family = None;
        self.axis.title_font_family = None;

        self.legend.title_font_family = None;
        self.legend.label_font_family = None;
        self.legend.tick_font_family = None;

        // Clear text mark font override
        self.mark_defaults
            .defaults
            .get_mut("text")
            .and_then(|text_defaults| text_defaults.shift_remove("font"));
    }

    /// Builder method to set font family for all text elements
    pub fn with_font_family(mut self, font: &str) -> Self {
        self.set_font_family(font);
        self
    }

    /// Get default color range for a scale type (implementation)
    fn get_color_range_impl(
        &self,
        scale_type: &str,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        match scale_type {
            "ordinal" => {
                // Use theme categorical colors
                let colors: Vec<ScalarValue> = self
                    .colors
                    .categorical
                    .iter()
                    .map(|c| ScalarValue::Utf8(Some(c.clone())))
                    .collect();

                // If we know the domain cardinality, only return that many colors
                match domain_cardinality {
                    Some(n) if n <= colors.len() => {
                        ScaleRange::Discrete(colors.into_iter().take(n).collect())
                    }
                    _ => ScaleRange::Discrete(colors),
                }
            }
            "linear" | "log" | "pow" | "sqrt" => {
                // Use theme sequential gradient
                ScaleRange::Color(self.colors.sequential.colors.clone())
            }
            "quantize" | "quantile" => {
                // Use theme quantized colors
                let colors: Vec<ScalarValue> = self
                    .colors
                    .quantized
                    .iter()
                    .map(|c| ScalarValue::Utf8(Some(c.clone())))
                    .collect();
                ScaleRange::Discrete(colors)
            }
            _ => {
                // Default single color from theme
                ScaleRange::Discrete(vec![ScalarValue::Utf8(Some(
                    self.colors.default_color.clone(),
                ))])
            }
        }
    }

    /// Get default shape range for ordinal scales (implementation)
    fn get_shape_range_impl(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let shapes = self.shapes.get_shape_strings(domain_cardinality);
        let scalars: Vec<ScalarValue> = shapes
            .into_iter()
            .map(|s| ScalarValue::Utf8(Some(s)))
            .collect();
        ScaleRange::new_discrete(scalars)
    }

    /// Get default dash pattern range for ordinal scales
    pub fn get_dash_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let patterns = self.dashes.get_pattern_strings(domain_cardinality);
        let scalars: Vec<ScalarValue> = patterns
            .into_iter()
            .map(|p| ScalarValue::Utf8(Some(p)))
            .collect();
        ScaleRange::new_discrete(scalars)
    }
}

impl Default for StructTheme {
    fn default() -> Self {
        Self {
            base_font_family: "Atkinson Hyperlegible Next".to_string(),
            base_font_size: 12.0,
            font_scale: FontScale::default(),
            title: TitleTheme::default(),
            colors: ColorPalettes::default(),
            shapes: ShapeSequence::default(),
            dashes: DashPatterns::default(),
            mark_defaults: MarkDefaults::default(),
            layout: LayoutTheme::default(),
            axis: AxisTheme::default(),
            legend: LegendTheme::default(),
            background: BackgroundTheme::default(),
        }
    }
}

impl Default for LayoutTheme {
    fn default() -> Self {
        Self {
            padding: Padding::uniform(5.0),
            legend_spacing: 10.0,
            facet_spacing: (10.0, 10.0),
        }
    }
}

impl Default for AxisTheme {
    fn default() -> Self {
        Self {
            grid_color: "#e0e0e0".to_string(), // #E0E0E0
            grid_opacity: 0.5,                 // 50% opacity
            grid_width: 0.5,
            grid_dash: None,
            domain_color: "#000".to_string(),
            domain_width: 1.0,
            tick_color: "#000".to_string(),
            tick_size: 5.0,
            tick_length: 5.0,
            label_padding: 3.0,
            label_angle: 0.0,
            // Label (tick label) typography
            label_color: "#5a5a5a".to_string(),
            label_font_family: None, // Defaults to base font family
            label_font_weight: 300.0,
            // Title typography
            title_color: "#2a2a2a".to_string(),
            title_font_family: None, // Defaults to base font family
            title_font_weight: 400.0,
        }
    }
}

impl Default for LegendTheme {
    fn default() -> Self {
        Self {
            background_fill: None,   // No background by default
            background_stroke: None, // No stroke by default
            background_padding: 8.0,
            background_corner_radius: 0.0, // No radius by default
            item_spacing: 10.0,
            symbol_size: 100.0,
            label_padding: 5.0,
            // Title typography
            title_color: "#2C2C2C".to_string(),
            title_font_family: None, // Defaults to base font family
            title_font_weight: 400.0,
            // Label typography (for discrete legends)
            label_color: "#3C3C3C".to_string(),
            label_font_family: None, // Defaults to base font family
            label_font_weight: 300.0,
            // Tick label typography (for continuous/colorbar legends)
            tick_color: "#5a5a5a".to_string(), // Same as axis labels
            tick_font_family: None,            // Defaults to base font family
            tick_font_weight: 300.0,
        }
    }
}

impl Default for TitleTheme {
    fn default() -> Self {
        Self {
            // Title typography
            title_color: "#1a1a1a".to_string(),
            title_font_family: None,  // Defaults to base font family
            title_font_weight: 500.0, // Medium weight
            // Subtitle typography
            subtitle_color: "#4a4a4a".to_string(),
            subtitle_font_family: None,  // Defaults to base font family
            subtitle_font_weight: 200.0, // Light weight
        }
    }
}

impl StructTheme {
    /// Builder method to set color palettes
    pub fn with_colors(mut self, colors: ColorPalettes) -> Self {
        self.colors = colors;
        self
    }

    /// Convenience method to set the default font family (calls set_font_family)
    pub fn with_font(mut self, font: &str) -> Self {
        self.set_font_family(font);
        self
    }

    /// Builder method to set mark defaults
    pub fn with_mark_defaults(mut self, mark_defaults: MarkDefaults) -> Self {
        self.mark_defaults = mark_defaults;
        self
    }

    /// Set a specific mark channel default
    pub fn set_mark_default(
        mut self,
        mark_type: &str,
        channel: &str,
        value: datafusion_common::ScalarValue,
    ) -> Self {
        self.mark_defaults = self.mark_defaults.with_default(mark_type, channel, value);
        self
    }

    /// Scale all fonts proportionally
    pub fn with_font_size(mut self, base_size: f32) -> Self {
        self.base_font_size = base_size;
        self
    }

    /// Adjust visual hierarchy with custom font scale
    pub fn with_font_scale(mut self, scale: FontScale) -> Self {
        self.font_scale = scale;
        self
    }

    /// Use compact font scaling
    pub fn with_compact_fonts(mut self) -> Self {
        self.font_scale = FontScale::compact();
        self
    }

    /// Use dramatic font scaling
    pub fn with_dramatic_fonts(mut self) -> Self {
        self.font_scale = FontScale::dramatic();
        self
    }

    /// Override title font family
    pub fn with_title_font_family(mut self, font: impl Into<String>) -> Self {
        self.title.title_font_family = Some(font.into());
        self
    }

    /// Override subtitle font family
    pub fn with_subtitle_font_family(mut self, font: impl Into<String>) -> Self {
        self.title.subtitle_font_family = Some(font.into());
        self
    }

    /// Override axis title font family
    pub fn with_axis_title_font_family(mut self, font: impl Into<String>) -> Self {
        self.axis.title_font_family = Some(font.into());
        self
    }

    /// Override axis label font family
    pub fn with_axis_label_font_family(mut self, font: impl Into<String>) -> Self {
        self.axis.label_font_family = Some(font.into());
        self
    }

    /// Override legend title font family
    pub fn with_legend_title_font_family(mut self, font: impl Into<String>) -> Self {
        self.legend.title_font_family = Some(font.into());
        self
    }

    /// Override legend label font family
    pub fn with_legend_label_font_family(mut self, font: impl Into<String>) -> Self {
        self.legend.label_font_family = Some(font.into());
        self
    }

    /// Override legend tick font family
    pub fn with_legend_tick_font_family(mut self, font: impl Into<String>) -> Self {
        self.legend.tick_font_family = Some(font.into());
        self
    }

    /// Override text mark font family default
    pub fn with_text_mark_font_family(mut self, font: impl Into<String>) -> Self {
        use datafusion_common::ScalarValue;
        self.mark_defaults =
            self.mark_defaults
                .with_default("text", "font", ScalarValue::Utf8(Some(font.into())));
        self
    }
}
