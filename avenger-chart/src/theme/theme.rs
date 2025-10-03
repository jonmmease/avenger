//! Full CSS-compliant theme system using cssparser and selectors

use crate::theme::parser;
use crate::theme::{ThemeContext, ThemeValue, select_available_font};
use indexmap::IndexMap;
use selectors::matching::SelectorCaches;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Default font family fallback used when no CSS theme is specified
#[allow(dead_code)]
pub const DEFAULT_FONT_FAMILY: &str = "sans-serif";

/// Default Okabe-Ito colorblind-safe palette
///
/// This palette is scientifically designed to be distinguishable by people with
/// the most common forms of color blindness. The colors are optimized for both
/// categorical data visualization and accessibility.
pub const DEFAULT_CATEGORICAL_COLORS: &[&str] = &[
    "#0072B2", // Blue
    "#E69F00", // Orange
    "#009E73", // Bluish Green
    "#F0E442", // Yellow
    "#D55E00", // Vermillion
    "#56B4E9", // Sky Blue
    "#CC79A7", // Reddish Purple
    "#999999", // Grey
];

/// Default shape names for discrete shape scales
///
/// These shapes are commonly supported across visualization libraries and provide
/// good visual differentiation for categorical data.
pub const DEFAULT_SHAPE_NAMES: &[&str] = &[
    "circle",
    "cross",
    "diamond",
    "square",
    "star",
    "triangle-up",
    "wye",
    "cushion",
];

/// Default dash pattern names for discrete stroke-dash scales
///
/// These patterns provide varying levels of visual distinctness for line-based marks.
pub const DEFAULT_DASH_NAMES: &[&str] = &[
    "solid",
    "dashed",
    "dotted",
    "long-dash",
    "dash-dot",
    "long-short",
    "even-short",
    "double-dash",
];

/// CSS-based theme with full selector support
#[derive(Debug, Clone)]
pub struct Theme {
    pub(crate) rules: Vec<CompiledRule>,
    pub(crate) variables: IndexMap<String, ThemeValue>,
    pub(crate) inherited_properties: std::collections::HashSet<&'static str>,
    pub(crate) base_font_size: f32,
    /// List of CSS sources in order they were added
    pub(crate) css_sources: Vec<String>,
}

/// A compiled CSS rule with selector and declarations
#[derive(Debug, Clone)]
pub(crate) struct CompiledRule {
    pub(crate) selector: selectors::parser::Selector<crate::theme::selector_impl::ChartSelectors>,
    pub(crate) specificity: u32,
    pub(crate) source_order: usize,
    pub(crate) declarations: IndexMap<String, ThemeValue>,
}

impl Theme {
    /// Light theme preset with default colors and styling
    pub fn light() -> Self {
        let css = r#"
            /* Base configuration */
            :root {
                font-family: "Atkinson Hyperlegible Next";
                font-size: 12px;

                /* Color palettes */
                --categorical-colors: #0072B2, #E69F00, #009E73, #F0E442, #D55E00, #56B4E9, #CC79A7, #999999;
                --viridis-colors: #440154, #31688E, #35B779, #FDE725;
            }

            /* Chart title styling */
            chart-title {
                color: #1a1a1a;
                font-weight: 500;
                font-size: 1.5rem; /* 18px @ 12px base */
            }

            chart-subtitle {
                color: #4a4a4a;
                font-weight: 200;
                font-size: 1.167rem; /* 14px @ 12px base */
            }

            /* Axis styling - container level */
            axis {
                /* Container properties if needed */
            }

            /* Axis child elements */
            axis domain {
                stroke: #000;
                stroke-width: 1.0;
            }

            axis tick {
                stroke: #000;
                size: 5.0;
            }

            axis title {
                color: #2a2a2a;
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            axis label {
                color: #5a5a5a;
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
                padding: 3;
            }

            axis grid {
                stroke: #e0e0e0;
                opacity: 0.5;
                stroke-width: 0.5;
            }

            /* Legend styling - container level */
            legend {
                spacing: 10;
                symbol-size: 100;
                label-padding: 5;
            }

            /* Legend child elements */
            legend title {
                color: #2C2C2C;
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            legend label {
                color: #3C3C3C;
                font-weight: 300;
                font-size: 0.917rem; /* 11px @ 12px base */
            }

            legend tick {
                color: #5a5a5a;
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
            }

            legend background {
                padding: 8;
            }

            /* Mark defaults */
            mark[type="symbol"] {
                fill: #4682b4;
                stroke: #000000;
                stroke-width: 1.0;
                size: 64;
                shape: circle;
                opacity: 1.0;
            }

            mark[type="rect"] {
                fill: #4682b4;
                stroke: #000000;
                stroke-width: 1.0;
                corner-radius: 0;
                opacity: 1.0;
            }

            mark[type="line"] {
                stroke: #4682b4;
                stroke-width: 2.0;
                stroke-dash: solid;
                stroke-cap: round;
                stroke-join: round;
                opacity: 1.0;
            }

            mark[type="text"] {
                fill: #000000;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            /* Range configurations */
            mark {
                color-discrete: var(--categorical-colors);
                color-continuous: var(--viridis-colors);
                shape-discrete: circle, cross, diamond, square, star, triangle-up, wye, cushion;
                size-discrete: 30, 80, 140, 200, 260;
                size-continuous: 30, 200;
                stroke-dash-discrete: solid, dashed, dotted, long-dash, dash-dot, long-short, even-short, double-dash;
                stroke-width-discrete: 0.5, 1.0, 2.0, 3.0, 5.0;
            }
        "#;

        Self::from_css(css).expect("Failed to parse built-in light theme CSS")
    }

    /// Dark theme preset with dark mode colors
    pub fn dark() -> Self {
        let css = r#"
            /* Base configuration */
            :root {
                font-family: "Atkinson Hyperlegible Next";
                font-size: 12px;

                /* Okabe-Ito colorblind-safe palette optimized for dark backgrounds */
                --categorical-colors: #56B4E9, #E69F00, #009E73, #F0E442, #0072B2, #D55E00, #CC79A7, #999999;
                --viridis-colors: #440154, #31688E, #35B779, #FDE725;
            }

            /* Backgrounds */
            canvas {
                background-color: #121212;
            }

            plot {
                background-color: #1E1E1E;
            }

            /* Chart title styling */
            chart-title {
                color: #FFFFFF;
                font-weight: 500;
                font-size: 1.5rem; /* 18px @ 12px base */
            }

            chart-subtitle {
                color: #FFFFFF;
                font-weight: 200;
                font-size: 1.167rem; /* 14px @ 12px base */
            }

            /* Axis styling - container level */
            axis {
                /* Container properties if needed */
            }

            /* Axis child elements */
            axis domain {
                stroke: #FFFFFF;
                stroke-width: 1.0;
            }

            axis tick {
                stroke: #FFFFFF;
                size: 5.0;
            }

            axis title {
                color: #FFFFFF;
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            axis label {
                color: #AAAAAA;
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
                padding: 3;
            }

            axis grid {
                stroke: #30363D;
                opacity: 0.5;
                stroke-width: 0.5;
            }

            /* Legend styling - container level */
            legend {
                spacing: 10;
                symbol-size: 100;
                label-padding: 5;
            }

            /* Legend child elements */
            legend title {
                color: #FFFFFF;
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            legend label {
                color: #E1E6EA;
                font-weight: 300;
                font-size: 0.917rem; /* 11px @ 12px base */
            }

            legend tick {
                color: #AAAAAA;
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
            }

            legend background {
                padding: 8;
            }

            /* Mark defaults - note that dark theme uses 0 stroke width by default */
            mark[type="symbol"] {
                fill: #56B4E9;
                stroke: #30363D;
                stroke-width: 0.0;
                size: 64;
                shape: circle;
                opacity: 1.0;
            }

            mark[type="rect"] {
                fill: #56B4E9;
                stroke: #30363D;
                stroke-width: 0.0;
                corner-radius: 0;
                opacity: 1.0;
            }

            mark[type="line"] {
                stroke: #56B4E9;
                stroke-width: 2.0;
                stroke-dash: solid;
                stroke-cap: round;
                stroke-join: round;
                opacity: 1.0;
            }

            mark[type="text"] {
                fill: #C9D1D9;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            /* Range configurations */
            mark {
                color-discrete: var(--categorical-colors);
                color-continuous: var(--viridis-colors);
                shape-discrete: circle, cross, diamond, square, star, triangle-up, wye, cushion;
                size-discrete: 30, 80, 140, 200, 260;
                size-continuous: 30, 200;
                stroke-dash-discrete: solid, dashed, dotted, long-dash, dash-dot, long-short, even-short, double-dash;
                stroke-width-discrete: 0.5, 1.0, 2.0, 3.0, 5.0;
            }
        "#;

        Self::from_css(css).expect("Failed to parse built-in dark theme CSS")
    }

    /// Create a theme from CSS string
    pub fn from_css(css: &str) -> Result<Self, String> {
        let rules = parser::parse_stylesheet(css)?;

        // Extract variables and base font size from rules
        let mut variables = IndexMap::new();
        let mut compiled_rules = Vec::new();
        let mut base_font_size = 12.0; // Default base font size

        for rule in rules {
            // Extract base font size from :root { font-size: ... }
            if let Some(size) = Self::extract_base_font_size(&rule, base_font_size) {
                base_font_size = size;
            }

            // Extract CSS variables
            Self::extract_variables(&rule, &mut variables);

            compiled_rules.push(rule);
        }

        Ok(Self {
            rules: compiled_rules,
            variables,
            inherited_properties: Self::default_inherited_properties(),
            base_font_size,
            css_sources: vec![css.to_string()],
        })
    }

    /// Get the default set of inherited CSS properties
    fn default_inherited_properties() -> std::collections::HashSet<&'static str> {
        let mut inherited_properties = std::collections::HashSet::new();
        inherited_properties.insert("color");
        inherited_properties.insert("font-family");
        inherited_properties.insert("font-size");
        inherited_properties.insert("font-weight");
        inherited_properties.insert("font-style");
        inherited_properties.insert("line-height");
        inherited_properties.insert("text-align");
        inherited_properties.insert("visibility");
        inherited_properties
    }

    /// Check if a selector targets :root
    fn is_root_selector(rule: &CompiledRule) -> bool {
        let components: Vec<_> = rule.selector.iter_raw_match_order().collect();
        components.len() == 1
            && matches!(
                components[0],
                selectors::parser::Component::ExplicitUniversalType
            )
    }

    /// Extract base font size from a rule if it's a :root rule with font-size
    fn extract_base_font_size(rule: &CompiledRule, current_base: f32) -> Option<f32> {
        if Self::is_root_selector(rule) {
            if let Some(font_size_value) = rule.declarations.get("font-size") {
                return font_size_value.as_font_size(current_base);
            }
        }
        None
    }

    /// Extract CSS variables from a rule
    fn extract_variables(rule: &CompiledRule, variables: &mut IndexMap<String, ThemeValue>) {
        for (key, value) in &rule.declarations {
            if key.starts_with("--") {
                variables.insert(key.clone(), value.clone());
            }
        }
    }

    /// Append additional CSS rules to the theme
    pub fn append_css(&mut self, css: &str) -> Result<(), String> {
        // Parse the new CSS
        let new_rules = parser::parse_stylesheet(css)?;

        // Get current max source_order
        let current_max_order = self.rules.iter().map(|r| r.source_order).max().unwrap_or(0);

        // Add new rules with updated source_order
        for (i, mut rule) in new_rules.into_iter().enumerate() {
            rule.source_order = current_max_order + i + 1;

            // Check if this rule targets :root and updates base font size
            // Extract base font size from :root { font-size: ... }
            if let Some(size) = Self::extract_base_font_size(&rule, self.base_font_size) {
                self.base_font_size = size;
            }

            // Extract any new variables
            Self::extract_variables(&rule, &mut self.variables);

            self.rules.push(rule);
        }

        // Store the CSS source
        self.css_sources.push(css.to_string());

        Ok(())
    }

    /// Get the combined CSS source
    pub fn to_css(&self) -> String {
        self.css_sources.join("\n\n")
    }

    /// Query a CSS property for an element
    ///
    /// Returns `Some(ThemeValue)` if a CSS rule matches, `None` if no rule is found.
    /// The caller is responsible for providing appropriate defaults.
    pub fn query(&self, context: &ThemeContext, property: &str) -> Option<ThemeValue> {
        use crate::theme::element::CssElement;

        // Convert ThemeContext to CssElement for selector matching
        let element = CssElement::from(context);

        // Create matching context
        let mut selector_caches = SelectorCaches::default();
        let mut matching_context = selectors::context::MatchingContext::new(
            selectors::context::MatchingMode::Normal,
            None,
            &mut selector_caches,
            selectors::context::QuirksMode::NoQuirks,
            selectors::context::NeedsSelectorFlags::No,
            selectors::context::MatchingForInvalidation::No,
        );

        // Find matching rules
        let mut matches = Vec::new();
        for rule in &self.rules {
            let is_match = selectors::matching::matches_selector(
                &rule.selector,
                0,
                None,
                &element,
                &mut matching_context,
            );
            if is_match {
                matches.push(rule);
            }
        }

        // Sort by specificity and source order (higher specificity should win, then later rules)
        matches.sort_by_key(|r| (r.specificity, r.source_order));

        // Find the property value - iterate from highest specificity
        for rule in matches.iter().rev() {
            if let Some(value) = rule.declarations.get(property) {
                // Resolve variables if needed
                if let ThemeValue::Variable(var_name) = value {
                    if let Some(resolved) = self.variables.get(var_name) {
                        return Some(resolved.clone());
                    }
                }
                return Some(value.clone());
            }
        }

        // Check if property is inherited and try to get it from parent
        if self.inherited_properties.contains(property) {
            // Try to get the value from the parent element
            if let Some(parent) = &context.parent {
                // Recursively query the parent for this property
                return self.query(parent, property);
            }
        }

        // No CSS rule found
        None
    }
}

/// Helper struct for serializing Theme
#[derive(Serialize, Deserialize)]
struct ThemeData {
    css_sources: Vec<String>,
    base_font_size: f32,
}

impl Serialize for Theme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let data = ThemeData {
            css_sources: self.css_sources.clone(),
            base_font_size: self.base_font_size,
        };
        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let data = ThemeData::deserialize(deserializer)?;

        // Start with empty theme
        let mut theme = Theme {
            rules: Vec::new(),
            variables: IndexMap::new(),
            inherited_properties: Self::default_inherited_properties(),
            base_font_size: data.base_font_size,
            css_sources: Vec::new(),
        };

        // Rebuild by parsing each CSS source in order
        for css in data.css_sources {
            theme.append_css(&css).map_err(serde::de::Error::custom)?;
        }

        Ok(theme)
    }
}

// ============================================================================
// Theme Query Methods
// ============================================================================

impl Theme {
    /// Get the base font size in pixels (used for rem unit conversion)
    pub fn base_font_size(&self) -> f32 {
        self.base_font_size
    }

    // Internal helper methods for building contexts

    /// Build a legend context with optional subtype
    fn legend_context(&self, subtype: Option<&str>) -> ThemeContext {
        let mut legend_ctx = ThemeContext::new("legend");
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        legend_ctx
    }

    /// Build an axis context with optional coordinate and axis types
    fn axis_context(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> ThemeContext {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        axis_ctx
    }

    /// Get font family for a context
    ///
    /// Returns the first available font from the font-family list, or None if not set in CSS.
    pub fn font_family(&self, context: &ThemeContext) -> Option<String> {
        let font_family_value = self.query(context, "font-family")?;

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
            _ => return None,
        };

        if fonts.is_empty() {
            return None;
        }

        // Return the first available font from the list
        Some(select_available_font(fonts))
    }

    /// Get font size for a context
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "font-size")
            .and_then(|v| v.as_font_size(self.base_font_size()))
    }

    /// Get font weight for a context
    pub fn font_weight(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "font-weight")
            .and_then(|v| v.as_number())
            .map(|n| n as f32)
    }

    /// Get color for a context as normalized RGBA array
    pub fn color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        self.query(context, "color")
            .and_then(|v| v.as_color_array())
    }

    /// Get fill color for a context as normalized RGBA array
    pub fn fill_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        self.query(context, "fill").and_then(|v| v.as_color_array())
    }

    /// Get stroke color for a context as normalized RGBA array
    pub fn stroke_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        self.query(context, "stroke")
            .and_then(|v| v.as_color_array())
    }

    /// Get stroke width for a context
    pub fn stroke_width(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "stroke-width")
            .and_then(|v| v.as_font_size(self.base_font_size()))
    }

    /// Get opacity for a context
    pub fn opacity(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "opacity")
            .and_then(|v| v.as_number())
            .map(|n| n as f32)
    }

    /// Get canvas background color as normalized RGBA array
    pub fn canvas_background(&self) -> Option<[f32; 4]> {
        let ctx = ThemeContext::new("canvas");
        self.query(&ctx, "background-color")
            .and_then(|v| v.as_color_array())
    }

    // Guide (coordinate system) theme methods

    /// Get guide background color as normalized RGBA array
    ///
    /// The subtype parameter allows targeting specific coordinate systems,
    /// e.g., "cartesian", "polar"
    pub fn guide_background_color(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        let mut guide_ctx = ThemeContext::new("guide");
        if let Some(t) = subtype {
            guide_ctx = guide_ctx.with_subtype(t);
        }
        self.query(&guide_ctx, "background-color")
            .and_then(|v| v.as_color_array())
    }

    /// Get base font family
    pub fn base_font_family(&self) -> Option<String> {
        self.font_family(&ThemeContext::new("base"))
    }

    // Legend-specific methods

    /// Get legend background fill as normalized RGBA array
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_fill(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        let ctx = self.legend_context(subtype).child("background");
        self.query(&ctx, "fill").and_then(|v| v.as_color_array())
    }

    /// Get legend background stroke as normalized RGBA array
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_stroke(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        let ctx = self.legend_context(subtype).child("background");
        self.query(&ctx, "stroke").and_then(|v| v.as_color_array())
    }

    /// Get legend background padding
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_padding(&self, subtype: Option<&str>) -> Option<f32> {
        let ctx = self.legend_context(subtype).child("background");
        self.query(&ctx, "padding")
            .and_then(|v| v.as_font_size(self.base_font_size()))
    }

    /// Get legend background corner radius
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_background_corner_radius(&self, subtype: Option<&str>) -> Option<f32> {
        let ctx = self.legend_context(subtype).child("background");
        self.query(&ctx, "corner-radius")
            .and_then(|v| v.as_font_size(self.base_font_size()))
    }

    /// Get legend title color
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_color(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        self.color(&self.legend_context(subtype).child("title"))
    }

    /// Get legend label color as normalized RGBA array
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_color(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        self.color(&self.legend_context(subtype).child("label"))
    }

    /// Get legend tick color as normalized RGBA array
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_color(&self, subtype: Option<&str>) -> Option<[f32; 4]> {
        self.color(&self.legend_context(subtype).child("tick"))
    }

    /// Get legend title font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_family(&self, subtype: Option<&str>) -> Option<String> {
        self.font_family(&self.legend_context(subtype).child("title"))
    }

    /// Get legend label font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_family(&self, subtype: Option<&str>) -> Option<String> {
        self.font_family(&self.legend_context(subtype).child("label"))
    }

    /// Get legend tick font family
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_family(&self, subtype: Option<&str>) -> Option<String> {
        self.font_family(&self.legend_context(subtype).child("tick"))
    }

    /// Get legend title font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_size(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_size(&self.legend_context(subtype).child("title"))
    }

    /// Get legend label font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_size(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_size(&self.legend_context(subtype).child("label"))
    }

    /// Get legend tick font size
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_size(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_size(&self.legend_context(subtype).child("tick"))
    }

    /// Get legend title font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_title_font_weight(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_weight(&self.legend_context(subtype).child("title"))
    }

    /// Get legend label font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_label_font_weight(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_weight(&self.legend_context(subtype).child("label"))
    }

    /// Get legend tick font weight
    ///
    /// # Arguments
    /// * `subtype` - Optional legend subtype (e.g., "symbol", "line", "colorbar")
    pub fn legend_tick_font_weight(&self, subtype: Option<&str>) -> Option<f32> {
        self.font_weight(&self.legend_context(subtype).child("tick"))
    }

    // Title-specific methods

    /// Get title color as normalized RGBA array
    pub fn title_color(&self) -> Option<[f32; 4]> {
        self.color(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle color as normalized RGBA array
    pub fn subtitle_color(&self) -> Option<[f32; 4]> {
        self.color(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font family
    pub fn title_font_family(&self) -> Option<String> {
        self.font_family(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font family
    pub fn subtitle_font_family(&self) -> Option<String> {
        self.font_family(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font size
    pub fn title_font_size(&self) -> Option<f32> {
        self.font_size(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font size
    pub fn subtitle_font_size(&self) -> Option<f32> {
        self.font_size(&ThemeContext::new("chart-subtitle"))
    }

    /// Get title font weight
    pub fn title_font_weight(&self) -> Option<f32> {
        self.font_weight(&ThemeContext::new("chart-title"))
    }

    /// Get subtitle font weight
    pub fn subtitle_font_weight(&self) -> Option<f32> {
        self.font_weight(&ThemeContext::new("chart-subtitle"))
    }

    // Axis-specific methods

    /// Get axis domain color as normalized RGBA array
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_domain_color(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<[f32; 4]> {
        let ctx = self.axis_context(coord_type, axis_type).child("domain");
        self.query(&ctx, "stroke").and_then(|v| v.as_color_array())
    }

    /// Get axis tick color as normalized RGBA array
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_tick_color(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<[f32; 4]> {
        let ctx = self.axis_context(coord_type, axis_type).child("tick");
        self.query(&ctx, "stroke").and_then(|v| v.as_color_array())
    }

    /// Get axis grid color as normalized RGBA array
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_color(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<[f32; 4]> {
        let ctx = self.axis_context(coord_type, axis_type).child("grid");
        self.stroke_color(&ctx)
    }

    /// Get axis grid opacity
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_opacity(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        let ctx = self.axis_context(coord_type, axis_type).child("grid");
        self.query(&ctx, "opacity")
            .and_then(|v| v.as_number())
            .map(|n| n as f32)
    }

    /// Get axis grid width
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_width(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        let ctx = self.axis_context(coord_type, axis_type).child("grid");
        self.stroke_width(&ctx)
    }

    /// Get axis label color as normalized RGBA array
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_color(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<[f32; 4]> {
        self.color(&self.axis_context(coord_type, axis_type).child("label"))
    }

    /// Get axis title color as normalized RGBA array
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_color(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<[f32; 4]> {
        self.color(&self.axis_context(coord_type, axis_type).child("title"))
    }

    /// Get axis tick length
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_tick_length(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        let ctx = self.axis_context(coord_type, axis_type).child("tick");
        self.query(&ctx, "size")
            .and_then(|v| v.as_font_size(self.base_font_size()))
    }

    /// Get axis label font size
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_size(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        self.font_size(&self.axis_context(coord_type, axis_type).child("label"))
    }

    /// Get axis label font weight
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_weight(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        self.font_weight(&self.axis_context(coord_type, axis_type).child("label"))
    }

    /// Get axis title font size
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_size(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        self.font_size(&self.axis_context(coord_type, axis_type).child("title"))
    }

    /// Get axis title font weight
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_weight(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<f32> {
        self.font_weight(&self.axis_context(coord_type, axis_type).child("title"))
    }

    /// Get axis label font family
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_label_font_family(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<String> {
        self.font_family(&self.axis_context(coord_type, axis_type).child("label"))
    }

    /// Get axis title font family
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_title_font_family(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
    ) -> Option<String> {
        self.font_family(&self.axis_context(coord_type, axis_type).child("title"))
    }

    // Access to color palettes, shapes, and dashes

    /// Get categorical color palette for discrete color scales
    ///
    /// Queries the theme for `color-discrete` values, falling back to the Okabe-Ito
    /// colorblind-safe palette if not specified in CSS. Returns colors as hex strings.
    pub fn categorical_colors(&self) -> Vec<String> {
        // Query categorical colors from CSS
        let context = ThemeContext::new("mark");
        let colors_value = self.query(&context, "color-discrete");

        // Parse the result into a list of colors
        match colors_value {
            Some(ThemeValue::String(s)) => {
                let colors = Self::parse_css_list(&s);
                if !colors.is_empty() {
                    return colors;
                }
            }
            Some(ThemeValue::List(values)) => {
                let mut colors = Vec::new();
                for val in values {
                    match val {
                        ThemeValue::String(s) => {
                            colors.push(s);
                        }
                        ThemeValue::Color(rgba) => {
                            let hex =
                                format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                            colors.push(hex);
                        }
                        _ => {}
                    }
                }
                if !colors.is_empty() {
                    return colors;
                }
            }
            _ => {}
        }

        // Fallback to Okabe-Ito colorblind-safe palette
        DEFAULT_CATEGORICAL_COLORS
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Get shape names for discrete shape scales
    ///
    /// Queries the theme for `shape-discrete` values, falling back to a default set
    /// of common shapes (circle, cross, diamond, square, star, etc.) if not specified in CSS.
    pub fn shape_names(&self) -> Vec<String> {
        // Query shape names from CSS
        let context = ThemeContext::new("mark");
        let shapes_value = self.query(&context, "shape-discrete");

        // Parse the result into a list of shapes
        match shapes_value {
            Some(ThemeValue::String(s)) => {
                let shapes = Self::parse_css_list(&s);
                if !shapes.is_empty() {
                    return shapes;
                }
            }
            Some(ThemeValue::List(values)) => {
                let mut shapes = Vec::new();
                for val in values {
                    if let ThemeValue::String(s) = val {
                        shapes.push(s);
                    }
                }
                if !shapes.is_empty() {
                    return shapes;
                }
            }
            _ => {}
        }

        // Fallback to default shapes
        DEFAULT_SHAPE_NAMES.iter().map(|s| s.to_string()).collect()
    }

    /// Get dash pattern names for discrete stroke-dash scales
    ///
    /// Queries the theme for `stroke-dash-discrete` values, falling back to a default set
    /// of dash patterns (solid, dashed, dotted, etc.) if not specified in CSS.
    pub fn dash_names(&self) -> Vec<String> {
        // Query dash patterns from CSS
        let context = ThemeContext::new("mark");
        let dashes_value = self.query(&context, "stroke-dash-discrete");

        // Parse the result into a list of dash patterns
        match dashes_value {
            Some(ThemeValue::String(s)) => {
                let dashes = Self::parse_css_list(&s);
                if !dashes.is_empty() {
                    return dashes;
                }
            }
            Some(ThemeValue::List(values)) => {
                let mut dashes = Vec::new();
                for val in values {
                    if let ThemeValue::String(s) = val {
                        dashes.push(s);
                    }
                }
                if !dashes.is_empty() {
                    return dashes;
                }
            }
            _ => {}
        }

        // Fallback to default dash patterns
        DEFAULT_DASH_NAMES.iter().map(|s| s.to_string()).collect()
    }

    /// Get default shape range for ordinal scales
    ///
    /// Returns a `ScaleRange` containing shape names from the theme, limited to
    /// `domain_cardinality` shapes if specified.
    pub fn get_shape_range(&self, domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        // Use the new get_range_for_channel with a default mark type
        self.get_range_for_channel(
            "symbol",
            "shape",
            avenger_scales::scales::RangeKind::Discrete,
            domain_cardinality,
        )
    }

    /// Get default dash pattern range for ordinal scales
    ///
    /// Returns a `ScaleRange` containing all available dash patterns from the theme.
    pub fn get_dash_range(&self, _domain_cardinality: Option<usize>) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use datafusion_common::ScalarValue;

        let patterns = self.dash_names();
        use crate::serialization::SerializableScalar;
        let scalars: Vec<SerializableScalar> = patterns
            .into_iter()
            .map(|p| SerializableScalar::new(ScalarValue::Utf8(Some(p))))
            .collect();
        ScaleRange::Discrete(scalars)
    }

    /// Get range for a specific channel based on mark type and range kind
    pub fn get_range_for_channel(
        &self,
        mark_type: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use avenger_scales::scales::RangeKind;

        // Build property name based on channel and range kind
        // Convert underscores to hyphens for CSS-friendly property names
        // e.g., "glow_color" -> "glow-color-continuous"
        let css_channel = channel.replace('_', "-");
        let property = format!(
            "{}-{}",
            css_channel,
            match range_kind {
                RangeKind::Discrete => "discrete",
                RangeKind::Continuous => "continuous",
            }
        );

        // Try mark-specific first, then general mark
        let contexts = vec![
            ThemeContext::new("mark").with_subtype(mark_type),
            ThemeContext::new("mark"),
        ];

        for context in contexts {
            let range_value = self.query(&context, &property);

            // Parse the range value into appropriate ScaleRange
            match range_value {
                Some(ThemeValue::String(s)) => {
                    // Parse comma-separated list
                    let values = Self::parse_css_list(&s);
                    if !values.is_empty() {
                        return self.create_scale_range(
                            &values,
                            channel,
                            range_kind,
                            domain_cardinality,
                        );
                    }
                }
                Some(ThemeValue::List(values)) => {
                    // Handle pre-parsed list of values
                    let mut parsed_values = Vec::new();
                    for val in values {
                        match val {
                            ThemeValue::String(s) => {
                                parsed_values.push(s.clone());
                            }
                            ThemeValue::Color(rgba) => {
                                let hex =
                                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                                parsed_values.push(hex);
                            }
                            ThemeValue::Number(n) => {
                                parsed_values.push(n.to_string());
                            }
                            _ => {}
                        }
                    }
                    if !parsed_values.is_empty() {
                        return self.create_scale_range(
                            &parsed_values,
                            channel,
                            range_kind,
                            domain_cardinality,
                        );
                    }
                }
                Some(ThemeValue::Variable(var_name)) => {
                    // Try to resolve variable manually
                    if let Some(resolved) = self.variables.get(&var_name) {
                        match resolved {
                            ThemeValue::String(s) => {
                                let values = Self::parse_css_list(&s);
                                if !values.is_empty() {
                                    return self.create_scale_range(
                                        &values,
                                        channel,
                                        range_kind,
                                        domain_cardinality,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Fall back to defaults based on channel and range kind
        self.default_range_for_channel(channel, range_kind, domain_cardinality)
    }

    /// Get mark default value for a channel
    ///
    /// Returns the default value for a specific mark type and channel.
    pub fn mark_default(
        &self,
        mark_type: &str,
        channel: &str,
    ) -> Option<datafusion_common::ScalarValue> {
        // Query CSS theme for mark defaults
        let context = ThemeContext::new("mark").with_subtype(mark_type);

        // Map channel to CSS property
        let css_property = match channel {
            "fill" => "fill",
            "stroke" => "stroke",
            "stroke_width" => "stroke-width",
            "size" => "size",
            _ => return None,
        };

        let theme_value = self.query(&context, css_property);

        match theme_value {
            Some(ThemeValue::String(s)) => Some(datafusion_common::ScalarValue::Utf8(Some(s))),
            Some(ThemeValue::Number(n)) => {
                Some(datafusion_common::ScalarValue::Float32(Some(n as f32)))
            }
            Some(ThemeValue::Length(n, _)) => {
                Some(datafusion_common::ScalarValue::Float32(Some(n as f32)))
            }
            Some(ThemeValue::Color(rgba)) => {
                let hex = format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                Some(datafusion_common::ScalarValue::Utf8(Some(hex)))
            }
            _ => None,
        }
    }

    /// Parse a CSS list value into individual string values
    /// Handles formats like: "#E69F00", "#56B4E9", "#009E73"
    fn parse_css_list(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Create a ScaleRange from parsed values
    fn create_scale_range(
        &self,
        values: &[String],
        _channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion::prelude::lit;
        use palette::Srgba;

        match range_kind {
            RangeKind::Discrete => {
                // For discrete ranges, take the requested number of values
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = values
                    .iter()
                    .take(domain_cardinality.unwrap_or(values.len()))
                    .map(|v| {
                        // Check if it's a number or string
                        let scalar = if let Ok(num) = v.parse::<f64>() {
                            datafusion_common::ScalarValue::Float32(Some(num as f32))
                        } else {
                            datafusion_common::ScalarValue::Utf8(Some(v.clone()))
                        };
                        SerializableScalar::new(scalar)
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            RangeKind::Continuous => {
                // First, try to parse as numbers - if successful, it's a numeric range
                // Only attempt color parsing if numeric parsing fails
                let first_is_number = values.first().and_then(|v| v.parse::<f64>().ok()).is_some();

                if first_is_number {
                    // Numeric continuous range - expect 2 values (min, max)
                    if values.len() >= 2 {
                        let min = values[0].parse::<f64>().unwrap_or(0.0);
                        let max = values[1].parse::<f64>().unwrap_or(1.0);
                        ScaleRange::new_interval(lit(min), lit(max))
                    } else if values.len() == 1 {
                        // Single value, use it as max with 0 as min
                        let max = values[0].parse::<f64>().unwrap_or(1.0);
                        ScaleRange::new_interval(lit(0.0), lit(max))
                    } else {
                        // Default range
                        ScaleRange::new_interval(lit(0.0), lit(1.0))
                    }
                } else {
                    // Try to detect if values are colors by attempting to parse them
                    // This works for any channel, not just hardcoded ones
                    let colors: Vec<Srgba> = values
                        .iter()
                        .filter_map(|v| {
                            // Try to parse as color using avenger-scales color parser
                            // This handles hex colors, rgb(), hsl(), and named colors
                            crate::utils::parse_color_string(v).and_then(|cog| match cog {
                                avenger_common::types::ColorOrGradient::Color(rgba) => {
                                    Some(Srgba::new(rgba[0], rgba[1], rgba[2], rgba[3]))
                                }
                                _ => None,
                            })
                        })
                        .collect();

                    if !colors.is_empty() {
                        // Successfully parsed as colors, use color range
                        ScaleRange::new_color(colors)
                    } else {
                        // Couldn't parse as numbers or colors, use default numeric range
                        ScaleRange::new_interval(lit(0.0), lit(1.0))
                    }
                }
            }
        }
    }

    /// Default ranges for channels when not specified in CSS
    fn default_range_for_channel(
        &self,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> crate::scales::ScaleRange {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;
        use datafusion::prelude::lit;

        match (channel, range_kind) {
            // Color channels
            ("fill" | "stroke" | "color", RangeKind::Discrete) => {
                // Use categorical_colors which queries from CSS or falls back to Okabe-Ito
                let colors = self.categorical_colors();
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = colors
                    .iter()
                    .take(domain_cardinality.unwrap_or(colors.len()))
                    .map(|c| {
                        SerializableScalar::new(datafusion_common::ScalarValue::Utf8(Some(
                            c.clone(),
                        )))
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("fill" | "stroke" | "color", RangeKind::Continuous) => {
                // Viridis-like gradient
                use palette::Srgba;
                let colors = vec![
                    Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
                    Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
                    Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
                    Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
                ];
                ScaleRange::new_color(colors)
            }

            // Size channels
            ("size", RangeKind::Discrete) => {
                let sizes = vec![20.0, 40.0, 60.0, 80.0, 100.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = sizes
                    .iter()
                    .take(domain_cardinality.unwrap_or(sizes.len()))
                    .map(|s| {
                        SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(
                            *s as f32,
                        )))
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("size", RangeKind::Continuous) => ScaleRange::new_interval(lit(10.0), lit(200.0)),

            // Opacity channels
            ("opacity", RangeKind::Discrete) => {
                let opacities = vec![0.3, 0.5, 0.7, 0.9, 1.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = opacities
                    .iter()
                    .take(domain_cardinality.unwrap_or(opacities.len()))
                    .map(|o| {
                        SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(
                            *o as f32,
                        )))
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("opacity", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.2), lit(1.0)),

            // Stroke width channels
            ("stroke_width", RangeKind::Discrete) => {
                let widths = vec![1.0, 2.0, 3.0, 4.0, 5.0];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = widths
                    .iter()
                    .take(domain_cardinality.unwrap_or(widths.len()))
                    .map(|w| {
                        SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(
                            *w as f32,
                        )))
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }
            ("stroke_width", RangeKind::Continuous) => ScaleRange::new_interval(lit(0.5), lit(5.0)),

            // Shape channel (always discrete)
            ("shape", _) => {
                let shapes = vec!["circle", "square", "triangle", "diamond", "cross"];
                use crate::serialization::SerializableScalar;
                let scalars: Vec<SerializableScalar> = shapes
                    .iter()
                    .take(domain_cardinality.unwrap_or(shapes.len()))
                    .map(|s| {
                        SerializableScalar::new(datafusion_common::ScalarValue::Utf8(Some(
                            s.to_string(),
                        )))
                    })
                    .collect();
                ScaleRange::Discrete(scalars)
            }

            // Default
            _ => match range_kind {
                RangeKind::Discrete => {
                    use crate::serialization::SerializableScalar;
                    ScaleRange::Discrete(vec![SerializableScalar::new(
                        datafusion_common::ScalarValue::Float32(Some(1.0)),
                    )])
                }
                RangeKind::Continuous => ScaleRange::new_interval(lit(0.0), lit(1.0)),
            },
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_theme_serialization() {
        // Create a Theme
        let theme = Theme::light();

        // Serialize to JSON
        let json = serde_json::to_string(&theme).unwrap();

        // Deserialize back
        let deserialized: Theme = serde_json::from_str(&json).unwrap();

        // Check that CSS sources are preserved
        assert_eq!(theme.css_sources.len(), deserialized.css_sources.len());
        assert_eq!(theme.base_font_size, deserialized.base_font_size);

        // Verify the theme works correctly after deserialization
        let context = crate::theme::ThemeContext {
            element_type: "axis".to_string(),
            subtype: Some("domain".to_string()),
            classes: vec![],
            id: None,
            parent: None,
        };

        // Both themes should return the same value
        let original_value = theme.query(&context, "stroke");
        let deserialized_value = deserialized.query(&context, "stroke");
        assert_eq!(original_value, deserialized_value);
    }

    #[test]
    fn test_theme_trait_object_serialization() {
        // Create a theme as a trait object
        let theme: Box<Theme> = Box::new(Theme::dark());

        // Serialize the trait object
        let json = serde_json::to_string(&theme).unwrap();

        // Deserialize back as trait object
        let deserialized: Box<Theme> = serde_json::from_str(&json).unwrap();

        // Test with a mark element which has direct styles
        let context = crate::theme::ThemeContext {
            element_type: "mark".to_string(),
            subtype: Some("symbol".to_string()),
            classes: vec![],
            id: None,
            parent: None,
        };

        let value = deserialized.query(&context, "fill");
        // Should get the fill value for mark[type="symbol"] from dark theme
        assert!(matches!(value, Some(ThemeValue::Color(_))));

        // Also test that base_font_size is preserved
        assert_eq!(deserialized.base_font_size(), 12.0);
    }

    #[test]
    fn test_append_css() {
        // Create a base theme
        let mut theme = Theme::from_css(
            r#"
            mark {
                fill: red;
            }
        "#,
        )
        .unwrap();

        // Append additional CSS
        theme
            .append_css(
                r#"
            mark {
                fill: blue;
                stroke: green;
            }
        "#,
            )
            .unwrap();

        // Check that we have both CSS sources
        assert_eq!(theme.css_sources.len(), 2);

        // Test that later rules override earlier ones
        let context = crate::theme::ThemeContext {
            element_type: "mark".to_string(),
            subtype: None,
            classes: vec![],
            id: None,
            parent: None,
        };

        let fill = theme.query(&context, "fill");
        // The second rule should override, so fill should be blue
        assert!(matches!(fill, Some(ThemeValue::Color(c)) if c.blue == 255));

        let stroke = theme.query(&context, "stroke");
        // Stroke was only defined in the second rule
        assert!(matches!(stroke, Some(ThemeValue::Color(c)) if c.green == 128));
    }

    #[test]
    fn test_append_css_with_serialization() {
        // Create a base theme and append CSS
        let mut theme = Theme::from_css(
            r#"
            mark {
                fill: red;
            }
        "#,
        )
        .unwrap();

        theme
            .append_css(
                r#"
            mark {
                fill: blue;
            }
            axis {
                stroke: black;
            }
        "#,
            )
            .unwrap();

        // Serialize and deserialize
        let json = serde_json::to_string(&theme).unwrap();
        let deserialized: Theme = serde_json::from_str(&json).unwrap();

        // Verify both CSS sources are preserved
        assert_eq!(deserialized.css_sources.len(), 2);

        // Test combined CSS export
        let combined_css = deserialized.to_css();
        assert!(combined_css.contains("fill: red"));
        assert!(combined_css.contains("fill: blue"));
        assert!(combined_css.contains("stroke: black"));

        // Verify behavior is preserved
        let context = crate::theme::ThemeContext {
            element_type: "mark".to_string(),
            subtype: None,
            classes: vec![],
            id: None,
            parent: None,
        };

        let fill = deserialized.query(&context, "fill");
        assert!(matches!(fill, Some(ThemeValue::Color(c)) if c.blue == 255));
    }

    #[test]
    fn test_root_font_size() {
        // Test that :root { font-size: ... } sets base_font_size
        let css = r#"
            :root {
                font-size: 16px;
            }

            mark {
                font-size: 2rem; /* Should be 32px with 16px base */
            }
        "#;

        let theme = Theme::from_css(css).unwrap();

        // Check base font size was set from :root
        assert_eq!(theme.base_font_size(), 16.0);

        // Check that rem values are calculated correctly
        let context = ThemeContext::new("mark");
        let font_size = theme.font_size(&context);
        assert_eq!(font_size, Some(32.0)); // 2rem * 16px = 32px
    }

    #[test]
    fn test_default_base_font_size() {
        // Test that base_font_size defaults to 12.0 when not specified
        let css = r#"
            mark {
                fill: blue;
            }
        "#;

        let theme = Theme::from_css(css).unwrap();
        assert_eq!(theme.base_font_size(), 12.0);
    }

    #[test]
    fn test_builtin_themes_base_font_size() {
        let light = Theme::light();
        assert_eq!(
            light.base_font_size(),
            12.0,
            "Light theme should have 12px base"
        );

        let dark = Theme::dark();
        assert_eq!(
            dark.base_font_size(),
            12.0,
            "Dark theme should have 12px base"
        );
    }

    #[test]
    fn test_append_css_with_new_base_font_size() {
        // Start with the default light theme (12px base)
        let mut theme = Theme::light();
        assert_eq!(theme.base_font_size(), 12.0);

        // Append CSS that changes the base font size to 18px
        theme
            .append_css(
                r#"
            :root {
                font-size: 18px;
            }

            test-element {
                font-size: 2rem;
            }
        "#,
            )
            .unwrap();

        // Base font size should now be 18px immediately (from the appended :root rule)
        assert_eq!(theme.base_font_size(), 18.0);

        // Verify rem calculations use the new base
        let context = ThemeContext::new("test-element");
        let font_size = theme.font_size(&context);
        assert_eq!(font_size, Some(36.0)); // 2rem * 18px = 36px

        // Also verify existing elements that use rem units are recalculated with new base
        let title_size = theme.title_font_size();
        assert_eq!(title_size, Some(27.0)); // 1.5rem * 18px = 27px (was 18px with 12px base)

        // Verify serialization preserves the updated base font size
        let json = serde_json::to_string(&theme).unwrap();
        let deserialized: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.base_font_size(), 18.0);
    }
}
