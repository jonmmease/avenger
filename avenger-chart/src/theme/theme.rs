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
                fill-discrete: var(--categorical-colors);
                fill-continuous: var(--viridis-colors);
                stroke-discrete: var(--categorical-colors);
                stroke-continuous: var(--viridis-colors);
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
                fill-discrete: var(--categorical-colors);
                fill-continuous: var(--viridis-colors);
                stroke-discrete: var(--categorical-colors);
                stroke-continuous: var(--viridis-colors);
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

    /// Get the base font size in pixels (used for rem unit conversion)
    pub fn base_font_size(&self) -> f32 {
        self.base_font_size
    }

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
    
    /// Get range for a specific channel based on mark type and range kind
    ///
    /// Returns `None` if the channel range is not specified in the CSS theme.
    /// Callers should provide their own fallback defaults.
    pub fn get_range_for_channel(
        &self,
        mark_type: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> Option<crate::scales::ScaleRange> {
        use avenger_scales::scales::RangeKind;

        // Build property name based on channel and range kind
        // Convert underscores to hyphens for CSS-friendly property names
        // e.g., "stroke_width" -> "stroke-width-discrete"
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
        let base_contexts = vec![
            ThemeContext::new("mark").with_subtype(mark_type),
            ThemeContext::new("mark"),
        ];

        // For discrete ranges, try cardinality-specific values with fallback logic
        if let (RangeKind::Discrete, Some(cardinality)) = (range_kind, domain_cardinality) {
            for base_context in &base_contexts {
                // First get the base range (without cardinality) for comparison
                let base_range_value = self.query(base_context, &property);

                // Try cardinalities from 1 up to requested, keeping track of best match
                // We want the LARGEST cardinality <= requested that has a specific rule
                let mut best_card: Option<usize> = None;
                // Check up to a reasonable maximum (e.g., 2x the requested cardinality)
                let max_check = cardinality * 2;
                for card in 1..=max_check {
                    let context_with_card = base_context
                        .clone()
                        .with_attribute("cardinality", card.to_string());
                    let card_range_value = self.query(&context_with_card, &property);

                    // Only consider this if it's different from base (meaning cardinality attribute matched)
                    if card_range_value.is_some() && card_range_value != base_range_value {
                        // Prefer cardinalities <= requested, but accept larger if nothing better exists
                        if card <= cardinality {
                            best_card = Some(card);
                        } else if best_card.is_none() {
                            best_card = Some(card);
                        }
                    }
                }

                // Use the best cardinality match if we found one
                if let Some(best) = best_card {
                    let context_with_best = base_context
                        .clone()
                        .with_attribute("cardinality", best.to_string());
                    if let Some(range) = self.try_get_range(&context_with_best, &property, channel, range_kind, domain_cardinality) {
                        return Some(range);
                    }
                }

                // Fall back to base palette (no cardinality attribute)
                if let Some(range) = self.try_get_range(base_context, &property, channel, range_kind, domain_cardinality) {
                    return Some(range);
                }
            }
        } else {
            // For continuous ranges or when cardinality is unknown, just try without cardinality
            for context in &base_contexts {
                if let Some(range) = self.try_get_range(context, &property, channel, range_kind, domain_cardinality) {
                    return Some(range);
                }
            }
        }

        None
    }

    /// Helper to try getting a range from a specific context
    fn try_get_range(
        &self,
        context: &ThemeContext,
        property: &str,
        channel: &str,
        range_kind: avenger_scales::scales::RangeKind,
        domain_cardinality: Option<usize>,
    ) -> Option<crate::scales::ScaleRange> {
        let range_value = self.query(context, property);

        // Parse the range value into appropriate ScaleRange
        match range_value {
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
                    return Some(self.create_scale_range(
                        &parsed_values,
                        channel,
                        range_kind,
                        domain_cardinality,
                    ));
                }
            }
            _ => {}
        }

        // No value found
        None
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

        // Convert underscore to hyphen for CSS property name
        let css_property = channel.replace('_', "-");

        let theme_value = self.query(&context, &css_property);

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
}


impl Serialize for Theme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as combined CSS string
        let css = self.to_css();
        css.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let css = String::deserialize(deserializer)?;
        Theme::from_css(&css).map_err(serde::de::Error::custom)
    }
}

/// A compiled CSS rule with selector and declarations
#[derive(Debug, Clone)]
pub(crate) struct CompiledRule {
    pub(crate) selector: selectors::parser::Selector<crate::theme::selector_impl::ChartSelectors>,
    pub(crate) specificity: u32,
    pub(crate) source_order: usize,
    pub(crate) declarations: IndexMap<String, ThemeValue>,
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
            attributes: std::collections::HashMap::new(),
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
            attributes: std::collections::HashMap::new(),
            parent: None,
        };

        let value = deserialized.query(&context, "fill");
        // Should get the fill value for mark[type="symbol"] from dark theme
        assert!(matches!(value, Some(ThemeValue::Color(_))));

        // Also test that base_font_size is preserved
        assert_eq!(deserialized.base_font_size(), 12.0);
    }

    #[test]
    fn test_css_list_parsing() {
        let css = r#"
            :root {
                --my-colors: #ff0000, #00ff00, #0000ff;
            }
            mark {
                fill-discrete: #ff0000, #00ff00, #0000ff;
                stroke-discrete: var(--my-colors);
            }
        "#;

        let theme = Theme::from_css(css).unwrap();

        // Test direct list value
        let ctx = ThemeContext::new("mark");
        let value = theme.query(&ctx, "fill-discrete");
        println!("Direct value type: {:?}", value);
        match value {
            Some(ThemeValue::List(items)) => {
                println!("Direct: It's a List with {} items!", items.len());
                assert_eq!(items.len(), 3);
            }
            Some(ThemeValue::String(s)) => panic!("Direct: Expected List but got String: {}", s),
            _ => panic!("Direct: Unexpected value type"),
        }

        // Test variable value - query() automatically resolves variables
        let var_value = theme.query(&ctx, "stroke-discrete");
        println!("Variable value type: {:?}", var_value);
        match var_value {
            Some(ThemeValue::List(items)) => {
                println!("Variable: It's a List with {} items!", items.len());
                assert_eq!(items.len(), 3);
            }
            Some(ThemeValue::String(s)) => panic!("Variable: Expected List but got String: {}", s),
            _ => panic!("Variable: Unexpected value type"),
        }
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
            attributes: std::collections::HashMap::new(),
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

        // After deserialization, CSS sources are combined into one
        assert_eq!(deserialized.css_sources.len(), 1);

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
            attributes: std::collections::HashMap::new(),
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
    fn test_cardinality_based_ranges() {
        use crate::scales::ScaleRange;
        use avenger_scales::scales::RangeKind;

        let css = r#"
            /* Specific palettes for different cardinalities */
            mark[type="symbol"][cardinality="2"] {
                fill-discrete: #1f77b4, #ff7f0e;
            }

            mark[type="symbol"][cardinality="3"] {
                fill-discrete: #1f77b4, #ff7f0e, #2ca02c;
            }

            mark[type="symbol"][cardinality="5"] {
                fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2;
            }

            /* Fallback for any cardinality */
            mark[type="symbol"] {
                fill-discrete: red, blue, green, yellow, purple, orange;
            }
        "#;

        let theme = Theme::from_css(css).unwrap();

        // Test 1: Exact match for cardinality 3
        let range_3 = theme.get_range_for_channel("symbol", "fill", RangeKind::Discrete, Some(3));
        assert!(range_3.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_3 {
            assert_eq!(values.len(), 3, "Should get 3-color palette");
        } else {
            panic!("Expected discrete range");
        }

        // Test 2: Exact match for cardinality 5
        let range_5 = theme.get_range_for_channel("symbol", "fill", RangeKind::Discrete, Some(5));
        assert!(range_5.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_5 {
            assert_eq!(values.len(), 5, "Should get 5-color palette");
        } else {
            panic!("Expected discrete range");
        }

        // Test 3: Fallback to largest available cardinality (3) when requesting 4
        // Since we have cardinality-specific rules for 2, 3, and 5, requesting 4 should
        // fall back to 3 (the largest cardinality < 4)
        let range_4 = theme.get_range_for_channel("symbol", "fill", RangeKind::Discrete, Some(4));
        assert!(range_4.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_4 {
            assert_eq!(values.len(), 3, "Should fall back to 3-color palette (largest < 4)");
        } else {
            panic!("Expected discrete range");
        }

        // Test 4: Use largest available cardinality (5) when requesting 10
        // Since we have no exact match and the largest defined is 5, use that
        let range_10 = theme.get_range_for_channel("symbol", "fill", RangeKind::Discrete, Some(10));
        assert!(range_10.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_10 {
            assert_eq!(values.len(), 5, "Should use largest available (5-color palette)");
        } else {
            panic!("Expected discrete range");
        }

        // Test 5: When cardinality is unknown, should get base palette
        let range_none = theme.get_range_for_channel("symbol", "fill", RangeKind::Discrete, None);
        assert!(range_none.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_none {
            assert_eq!(values.len(), 6, "Should get base 6-color palette");
        } else {
            panic!("Expected discrete range");
        }
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
