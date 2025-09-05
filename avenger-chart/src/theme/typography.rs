//! Typography settings for text elements

/// Font weight specification
#[derive(Clone, Debug)]
pub enum FontWeight {
    /// Numeric weight (100-900)
    Number(f32),
    /// Named weight
    Named(NamedFontWeight),
}

/// Named font weights
#[derive(Clone, Debug)]
pub enum NamedFontWeight {
    Thin,
    ExtraLight,
    Light,
    Normal,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Black,
}

impl FontWeight {
    pub fn to_number(&self) -> f32 {
        match self {
            FontWeight::Number(n) => *n,
            FontWeight::Named(named) => match named {
                NamedFontWeight::Thin => 100.0,
                NamedFontWeight::ExtraLight => 200.0,
                NamedFontWeight::Light => 300.0,
                NamedFontWeight::Normal => 400.0,
                NamedFontWeight::Medium => 500.0,
                NamedFontWeight::SemiBold => 600.0,
                NamedFontWeight::Bold => 700.0,
                NamedFontWeight::ExtraBold => 800.0,
                NamedFontWeight::Black => 900.0,
            },
        }
    }
}

/// Typography configuration for all text elements
#[derive(Clone, Debug)]
pub struct Typography {
    // Font families
    /// Default font for all text
    pub default_font: String,

    /// Font for titles (None means use default_font)
    pub title_font: Option<String>,

    /// Font for axis labels (None means use default_font)
    pub axis_label_font: Option<String>,

    /// Font for legend text (None means use default_font)
    pub legend_font: Option<String>,

    // Font sizes
    /// Title font size
    pub title_size: f32,

    /// Subtitle font size
    pub subtitle_size: f32,

    /// Axis label font size
    pub axis_label_size: f32,

    /// Axis tick label font size
    pub axis_tick_size: f32,

    /// Legend title font size
    pub legend_title_size: f32,

    /// Legend item font size
    pub legend_item_size: f32,

    // Font weights
    /// Title font weight
    pub title_weight: FontWeight,

    /// Subtitle font weight
    pub subtitle_weight: FontWeight,

    /// Axis label font weight
    pub axis_label_weight: FontWeight,

    /// Default font weight
    pub default_weight: FontWeight,

    // Colors
    /// Title text color
    pub title_color: String,

    /// Subtitle text color
    pub subtitle_color: String,

    /// Default text color
    pub default_color: String,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            // Font families
            default_font: "Atkinson Hyperlegible Next".to_string(),
            title_font: None,
            axis_label_font: None,
            legend_font: None,

            // Font sizes
            title_size: 18.0,
            subtitle_size: 14.0,
            axis_label_size: 12.0,
            axis_tick_size: 10.0,
            legend_title_size: 12.0,
            legend_item_size: 10.0,

            // Font weights
            title_weight: FontWeight::Named(NamedFontWeight::Medium),
            subtitle_weight: FontWeight::Number(200.0),
            axis_label_weight: FontWeight::Named(NamedFontWeight::Normal),
            default_weight: FontWeight::Named(NamedFontWeight::Normal),

            // Colors
            title_color: "#1a1a1a".to_string(),
            subtitle_color: "#4a4a4a".to_string(),
            default_color: "#000000".to_string(),
        }
    }
}

impl Typography {
    /// Get the effective font for titles
    pub fn title_font(&self) -> &str {
        self.title_font.as_deref().unwrap_or(&self.default_font)
    }

    /// Get the effective font for axis labels
    pub fn axis_label_font(&self) -> &str {
        self.axis_label_font
            .as_deref()
            .unwrap_or(&self.default_font)
    }

    /// Get the effective font for legend text
    pub fn legend_font(&self) -> &str {
        self.legend_font.as_deref().unwrap_or(&self.default_font)
    }

    /// Get title font weight as a number
    pub fn title_font_weight(&self) -> f32 {
        self.title_weight.to_number()
    }

    /// Get subtitle font weight as a number
    pub fn subtitle_font_weight(&self) -> f32 {
        self.subtitle_weight.to_number()
    }
}
