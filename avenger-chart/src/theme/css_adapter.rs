//! Adapter to use CSS themes with the existing trait-based system

use super::{
    Theme as ThemeTrait, ThemeContext as TraitContext, ThemeProperty, ThemeValue as TraitValue,
};
use crate::theme::css::props;
use crate::theme::css::{Theme as CssTheme, ThemeContext as CssContext, ThemeValue as CssValue};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

/// Adapter that implements the Theme trait using a CSS theme
#[derive(Clone, Debug)]
pub struct CssThemeAdapter {
    css_theme: CssTheme,

    // Cache commonly used values to match existing behavior
    categorical_colors: Vec<String>,
    shape_names: Vec<String>,
    dash_names: Vec<String>,
}

impl CssThemeAdapter {
    /// Create a new adapter with the default CSS theme
    pub fn default() -> Self {
        let css_theme = CssTheme::light().unwrap_or_else(|_| {
            // Fallback to empty theme if parsing fails
            CssTheme {
                rules: vec![],
                variables: Default::default(),
                inherited_properties: Default::default(),
                source_map: None,
                base_font_size: 12.0,
                chart_width: None,
                chart_height: None,
            }
        });

        Self {
            css_theme,
            categorical_colors: CssTheme::categorical_colors(),
            shape_names: CssTheme::shape_names()
                .iter()
                .map(|&s| s.to_string())
                .collect(),
            dash_names: CssTheme::dash_patterns()
                .iter()
                .map(|&s| s.to_string())
                .collect(),
        }
    }

    /// Create a new adapter with the dark CSS theme
    pub fn dark() -> Self {
        let css_theme = CssTheme::dark().unwrap_or_else(|_| CssTheme {
            rules: vec![],
            variables: Default::default(),
            inherited_properties: Default::default(),
            source_map: None,
            base_font_size: 12.0,
            chart_width: None,
            chart_height: None,
        });

        Self {
            css_theme,
            // Dark theme uses different color order
            categorical_colors: vec![
                "#56B4E9".to_string(), // Sky blue
                "#E69F00".to_string(), // Orange
                "#009E73".to_string(), // Green
                "#F0E442".to_string(), // Yellow
                "#0072B2".to_string(), // Blue
                "#D55E00".to_string(), // Vermillion
                "#CC79A7".to_string(), // Reddish purple
                "#999999".to_string(), // Grey
            ],
            shape_names: CssTheme::shape_names()
                .iter()
                .map(|&s| s.to_string())
                .collect(),
            dash_names: CssTheme::dash_patterns()
                .iter()
                .map(|&s| s.to_string())
                .collect(),
        }
    }

    /// Convert trait context to CSS context
    fn convert_context(&self, context: &TraitContext) -> CssContext {
        let mut css_context = CssContext::new(&context.element_type);

        // Add classes
        for class in &context.classes {
            css_context = css_context.with_class(class);
        }

        // Add mark type if present
        if let Some(ref mark) = context.mark_type {
            css_context = css_context.with_mark(mark);
        }

        // Note: coord_type doesn't exist in TraitContext, skip it

        css_context
    }

    /// Convert CSS property name from trait property
    fn property_to_css(&self, property: &ThemeProperty) -> String {
        match property {
            ThemeProperty::FontFamily => props::FONT_FAMILY.to_string(),
            ThemeProperty::FontSize => props::FONT_SIZE.to_string(),
            ThemeProperty::FontWeight => props::FONT_WEIGHT.to_string(),
            ThemeProperty::Color => props::COLOR.to_string(),
            ThemeProperty::FillColor => props::FILL.to_string(),
            ThemeProperty::StrokeColor => props::STROKE.to_string(),
            ThemeProperty::StrokeWidth => props::STROKE_WIDTH.to_string(),
            ThemeProperty::BackgroundColor => props::BACKGROUND_COLOR.to_string(),
            ThemeProperty::GridColor => props::GRID_COLOR.to_string(),
            ThemeProperty::GridOpacity => props::GRID_OPACITY.to_string(),
            ThemeProperty::Size => props::SIZE.to_string(),
            ThemeProperty::Padding => "padding".to_string(),
            ThemeProperty::Spacing => "spacing".to_string(),
            ThemeProperty::Opacity => props::OPACITY.to_string(),
            ThemeProperty::CornerRadius => "corner-radius".to_string(),
            ThemeProperty::LabelAngle => props::LABEL_ANGLE.to_string(),
            ThemeProperty::Custom(name) => name.clone(),
        }
    }

    /// Convert CSS value to trait value
    fn convert_value(&self, css_value: CssValue) -> TraitValue {
        match css_value {
            CssValue::String(s) => TraitValue::String(s),
            CssValue::Keyword(k) => TraitValue::String(k),
            CssValue::Number(n) => TraitValue::Float(n as f32),
            CssValue::Dimension(n, _) => TraitValue::Float(n as f32),
            CssValue::Color(rgba) => {
                // Convert RGBA to hex string
                let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                TraitValue::String(hex)
            }
            CssValue::None => TraitValue::None,
            _ => TraitValue::None,
        }
    }
}

impl ThemeTrait for CssThemeAdapter {
    fn query(&self, context: &TraitContext, property: &ThemeProperty) -> TraitValue {
        let css_context = self.convert_context(context);
        let css_property = self.property_to_css(property);
        let css_value = self.css_theme.query(&css_context, &css_property);
        self.convert_value(css_value)
    }

    fn clone_box(&self) -> Box<dyn ThemeTrait> {
        Box::new(self.clone())
    }

    fn get_color_range(
        &self,
        scale_type: &str,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;

        match scale_type {
            "ordinal" => {
                // Use categorical colors
                let colors: Vec<ScalarValue> = match domain_cardinality {
                    Some(n) if n <= self.categorical_colors.len() => self
                        .categorical_colors
                        .iter()
                        .take(n)
                        .map(|c| ScalarValue::Utf8(Some(c.clone())))
                        .collect(),
                    _ => self
                        .categorical_colors
                        .iter()
                        .map(|c| ScalarValue::Utf8(Some(c.clone())))
                        .collect(),
                };
                ScaleRange::Discrete(colors)
            }
            _ => {
                // Default single color
                ScaleRange::Discrete(vec![ScalarValue::Utf8(Some("#4682b4".to_string()))])
            }
        }
    }

    fn get_shape_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;

        let shapes: Vec<ScalarValue> = match domain_cardinality {
            Some(n) if n <= self.shape_names.len() => self
                .shape_names
                .iter()
                .take(n)
                .map(|s| ScalarValue::Utf8(Some(s.clone())))
                .collect(),
            _ => self
                .shape_names
                .iter()
                .map(|s| ScalarValue::Utf8(Some(s.clone())))
                .collect(),
        };
        ScaleRange::new_discrete(shapes)
    }

    fn categorical_colors(&self) -> Vec<String> {
        self.categorical_colors.clone()
    }

    fn shape_names(&self) -> Vec<String> {
        self.shape_names.clone()
    }

    fn dash_names(&self) -> Vec<String> {
        self.dash_names.clone()
    }

    fn mark_defaults_map(&self) -> IndexMap<String, IndexMap<String, ScalarValue>> {
        let mut defaults = IndexMap::new();

        // Build mark defaults from CSS theme
        for mark_type in &["symbol", "rect", "line", "area", "text", "arc"] {
            let mut mark_defaults = IndexMap::new();
            let context = CssContext::new("mark").with_mark(*mark_type);

            // Query common mark properties
            if let Some(fill) = self.css_theme.get_string(&context, props::FILL) {
                mark_defaults.insert("fill".to_string(), ScalarValue::Utf8(Some(fill)));
            }
            if let Some(stroke) = self.css_theme.get_string(&context, props::STROKE) {
                mark_defaults.insert("stroke".to_string(), ScalarValue::Utf8(Some(stroke)));
            }
            if let Some(stroke_width) = self.css_theme.get_f32(&context, props::STROKE_WIDTH) {
                mark_defaults.insert(
                    "stroke_width".to_string(),
                    ScalarValue::Float32(Some(stroke_width)),
                );
            }
            if let Some(size) = self.css_theme.get_f32(&context, props::SIZE) {
                mark_defaults.insert("size".to_string(), ScalarValue::Float32(Some(size)));
            }

            if !mark_defaults.is_empty() {
                defaults.insert(mark_type.to_string(), mark_defaults);
            }
        }

        defaults
    }

    fn mark_default(&self, mark_type: &str, channel: &str) -> Option<ScalarValue> {
        let context = CssContext::new("mark").with_mark(mark_type);

        // Map channel to CSS property
        let css_property = match channel {
            "fill" => props::FILL,
            "stroke" => props::STROKE,
            "stroke_width" => props::STROKE_WIDTH,
            "size" => props::SIZE,
            "font" => props::FONT_FAMILY,
            "font_size" => props::FONT_SIZE,
            _ => return None,
        };

        match self.css_theme.query(&context, css_property) {
            CssValue::String(s) | CssValue::Keyword(s) => Some(ScalarValue::Utf8(Some(s))),
            CssValue::Number(n) | CssValue::Dimension(n, _) => {
                Some(ScalarValue::Float32(Some(n as f32)))
            }
            CssValue::Color(rgba) => {
                let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                Some(ScalarValue::Utf8(Some(hex)))
            }
            _ => None,
        }
    }

    fn mark_default_with_computed_fonts(
        &self,
        mark_type: &str,
        channel: &str,
        computed_font_size: f32,
        base_font_family: &str,
    ) -> Option<ScalarValue> {
        // Handle text mark with computed fonts
        if mark_type == "text" {
            match channel {
                "font_size" => return Some(ScalarValue::Float32(Some(computed_font_size))),
                "font" => return Some(ScalarValue::Utf8(Some(base_font_family.to_string()))),
                _ => {}
            }
        }

        // For all other cases, use the regular mark_default
        self.mark_default(mark_type, channel)
    }

    fn font_size(&self, context: &TraitContext) -> f32 {
        let css_context = self.convert_context(context);
        self.css_theme
            .get_f32(&css_context, props::FONT_SIZE)
            .unwrap_or(12.0)
    }

    fn font_family(&self, context: &TraitContext) -> String {
        let css_context = self.convert_context(context);
        self.css_theme
            .get_string(&css_context, props::FONT_FAMILY)
            .unwrap_or_else(|| "Atkinson Hyperlegible Next".to_string())
    }

    fn font_weight(&self, context: &TraitContext) -> f32 {
        let css_context = self.convert_context(context);
        self.css_theme
            .get_f32(&css_context, props::FONT_WEIGHT)
            .unwrap_or(400.0)
    }

    fn color(&self, context: &TraitContext) -> String {
        let css_context = self.convert_context(context);
        match self.css_theme.get_color(&css_context, props::COLOR) {
            Some(rgba) => format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue),
            None => "#000000".to_string(),
        }
    }
}
