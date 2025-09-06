//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts, including colors, fonts, shapes, and other styling properties.

mod colors;
mod dashes;
mod marks;
mod presets;
mod shapes;

pub use colors::ColorPalettes;
pub use dashes::DashPatterns;
pub use marks::MarkDefaults;
pub use shapes::ShapeSequence;

/// Complete theme configuration for chart styling
#[derive(Clone, Debug)]
pub struct Theme {
    /// Default font family (others inherit from this if not specified)
    pub default_font: String,
    
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
    
    /// Label font family
    pub label_font_family: String,
    
    /// Label font size
    pub label_font_size: f32,
    
    /// Label font weight
    pub label_font_weight: f32,
    
    // Axis title typography
    /// Title text color
    pub title_color: String,
    
    /// Title font family
    pub title_font_family: String,
    
    /// Title font size
    pub title_font_size: f32,
    
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
    
    /// Title font family
    pub title_font_family: String,
    
    /// Title font size
    pub title_font_size: f32,
    
    /// Title font weight
    pub title_font_weight: f32,
    
    // Legend label typography (for discrete legends)
    /// Label text color
    pub label_color: String,
    
    /// Label font family
    pub label_font_family: String,
    
    /// Label font size
    pub label_font_size: f32,
    
    /// Label font weight
    pub label_font_weight: f32,
    
    // Legend tick label typography (for continuous/colorbar legends)
    /// Tick label text color
    pub tick_color: String,
    
    /// Tick label font family
    pub tick_font_family: String,
    
    /// Tick label font size
    pub tick_font_size: f32,
    
    /// Tick label font weight
    pub tick_font_weight: f32,
}

/// Title theme configuration for plot titles and subtitles
#[derive(Clone, Debug)]
pub struct TitleTheme {
    // Main title typography
    /// Title text color
    pub title_color: String,
    
    /// Title font family
    pub title_font_family: String,
    
    /// Title font size
    pub title_font_size: f32,
    
    /// Title font weight
    pub title_font_weight: f32,
    
    // Subtitle typography
    /// Subtitle text color
    pub subtitle_color: String,
    
    /// Subtitle font family
    pub subtitle_font_family: String,
    
    /// Subtitle font size
    pub subtitle_font_size: f32,
    
    /// Subtitle font weight
    pub subtitle_font_weight: f32,
}

/// Background color configuration
#[derive(Clone, Debug)]
pub struct BackgroundTheme {
    /// Plot area background (None for transparent)
    pub plot_background: Option<String>,

    /// Canvas background (None for transparent)
    pub canvas_background: Option<String>,
}

impl Theme {
    /// Set the font family for all text elements in the theme
    pub fn set_font_family(&mut self, font: &str) {
        self.default_font = font.to_string();
        
        // Update title fonts
        self.title.title_font_family = font.to_string();
        self.title.subtitle_font_family = font.to_string();
        
        // Update axis fonts
        self.axis.label_font_family = font.to_string();
        self.axis.title_font_family = font.to_string();
        
        // Update legend fonts
        self.legend.title_font_family = font.to_string();
        self.legend.label_font_family = font.to_string();
        self.legend.tick_font_family = font.to_string();
    }

    /// Builder method to set font family for all text elements
    pub fn with_font_family(mut self, font: &str) -> Self {
        self.set_font_family(font);
        self
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            default_font: "Atkinson Hyperlegible Next".to_string(),
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
            label_font_family: "Atkinson Hyperlegible Next".to_string(),
            label_font_size: 10.0,
            label_font_weight: 300.0,
            // Title typography
            title_color: "#2a2a2a".to_string(),
            title_font_family: "Atkinson Hyperlegible Next".to_string(),
            title_font_size: 12.0,
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
            title_font_family: "Atkinson Hyperlegible Next".to_string(),
            title_font_size: 12.0,
            title_font_weight: 400.0,
            // Label typography (for discrete legends)
            label_color: "#3C3C3C".to_string(),
            label_font_family: "Atkinson Hyperlegible Next".to_string(),
            label_font_size: 11.0,
            label_font_weight: 300.0,
            // Tick label typography (for continuous/colorbar legends)
            tick_color: "#5a5a5a".to_string(), // Same as axis labels
            tick_font_family: "Atkinson Hyperlegible Next".to_string(),
            tick_font_size: 10.0,
            tick_font_weight: 300.0,
        }
    }
}

impl Default for TitleTheme {
    fn default() -> Self {
        Self {
            // Title typography
            title_color: "#1a1a1a".to_string(),
            title_font_family: "Atkinson Hyperlegible Next".to_string(),
            title_font_size: 18.0,
            title_font_weight: 500.0, // Medium weight
            // Subtitle typography
            subtitle_color: "#4a4a4a".to_string(),
            subtitle_font_family: "Atkinson Hyperlegible Next".to_string(),
            subtitle_font_size: 14.0,
            subtitle_font_weight: 200.0, // Light weight
        }
    }
}

impl Default for BackgroundTheme {
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
}
