//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts, including colors, fonts, shapes, and other styling properties.

mod colors;
mod dashes;
mod marks;
mod presets;
mod shapes;
mod typography;

pub use colors::ColorPalettes;
pub use dashes::DashPatterns;
pub use marks::MarkDefaults;
pub use shapes::ShapeSequence;
pub use typography::{FontWeight, NamedFontWeight, Typography};

/// Complete theme configuration for chart styling
#[derive(Clone, Debug)]
pub struct Theme {
    /// Typography settings for text elements
    pub typography: Typography,

    /// Color palettes for different scale types
    pub colors: ColorPalettes,

    /// Shape sequences for categorical shape encoding
    pub shapes: ShapeSequence,

    /// Dash patterns for line styling
    pub dashes: DashPatterns,

    /// Default values for mark properties
    pub mark_defaults: MarkDefaults,

    /// Layout and spacing defaults
    pub layout: LayoutDefaults,

    /// Axis and grid styling
    pub axis: AxisDefaults,

    /// Legend styling defaults
    pub legend: LegendDefaults,

    /// Background colors for plot and canvas
    pub background: BackgroundDefaults,
}

/// Layout and spacing configuration
#[derive(Clone, Debug)]
pub struct LayoutDefaults {
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

/// Axis and grid styling
#[derive(Clone, Debug)]
pub struct AxisDefaults {
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
}

/// Legend styling configuration
#[derive(Clone, Debug)]
pub struct LegendDefaults {
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
}

/// Background color configuration
#[derive(Clone, Debug)]
pub struct BackgroundDefaults {
    /// Plot area background (None for transparent)
    pub plot_background: Option<String>,

    /// Canvas background (None for transparent)
    pub canvas_background: Option<String>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            typography: Typography::default(),
            colors: ColorPalettes::default(),
            shapes: ShapeSequence::default(),
            dashes: DashPatterns::default(),
            mark_defaults: MarkDefaults::default(),
            layout: LayoutDefaults::default(),
            axis: AxisDefaults::default(),
            legend: LegendDefaults::default(),
            background: BackgroundDefaults::default(),
        }
    }
}

impl Default for LayoutDefaults {
    fn default() -> Self {
        Self {
            padding: Padding::uniform(5.0),
            legend_spacing: 10.0,
            facet_spacing: (10.0, 10.0),
        }
    }
}

impl Default for AxisDefaults {
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
        }
    }
}

impl Default for LegendDefaults {
    fn default() -> Self {
        Self {
            background_fill: None,   // No background by default
            background_stroke: None, // No stroke by default
            background_padding: 8.0,
            background_corner_radius: 0.0, // No radius by default
            item_spacing: 10.0,
            symbol_size: 100.0,
            label_padding: 5.0,
        }
    }
}

impl Default for BackgroundDefaults {
    fn default() -> Self {
        Self {
            plot_background: None,
            canvas_background: None,
        }
    }
}

impl Theme {
    /// Builder method to set color palettes
    pub fn with_colors(mut self, colors: ColorPalettes) -> Self {
        self.colors = colors;
        self
    }

    /// Builder method to set typography
    pub fn with_typography(mut self, typography: Typography) -> Self {
        self.typography = typography;
        self
    }

    /// Convenience method to set the default font family
    pub fn with_font(mut self, font: &str) -> Self {
        self.typography.default_font = font.to_string();
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
}
