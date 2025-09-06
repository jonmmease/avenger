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
    
    /// Axis title text color
    pub axis_title_color: String,
    
    /// Axis label text color
    pub axis_label_color: String,

    /// Legend title text color
    pub legend_title_color: String,

    /// Legend label text color
    pub legend_label_color: String,

    /// Default text color
    pub default_color: String,
    
    // Additional axis typography
    /// Axis title font family
    pub axis_title_font_family: String,
    
    /// Axis label font family (for tick labels)
    pub axis_label_font_family: String,
    
    /// Axis title font size
    pub axis_title_size: f32,
    
    /// Axis title font weight
    pub axis_title_weight: f32,
    
    /// Legend title font family
    pub legend_title_font_family: String,
    
    /// Legend label font family
    pub legend_label_font_family: String,
    
    /// Legend title font weight
    pub legend_title_weight: f32,
    
    /// Legend label font weight
    pub legend_label_weight: f32,
    
    /// Legend tick label font family (for colorbar legends)
    pub legend_tick_font_family: String,
    
    /// Legend tick label font size (for colorbar legends)
    pub legend_tick_font_size: f32,
    
    /// Legend tick label font weight (for colorbar legends)
    pub legend_tick_font_weight: f32,
    
    /// Legend tick label color (for colorbar legends)
    pub legend_tick_color: String,
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
            legend_item_size: 11.0,

            // Font weights
            title_weight: FontWeight::Named(NamedFontWeight::Medium),
            subtitle_weight: FontWeight::Number(200.0),
            axis_label_weight: FontWeight::Number(300.0),  // Light weight for axis labels
            default_weight: FontWeight::Named(NamedFontWeight::Normal),

            // Colors
            title_color: "#1a1a1a".to_string(),
            subtitle_color: "#4a4a4a".to_string(),
            axis_title_color: "#2a2a2a".to_string(),  // Axis title color
            axis_label_color: "#5a5a5a".to_string(),  // Same as we use for axis tick labels
            legend_title_color: "#2C2C2C".to_string(), // Same as hardcoded default
            legend_label_color: "#3C3C3C".to_string(), // Same as hardcoded default
            default_color: "#000000".to_string(),
            
            // Additional font families and sizes
            axis_title_font_family: "Atkinson Hyperlegible Next".to_string(),
            axis_label_font_family: "Atkinson Hyperlegible Next".to_string(),
            axis_title_size: 12.0,  // Axis title font size (matching old default)
            axis_title_weight: 400.0,  // Normal weight for axis titles
            legend_title_font_family: "Atkinson Hyperlegible Next".to_string(),
            legend_label_font_family: "Atkinson Hyperlegible Next".to_string(),
            
            // Additional font weights
            legend_title_weight: 400.0,
            legend_label_weight: 300.0,
            
            // Legend tick label typography (matches axis labels)
            legend_tick_font_family: "Atkinson Hyperlegible Next".to_string(),
            legend_tick_font_size: 10.0,  // Same as axis labels
            legend_tick_font_weight: 300.0,  // Same as axis labels
            legend_tick_color: "#5a5a5a".to_string(),  // Same as axis labels (#5A5A5A)
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
