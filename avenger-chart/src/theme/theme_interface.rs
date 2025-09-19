//! Theme trait for flexible theme implementations
//!
//! This trait defines a query-based interface for themes that can be implemented
//! by different backends (struct-based, CSS-based, etc.)

use std::sync::Arc;

/// Context for theme queries, providing information about the element being styled
#[derive(Debug, Clone)]
pub struct ThemeContext {
    /// Element type (e.g., "axis", "legend", "mark", "title")
    pub element_type: String,

    /// Optional subtype for the element:
    /// - For marks: "symbol", "line", "rect", "text", etc.
    /// - For axes: "x", "y", "top", "bottom", "left", "right"
    /// - For legends: "discrete", "continuous", "symbol", "gradient"
    pub subtype: Option<String>,

    /// Optional classes/tags for the element
    pub classes: Vec<String>,

    /// Optional unique identifier
    pub id: Option<String>,

    /// Parent context for hierarchical CSS selectors
    pub parent: Option<Arc<ThemeContext>>,

    // Position info for CSS pseudo-class selectors (e.g., third mark, second legend)
    /// Whether this is the first child of its parent
    pub is_first_child: bool,

    /// Whether this is the last child of its parent
    pub is_last_child: bool,

    /// Index of this child (0-based)
    pub child_index: usize,
}

impl ThemeContext {
    /// Create a new context for a specific element
    pub fn new(element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            parent: None,
            is_first_child: false,
            is_last_child: false,
            child_index: 0,
        }
    }

    /// Create a child context
    pub fn child(&self, element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            parent: Some(Arc::new(self.clone())),
            is_first_child: false,
            is_last_child: false,
            child_index: 0,
        }
    }

    /// Set the subtype for the element
    pub fn with_subtype(mut self, subtype: impl Into<String>) -> Self {
        self.subtype = Some(subtype.into());
        self
    }

    /// Deprecated: Use with_subtype() instead
    pub fn with_mark(mut self, mark: impl Into<String>) -> Self {
        self.subtype = Some(mark.into());
        self
    }

    /// Add a channel to the context (adds as a class)
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.classes.push(channel.into());
        self
    }

    /// Add a class to the context
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.classes.push(class.into());
        self
    }

    /// Set the ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set child position info for CSS pseudo-class selectors
    pub fn with_child_info(mut self, index: usize, is_first: bool, is_last: bool) -> Self {
        self.child_index = index;
        self.is_first_child = is_first;
        self.is_last_child = is_last;
        self
    }
}

/// Property names that can be queried from themes
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThemeProperty {
    // Font properties
    FontFamily,
    FontSize,
    FontWeight,

    // Color properties
    Color,
    BackgroundColor,
    FillColor,
    StrokeColor,
    GridColor,

    // Size properties
    StrokeWidth,
    Size,
    Padding,
    Spacing,

    // Opacity
    Opacity,
    GridOpacity,

    // Other properties
    CornerRadius,
    LabelAngle,

    // Custom property
    Custom(String),
}

/// Value types that themes can return
#[derive(Debug, Clone, PartialEq)]
pub enum ThemeValue {
    /// String value (keywords, identifiers, font families, etc.)
    String(String),

    /// Keyword value (CSS keywords like "bold", "center", etc.)
    Keyword(String),

    /// Floating point value (sizes, opacities, angles, weights)
    Float(f32),

    /// Double precision float (for higher precision values)
    Double(f64),

    /// Integer value
    Integer(i32),

    /// Boolean value
    Boolean(bool),

    /// Length with unit
    Length(f64, LengthUnit),

    /// Percentage value
    Percentage(f64),

    /// Color value
    Color(Rgba),

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

/// RGBA color representation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

/// Length units
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Percent,
    Pt,
}

impl ThemeValue {
    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ThemeValue::String(s) | ThemeValue::Keyword(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f32> {
        match self {
            ThemeValue::Float(f) => Some(*f),
            ThemeValue::Double(d) => Some(*d as f32),
            ThemeValue::Integer(i) => Some(*i as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Pt) => Some((*n * 1.333) as f32),
            ThemeValue::Percentage(p) => Some(*p as f32),
            _ => None,
        }
    }

    /// Try to get as f64
    pub fn as_double(&self) -> Option<f64> {
        match self {
            ThemeValue::Double(d) => Some(*d),
            ThemeValue::Float(f) => Some(*f as f64),
            ThemeValue::Integer(i) => Some(*i as f64),
            ThemeValue::Length(n, _) => Some(*n),
            ThemeValue::Percentage(p) => Some(*p),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i32> {
        match self {
            ThemeValue::Integer(i) => Some(*i),
            ThemeValue::Float(f) => Some(*f as i32),
            ThemeValue::Double(d) => Some(*d as i32),
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

    /// Try to get as color
    pub fn as_color(&self) -> Option<Rgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),
            ThemeValue::String(s) | ThemeValue::Keyword(s) => {
                // Try to parse hex color or named color
                if s.starts_with('#') {
                    parse_hex_color(s)
                } else {
                    parse_named_color(s)
                }
            }
            _ => None,
        }
    }

    /// Convert to string value (for display/serialization)
    pub fn to_string_value(&self) -> Option<String> {
        match self {
            ThemeValue::String(s) | ThemeValue::Keyword(s) => Some(s.clone()),
            ThemeValue::Color(rgba) => {
                if rgba.alpha < 255 {
                    Some(format!(
                        "#{:02x}{:02x}{:02x}{:02x}",
                        rgba.red, rgba.green, rgba.blue, rgba.alpha
                    ))
                } else {
                    Some(format!(
                        "#{:02x}{:02x}{:02x}",
                        rgba.red, rgba.green, rgba.blue
                    ))
                }
            }
            _ => None,
        }
    }

    /// Check if value is None
    pub fn is_none(&self) -> bool {
        matches!(self, ThemeValue::None)
    }
}

/// Parse a hex color
pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
    let hex = hex.trim_start_matches('#');

    let (r, g, b) = match hex.len() {
        3 => {
            // Short form: #RGB
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            (r * 17, g * 17, b * 17)
        }
        6 => {
            // Long form: #RRGGBB
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };

    Some(Rgba {
        red: r,
        green: g,
        blue: b,
        alpha: 255,
    })
}

/// Parse a named color
pub fn parse_named_color(name: &str) -> Option<Rgba> {
    let color = match name.to_lowercase().as_str() {
        // Basic colors
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" => (0, 255, 255),
        "magenta" => (255, 0, 255),

        // Grays
        "gray" | "grey" => (128, 128, 128),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "dimgray" | "dimgrey" => (105, 105, 105),

        // Extended colors
        "orange" => (255, 165, 0),
        "purple" => (128, 0, 128),
        "brown" => (165, 42, 42),
        "pink" => (255, 192, 203),
        "lime" => (0, 255, 0),
        "navy" => (0, 0, 128),
        "teal" => (0, 128, 128),
        "olive" => (128, 128, 0),
        "maroon" => (128, 0, 0),

        // Common web colors
        "steelblue" => (70, 130, 180),
        "cornflowerblue" => (100, 149, 237),
        "dodgerblue" => (30, 144, 255),
        "lightblue" => (173, 216, 230),
        "skyblue" => (135, 206, 235),

        _ => return None,
    };

    Some(Rgba {
        red: color.0,
        green: color.1,
        blue: color.2,
        alpha: 255,
    })
}

/// Main theme trait that can be implemented by different backends
pub trait Theme: Send + Sync {
    /// Query a theme property for a given context
    fn query(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue;

    /// Query with fallback value if property is not found
    fn query_or(
        &self,
        context: &ThemeContext,
        property: &ThemeProperty,
        fallback: ThemeValue,
    ) -> ThemeValue {
        let value = self.query(context, property);
        match value {
            ThemeValue::None => fallback,
            _ => value,
        }
    }

    /// Get font family for a context
    fn font_family(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::FontFamily)
            .as_string()
            .unwrap_or("Atkinson Hyperlegible Next")
            .to_string()
    }

    /// Get font size for a context
    fn font_size(&self, context: &ThemeContext) -> f32 {
        self.query(context, &ThemeProperty::FontSize)
            .as_float()
            .unwrap_or(12.0)
    }

    /// Get font weight for a context
    fn font_weight(&self, context: &ThemeContext) -> f32 {
        self.query(context, &ThemeProperty::FontWeight)
            .as_float()
            .unwrap_or(400.0)
    }

    /// Get color for a context
    fn color(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::Color)
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get fill color for a context
    fn fill_color(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::FillColor)
            .to_string_value()
            .unwrap_or_else(|| "#4682b4".to_string())
    }

    /// Get stroke color for a context
    fn stroke_color(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::StrokeColor)
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get stroke width for a context
    fn stroke_width(&self, context: &ThemeContext) -> f32 {
        self.query(context, &ThemeProperty::StrokeWidth)
            .as_float()
            .unwrap_or(1.0)
    }

    /// Get opacity for a context
    fn opacity(&self, context: &ThemeContext) -> f32 {
        self.query(context, &ThemeProperty::Opacity)
            .as_float()
            .unwrap_or(1.0)
    }

    /// Clone the theme into a boxed trait object
    fn clone_box(&self) -> Box<dyn Theme>;

    // Additional methods for accessing specific theme properties

    /// Get canvas background color
    fn canvas_background(&self) -> Option<String> {
        let ctx = ThemeContext::new("canvas");
        match self.query(&ctx, &ThemeProperty::BackgroundColor) {
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
        match self.query(&ctx, &ThemeProperty::FillColor) {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background stroke
    fn legend_background_stroke(&self) -> Option<String> {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        match self.query(&ctx, &ThemeProperty::StrokeColor) {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background padding
    fn legend_background_padding(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        self.query(&ctx, &ThemeProperty::Padding)
            .as_float()
            .unwrap_or(5.0)
    }

    /// Get legend background corner radius
    fn legend_background_corner_radius(&self) -> f32 {
        let legend_ctx = ThemeContext::new("legend");
        let ctx = legend_ctx.child("background");
        self.query(&ctx, &ThemeProperty::CornerRadius)
            .as_float()
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
        self.query(&ctx, &ThemeProperty::StrokeColor)
            .to_string_value()
            .unwrap_or_else(|| "#000000".to_string())
    }

    /// Get axis tick color
    fn axis_tick_color(&self) -> String {
        let axis_ctx = ThemeContext::new("axis");
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, &ThemeProperty::StrokeColor)
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
        self.query(&ctx, &ThemeProperty::GridOpacity)
            .as_float()
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
        self.query(&ctx, &ThemeProperty::Size)
            .as_float()
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

/// Helper trait for building contexts fluently
pub trait ContextBuilder {
    fn axis_context(subtype: &str) -> ThemeContext {
        ThemeContext::new("axis").with_subtype(subtype)
    }

    fn legend_context(subtype: &str) -> ThemeContext {
        ThemeContext::new("legend").with_subtype(subtype)
    }

    fn mark_context(mark_type: &str) -> ThemeContext {
        ThemeContext::new("mark").with_subtype(mark_type)
    }

    fn title_context() -> ThemeContext {
        ThemeContext::new("title")
    }

    fn subtitle_context() -> ThemeContext {
        ThemeContext::new("subtitle")
    }
}

// Blanket implementation
impl ContextBuilder for ThemeContext {}

// Implement Theme for Arc<dyn Theme> to allow passing around shared references
impl Theme for std::sync::Arc<dyn Theme> {
    fn query(&self, context: &ThemeContext, property: &ThemeProperty) -> ThemeValue {
        (**self).query(context, property)
    }

    fn clone_box(&self) -> Box<dyn Theme> {
        (**self).clone_box()
    }
}
