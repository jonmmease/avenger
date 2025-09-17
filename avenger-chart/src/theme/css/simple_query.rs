//! Simplified theme query functionality

use crate::theme::css::types::{
    PropertyExplanation, RGBA, Rule, Theme, ThemeContext, ThemeValue, Unit, UsageTracker,
};
use std::collections::HashMap;

impl Theme {
    /// Query a property value for a given context
    pub fn query(&self, context: &ThemeContext, property: &str) -> ThemeValue {
        self.query_with_tracker(context, property, None)
    }

    /// Query a property value with optional usage tracking
    pub fn query_with_tracker(
        &self,
        context: &ThemeContext,
        property: &str,
        mut tracker: Option<&mut UsageTracker>,
    ) -> ThemeValue {
        let property_lower = property.to_lowercase();

        if let Some(ref mut t) = tracker {
            t.record_query(&property_lower);
            t.record_context(context.clone());
        }

        // Find matching rules (simplified matching)
        let matching_rules = self.find_matching_rules_simple(context, tracker.as_deref_mut());

        // Sort by specificity and cascade order
        let sorted_rules = self.sort_by_specificity(matching_rules);

        // Find property value
        for rule in sorted_rules {
            if let Some(value) = self.get_property_from_rule(rule, &property_lower) {
                return self.resolve_value(value, context);
            }
        }

        // Check inheritance
        if self.is_inherited(&property_lower) {
            if let Some(parent) = &context.parent {
                return self.query_with_tracker(parent.as_ref(), property, tracker);
            }
        }

        ThemeValue::None
    }

    /// Find rules that match using simple selector matching
    fn find_matching_rules_simple(
        &self,
        context: &ThemeContext,
        mut tracker: Option<&mut UsageTracker>,
    ) -> Vec<(usize, &Rule)> {
        let mut matched = Vec::new();

        for (i, rule) in self.rules.iter().enumerate() {
            if self.simple_selector_matches(&rule.selector, context) {
                if let Some(ref mut t) = tracker {
                    t.record_match(i);
                }
                matched.push((i, rule));
            }
        }

        matched
    }

    /// Simple selector matching without complex parsing
    fn simple_selector_matches(&self, selector: &str, context: &ThemeContext) -> bool {
        let selector = selector.trim();

        // Handle universal selector
        if selector == "*" {
            return true;
        }

        // Handle :root selector
        if selector == ":root" {
            return context.parent.is_none();
        }

        // Split compound selectors
        let parts: Vec<&str> = selector.split_whitespace().collect();
        if parts.is_empty() {
            return false;
        }

        // Check the last part (the actual element being targeted)
        let last_part = parts.last().unwrap();

        // Check element type
        if !last_part.contains('.') && !last_part.contains('#') && !last_part.contains(':') {
            // Simple element selector
            return &context.element_type == last_part;
        }

        // Parse the selector part
        let mut element = "";
        let mut classes = Vec::new();
        let mut id = "";

        // Extract element, classes, and ID from the selector
        let mut current = *last_part;

        // Find element name (before any . # or :)
        if !current.starts_with('.') && !current.starts_with('#') && !current.starts_with(':') {
            if let Some(idx) = current.find(['.', '#', ':']) {
                element = &current[..idx];
                current = &current[idx..];
            } else {
                element = current;
                current = "";
            }
        }

        // Extract classes and IDs
        while !current.is_empty() {
            if current.starts_with('.') {
                let end = current[1..]
                    .find(['.', '#', ':'])
                    .map(|i| i + 1)
                    .unwrap_or(current.len());
                classes.push(&current[1..end]);
                current = &current[end..];
            } else if current.starts_with('#') {
                let end = current[1..]
                    .find(['.', '#', ':'])
                    .map(|i| i + 1)
                    .unwrap_or(current.len());
                id = &current[1..end];
                current = &current[end..];
            } else if current.starts_with(':') {
                // Skip pseudo-classes for now
                let end = current[1..]
                    .find(['.', '#', ':'])
                    .map(|i| i + 1)
                    .unwrap_or(current.len());
                current = &current[end..];
            } else {
                break;
            }
        }

        // Match element type
        if !element.is_empty() && context.element_type != element {
            return false;
        }

        // Match ID
        if !id.is_empty() {
            if context.id.as_deref() != Some(id) {
                return false;
            }
        }

        // Match classes
        for class in classes {
            if !context.classes.iter().any(|c| c == class) {
                return false;
            }
        }

        true
    }

    /// Sort rules by specificity and source order
    fn sort_by_specificity<'a>(&self, mut rules: Vec<(usize, &'a Rule)>) -> Vec<&'a Rule> {
        rules.sort_by(|a, b| {
            // First sort by specificity (higher specificity first)
            let spec_cmp = b.1.specificity.cmp(&a.1.specificity);
            if spec_cmp != std::cmp::Ordering::Equal {
                return spec_cmp;
            }
            // Then by source order (later rules first)
            b.0.cmp(&a.0)
        });

        rules.into_iter().map(|(_, rule)| rule).collect()
    }

    /// Get a property value from a rule
    fn get_property_from_rule(&self, rule: &Rule, property: &str) -> Option<ThemeValue> {
        rule.declarations
            .iter()
            .rev() // Check in reverse order (last declaration wins)
            .find(|decl| decl.property.to_lowercase() == property)
            .map(|decl| decl.value.clone())
    }

    /// Check if a property inherits
    fn is_inherited(&self, property: &str) -> bool {
        self.inherited_properties.contains(property)
    }

    /// Resolve a value (handle variables, calc, units)
    fn resolve_value(&self, value: ThemeValue, context: &ThemeContext) -> ThemeValue {
        match value {
            ThemeValue::Variable(ref var_name, ref fallback) => {
                // Look up variable value
                if let Some(var_value) = self.variables.get(var_name) {
                    self.resolve_value(var_value.clone(), context)
                } else if let Some(fallback) = fallback {
                    self.resolve_value(fallback.as_ref().clone(), context)
                } else {
                    ThemeValue::None
                }
            }
            ThemeValue::Calc(_) => {
                // For now, just return as-is
                // TODO: Implement calc evaluation
                value
            }
            ThemeValue::Dimension(val, unit) => {
                // Convert units if needed
                let resolved = self.resolve_unit(val as f32, unit, context);
                ThemeValue::Dimension(resolved as f64, Unit::Px)
            }
            _ => value,
        }
    }

    /// Resolve a unit value to pixels
    fn resolve_unit(&self, value: f32, unit: Unit, _context: &ThemeContext) -> f32 {
        match unit {
            Unit::Px => value,
            Unit::Rem => value * self.base_font_size,
            Unit::Em => value * self.base_font_size, // Simplified for now
            Unit::Percent => value,                  // Context-dependent, return as-is
            Unit::Vw => {
                if let Some(width) = self.chart_width {
                    value * width / 100.0
                } else {
                    value
                }
            }
            Unit::Vh => {
                if let Some(height) = self.chart_height {
                    value * height / 100.0
                } else {
                    value
                }
            }
            Unit::None => value,
        }
    }

    // Helper methods for typed access

    /// Get a float value
    pub fn get_f32(&self, context: &ThemeContext, property: &str) -> Option<f32> {
        match self.query(context, property) {
            ThemeValue::Number(n) => Some(n as f32),
            ThemeValue::Dimension(n, _) => Some(n as f32),
            _ => None,
        }
    }

    /// Get a color value
    pub fn get_color(&self, context: &ThemeContext, property: &str) -> Option<RGBA> {
        match self.query(context, property) {
            ThemeValue::Color(color) => Some(color),
            _ => None,
        }
    }

    /// Get a string value
    pub fn get_string(&self, context: &ThemeContext, property: &str) -> Option<String> {
        match self.query(context, property) {
            ThemeValue::String(s) => Some(s),
            ThemeValue::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// Get all properties for a context
    pub fn all_properties(&self, context: &ThemeContext) -> HashMap<String, ThemeValue> {
        let mut props = HashMap::new();

        // Find all matching rules
        let matching_rules = self.find_matching_rules_simple(context, None);
        let sorted_rules = self.sort_by_specificity(matching_rules);

        // Collect all properties (later rules override earlier ones)
        for rule in sorted_rules.iter().rev() {
            for decl in &rule.declarations {
                if !props.contains_key(&decl.property) {
                    let value = self.resolve_value(decl.value.clone(), context);
                    props.insert(decl.property.clone(), value);
                }
            }
        }

        props
    }

    /// Explain why a property has its value
    pub fn explain_property(&self, context: &ThemeContext, property: &str) -> PropertyExplanation {
        let matching_rules = self.find_matching_rules_simple(context, None);
        let sorted_rules = self.sort_by_specificity(matching_rules);

        let property_lower = property.to_lowercase();
        let mut winning_rule = None;
        let mut final_value = ThemeValue::None;

        // Find the winning rule for this property
        for rule in &sorted_rules {
            if let Some(value) = self.get_property_from_rule(rule, &property_lower) {
                final_value = self.resolve_value(value, context);
                winning_rule = Some((*rule).clone());
                break;
            }
        }

        // If no direct match, check inheritance
        if winning_rule.is_none() && self.is_inherited(&property_lower) {
            if let Some(parent) = &context.parent {
                let parent_explanation = self.explain_property(parent.as_ref(), property);
                return parent_explanation;
            }
        }

        PropertyExplanation {
            final_value,
            matching_rules: sorted_rules.into_iter().cloned().collect(),
            winning_rule,
            specificity_order: vec![],
        }
    }

    /// Find rules that match a context
    pub fn matching_rules(&self, context: &ThemeContext) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| self.simple_selector_matches(&rule.selector, context))
            .collect()
    }

    // CSS variable support

    /// Set a CSS custom property value
    pub fn set_variable(mut self, name: &str, value: impl Into<ThemeValue>) -> Self {
        let name = if name.starts_with("--") {
            name.to_string()
        } else {
            format!("--{}", name)
        };
        self.variables.insert(name, value.into());
        self
    }

    /// Set base font size for rem calculations
    pub fn set_base_font_size(mut self, size: f32) -> Self {
        self.base_font_size = size;
        self.set_variable(
            "--base-font-size",
            ThemeValue::Dimension(size as f64, Unit::Px),
        )
    }

    /// Builder pattern for common customizations
    pub fn with_primary_color(self, color: &str) -> Self {
        self.set_variable("--primary", ThemeValue::String(color.to_string()))
    }

    pub fn with_font_family(self, font: &str) -> Self {
        self.set_variable("--font-family", ThemeValue::String(font.to_string()))
    }

    pub fn with_scale(self, scale: f32) -> Self {
        self.set_variable("--scale", ThemeValue::Number(scale as f64))
            .set_base_font_size(12.0 * scale)
    }
}

// Conversion helpers for ThemeValue

impl ThemeValue {
    /// Convert to float if possible
    pub fn as_float(&self) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Dimension(n, _) => Some(*n as f32),
            _ => None,
        }
    }

    /// Convert to color if possible
    pub fn as_color(&self) -> Option<RGBA> {
        match self {
            ThemeValue::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// Convert to string if possible
    pub fn as_string(&self) -> Option<String> {
        match self {
            ThemeValue::String(s) => Some(s.clone()),
            ThemeValue::Keyword(k) => Some(k.clone()),
            _ => None,
        }
    }
}
