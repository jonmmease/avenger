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

/// Default base font size when :root font-size is not specified
const DEFAULT_BASE_FONT_SIZE: f32 = 12.0;

/// CSS-based theme with full selector support
#[derive(Debug, Clone)]
pub struct Theme {
    pub(crate) rules: Vec<CompiledRule>,
    pub(crate) variables: IndexMap<String, ThemeValue>,
    pub(crate) inherited_properties: std::collections::HashSet<&'static str>,
    /// List of CSS sources in order they were added
    pub(crate) css_sources: Vec<String>,
    /// Default color-scheme for light-dark() resolution when no param is provided
    pub(crate) default_color_scheme: String,
}

impl Theme {
    /// Create the unified default theme with light-dark() functions
    ///
    /// This is the base theme that adapts to light or dark mode based on
    /// the "color-scheme" parameter.
    fn default_theme() -> Self {
        let css = r#"
            /* Base configuration */
            :root {
                font-family: "Atkinson Hyperlegible Next";
                --base-font-size: 12px;  /* Can be overridden via parameter */
                font-size: var(--base-font-size);  /* Base font size for rem calculations */

                /* === Core Color System === */
                /* Define background and text color, derive everything else */
                --bg-color: light-dark(white, #121212);
                --text-color: light-dark(black, white);

                /* === Derived Grays via color-mix === */
                /* Grid: Very subtle, 88% background */
                --grid-color: color-mix(in srgb, var(--bg-color) 88%, var(--text-color) 12%);

                /* Secondary text: Medium mix, 20% toward background */
                --text-secondary: color-mix(in srgb, var(--text-color) 80%, var(--bg-color) 20%);

                /* Tertiary text: Lighter, 35% toward background */
                --text-tertiary: color-mix(in srgb, var(--text-color) 65%, var(--bg-color) 35%);

                /* Border/domain: Very close to text, 10% toward background */
                --border-color: color-mix(in srgb, var(--text-color) 90%, var(--bg-color) 10%);

                /* Okabe-Ito color palette (colorblind-friendly) */
                --categorical-color-0: #0072B2;
                --categorical-colors:
                    var(--categorical-color-0),
                    #E69F00,
                    #009E73,
                    #F0E442,
                    #D55E00,
                    #56B4E9,
                    #CC79A7,
                    #999999;

                --viridis-colors: #440154, #3b528b, #21918c, #5ec962, #FDE725;
            }

            /* Backgrounds */
            canvas {
                background-color: var(--bg-color);
                margin: 10px;  /* Default margins around chart */
            }

            plot {
                background-color: var(--bg-color);
            }

            /* === Chart Titles === */
            chart-title {
                color: var(--text-color);
                font-weight: 500;
                font-size: 1.5rem; /* 18px @ 12px base */
                text-align: left;
                width: canvas;  /* or plot-area */
            }

            chart-subtitle {
                color: var(--text-tertiary);
                font-weight: 200;
                font-size: 1.167rem; /* 14px @ 12px base */
                text-align: left;
                width: canvas;  /* or plot-area */
            }

            /* === Axis Elements === */
            axis domain {
                stroke: var(--border-color);
                stroke-width: 1.0;
            }

            axis tick {
                stroke: var(--border-color);
                size: 5.0;
            }

            axis title {
                color: var(--text-color);
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            /* Facet title should match axis title typography */
            facet title {
                color: var(--text-color);
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            axis label {
                color: var(--text-tertiary);
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
                padding: 3;
            }

            /* Facet labels should match axis label typography */
            facet label {
                color: var(--text-tertiary);
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
            }

            axis grid {
                stroke: var(--grid-color);
                opacity: 0.5;
                stroke-width: 0.5;
            }

            /* === Legend Elements === */
            legend {
                spacing: 10;
                label-padding: 5;
                columns: 1;
                label-limit: 0;  /* 0 means no limit */
            }

            legend[type="symbol"] {
                symbol-size: 64;
            }

            legend[type="colorbar"] {
                gradient-thickness: 15;
            }

            legend[type="rect"] {
                symbol-size: 64;
            }

            legend title {
                color: var(--text-color);
                font-weight: 400;
                font-size: 1.0rem; /* 12px @ 12px base */
            }

            legend label {
                color: var(--text-secondary);
                font-weight: 300;
                font-size: 0.917rem; /* 11px @ 12px base */
            }

            legend tick {
                color: var(--text-tertiary);
                font-weight: 300;
                font-size: 0.833rem; /* 10px @ 12px base */
                stroke: var(--border-color);
            }

            legend background {
                padding: 4;
            }

            /* === Facet Elements === */
            facet {
                spacing: 3;
            }

            /* === Mark Defaults === */
            mark[type="symbol"] {
                stroke: var(--categorical-color-0);
                stroke: var(--bg-color);
                stroke-width: 0.5;
                size: 72;
                shape: circle;
                opacity: 1.0;
            }

            mark[type="rect"] {
                stroke: var(--categorical-color-0);
                stroke: var(--bg-color);
                stroke-width: 0.5;
                corner-radius: 0;
                opacity: 1.0;
            }

            mark[type="line"] {
                stroke: var(--categorical-color-0);
                stroke-width: 2.0;
                stroke-dash: solid;
                stroke-cap: round;
                stroke-join: round;
                opacity: 1.0;
            }

            mark[type="text"] {
                fill: var(--text-color);
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

        let mut theme = Self::from_css(css).expect("Failed to parse unified default theme CSS");
        // Default to light mode (will be overridden by light()/dark() methods)
        theme.default_color_scheme = "light".to_string();
        theme
    }

    /// Light theme preset with default colors and styling
    ///
    /// Returns the default adaptive theme with color-scheme set to "light".
    /// The theme uses light-dark() functions internally, so it can be switched
    /// to dark mode at render time by setting the "color-scheme" parameter to "dark".
    pub fn light() -> Self {
        let mut theme = Self::default_theme();
        theme.default_color_scheme = "light".to_string();
        theme
    }

    /// Dark theme preset with dark mode colors
    ///
    /// Returns the default adaptive theme with color-scheme set to "dark".
    /// The theme uses light-dark() functions internally, so it can be switched
    /// to light mode at render time by setting the "color-scheme" parameter to "light".
    pub fn dark() -> Self {
        let mut theme = Self::default_theme();
        theme.default_color_scheme = "dark".to_string();
        theme
    }

    /// Create a theme from CSS string
    pub fn from_css(css: &str) -> Result<Self, String> {
        let rules = parser::parse_stylesheet(css)?;

        // Extract variables from rules
        let mut variables = IndexMap::new();
        let mut compiled_rules = Vec::new();

        for rule in rules {
            // Extract CSS variables
            Self::extract_variables(&rule, &mut variables);

            compiled_rules.push(rule);
        }

        Ok(Self {
            rules: compiled_rules,
            variables,
            inherited_properties: Self::default_inherited_properties(),
            css_sources: vec![css.to_string()],
            default_color_scheme: "light".to_string(), // Default to light mode
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

    /// Extract CSS variables from a rule
    fn extract_variables(rule: &CompiledRule, variables: &mut IndexMap<String, ThemeValue>) {
        for (key, declaration) in &rule.declarations {
            if key.starts_with("--") {
                variables.insert(key.clone(), declaration.value.clone());
            }
        }
    }

    /// Convert a DataFusion ScalarValue to a ThemeValue
    ///
    /// This enables params to be used as CSS variable values.
    fn scalar_to_theme_value(scalar: &datafusion_common::ScalarValue) -> Option<ThemeValue> {
        use datafusion_common::ScalarValue;

        match scalar {
            ScalarValue::Utf8(Some(s)) => {
                // Try parsing as color first
                if let Some(color) = crate::theme::value::parse_color_string(s) {
                    Some(ThemeValue::Color(color))
                } else if let Ok(n) = s.parse::<f64>() {
                    // Try parsing as number
                    Some(ThemeValue::Number(n))
                } else if let Some(length) = crate::theme::value::parse_length_string(s) {
                    // Try parsing as length (e.g., "16px", "1.5rem")
                    Some(length)
                } else {
                    // Keep as string
                    Some(ThemeValue::String(s.clone()))
                }
            }
            ScalarValue::Float32(Some(f)) => Some(ThemeValue::Number(*f as f64)),
            ScalarValue::Float64(Some(f)) => Some(ThemeValue::Number(*f)),
            ScalarValue::Int32(Some(i)) => Some(ThemeValue::Number(*i as f64)),
            ScalarValue::Int64(Some(i)) => Some(ThemeValue::Number(*i as f64)),
            ScalarValue::Boolean(Some(b)) => Some(ThemeValue::Boolean(*b)),
            _ => None,
        }
    }

    /// Recursively resolve a theme value with parameters
    ///
    /// This handles:
    /// - Variable references: checks params first (without `--` prefix), then theme.variables
    /// - light-dark() functions: uses "color-scheme" param to choose branch (default "light")
    /// - Nested structures: recursively resolves until fully expanded
    ///
    /// # Arguments
    /// * `value` - The ThemeValue to resolve
    /// * `params` - Parameter values from the plot/rendering context
    /// * `depth` - Current recursion depth (prevents infinite loops)
    ///
    /// # Returns
    /// Resolved ThemeValue (may still be unresolved if params/variables not found)
    fn resolve_theme_value(
        &self,
        value: ThemeValue,
        params: &IndexMap<String, datafusion_common::ScalarValue>,
        depth: usize,
    ) -> ThemeValue {
        // Prevent infinite recursion
        const MAX_DEPTH: usize = 10;
        if depth >= MAX_DEPTH {
            return value;
        }

        match value {
            ThemeValue::Variable(var_name) => {
                // Variable resolution priority:
                // 1. Check params (with -- prefix to make it clear it's a CSS variable)
                // 2. Fall back to theme.variables

                // Try params first (keep -- prefix)
                if let Some(param_value) = params.get(&var_name) {
                    if let Some(theme_val) = Self::scalar_to_theme_value(param_value) {
                        // Recursively resolve in case param contains another reference
                        return self.resolve_theme_value(theme_val, params, depth + 1);
                    }
                }

                // Fall back to theme variables
                if let Some(var_value) = self.variables.get(&var_name) {
                    // Recursively resolve in case variable contains another reference
                    return self.resolve_theme_value(var_value.clone(), params, depth + 1);
                }

                // Unresolved - return as-is
                ThemeValue::Variable(var_name)
            }

            ThemeValue::LightDark(light, dark) => {
                // Check color-scheme param, fall back to theme's default
                let scheme = params
                    .get("color-scheme")
                    .and_then(|v| match v {
                        datafusion_common::ScalarValue::Utf8(Some(s)) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or(&self.default_color_scheme);

                // Choose branch based on color-scheme
                let chosen = if scheme == "dark" { *dark } else { *light };

                // Recursively resolve the chosen branch
                self.resolve_theme_value(chosen, params, depth + 1)
            }

            // For List values, recursively resolve each element
            ThemeValue::List(values) => ThemeValue::List(
                values
                    .into_iter()
                    .map(|v| self.resolve_theme_value(v, params, depth + 1))
                    .collect(),
            ),

            // For Function values, recursively resolve arguments
            ThemeValue::Function(name, args) => ThemeValue::Function(
                name,
                args.into_iter()
                    .map(|v| self.resolve_theme_value(v, params, depth + 1))
                    .collect(),
            ),

            // All other values pass through unchanged
            _ => value,
        }
    }

    /// Query a CSS property with parameter resolution
    ///
    /// This resolves:
    /// - CSS variables (var()) using params from context or theme defaults
    /// - light-dark() functions using the "color-scheme" param from context
    ///
    /// # Arguments
    /// * `context` - The element context for CSS selector matching (includes params)
    /// * `property` - The CSS property name
    ///
    /// # Returns
    /// Resolved ThemeValue if a matching rule is found, None otherwise
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

        // Determine base font size for rem conversion and media query evaluation
        // Bootstrap case: When querying :root { font-size } itself, use default to avoid recursion
        // Normal case: Query :root { font-size } with full cascade (including media queries)
        let base_font_size = if context.element_type == ":root" && property == "font-size" {
            // Bootstrap: Use constant to avoid infinite recursion
            DEFAULT_BASE_FONT_SIZE
        } else {
            // Normal: Get base font size from :root with media query support
            self.get_base_font_size(&context.params)
        };

        // Find matching rules
        let mut matches = Vec::new();
        for rule in &self.rules {
            // Check media query condition first (if present)
            if let Some(media_cond) = &rule.media_condition {
                if !media_cond.evaluate(&context.params, base_font_size) {
                    continue; // Media query doesn't match, skip rule
                }
            }

            // Then check selector match
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

        // CSS cascade: !important declarations win over non-important declarations
        // Search important declarations first (from highest specificity)
        for rule in matches.iter().rev() {
            if let Some(declaration) = rule.declarations.get(property) {
                if declaration.important {
                    // Resolve the value with params from context
                    return Some(self.resolve_theme_value(
                        declaration.value.clone(),
                        &context.params,
                        0,
                    ));
                }
            }
        }

        // If no important declaration found, search non-important declarations
        for rule in matches.iter().rev() {
            if let Some(declaration) = rule.declarations.get(property) {
                if !declaration.important {
                    // Resolve the value with params from context
                    return Some(self.resolve_theme_value(
                        declaration.value.clone(),
                        &context.params,
                        0,
                    ));
                }
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

    /// Append additional CSS rules to the theme
    pub fn append_css(&mut self, css: &str) -> Result<(), String> {
        // Parse the new CSS
        let new_rules = parser::parse_stylesheet(css)?;

        // Get current max source_order
        let current_max_order = self.rules.iter().map(|r| r.source_order).max().unwrap_or(0);

        // Add new rules with updated source_order
        for (i, mut rule) in new_rules.into_iter().enumerate() {
            rule.source_order = current_max_order + i + 1;

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

    /// Get the base font size with parameter and media query support
    ///
    /// Resolves the base font size by:
    /// 1. Querying :root { font-size } with full CSS cascade (including media queries)
    /// 2. Falling back to DEFAULT_BASE_FONT_SIZE (12px)
    ///
    /// This method properly evaluates media queries, so font-size changes in
    /// responsive contexts will cascade to all rem-based values.
    ///
    /// Uses a bootstrap default (DEFAULT_BASE_FONT_SIZE) internally when evaluating
    /// media query conditions to avoid circular dependencies.
    pub fn get_base_font_size(
        &self,
        params: &IndexMap<String, datafusion_common::ScalarValue>,
    ) -> f32 {
        // Create a :root context to query font-size
        let root_context = ThemeContext::new(":root", params.clone());

        // Query :root { font-size } which will evaluate media queries
        // Note: query() will use DEFAULT_BASE_FONT_SIZE internally to avoid recursion
        if let Some(font_size_value) = self.query(&root_context, "font-size") {
            // Convert the value to pixels
            // Use DEFAULT_BASE_FONT_SIZE for any rem units in the value itself
            // (though typically :root font-size is in px)
            if let Some(size) = font_size_value.as_font_size(params, DEFAULT_BASE_FONT_SIZE) {
                return size;
            }
        }

        // Fall back to default
        DEFAULT_BASE_FONT_SIZE
    }

    /// Build a legend context with optional subtype
    pub fn legend_context(&self, subtype: Option<&str>) -> ThemeContext {
        self.legend_context_with_params(subtype, IndexMap::new())
    }

    /// Build a legend context with optional subtype and params
    pub fn legend_context_with_params(
        &self,
        subtype: Option<&str>,
        params: IndexMap<String, datafusion_common::ScalarValue>,
    ) -> ThemeContext {
        let mut legend_ctx = ThemeContext::new("legend", params);
        if let Some(t) = subtype {
            legend_ctx = legend_ctx.with_subtype(t);
        }
        legend_ctx
    }

    /// Check if there are media queries affecting a specific property for an element
    pub fn has_media_queries_for_property(&self, context: &ThemeContext, property: &str) -> bool {
        use crate::theme::element::CssElement;

        // Convert ThemeContext to CssElement for selector matching
        let element = CssElement::from(context);
        let mut selector_caches = selectors::matching::SelectorCaches::default();
        let mut matching_context = selectors::context::MatchingContext::new(
            selectors::context::MatchingMode::Normal,
            None,
            &mut selector_caches,
            selectors::context::QuirksMode::NoQuirks,
            selectors::context::NeedsSelectorFlags::No,
            selectors::context::MatchingForInvalidation::No,
        );

        // Check if any rule with media conditions matches this element and has the property
        for rule in &self.rules {
            if rule.media_condition.is_some() {
                let is_match = selectors::matching::matches_selector(
                    &rule.selector,
                    0,
                    None,
                    &element,
                    &mut matching_context,
                );

                if is_match && rule.declarations.contains_key(property) {
                    return true;
                }
            }
        }

        false
    }

    /// Build an axis context with optional coordinate and axis types
    pub fn axis_context(&self, coord_type: Option<&str>, axis_type: Option<&str>) -> ThemeContext {
        self.axis_context_with_params(coord_type, axis_type, IndexMap::new())
    }

    /// Build an axis context with optional coordinate and axis types and params
    pub fn axis_context_with_params(
        &self,
        coord_type: Option<&str>,
        axis_type: Option<&str>,
        params: IndexMap<String, datafusion_common::ScalarValue>,
    ) -> ThemeContext {
        let mut guide_ctx = ThemeContext::new("guide", params);
        if let Some(ct) = coord_type {
            guide_ctx = guide_ctx.with_subtype(ct);
        }
        let mut axis_ctx = guide_ctx.child("axis");
        if let Some(at) = axis_type {
            axis_ctx = axis_ctx.with_subtype(at);
        }
        axis_ctx
    }

    /// Build a title context
    pub fn title_context(&self) -> ThemeContext {
        self.title_context_with_params(IndexMap::new())
    }

    /// Build a title context with params
    pub fn title_context_with_params(
        &self,
        params: IndexMap<String, datafusion_common::ScalarValue>,
    ) -> ThemeContext {
        ThemeContext::new("chart-title", params)
    }

    /// Build a subtitle context
    pub fn subtitle_context(&self) -> ThemeContext {
        self.subtitle_context_with_params(IndexMap::new())
    }

    /// Build a subtitle context with params
    pub fn subtitle_context_with_params(
        &self,
        params: IndexMap<String, datafusion_common::ScalarValue>,
    ) -> ThemeContext {
        ThemeContext::new("chart-subtitle", params)
    }

    /// Build a facet context
    pub fn facet_context(&self) -> ThemeContext {
        self.facet_context_with_params(IndexMap::new())
    }

    /// Build a facet context with params
    pub fn facet_context_with_params(
        &self,
        params: IndexMap<String, datafusion_common::ScalarValue>,
    ) -> ThemeContext {
        ThemeContext::new("facet", params)
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

    /// Get font size for a context with parameter support
    ///
    /// This allows overriding the base font size via the "base-font-size" parameter.
    /// If the parameter is provided, it will be used instead of the CSS-defined base font size.
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        let base_font_size = self.get_base_font_size(&context.params);

        self.query(context, "font-size")
            .and_then(|v| v.as_font_size(&context.params, base_font_size))
    }

    /// Get font weight for a context
    /// Get font-weight for a context, converting CSS keywords to numeric values
    pub fn font_weight(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "font-weight").and_then(|v| {
            match v {
                ThemeValue::Number(n) => Some(n as f32),
                ThemeValue::String(s) => {
                    // Convert CSS font-weight keywords to numeric values
                    match s.to_lowercase().as_str() {
                        "thin" => Some(100.0),
                        "hairline" => Some(100.0),
                        "extralight" | "extra-light" | "ultra-light" | "ultralight" => Some(200.0),
                        "light" => Some(300.0),
                        "normal" | "regular" => Some(400.0),
                        "medium" => Some(500.0),
                        "semibold" | "semi-bold" | "demi-bold" | "demibold" => Some(600.0),
                        "bold" => Some(700.0),
                        "extrabold" | "extra-bold" | "ultra-bold" | "ultrabold" => Some(800.0),
                        "black" | "heavy" => Some(900.0),
                        "extra-black" | "ultra-black" => Some(950.0),
                        // Try parsing as number if not a keyword
                        _ => s.parse::<f32>().ok(),
                    }
                }
                _ => None,
            }
        })
    }

    /// Get text-align for a context
    pub fn text_align(&self, context: &ThemeContext) -> Option<String> {
        self.query(context, "text-align")
            .and_then(|v| v.as_string().map(|s| s.to_string()))
    }

    /// Get color for a context as normalized RGBA array
    /// Supports relative color syntax with runtime parameters
    pub fn text_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        let base_font_size = self.get_base_font_size(&context.params);

        // Ensure color-scheme param is set (use theme default if not provided)
        let mut params = context.params.clone();
        if !params.contains_key("color-scheme") {
            params.insert(
                "color-scheme".to_string(),
                datafusion_common::ScalarValue::Utf8(Some(self.default_color_scheme.clone())),
            );
        }

        // Create updated context with color-scheme param for query
        let mut context_with_params = context.clone();
        context_with_params.params = params.clone();

        self.query(&context_with_params, "color").and_then(|v| {
            v.as_color_with_params(&params, base_font_size)
                .map(|css_rgba| css_rgba.to_array())
        })
    }

    /// Get fill color for a context as normalized RGBA array
    /// Supports relative color syntax with runtime parameters
    pub fn fill_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        let base_font_size = self.get_base_font_size(&context.params);

        // Ensure color-scheme param is set (use theme default if not provided)
        let mut params = context.params.clone();
        if !params.contains_key("color-scheme") {
            params.insert(
                "color-scheme".to_string(),
                datafusion_common::ScalarValue::Utf8(Some(self.default_color_scheme.clone())),
            );
        }

        // Create updated context with color-scheme param for query
        let mut context_with_params = context.clone();
        context_with_params.params = params.clone();

        self.query(&context_with_params, "fill").and_then(|v| {
            v.as_color_with_params(&params, base_font_size)
                .map(|css_rgba| css_rgba.to_array())
        })
    }

    /// Get stroke color for a context as normalized RGBA array
    /// Supports relative color syntax with runtime parameters
    pub fn stroke_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        let base_font_size = self.get_base_font_size(&context.params);

        // Ensure color-scheme param is set (use theme default if not provided)
        let mut params = context.params.clone();
        if !params.contains_key("color-scheme") {
            params.insert(
                "color-scheme".to_string(),
                datafusion_common::ScalarValue::Utf8(Some(self.default_color_scheme.clone())),
            );
        }

        // Create updated context with color-scheme param for query
        let mut context_with_params = context.clone();
        context_with_params.params = params.clone();

        self.query(&context_with_params, "stroke").and_then(|v| {
            v.as_color_with_params(&params, base_font_size)
                .map(|css_rgba| css_rgba.to_array())
        })
    }

    /// Get stroke width for a context
    pub fn stroke_width(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "stroke-width")
            .and_then(|v| v.as_font_size(&context.params, self.get_base_font_size(&context.params)))
    }

    /// Get opacity for a context
    pub fn opacity(&self, context: &ThemeContext) -> Option<f32> {
        self.query(context, "opacity")
            .and_then(|v| v.as_number())
            .map(|n| n as f32)
    }

    // Guide (coordinate system) theme methods

    /// Get axis grid width
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_grid_width(&self, axis_ctx: &ThemeContext) -> Option<f32> {
        let ctx = axis_ctx.child("grid");
        self.stroke_width(&ctx)
    }

    /// Get axis tick length
    ///
    /// # Arguments
    /// * `coord_type` - Optional coordinate system type (e.g., "cartesian", "polar")
    /// * `axis_type` - Optional axis subtype (e.g., "x", "y", "r", "theta")
    pub fn axis_tick_length(&self, axis_ctx: &ThemeContext) -> Option<f32> {
        let ctx = axis_ctx.child("tick");
        self.query(&ctx, "size")
            .and_then(|v| v.as_font_size(&ctx.params, self.get_base_font_size(&ctx.params)))
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
        params: &IndexMap<String, datafusion_common::ScalarValue>,
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

        // Use provided params, ensuring color-scheme is set for light-dark() resolution
        let mut params = params.clone();
        if !params.contains_key("color-scheme") {
            params.insert(
                "color-scheme".to_string(),
                datafusion_common::ScalarValue::Utf8(Some(self.default_color_scheme.clone())),
            );
        }

        // Try mark-specific first, then general mark
        let base_contexts = vec![
            ThemeContext::new("mark", params.clone()).with_subtype(mark_type),
            ThemeContext::new("mark", params),
        ];

        // For discrete ranges, try cardinality-specific values with fallback logic
        if let (RangeKind::Discrete, Some(cardinality)) = (range_kind, domain_cardinality) {
            for base_context in &base_contexts {
                // First get the base range (without cardinality) for comparison
                let base_range_value = self.query(base_context, &property);

                // Try cardinalities from 1 upward, keeping track of best match
                // We want the SMALLEST cardinality >= requested to minimize cycling
                // If no such palette exists, fall back to LARGEST < requested
                let mut best_card: Option<usize> = None;
                let mut largest_below: Option<usize> = None;
                // Check up to a reasonable maximum (e.g., 2x the requested cardinality)
                let max_check = cardinality * 2;
                for card in 1..=max_check {
                    let context_with_card = base_context
                        .clone()
                        .with_attribute("cardinality", card.to_string());
                    let card_range_value = self.query(&context_with_card, &property);

                    // Only consider this if it's different from base (meaning cardinality attribute matched)
                    if card_range_value.is_some() && card_range_value != base_range_value {
                        if card >= cardinality {
                            // Take the first (smallest) cardinality >= requested
                            best_card = Some(card);
                            break; // Found smallest match, stop searching
                        } else {
                            // Track largest cardinality < requested as fallback
                            largest_below = Some(card);
                        }
                    }
                }

                // If no cardinality >= requested, use largest < requested
                if best_card.is_none() {
                    best_card = largest_below;
                }

                // Use the best cardinality match if we found one
                if let Some(best) = best_card {
                    let context_with_best = base_context
                        .clone()
                        .with_attribute("cardinality", best.to_string());
                    // Use the selected cardinality (best) not the requested one to get full palette
                    if let Some(range) = self.try_get_range(
                        &context_with_best,
                        &property,
                        channel,
                        range_kind,
                        Some(best), // Use selected cardinality, not requested
                    ) {
                        return Some(range);
                    }
                }

                // Fall back to base palette (no cardinality attribute)
                if let Some(range) = self.try_get_range(
                    base_context,
                    &property,
                    channel,
                    range_kind,
                    domain_cardinality,
                ) {
                    return Some(range);
                }
            }
        } else {
            // For continuous ranges or when cardinality is unknown, just try without cardinality
            for context in &base_contexts {
                if let Some(range) =
                    self.try_get_range(context, &property, channel, range_kind, domain_cardinality)
                {
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
                let base_font_size = self.get_base_font_size(&context.params);

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
                        // Handle color functions (color-mix, contrast-color, light-dark, etc.)
                        ThemeValue::Function(_, _) | ThemeValue::LightDark(_, _) => {
                            // Resolve the function to a color
                            if let Some(rgba) =
                                val.as_color_with_params(&context.params, base_font_size)
                            {
                                let hex =
                                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue);
                                parsed_values.push(hex);
                            }
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
    /// Returns the default value for a specific mark type and channel using type-directed evaluation.
    ///
    /// This method uses the new architecture where:
    /// 1. The expected type is determined from the CSS property name
    /// 2. The theme value is evaluated to that type using eval_as_*() methods
    /// 3. All ThemeValue variants are properly handled (including Calc, RelativeColor, etc.)
    pub fn mark_default(
        &self,
        mark_type: &str,
        channel: &str,
        params: &IndexMap<String, datafusion_common::ScalarValue>,
    ) -> Option<datafusion_common::ScalarValue> {
        use crate::theme::eval::{EvalContext, TargetType, get_channel_type};

        // Query CSS theme for mark defaults
        let context = ThemeContext::new("mark", params.clone()).with_subtype(mark_type);

        // Convert underscore to hyphen for CSS property name
        let css_property = channel.replace('_', "-");

        let theme_value = self.query(&context, &css_property)?;

        // Determine expected type from channel name
        let channel_type = get_channel_type(&css_property)?;

        // Create evaluation context
        let base_font_size = self.get_base_font_size(params);
        let eval_ctx = EvalContext::new(params, base_font_size);

        // Evaluate based on expected type
        match channel_type {
            TargetType::Color => {
                // Evaluate as color and convert to string
                let rgba = theme_value.eval_as_color(&eval_ctx).ok()?;
                let color_str = if rgba.alpha == 255 {
                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue)
                } else {
                    let alpha = rgba.alpha as f32 / 255.0;
                    format!(
                        "rgba({}, {}, {}, {})",
                        rgba.red, rgba.green, rgba.blue, alpha
                    )
                };
                Some(datafusion_common::ScalarValue::Utf8(Some(color_str)))
            }

            TargetType::Number => {
                // Evaluate as number and convert to Float32
                let n = theme_value.eval_as_number(&eval_ctx).ok()?;
                Some(datafusion_common::ScalarValue::Float32(Some(n as f32)))
            }

            TargetType::Length => {
                // Evaluate as length (in pixels) and convert to Float32
                let px = theme_value.eval_as_length(&eval_ctx).ok()?;
                Some(datafusion_common::ScalarValue::Float32(Some(px as f32)))
            }

            TargetType::String => {
                // Evaluate as string
                let s = theme_value.eval_as_string(&eval_ctx).ok()?;
                Some(datafusion_common::ScalarValue::Utf8(Some(s)))
            }

            TargetType::Boolean => {
                // Try to get as boolean directly
                match theme_value {
                    ThemeValue::Boolean(b) => {
                        Some(datafusion_common::ScalarValue::Boolean(Some(b)))
                    }
                    _ => None,
                }
            }

            TargetType::Angle => {
                // Evaluate as number (degrees) and convert to Float32
                let degrees = theme_value.eval_as_number(&eval_ctx).ok()?;
                Some(datafusion_common::ScalarValue::Float32(Some(
                    degrees as f32,
                )))
            }

            TargetType::Percentage => {
                // Evaluate as number (decimal) and convert to Float32
                let decimal = theme_value.eval_as_number(&eval_ctx).ok()?;
                Some(datafusion_common::ScalarValue::Float32(Some(
                    decimal as f32,
                )))
            }
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

/// Helper struct for Theme serialization
#[derive(Serialize, Deserialize)]
struct ThemeSerializationHelper {
    css: String,
    default_color_scheme: String,
}

impl Serialize for Theme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize both CSS and default_color_scheme
        let helper = ThemeSerializationHelper {
            css: self.to_css(),
            default_color_scheme: self.default_color_scheme.clone(),
        };
        helper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = ThemeSerializationHelper::deserialize(deserializer)?;
        let mut theme = Theme::from_css(&helper.css).map_err(serde::de::Error::custom)?;
        theme.default_color_scheme = helper.default_color_scheme;
        Ok(theme)
    }
}

/// A CSS declaration with its value and importance flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Declaration {
    pub(crate) value: ThemeValue,
    pub(crate) important: bool,
}

impl Declaration {
    pub fn new(value: ThemeValue, important: bool) -> Self {
        Self { value, important }
    }
}

/// A compiled CSS rule with selector and declarations
#[derive(Debug, Clone)]
pub(crate) struct CompiledRule {
    pub(crate) selector: selectors::parser::Selector<crate::theme::selector_impl::ChartSelectors>,
    pub(crate) specificity: u32,
    pub(crate) source_order: usize,
    pub(crate) declarations: IndexMap<String, Declaration>,
    pub(crate) media_condition: Option<crate::theme::media_query::MediaCondition>,
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
        assert_eq!(
            theme.get_base_font_size(&IndexMap::new()),
            deserialized.get_base_font_size(&IndexMap::new())
        );

        // Verify the theme works correctly after deserialization
        let context = crate::theme::ThemeContext {
            element_type: "axis".to_string(),
            subtype: Some("domain".to_string()),
            classes: vec![],
            id: None,
            attributes: std::collections::HashMap::new(),
            parent: None,
            params: IndexMap::new(),
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
            params: IndexMap::new(),
        };

        // Test querying a property that exists (stroke)
        let value = deserialized.query(&context, "stroke");
        // Should get a value (variable reference to --categorical-color-0 or --bg-color)
        assert!(value.is_some(), "Should have stroke value");

        // Also test that base_font_size is preserved
        assert_eq!(deserialized.get_base_font_size(&IndexMap::new()), 12.0);
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
        let ctx = ThemeContext::new("mark", IndexMap::new());
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
            params: IndexMap::new(),
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
            params: IndexMap::new(),
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
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 16.0);

        // Check that rem values are calculated correctly
        let context = ThemeContext::new("mark", IndexMap::new());
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
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 12.0);
    }

    #[test]
    fn test_color_mix_parsing_error() {
        // First test: Does Theme::from_css succeed with color-mix?
        let css = r#"
            mark {
                fill-discrete: color-mix(in srgb, red, blue);
            }
        "#;

        match Theme::from_css(css) {
            Ok(theme) => {
                println!("Theme created successfully");
                let ctx = ThemeContext::new("mark", IndexMap::new());
                let value = theme.query(&ctx, "fill-discrete");
                println!("fill-discrete value: {:?}", value);
            }
            Err(e) => {
                println!("Failed to parse CSS: {}", e);
                panic!("CSS parsing failed: {}", e);
            }
        }
    }

    #[test]
    fn test_color_mix_in_fill_discrete() {
        let css = r#"
            mark {
                fill-discrete:
                    color-mix(in srgb, red, blue),
                    color-mix(in srgb, red 75%, blue 25%);
            }
        "#;

        let theme = Theme::from_css(css).unwrap();
        let ctx = ThemeContext::new("mark", IndexMap::new());
        let value = theme.query(&ctx, "fill-discrete");

        println!("fill-discrete value: {:?}", value);

        match value {
            Some(ThemeValue::List(items)) => {
                println!("Got a list with {} items", items.len());
                for (i, item) in items.iter().enumerate() {
                    println!("  Item {}: {:?}", i, item);
                }
                assert_eq!(items.len(), 2, "Should have 2 color-mix results");

                // Both should be Function values (deferred resolution)
                for item in &items {
                    assert!(
                        matches!(item, ThemeValue::Function(name, _) if name == "color-mix"),
                        "Each item should be a color-mix Function, got {:?}",
                        item
                    );
                }

                // Verify they can be resolved to colors with params
                let params = IndexMap::new();
                let base_font_size = theme.get_base_font_size(&params);
                for item in &items {
                    let color = item.as_color_with_params(&params, base_font_size);
                    assert!(color.is_some(), "color-mix should resolve to a color");
                }
            }
            other => panic!("Expected List, got {:?}", other),
        }
    }

    #[test]
    fn test_color_mix_in_stroke_with_var() {
        // Test that color-mix() with var() works in stroke property
        let css = r#"
            mark {
                fill: var(--accent);
                stroke: color-mix(in srgb, var(--accent) 60%, black);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        let mut params = IndexMap::new();
        params.insert(
            "--accent".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("#2563eb".into())),
        );

        let ctx = ThemeContext::new("mark", params.clone());

        let fill = theme.fill_color(&ctx);
        assert!(fill.is_some(), "fill with var(--accent) should work");

        let stroke = theme.stroke_color(&ctx);
        assert!(
            stroke.is_some(),
            "stroke with color-mix(var(--accent)) should work"
        );

        // Verify the stroke is darker than fill (mixed with black)
        let fill_rgba = fill.unwrap();
        let stroke_rgba = stroke.unwrap();
        assert!(
            stroke_rgba[0] < fill_rgba[0],
            "Stroke red component should be darker (less than fill)"
        );

        // Verify color-mix computed correctly: color-mix(in srgb, #2563eb 60%, black 40%)
        // #2563eb = rgb(37, 99, 235) => 60% should give approximately (0.087, 0.233, 0.553)
        assert!(
            (stroke_rgba[0] - 0.087).abs() < 0.02,
            "Red should be ~0.087, got {}",
            stroke_rgba[0]
        );
        assert!(
            (stroke_rgba[1] - 0.233).abs() < 0.02,
            "Green should be ~0.233, got {}",
            stroke_rgba[1]
        );
        assert!(
            (stroke_rgba[2] - 0.553).abs() < 0.02,
            "Blue should be ~0.553, got {}",
            stroke_rgba[2]
        );
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
        let range_3 = theme.get_range_for_channel(
            "symbol",
            "fill",
            RangeKind::Discrete,
            Some(3),
            &IndexMap::new(),
        );
        assert!(range_3.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_3 {
            assert_eq!(values.len(), 3, "Should get 3-color palette");
        } else {
            panic!("Expected discrete range");
        }

        // Test 2: Exact match for cardinality 5
        let range_5 = theme.get_range_for_channel(
            "symbol",
            "fill",
            RangeKind::Discrete,
            Some(5),
            &IndexMap::new(),
        );
        assert!(range_5.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_5 {
            assert_eq!(values.len(), 5, "Should get 5-color palette");
        } else {
            panic!("Expected discrete range");
        }

        // Test 3: Use smallest available cardinality >= requested (5) when requesting 4
        // Since we have cardinality-specific rules for 2, 3, and 5, requesting 4 should
        // use 5 (the smallest cardinality >= 4) to minimize cycling
        let range_4 = theme.get_range_for_channel(
            "symbol",
            "fill",
            RangeKind::Discrete,
            Some(4),
            &IndexMap::new(),
        );
        assert!(range_4.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_4 {
            assert_eq!(
                values.len(),
                5,
                "Should use 5-color palette (smallest >= 4) to avoid cycling"
            );
        } else {
            panic!("Expected discrete range");
        }

        // Test 4: Use largest available cardinality (5) when requesting 10
        // Since we have no exact match and the largest defined is 5, use that
        let range_10 = theme.get_range_for_channel(
            "symbol",
            "fill",
            RangeKind::Discrete,
            Some(10),
            &IndexMap::new(),
        );
        assert!(range_10.is_some());
        if let Some(ScaleRange::Discrete(values)) = range_10 {
            assert_eq!(
                values.len(),
                5,
                "Should use largest available (5-color palette)"
            );
        } else {
            panic!("Expected discrete range");
        }

        // Test 5: When cardinality is unknown, should get base palette
        let range_none = theme.get_range_for_channel(
            "symbol",
            "fill",
            RangeKind::Discrete,
            None,
            &IndexMap::new(),
        );
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
            light.get_base_font_size(&IndexMap::new()),
            12.0,
            "Light theme should have 12px base"
        );

        let dark = Theme::dark();
        assert_eq!(
            dark.get_base_font_size(&IndexMap::new()),
            12.0,
            "Dark theme should have 12px base"
        );
    }

    #[test]
    fn test_append_css_with_new_base_font_size() {
        // Start with the default light theme (12px base)
        let mut theme = Theme::light();
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 12.0);

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
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 18.0);

        // Verify rem calculations use the new base
        let context = ThemeContext::new("test-element", IndexMap::new());
        let font_size = theme.font_size(&context);
        assert_eq!(font_size, Some(36.0)); // 2rem * 18px = 36px

        // Also verify existing elements that use rem units are recalculated with new base
        let title_ctx = theme.title_context();
        let title_size = theme.font_size(&title_ctx);
        assert_eq!(title_size, Some(27.0)); // 1.5rem * 18px = 27px (was 18px with 12px base)

        // Verify serialization preserves the updated base font size
        let json = serde_json::to_string(&theme).unwrap();
        let deserialized: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.get_base_font_size(&IndexMap::new()), 18.0);
    }

    // ============================================================
    // Tests for light-dark() and parameter resolution
    // ============================================================

    #[test]
    fn test_parse_light_dark_basic() {
        // Test basic light-dark() parsing
        let css = r#"
            mark {
                fill: light-dark(#ffffff, #000000);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");
        let ctx = ThemeContext::new("mark", IndexMap::new());

        // Query without params - should default to light mode
        let value = theme.query(&ctx, "fill");
        println!("Basic light-dark value: {:?}", value);
        match value {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 255);
                assert_eq!(c.green, 255);
                assert_eq!(c.blue, 255);
            }
            _ => panic!("Expected color value from light-dark(), got: {:?}", value),
        }
    }

    #[test]
    fn test_light_dark_with_one_var() {
        // Test light-dark() with one var() argument
        let css = r#"
            :root {
                --light-bg: #ffffff;
            }
            mark {
                fill: light-dark(var(--light-bg), #000000);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");
        println!("One var theme variables: {:?}", theme.variables);

        let ctx = ThemeContext::new("mark", IndexMap::new());
        let raw_value = theme.query(&ctx, "fill");
        println!("One var fill value: {:?}", raw_value);

        // Should resolve to white in light mode
        match raw_value {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 255);
            }
            _ => panic!("Expected resolved color, got: {:?}", raw_value),
        }
    }

    #[test]
    fn test_light_dark_with_color_scheme_param() {
        let css = r#"
            mark {
                fill: light-dark(#e8f4f8, #1a2332);
                stroke: light-dark(#333333, #cccccc);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Test with light mode
        let mut params_light = IndexMap::new();
        params_light.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("light".to_string())),
        );
        let ctx_light = ThemeContext::new("mark", params_light);

        let fill_light = theme.query(&ctx_light, "fill");
        match fill_light {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 232); // #e8f4f8
                assert_eq!(c.green, 244);
                assert_eq!(c.blue, 248);
            }
            _ => panic!("Expected light color"),
        }

        // Test with dark mode
        let mut params_dark = IndexMap::new();
        params_dark.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let ctx_dark = ThemeContext::new("mark", params_dark);

        let fill_dark = theme.query(&ctx_dark, "fill");
        match fill_dark {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 26); // #1a2332
                assert_eq!(c.green, 35);
                assert_eq!(c.blue, 50);
            }
            _ => panic!("Expected dark color"),
        }

        let stroke_dark = theme.query(&ctx_dark, "stroke");
        match stroke_dark {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 204); // #cccccc
                assert_eq!(c.green, 204);
                assert_eq!(c.blue, 204);
            }
            _ => panic!("Expected dark stroke color"),
        }
    }

    #[test]
    fn test_css_variable_override_with_params() {
        let css = r#"
            :root {
                --accent: #4682b4;
            }
            mark {
                fill: var(--accent);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");
        let ctx = ThemeContext::new("mark", IndexMap::new());

        // Without params - should use theme variable
        let value_default = theme.query(&ctx, "fill");
        match value_default {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 70); // #4682b4
                assert_eq!(c.green, 130);
                assert_eq!(c.blue, 180);
            }
            _ => panic!("Expected default accent color"),
        }

        // With param override (use -- prefix to match CSS variable name)
        let mut params = IndexMap::new();
        params.insert(
            "--accent".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("#ff6b6b".to_string())),
        );

        let mut ctx_with_params = ctx.clone();
        ctx_with_params.params = params;
        let value_override = theme.query(&ctx_with_params, "fill");
        match value_override {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 255); // #ff6b6b
                assert_eq!(c.green, 107);
                assert_eq!(c.blue, 107);
            }
            _ => panic!("Expected overridden accent color"),
        }
    }

    #[test]
    fn test_nested_light_dark_with_variables() {
        // Test light-dark() containing var() references
        let css = r#"
            :root {
                --light-bg: #ffffff;
                --dark-bg: #000000;
            }
            mark {
                fill: light-dark(var(--light-bg), var(--dark-bg));
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");
        println!("Theme variables: {:?}", theme.variables);

        let ctx = ThemeContext::new("mark", IndexMap::new());

        // First check what raw value is stored (before param resolution)
        let raw_value = theme.query(&ctx, "fill");
        println!("Raw fill value: {:?}", raw_value);

        // Test light mode
        let mut params_light = IndexMap::new();
        params_light.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("light".to_string())),
        );
        let ctx_light = ThemeContext::new("mark", params_light);

        let value_light = theme.query(&ctx_light, "fill");
        println!("Light value: {:?}", value_light);
        match value_light {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 255);
                assert_eq!(c.green, 255);
                assert_eq!(c.blue, 255);
            }
            _ => panic!(
                "Expected white from nested light-dark() + var(), got: {:?}",
                value_light
            ),
        }

        // Test dark mode
        let mut params_dark = IndexMap::new();
        params_dark.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let ctx_dark = ThemeContext::new("mark", params_dark);

        let value_dark = theme.query(&ctx_dark, "fill");
        match value_dark {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 0);
                assert_eq!(c.green, 0);
                assert_eq!(c.blue, 0);
            }
            _ => panic!("Expected black from nested light-dark() + var()"),
        }
    }

    #[test]
    fn test_nested_variables_and_light_dark_with_param_overrides() {
        // Complex test: light-dark() with vars, and params override both vars and color-scheme
        let css = r#"
            :root {
                --primary: #3498db;
                --secondary: #e74c3c;
            }
            mark {
                fill: light-dark(var(--primary), var(--secondary));
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Override both color-scheme AND variables
        let mut params = IndexMap::new();
        params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        params.insert(
            "--primary".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("#ff0000".to_string())),
        );
        params.insert(
            "--secondary".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("#00ff00".to_string())),
        );
        let ctx = ThemeContext::new("mark", params);

        let value = theme.query(&ctx, "fill");
        match value {
            Some(ThemeValue::Color(c)) => {
                // Should resolve to dark mode (secondary), which is overridden to green
                assert_eq!(c.red, 0);
                assert_eq!(c.green, 255);
                assert_eq!(c.blue, 0);
            }
            _ => panic!("Expected param-overridden color"),
        }
    }

    #[test]
    fn test_light_dark_in_list() {
        // Test light-dark() within a list (for discrete ranges)
        let css = r#"
            mark {
                fill-discrete:
                    light-dark(#e8f4f8, #1a2332),
                    light-dark(#f0e442, #d55e00),
                    light-dark(#009e73, #56b4e9);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Test with dark mode
        let mut params = IndexMap::new();
        params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let ctx = ThemeContext::new("mark", params);

        let value = theme.query(&ctx, "fill-discrete");
        match value {
            Some(ThemeValue::List(colors)) => {
                assert_eq!(colors.len(), 3);

                // Check first color is resolved to dark variant
                match &colors[0] {
                    ThemeValue::Color(c) => {
                        assert_eq!(c.red, 26); // #1a2332
                    }
                    _ => panic!("Expected first color to be resolved"),
                }
            }
            _ => panic!("Expected list of colors"),
        }
    }

    #[test]
    fn test_scalar_to_theme_value_conversion() {
        // Test that various ScalarValue types convert correctly
        use datafusion_common::ScalarValue;

        // Test color string
        let color_scalar = ScalarValue::Utf8(Some("#ff0000".to_string()));
        let color_val = Theme::scalar_to_theme_value(&color_scalar);
        match color_val {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 255);
                assert_eq!(c.green, 0);
                assert_eq!(c.blue, 0);
            }
            _ => panic!("Expected color from string"),
        }

        // Test number string
        let num_str_scalar = ScalarValue::Utf8(Some("42.5".to_string()));
        let num_str_val = Theme::scalar_to_theme_value(&num_str_scalar);
        match num_str_val {
            Some(ThemeValue::Number(n)) => assert_eq!(n, 42.5),
            _ => panic!("Expected number from string"),
        }

        // Test plain string
        let str_scalar = ScalarValue::Utf8(Some("hello".to_string()));
        let str_val = Theme::scalar_to_theme_value(&str_scalar);
        match str_val {
            Some(ThemeValue::String(s)) => assert_eq!(s, "hello"),
            _ => panic!("Expected string"),
        }

        // Test numeric types
        let float32_scalar = ScalarValue::Float32(Some(3.14));
        match Theme::scalar_to_theme_value(&float32_scalar) {
            Some(ThemeValue::Number(n)) => assert!((n - 3.14).abs() < 0.001),
            _ => panic!("Expected number from Float32"),
        }

        let int32_scalar = ScalarValue::Int32(Some(42));
        match Theme::scalar_to_theme_value(&int32_scalar) {
            Some(ThemeValue::Number(n)) => assert_eq!(n, 42.0),
            _ => panic!("Expected number from Int32"),
        }

        // Test boolean
        let bool_scalar = ScalarValue::Boolean(Some(true));
        match Theme::scalar_to_theme_value(&bool_scalar) {
            Some(ThemeValue::Boolean(b)) => assert!(b),
            _ => panic!("Expected boolean"),
        }
    }

    #[test]
    fn test_param_with_double_dash_prefix() {
        // Verify that params must use -- prefix to match CSS variables
        let css = r#"
            :root {
                --my-var: #ff0000;
            }
            mark {
                fill: var(--my-var);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Param named "--my-var" (with --) should override "--my-var"
        let mut params = IndexMap::new();
        params.insert(
            "--my-var".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("#00ff00".to_string())),
        );

        let ctx = ThemeContext::new("mark", params);

        let value = theme.query(&ctx, "fill");
        match value {
            Some(ThemeValue::Color(c)) => {
                assert_eq!(c.red, 0);
                assert_eq!(c.green, 255);
                assert_eq!(c.blue, 0);
            }
            _ => panic!("Expected overridden color"),
        }
    }

    #[test]
    fn test_max_recursion_depth() {
        // Test that deeply nested resolution doesn't cause stack overflow
        let css = r#"
            :root {
                --a: var(--b);
                --b: var(--c);
                --c: var(--d);
                --d: var(--e);
                --e: var(--f);
                --f: var(--g);
                --g: var(--h);
                --h: var(--i);
                --i: var(--j);
                --j: var(--k);
                --k: #ff0000;
            }
            mark {
                fill: var(--a);
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");
        let ctx = ThemeContext::new("mark", IndexMap::new());

        // This should not panic, but may not fully resolve due to depth limit
        let value = theme.query(&ctx, "fill");
        // The resolution will hit the depth limit, but shouldn't crash
        assert!(value.is_some());
    }

    #[test]
    fn test_unified_theme_light_and_dark_modes() {
        // Test that the unified theme works in both light and dark modes
        // This demonstrates the key feature: compile once, render in different modes

        // Create theme with light-dark() functions
        let theme = Theme::light(); // Uses default unified theme
        let base_font_size = 16.0;

        // Test light mode - check background color which uses light-dark()
        let mut light_params = IndexMap::new();
        light_params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("light".to_string())),
        );
        let ctx_light = ThemeContext::new("canvas", light_params.clone());

        let bg_value = theme.query(&ctx_light, "background-color");
        let bg_light = bg_value.and_then(|v| v.as_color_with_params(&light_params, base_font_size));

        match bg_light {
            Some(color) => {
                // Should be white in light mode
                assert_eq!(color.red, 255, "Light mode should have white background");
                assert_eq!(color.green, 255);
                assert_eq!(color.blue, 255);
            }
            _ => panic!("Expected light mode background color"),
        }

        // Test that Theme::dark() defaults to dark mode
        let dark_theme = Theme::dark();
        let mut dark_params = IndexMap::new();
        dark_params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let ctx_dark = ThemeContext::new("canvas", dark_params.clone());

        let bg_value_dark = dark_theme.query(&ctx_dark, "background-color");
        let bg_dark =
            bg_value_dark.and_then(|v| v.as_color_with_params(&dark_params, base_font_size));

        match bg_dark {
            Some(color) => {
                // Should be dark in dark mode (#121212 = rgb(18, 18, 18))
                assert_eq!(color.red, 18, "Dark mode should have dark background");
                assert_eq!(color.green, 18);
                assert_eq!(color.blue, 18);
            }
            _ => panic!("Expected dark mode background color"),
        }

        // Test that Theme::dark() can be switched to light mode with param
        let ctx_light_override = ThemeContext::new("canvas", light_params.clone());

        let bg_value_override = dark_theme.query(&ctx_light_override, "background-color");
        let bg_light_override =
            bg_value_override.and_then(|v| v.as_color_with_params(&light_params, base_font_size));

        match bg_light_override {
            Some(color) => {
                // Should be light mode background (white)
                assert_eq!(
                    color.red, 255,
                    "Light mode override should have white background"
                );
                assert_eq!(color.green, 255);
                assert_eq!(color.blue, 255);
            }
            _ => panic!("Expected light mode background color with override"),
        }
    }

    #[test]
    fn test_categorical_color_with_contrast_adjustment() {
        // Test that categorical colors resolve correctly
        // With the current theme (no adjustment), colors should be the same in both modes
        let theme = Theme::light();

        // Get the resolved color in light mode
        let base_font_size = 16.0;
        let mut params = indexmap::IndexMap::new();
        params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("light".to_string())),
        );

        // Query with params set
        let ctx = ThemeContext::new(":root", params.clone());
        let color_value = theme.query(&ctx, "--categorical-color-0");

        if let Some(color_value) = color_value {
            if let Some(rgba) = color_value.as_color_with_params(&params, base_font_size) {
                // Original #0072B2 is rgb(0, 114, 178)
                assert_eq!(rgba.red, 0, "Red should be 0");
                assert_eq!(rgba.green, 114, "Green should be 114");
                assert_eq!(rgba.blue, 178, "Blue should be 178");
            } else {
                panic!("Failed to resolve light mode categorical-color-0");
            }

            // Test dark mode - should be the same color now
            let mut params_dark = indexmap::IndexMap::new();
            params_dark.insert(
                "color-scheme".to_string(),
                datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
            );

            let ctx_dark = ThemeContext::new(":root", params_dark.clone());
            let color_value_dark = theme.query(&ctx_dark, "--categorical-color-0");

            if let Some(color_value_dark) = color_value_dark {
                if let Some(rgba) =
                    color_value_dark.as_color_with_params(&params_dark, base_font_size)
                {
                    // Should be the same as light mode
                    assert_eq!(rgba.red, 0, "Red should be 0");
                    assert_eq!(rgba.green, 114, "Green should be 114");
                    assert_eq!(rgba.blue, 178, "Blue should be 178");
                } else {
                    panic!("Failed to resolve dark mode categorical-color-0");
                }
            } else {
                panic!("Failed to query dark mode --categorical-color-0");
            }
        } else {
            panic!("Failed to resolve light mode --categorical-color-0 color");
        }
    }

    #[test]
    fn test_color_mix_text_secondary_variable() {
        // Test that --text-secondary resolves correctly via color-mix() in both modes
        // Note: Actual percentages from theme may vary
        // Currently seeing: ~65% text + 35% background
        // Light mode: 65% black + 35% white ≈ 89
        // Dark mode: 65% white + 35% #121212 ≈ 172

        let theme = Theme::light();

        // Test light mode - axis label uses --text-secondary
        let ctx_light = ThemeContext::new("axis", IndexMap::new()).child("label");

        // Debug: Check what the raw query returns
        let raw_value = theme.query(&ctx_light, "color");
        println!("Raw query result: {:?}", raw_value);

        let color_light = theme.text_color(&ctx_light);

        match color_light {
            Some([r, g, b, a]) => {
                println!(
                    "Light mode --text-secondary: rgb({}, {}, {})",
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8
                );
                // Expect a darker gray (closer to text than background)
                assert!(
                    r * 255.0 > 50.0 && r * 255.0 < 120.0,
                    "Light mode should be a medium-dark gray, got {}",
                    r * 255.0
                );
                assert_eq!(r, g, "Should be neutral gray");
                assert_eq!(g, b, "Should be neutral gray");
                assert_eq!(a, 1.0, "Alpha should be 1.0");
            }
            None => panic!("Expected light mode text-secondary color"),
        }

        // Test dark mode
        let mut params = IndexMap::new();
        params.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let ctx_dark = ThemeContext::new("axis", params).child("label");
        let color_dark = theme.text_color(&ctx_dark);

        match color_dark {
            Some([r, g, b, a]) => {
                println!(
                    "Dark mode --text-secondary: rgb({}, {}, {})",
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8
                );
                // Expect a lighter gray (closer to white than dark background)
                assert!(
                    r * 255.0 > 150.0 && r * 255.0 < 220.0,
                    "Dark mode should be a light gray, got {}",
                    r * 255.0
                );
                assert_eq!(r, g, "Should be neutral gray");
                assert_eq!(g, b, "Should be neutral gray");
                assert_eq!(a, 1.0, "Alpha should be 1.0");
            }
            None => panic!("Expected dark mode text-secondary color"),
        }
    }

    #[test]
    fn test_dark_theme_default_color_scheme_field() {
        // Verify that Theme::dark() has correct default_color_scheme
        let dark_theme = Theme::dark();
        assert_eq!(dark_theme.default_color_scheme, "dark");

        let light_theme = Theme::light();
        assert_eq!(light_theme.default_color_scheme, "light");

        // Verify dark theme produces dark text color without any params
        let ctx = ThemeContext::new("axis", IndexMap::new()).child("title");
        let text_color = dark_theme.text_color(&ctx);

        match text_color {
            Some([r, g, b, _]) => {
                // Should be white in dark mode
                println!("Dark theme text color: r={}, g={}, b={}", r, g, b);
                assert_eq!(r, 1.0, "Expected white text in dark mode");
                assert_eq!(g, 1.0);
                assert_eq!(b, 1.0);
            }
            None => panic!("Expected dark mode text color"),
        }
    }

    #[test]
    fn test_dark_theme_serialization_preserves_color_scheme() {
        // Verify that default_color_scheme is preserved during serialization
        let dark_theme = Theme::dark();

        // Serialize and deserialize
        let serialized = bincode::serialize(&dark_theme).expect("Failed to serialize dark theme");
        let deserialized: Theme =
            bincode::deserialize(&serialized).expect("Failed to deserialize dark theme");

        // Verify default_color_scheme is preserved
        assert_eq!(deserialized.default_color_scheme, "dark");

        // Verify the deserialized theme still produces dark text color
        let ctx = ThemeContext::new("axis", IndexMap::new()).child("title");
        let text_color = deserialized.text_color(&ctx);

        match text_color {
            Some([r, g, b, _]) => {
                // Should be white in dark mode
                assert_eq!(r, 1.0, "Expected white text in dark mode");
                assert_eq!(g, 1.0);
                assert_eq!(b, 1.0);
            }
            None => panic!("Expected dark mode text color after deserialization"),
        }
    }

    #[test]
    fn test_lab_lch_colors_in_fill_discrete() {
        // Test that lab/lch/oklab/oklch colors parse correctly in fill-discrete lists
        let css_theme = r#"
            mark {
                fill-discrete:
                    oklab(0.6 0.1 -0.1),
                    oklch(0.6 0.14 315),
                    lab(60 20 -30),
                    lch(60 36 303);
            }
        "#;

        let theme = Theme::from_css(css_theme).expect("Failed to create theme from CSS");
        let ctx = ThemeContext::new("mark", IndexMap::new()).with_subtype("rect");

        let fill_discrete = theme.query(&ctx, "fill-discrete");

        // Should get a list of 4 colors
        match fill_discrete {
            Some(ThemeValue::List(colors)) => {
                assert_eq!(colors.len(), 4, "Expected 4 colors in fill-discrete list");
                // Check that they're all colors (not light-dark or other types)
                for color in colors.iter() {
                    assert!(
                        matches!(color, ThemeValue::Color(_)),
                        "Expected all values to be Color, got {:?}",
                        color
                    );
                }
            }
            other => panic!("Expected List of colors, got {:?}", other),
        }
    }

    #[test]
    fn test_font_size_with_base_font_size_param() {
        // Test that base-font-size parameter affects rem-based font size calculations
        let theme = Theme::light();
        let ctx = ThemeContext::new("chart-title", IndexMap::new());

        // Default: title uses 1.5rem which is 18px (1.5 * 12px)
        let default_size = theme.font_size(&ctx);
        assert_eq!(default_size, Some(18.0), "Title should be 18px by default");

        // With --base-font-size param = "16px": 1.5rem = 24px (1.5 * 16px)
        let mut params = IndexMap::new();
        params.insert(
            "--base-font-size".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("16px".to_string())),
        );

        let mut ctx_with_params = ctx.clone();
        ctx_with_params.params = params;
        let custom_size = theme.font_size(&ctx_with_params);
        assert_eq!(
            custom_size,
            Some(24.0),
            "Title should be 24px with 16px base"
        );

        // With --base-font-size param = "8px": 1.5rem = 12px (1.5 * 8px)
        let mut small_params = IndexMap::new();
        small_params.insert(
            "--base-font-size".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("8px".to_string())),
        );

        let mut ctx_with_small_params = ctx.clone();
        ctx_with_small_params.params = small_params;
        let small_size = theme.font_size(&ctx_with_small_params);
        assert_eq!(small_size, Some(12.0), "Title should be 12px with 8px base");
    }

    #[test]
    fn test_font_weight_keyword_conversion() {
        // Test that CSS font-weight keywords are converted to numeric values
        let css = r#"
            title { font-weight: bold; }
            label { font-weight: normal; }
            tick { font-weight: light; }
            axis { font-weight: 600; }
            legend { font-weight: semi-bold; }
            text { font-weight: heavy; }
            guide { font-weight: extra-light; }
        "#;

        let theme = Theme::from_css(css).expect("Failed to create theme from CSS");

        // Test bold -> 700
        let bold_ctx = ThemeContext::new("title", IndexMap::new());
        assert_eq!(theme.font_weight(&bold_ctx), Some(700.0));

        // Test normal -> 400
        let normal_ctx = ThemeContext::new("label", IndexMap::new());
        assert_eq!(theme.font_weight(&normal_ctx), Some(400.0));

        // Test light -> 300
        let light_ctx = ThemeContext::new("tick", IndexMap::new());
        assert_eq!(theme.font_weight(&light_ctx), Some(300.0));

        // Test numeric value passes through
        let numeric_ctx = ThemeContext::new("axis", IndexMap::new());
        assert_eq!(theme.font_weight(&numeric_ctx), Some(600.0));

        // Test semi-bold -> 600
        let semi_bold_ctx = ThemeContext::new("legend", IndexMap::new());
        assert_eq!(theme.font_weight(&semi_bold_ctx), Some(600.0));

        // Test heavy -> 900 (avoiding "black" which is also a color)
        let heavy_ctx = ThemeContext::new("text", IndexMap::new());
        assert_eq!(theme.font_weight(&heavy_ctx), Some(900.0));

        // Test extra-light -> 200
        let extra_light_ctx = ThemeContext::new("guide", IndexMap::new());
        assert_eq!(theme.font_weight(&extra_light_ctx), Some(200.0));
    }

    #[test]
    fn test_root_font_size_with_variable() {
        // Test that :root { font-size: var(--base-font-size) } works with param override
        let css = r#"
            :root {
                --base-font-size: 14px;
                font-size: var(--base-font-size);
            }
            mark {
                font-size: 2rem;
            }
        "#;

        let theme = Theme::from_css(css).unwrap();

        // Default: 2rem * 14px = 28px
        let ctx = ThemeContext::new("mark", IndexMap::new());
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 14.0);
        assert_eq!(theme.font_size(&ctx), Some(28.0));

        // Override with param: 2rem * 20px = 40px
        let mut params = IndexMap::new();
        params.insert(
            "--base-font-size".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("20px".to_string())),
        );

        assert_eq!(theme.get_base_font_size(&params), 20.0);
        let mut ctx_with_params = ctx.clone();
        ctx_with_params.params = params;
        assert_eq!(theme.font_size(&ctx_with_params), Some(40.0));
    }

    #[test]
    fn test_arbitrary_variable_for_base_font_size() {
        // Test that ANY variable name can be used for base font size
        let css = r#"
            :root {
                --my-custom-size: 10px;
                font-size: var(--my-custom-size);
            }
            mark {
                font-size: 3rem;
            }
        "#;

        let theme = Theme::from_css(css).unwrap();

        // Default: 3rem * 10px = 30px
        let ctx = ThemeContext::new("mark", IndexMap::new());
        assert_eq!(theme.get_base_font_size(&IndexMap::new()), 10.0);
        assert_eq!(theme.font_size(&ctx), Some(30.0));

        // Override --my-custom-size param: 3rem * 25px = 75px
        let mut params = IndexMap::new();
        params.insert(
            "--my-custom-size".to_string(), // Must use -- prefix to match CSS variable
            datafusion_common::ScalarValue::Utf8(Some("25px".to_string())),
        );

        assert_eq!(theme.get_base_font_size(&params), 25.0);
        let mut ctx_with_params = ctx.clone();
        ctx_with_params.params = params;
        assert_eq!(theme.font_size(&ctx_with_params), Some(75.0));
    }

    #[test]
    fn test_media_query_evaluation_in_theme_query() {
        use crate::theme::media_query::{
            DimensionValue, MediaCondition, MediaFeature, MediaOperator,
        };
        use crate::theme::parser::parse_stylesheet;

        // Create a simple CSS with media queries (simulated manually for now)
        let css = r#"
            test-element { fill: blue; }
        "#;

        let mut theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Add a rule with a media condition manually
        let rules = parse_stylesheet("test-element { fill: red; }").expect("Failed to parse rule");
        let mut rule = rules
            .into_iter()
            .next()
            .expect("Expected at least one rule");

        // Add a media condition to the rule
        rule.media_condition = Some(MediaCondition::Feature(MediaFeature::Single {
            name: "width".to_string(),
            op: MediaOperator::GreaterEqual,
            value: DimensionValue::Pixels(600.0),
        }));

        // Give it a high source_order so it overrides defaults
        rule.source_order = 1000;

        theme.rules.push(rule);

        // Test without width param - media query should not match, get blue
        let ctx = ThemeContext::new("test-element", IndexMap::new());
        let fill = theme.query(&ctx, "fill");
        assert!(fill.is_some(), "Should have fill value");
        if let Some(ThemeValue::Color(c)) = fill {
            assert_eq!(
                c.blue, 255,
                "Without width param, should get blue (default rule)"
            );
            assert_eq!(c.red, 0, "Should not be red");
        } else {
            panic!("Expected color value");
        }

        // Test with width < 600 - media query should not match, get blue
        let mut params_small = IndexMap::new();
        params_small.insert(
            "width".to_string(),
            datafusion_common::ScalarValue::Float32(Some(400.0)),
        );
        let ctx_small = ThemeContext::new("test-element", params_small);
        let fill_small = theme.query(&ctx_small, "fill");
        if let Some(ThemeValue::Color(c)) = fill_small {
            assert_eq!(
                c.blue, 255,
                "With width < 600, should get blue (default rule)"
            );
            assert_eq!(c.red, 0, "Should not be red");
        }

        // Test with width >= 600 - media query SHOULD match, get red
        let mut params_large = IndexMap::new();
        params_large.insert(
            "width".to_string(),
            datafusion_common::ScalarValue::Float32(Some(800.0)),
        );
        let ctx_large = ThemeContext::new("test-element", params_large);
        let fill_large = theme.query(&ctx_large, "fill");
        if let Some(ThemeValue::Color(c)) = fill_large {
            assert_eq!(
                c.red, 255,
                "With width >= 600, should get red from media query rule"
            );
            assert_eq!(c.green, 0);
            assert_eq!(c.blue, 0);
        } else {
            panic!("Expected media query rule to match");
        }
    }

    // ===================================================================================
    // Phase 4: Comprehensive tests for type-directed evaluation architecture
    // ===================================================================================
    //
    // These tests verify that the new architecture handles all variant × channel type
    // combinations correctly, including previously unsupported cases like:
    // - calc() in numeric channels
    // - calc() in length channels
    // - Percentage values
    // - Angle values
    // - RelativeColor in color channels

    #[test]
    fn test_calc_in_size_channel() {
        // Test calc() expressions in size (length) channel
        let theme = Theme::from_css(
            r#"
            mark[type="symbol"] {
                size: calc(10px * 2);
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("symbol", "size", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(size))) = result {
            assert_eq!(size, 20.0, "calc(10px * 2) should evaluate to 20");
        } else {
            panic!("Expected Float32 size value from calc()");
        }
    }

    #[test]
    fn test_calc_with_variable_in_size() {
        // Test calc() with variables in size channel
        let theme = Theme::from_css(
            r#"
            mark[type="symbol"] {
                size: calc(var(--base-size) * 2);
            }
            "#,
        )
        .unwrap();

        let mut params = IndexMap::new();
        params.insert(
            "--base-size".to_string(),
            datafusion_common::ScalarValue::Float64(Some(15.0)),
        );

        let result = theme.mark_default("symbol", "size", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(size))) = result {
            assert_eq!(
                size, 30.0,
                "calc(var(--base-size) * 2) with --base-size=15 should be 30"
            );
        } else {
            panic!("Expected Float32 size value from calc() with variable");
        }
    }

    #[test]
    fn test_calc_in_opacity_channel() {
        // Test calc() in opacity (number) channel
        let theme = Theme::from_css(
            r#"
            mark[type="line"] {
                opacity: calc(0.5 + 0.3);
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("line", "opacity", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(opacity))) = result {
            assert!(
                (opacity - 0.8).abs() < 0.001,
                "calc(0.5 + 0.3) should be 0.8"
            );
        } else {
            panic!("Expected Float32 opacity value from calc()");
        }
    }

    #[test]
    fn test_percentage_in_opacity() {
        // Test percentage values in opacity channel
        // Note: CSS percentage values in opacity range from 0-100, where 75% = 75.0 stored
        // but eval_as_number() should convert Percentage(75.0) to 75.0 (not 0.75)
        // The channel semantics determine whether to interpret as 0-1 or 0-100 range
        let theme = Theme::from_css(
            r#"
            mark[type="rect"] {
                opacity: 75%;
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("rect", "opacity", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(opacity))) = result {
            // Percentage values are stored as-is (75.0), not as decimal (0.75)
            // This matches CSS behavior where percentages keep their numeric value
            assert!((opacity - 75.0).abs() < 0.001, "75% should be 75.0");
        } else {
            panic!(
                "Expected Float32 opacity from percentage, got: {:?}",
                result
            );
        }
    }

    #[test]
    fn test_angle_in_rotation() {
        // Test angle values with units in rotation channel
        let theme = Theme::from_css(
            r#"
            mark[type="symbol"] {
                rotation: 90deg;
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("symbol", "rotation", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(angle))) = result {
            assert_eq!(angle, 90.0, "90deg should be 90.0");
        } else {
            panic!("Expected Float32 rotation from angle");
        }
    }

    #[test]
    fn test_relative_color_in_fill() {
        // Test RelativeColor in fill channel
        // Note: Using a simpler relative color syntax that's properly supported
        let theme = Theme::from_css(
            r#"
            mark[type="symbol"] {
                fill: rgb(from blue r g 128);
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("symbol", "fill", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Utf8(Some(color))) = result {
            // Should be a valid color string (hex or rgba)
            assert!(
                color.starts_with('#') || color.starts_with("rgba("),
                "Expected color string from RelativeColor, got: {}",
                color
            );
        } else {
            panic!(
                "Expected Utf8 color value from RelativeColor, got: {:?}",
                result
            );
        }
    }

    #[test]
    fn test_light_dark_in_stroke() {
        // Test light-dark() in stroke channel
        let theme = Theme::from_css(
            r#"
            mark[type="line"] {
                stroke: light-dark(#333, #ccc);
            }
            "#,
        )
        .unwrap();

        // Test light mode (default)
        let params = IndexMap::new();
        let result = theme.mark_default("line", "stroke", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Utf8(Some(color))) = result {
            // Should be dark color in light mode
            assert!(
                color.contains("33"),
                "Light mode should use #333 (dark color)"
            );
        } else {
            panic!("Expected color from light-dark() in light mode");
        }

        // Test dark mode
        let mut params_dark = IndexMap::new();
        params_dark.insert(
            "color-scheme".to_string(),
            datafusion_common::ScalarValue::Utf8(Some("dark".to_string())),
        );
        let result_dark = theme.mark_default("line", "stroke", &params_dark);

        assert!(result_dark.is_some());
        if let Some(datafusion_common::ScalarValue::Utf8(Some(color))) = result_dark {
            // Should be light color in dark mode
            assert!(
                color.contains("cc"),
                "Dark mode should use #ccc (light color)"
            );
        } else {
            panic!("Expected color from light-dark() in dark mode");
        }
    }

    #[test]
    fn test_variable_in_numeric_channel() {
        // Test variables resolving to numbers in numeric channels
        let theme = Theme::from_css(
            r#"
            mark[type="line"] {
                stroke-width: var(--line-width);
            }
            "#,
        )
        .unwrap();

        let mut params = IndexMap::new();
        params.insert(
            "--line-width".to_string(),
            datafusion_common::ScalarValue::Float64(Some(2.5)),
        );

        let result = theme.mark_default("line", "stroke_width", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(width))) = result {
            assert_eq!(width, 2.5, "var(--line-width) should resolve to 2.5");
        } else {
            panic!("Expected Float32 from variable in numeric channel");
        }
    }

    #[test]
    fn test_rem_units_in_size() {
        // Test rem units in size channel with custom base font size
        let theme = Theme::from_css(
            r#"
            :root {
                font-size: 16px;
            }
            mark[type="symbol"] {
                size: 2rem;
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("symbol", "size", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(size))) = result {
            assert_eq!(size, 32.0, "2rem with 16px base should be 32px");
        } else {
            panic!("Expected Float32 from rem unit");
        }
    }

    #[test]
    fn test_unknown_property_returns_none() {
        // Test that unknown properties return None gracefully
        let theme = Theme::from_css(
            r#"
            mark[type="symbol"] {
                fill: red;
            }
            "#,
        )
        .unwrap();

        let params = IndexMap::new();
        let result = theme.mark_default("symbol", "unknown_property", &params);

        assert!(result.is_none(), "Unknown property should return None");
    }

    #[test]
    fn test_calc_with_var_and_math() {
        // Test calc() expressions with variables and arithmetic
        let theme = Theme::from_css(
            r#"
            mark[type="rect"] {
                stroke-width: calc((var(--base) + 1) * 2);
            }
            "#,
        )
        .unwrap();

        let mut params = IndexMap::new();
        params.insert(
            "--base".to_string(),
            datafusion_common::ScalarValue::Float64(Some(3.0)),
        );

        let result = theme.mark_default("rect", "stroke_width", &params);

        assert!(result.is_some());
        if let Some(datafusion_common::ScalarValue::Float32(Some(width))) = result {
            assert_eq!(width, 8.0, "calc((3 + 1) * 2) should be 8.0");
        } else {
            panic!(
                "Expected Float32 from calc() with variable, got: {:?}",
                result
            );
        }
    }
}
