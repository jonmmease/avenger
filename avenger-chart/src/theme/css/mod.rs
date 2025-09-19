//! Full CSS-compliant theme system using cssparser and selectors

mod cascade;
mod element;
mod parser;
mod selector_impl;
mod theme_impl;
mod value;
pub use selector_impl::ChartString;

use crate::theme::{LengthUnit, Rgba, ThemeValue};
use indexmap::IndexMap;
use selectors::matching::SelectorCaches;

/// CSS-based theme with full selector support
#[derive(Debug, Clone)]
pub struct CssTheme {
    rules: Vec<CompiledRule>,
    variables: IndexMap<String, ThemeValue>,
    inherited_properties: std::collections::HashSet<&'static str>,
    base_font_size: f32,
}

/// A compiled CSS rule with selector and declarations
#[derive(Debug, Clone)]
struct CompiledRule {
    selector: selectors::parser::Selector<selector_impl::ChartSelectors>,
    specificity: u32,
    source_order: usize,
    declarations: IndexMap<String, ThemeValue>,
}

impl CssTheme {
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
                stroke_dash-discrete: solid, dashed, dotted, long-dash, dash-dot, long-short, even-short, double-dash;
                stroke_width-discrete: 0.5, 1.0, 2.0, 3.0, 5.0;
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
                stroke_dash-discrete: solid, dashed, dotted, long-dash, dash-dot, long-short, even-short, double-dash;
                stroke_width-discrete: 0.5, 1.0, 2.0, 3.0, 5.0;
            }
        "#;

        Self::from_css(css).expect("Failed to parse built-in dark theme CSS")
    }

    /// Create a theme from CSS string
    pub fn from_css(css: &str) -> Result<Self, String> {
        let rules = parser::parse_stylesheet(css)?;

        // Extract variables from rules
        let mut variables = IndexMap::new();
        let mut compiled_rules = Vec::new();

        for rule in rules {
            // Check for variable declarations
            for (key, value) in &rule.declarations {
                if key.starts_with("--") {
                    variables.insert(key.clone(), value.clone());
                }
            }
            compiled_rules.push(rule);
        }

        // Define inherited properties
        let mut inherited_properties = std::collections::HashSet::new();
        inherited_properties.insert("color");
        inherited_properties.insert("font-family");
        inherited_properties.insert("font-size");
        inherited_properties.insert("font-weight");
        inherited_properties.insert("font-style");
        inherited_properties.insert("line-height");
        inherited_properties.insert("text-align");
        inherited_properties.insert("visibility");

        Ok(Self {
            rules: compiled_rules,
            variables,
            inherited_properties,
            base_font_size: 12.0,
        })
    }

    /// Query a CSS property for an element
    pub fn query_css(&self, context: &crate::theme::ThemeContext, property: &str) -> ThemeValue {
        use crate::theme::css::element::CssElement;

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
                        // Check if this is a comma-separated list that should be parsed
                        if property.ends_with("-range") || property == "range" {
                            // For range properties, check if the resolved value is a comma-separated list
                            if let ThemeValue::String(s) | ThemeValue::Keyword(s) = resolved {
                                if s.contains(',') {
                                    // Parse as a list of values
                                    let items: Vec<ThemeValue> = s
                                        .split(',')
                                        .map(|item| {
                                            let trimmed = item.trim();
                                            // Try to parse as color if it starts with #
                                            if trimmed.starts_with('#') {
                                                ThemeValue::Keyword(trimmed.to_string())
                                            } else {
                                                ThemeValue::Keyword(trimmed.to_string())
                                            }
                                        })
                                        .collect();
                                    return ThemeValue::List(items);
                                }
                            }
                        }
                        return resolved.clone();
                    }
                }
                return value.clone();
            }
        }

        // Check if property is inherited and try to get it from parent
        if self.inherited_properties.contains(property) {
            // Try to get the value from the parent element
            if let Some(parent) = &context.parent {
                // Recursively query the parent for this property
                return self.query_css(parent, property);
            }
            // No parent, return default inherited value
            return self.get_inherited_default(property);
        }

        // Return initial value based on context
        self.get_initial_value(context, property)
    }

    /// Get default inherited value for a property
    fn get_inherited_default(&self, property: &str) -> ThemeValue {
        match property {
            "color" => ThemeValue::Color(Rgba {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255,
            }),
            "font-family" => ThemeValue::String("sans-serif".to_string()),
            "font-size" => ThemeValue::Length(self.base_font_size as f64, LengthUnit::Px),
            "font-weight" => ThemeValue::Double(400.0),
            _ => ThemeValue::Initial,
        }
    }

    /// Get initial value for a property based on context
    fn get_initial_value(
        &self,
        context: &crate::theme::ThemeContext,
        property: &str,
    ) -> ThemeValue {
        // Context-aware defaults
        if context.element_type == "tick" {
            match property {
                "size" => return ThemeValue::Double(5.0), // Tick marks should be small
                "stroke" => {
                    return ThemeValue::Color(Rgba {
                        red: 0,
                        green: 0,
                        blue: 0,
                        alpha: 255,
                    });
                }
                "stroke-width" => return ThemeValue::Double(1.0),
                _ => {}
            }
        }

        // Grid-specific defaults
        if context.element_type == "grid" {
            match property {
                "stroke" => {
                    return ThemeValue::Color(Rgba {
                        red: 224, // #e0e0e0
                        green: 224,
                        blue: 224,
                        alpha: 255,
                    });
                }
                "stroke-width" => return ThemeValue::Double(0.5),
                "opacity" => return ThemeValue::Double(0.5),
                _ => {}
            }
        }

        // General defaults
        match property {
            "fill" => ThemeValue::Color(Rgba {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 255,
            }),
            "stroke" => ThemeValue::None,
            "stroke-width" => ThemeValue::Double(1.0),
            "opacity" => ThemeValue::Double(1.0),
            "size" => ThemeValue::Double(60.0),
            "padding" => ThemeValue::Double(8.0),
            "spacing" => ThemeValue::Double(10.0),
            _ => ThemeValue::Initial,
        }
    }
}
