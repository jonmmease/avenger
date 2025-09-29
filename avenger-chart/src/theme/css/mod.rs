//! Full CSS-compliant theme system using cssparser and selectors

mod element;
mod parser;
mod selector_impl;
mod theme_impl;
mod value;

pub use selector_impl::ChartString;

use crate::theme::{LengthUnit, Rgba, ThemeValue};
use indexmap::IndexMap;
use selectors::matching::SelectorCaches;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// CSS-based theme with full selector support
#[derive(Debug, Clone)]
pub struct CssTheme {
    rules: Vec<CompiledRule>,
    variables: IndexMap<String, ThemeValue>,
    inherited_properties: std::collections::HashSet<&'static str>,
    base_font_size: f32,
    /// List of CSS sources in order they were added
    css_sources: Vec<String>,
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

        Ok(Self {
            rules: compiled_rules,
            variables,
            inherited_properties: Self::default_inherited_properties(),
            base_font_size: 12.0,
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

    /// Append additional CSS rules to the theme
    pub fn append_css(&mut self, css: &str) -> Result<(), String> {
        // Parse the new CSS
        let new_rules = parser::parse_stylesheet(css)?;

        // Get current max source_order
        let current_max_order = self.rules
            .iter()
            .map(|r| r.source_order)
            .max()
            .unwrap_or(0);

        // Add new rules with updated source_order
        for (i, mut rule) in new_rules.into_iter().enumerate() {
            rule.source_order = current_max_order + i + 1;

            // Extract any new variables
            for (key, value) in &rule.declarations {
                if key.starts_with("--") {
                    self.variables.insert(key.clone(), value.clone());
                }
            }

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
            "font-weight" => ThemeValue::Number(400.0),
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
                "size" => return ThemeValue::Number(5.0),
                "stroke" => {
                    return ThemeValue::Color(Rgba {
                        red: 0,
                        green: 0,
                        blue: 0,
                        alpha: 255,
                    });
                }
                "stroke-width" => return ThemeValue::Number(1.0),
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
                "stroke-width" => return ThemeValue::Number(0.5),
                "opacity" => return ThemeValue::Number(0.5),
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
            "stroke-width" => ThemeValue::Number(1.0),
            "opacity" => ThemeValue::Number(1.0),
            "size" => ThemeValue::Number(60.0),
            "padding" => ThemeValue::Number(8.0),
            "spacing" => ThemeValue::Number(10.0),
            _ => ThemeValue::Initial,
        }
    }
}

/// Helper struct for serializing CssTheme
#[derive(Serialize, Deserialize)]
struct CssThemeData {
    css_sources: Vec<String>,
    base_font_size: f32,
}

impl Serialize for CssTheme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let data = CssThemeData {
            css_sources: self.css_sources.clone(),
            base_font_size: self.base_font_size,
        };
        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CssTheme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let data = CssThemeData::deserialize(deserializer)?;

        // Start with empty theme
        let mut theme = CssTheme {
            rules: Vec::new(),
            variables: IndexMap::new(),
            inherited_properties: Self::default_inherited_properties(),
            base_font_size: data.base_font_size,
            css_sources: Vec::new(),
        };

        // Rebuild by parsing each CSS source in order
        for css in data.css_sources {
            theme
                .append_css(&css)
                .map_err(serde::de::Error::custom)?;
        }

        Ok(theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn test_css_theme_serialization() {
        // Create a CssTheme
        let theme = CssTheme::light();

        // Serialize to JSON
        let json = serde_json::to_string(&theme).unwrap();

        // Deserialize back
        let deserialized: CssTheme = serde_json::from_str(&json).unwrap();

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
        let theme: Box<dyn Theme> = Box::new(CssTheme::dark());

        // Serialize the trait object
        let json = serde_json::to_string(&theme).unwrap();

        // Deserialize back as trait object
        let deserialized: Box<dyn Theme> = serde_json::from_str(&json).unwrap();

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
        assert!(matches!(value, ThemeValue::Color(_)));

        // Also test that base_font_size is preserved
        assert_eq!(deserialized.base_font_size(), 12.0);
    }

    #[test]
    fn test_append_css() {
        // Create a base theme
        let mut theme = CssTheme::from_css(r#"
            mark {
                fill: red;
            }
        "#).unwrap();

        // Append additional CSS
        theme.append_css(r#"
            mark {
                fill: blue;
                stroke: green;
            }
        "#).unwrap();

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
        assert!(matches!(fill, ThemeValue::Color(c) if c.blue == 255));

        let stroke = theme.query(&context, "stroke");
        // Stroke was only defined in the second rule
        assert!(matches!(stroke, ThemeValue::Color(c) if c.green == 128));
    }

    #[test]
    fn test_append_css_with_serialization() {
        // Create a base theme and append CSS
        let mut theme = CssTheme::from_css(r#"
            mark {
                fill: red;
            }
        "#).unwrap();

        theme.append_css(r#"
            mark {
                fill: blue;
            }
            axis {
                stroke: black;
            }
        "#).unwrap();

        // Serialize and deserialize
        let json = serde_json::to_string(&theme).unwrap();
        let deserialized: CssTheme = serde_json::from_str(&json).unwrap();

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
        assert!(matches!(fill, ThemeValue::Color(c) if c.blue == 255));
    }
}
