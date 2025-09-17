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
pub struct Theme {
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

/// Cache key for query results
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct QueryKey {
    element_key: String,
    property: String,
}

impl Theme {
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

        // Check if property is inherited
        if self.inherited_properties.contains(property) {
            // For now, return a default inherited value
            // In a full implementation, we'd check the parent element
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
        if context.element_type == "axis" && context.classes.contains(&"tick".to_string()) {
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
            _ => ThemeValue::Initial,
        }
    }
}
