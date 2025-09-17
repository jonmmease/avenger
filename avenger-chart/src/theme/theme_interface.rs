//! Theme trait for flexible theme implementations
//!
//! This trait defines a query-based interface for themes that can be implemented
//! by different backends (struct-based, CSS-based, etc.)

/// Context for theme queries, providing information about the element being styled
#[derive(Debug, Clone)]
pub struct ThemeContext {
    /// Element type (e.g., "axis", "legend", "mark", "title")
    pub element_type: String,

    /// Optional element subtype (e.g., "x" or "y" for axis, "discrete" or "continuous" for legend)
    pub element_subtype: Option<String>,

    /// Optional mark type (e.g., "symbol", "line", "rect")
    pub mark_type: Option<String>,

    /// Optional channel being styled (e.g., "fill", "stroke", "size")
    pub channel: Option<String>,

    /// Optional classes/tags for the element
    pub classes: Vec<String>,

    /// Optional unique identifier
    pub id: Option<String>,

    /// Parent context (for inheritance)
    pub parent: Option<Box<ThemeContext>>,
}

impl ThemeContext {
    /// Create a new context for a specific element
    pub fn new(element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            element_subtype: None,
            mark_type: None,
            channel: None,
            classes: Vec::new(),
            id: None,
            parent: None,
        }
    }

    /// Add a subtype to the context
    pub fn with_subtype(mut self, subtype: impl Into<String>) -> Self {
        self.element_subtype = Some(subtype.into());
        self
    }

    /// Add a mark type to the context
    pub fn with_mark(mut self, mark: impl Into<String>) -> Self {
        self.mark_type = Some(mark.into());
        self
    }

    /// Add a channel to the context
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
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

    /// Set parent context for inheritance
    pub fn with_parent(mut self, parent: ThemeContext) -> Self {
        self.parent = Some(Box::new(parent));
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
#[derive(Debug, Clone)]
pub enum ThemeValue {
    /// String value (colors, font families, etc.)
    String(String),

    /// Floating point value (sizes, opacities, angles, weights)
    Float(f32),

    /// Integer value
    Integer(i32),

    /// Boolean value
    Boolean(bool),

    /// Multiple values (for padding, margin, etc.)
    List(Vec<ThemeValue>),

    /// No value (property not set)
    None,
}

impl ThemeValue {
    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f32> {
        match self {
            ThemeValue::Float(f) => Some(*f),
            ThemeValue::Integer(i) => Some(*i as f32),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i32> {
        match self {
            ThemeValue::Integer(i) => Some(*i),
            ThemeValue::Float(f) => Some(*f as i32),
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
            .as_string()
            .unwrap_or("#000000")
            .to_string()
    }

    /// Get fill color for a context
    fn fill_color(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::FillColor)
            .as_string()
            .unwrap_or("#4682b4")
            .to_string()
    }

    /// Get stroke color for a context
    fn stroke_color(&self, context: &ThemeContext) -> String {
        self.query(context, &ThemeProperty::StrokeColor)
            .as_string()
            .unwrap_or("#000000")
            .to_string()
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
        let ctx = ThemeContext::new("canvas").with_class("background");
        match self.query(&ctx, &ThemeProperty::BackgroundColor) {
            ThemeValue::String(s) => Some(s),
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
        let ctx = ThemeContext::new("legend").with_class("background");
        match self.query(&ctx, &ThemeProperty::FillColor) {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background stroke
    fn legend_background_stroke(&self) -> Option<String> {
        let ctx = ThemeContext::new("legend").with_class("background");
        match self.query(&ctx, &ThemeProperty::StrokeColor) {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get legend background padding
    fn legend_background_padding(&self) -> f32 {
        let ctx = ThemeContext::new("legend").with_class("background");
        self.query(&ctx, &ThemeProperty::Padding)
            .as_float()
            .unwrap_or(5.0)
    }

    /// Get legend background corner radius
    fn legend_background_corner_radius(&self) -> f32 {
        let ctx = ThemeContext::new("legend").with_class("background");
        self.query(&ctx, &ThemeProperty::CornerRadius)
            .as_float()
            .unwrap_or(5.0)
    }

    /// Get legend title color
    fn legend_title_color(&self) -> String {
        self.color(&ThemeContext::new("legend").with_class("title"))
    }

    /// Get legend label color
    fn legend_label_color(&self) -> String {
        self.color(&ThemeContext::new("legend").with_class("label"))
    }

    /// Get legend tick color
    fn legend_tick_color(&self) -> String {
        self.color(&ThemeContext::new("legend").with_class("tick"))
    }

    /// Get legend title font family
    fn legend_title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("title"))
    }

    /// Get legend label font family
    fn legend_label_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("label"))
    }

    /// Get legend tick font family
    fn legend_tick_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("legend").with_class("tick"))
    }

    /// Get legend title font size
    fn legend_title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("title"))
    }

    /// Get legend label font size
    fn legend_label_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("label"))
    }

    /// Get legend tick font size
    fn legend_tick_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("legend").with_class("tick"))
    }

    /// Get legend title font weight
    fn legend_title_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("legend").with_class("title"))
    }

    /// Get legend label font weight
    fn legend_label_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("legend").with_class("label"))
    }

    /// Get legend tick font weight
    fn legend_tick_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("legend").with_class("tick"))
    }

    // Title-specific methods

    /// Get title color
    fn title_color(&self) -> String {
        self.color(&ThemeContext::new("title"))
    }

    /// Get subtitle color
    fn subtitle_color(&self) -> String {
        self.color(&ThemeContext::new("subtitle"))
    }

    /// Get title font family
    fn title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("title"))
    }

    /// Get subtitle font family
    fn subtitle_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("subtitle"))
    }

    /// Get title font size
    fn title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("title"))
    }

    /// Get subtitle font size
    fn subtitle_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("subtitle"))
    }

    /// Get title font weight
    fn title_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("title"))
    }

    /// Get subtitle font weight
    fn subtitle_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("subtitle"))
    }

    // Axis-specific methods

    /// Get axis domain color
    fn axis_domain_color(&self) -> String {
        let ctx = ThemeContext::new("axis").with_class("domain");
        self.color(&ctx)
    }

    /// Get axis tick color
    fn axis_tick_color(&self) -> String {
        let ctx = ThemeContext::new("axis").with_class("tick");
        self.color(&ctx)
    }

    /// Get axis grid color
    fn axis_grid_color(&self) -> String {
        let ctx = ThemeContext::new("axis").with_class("grid");
        self.query(&ctx, &ThemeProperty::GridColor)
            .as_string()
            .unwrap_or("#d0d0d0")
            .to_string()
    }

    /// Get axis grid opacity
    fn axis_grid_opacity(&self) -> f32 {
        let ctx = ThemeContext::new("axis").with_class("grid");
        self.query(&ctx, &ThemeProperty::GridOpacity)
            .as_float()
            .unwrap_or(0.5)
    }

    /// Get axis grid width
    fn axis_grid_width(&self) -> f32 {
        let ctx = ThemeContext::new("axis").with_class("grid");
        self.stroke_width(&ctx)
    }

    /// Get axis label color
    fn axis_label_color(&self) -> String {
        self.color(&ThemeContext::new("axis").with_class("label"))
    }

    /// Get axis title color
    fn axis_title_color(&self) -> String {
        self.color(&ThemeContext::new("axis").with_class("title"))
    }

    /// Get axis tick length
    fn axis_tick_length(&self) -> f32 {
        let ctx = ThemeContext::new("axis").with_class("tick");
        self.query(&ctx, &ThemeProperty::Size)
            .as_float()
            .unwrap_or(5.0)
    }

    /// Get axis label font size
    fn axis_label_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("axis").with_class("label"))
    }

    /// Get axis label font weight
    fn axis_label_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("axis").with_class("label"))
    }

    /// Get axis title font size
    fn axis_title_font_size(&self) -> f32 {
        self.font_size(&ThemeContext::new("axis").with_class("title"))
    }

    /// Get axis title font weight
    fn axis_title_font_weight(&self) -> f32 {
        self.font_weight(&ThemeContext::new("axis").with_class("title"))
    }

    /// Get axis label font family
    fn axis_label_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("axis").with_class("label"))
    }

    /// Get axis title font family
    fn axis_title_font_family(&self) -> String {
        self.font_family(&ThemeContext::new("axis").with_class("title"))
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

    /// Get default color range for a scale type
    fn get_color_range(
        &self,
        scale_type: &str,
        _domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        match scale_type {
            "ordinal" => {
                // Use theme categorical colors
                let colors = self.categorical_colors();
                let scalars: Vec<ScalarValue> = colors
                    .into_iter()
                    .map(|c| ScalarValue::Utf8(Some(c)))
                    .collect();
                ScaleRange::new_discrete(scalars)
            }
            _ => {
                // For continuous scales, use a default gradient
                // This is a simple implementation - actual would be more sophisticated
                ScaleRange::new_interval(
                    datafusion::logical_expr::lit("#4682b4"),
                    datafusion::logical_expr::lit("#ff7f0e"),
                )
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
        ThemeContext::new("mark").with_mark(mark_type)
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
