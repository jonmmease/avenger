//! Color component representation for relative color syntax
//!
//! This module provides the `ColorComponent` type which can represent a single
//! color component as:
//! - A literal value (0.5, 180.0, etc.)
//! - A channel keyword (l, c, h, r, g, b, etc.)
//! - A calc() expression (may contain channel keywords)
//! - The "none" keyword
//!
//! This is used to implement CSS Color Level 5 relative color syntax:
//! ```css
//! oklch(from blue calc(l - 0.2) c h)
//! ```

use indexmap::IndexMap;

use avenger_color::{AbsoluteColor, ColorChannel};

use super::{
    calc::{CalcLeaf, CalcNode},
    value::ThemeValue,
};

/// A single color component that may contain channel keywords or calc expressions
///
/// Used in relative color syntax to represent components that can reference
/// the origin color's channels.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColorComponent {
    /// A literal numeric value (0.5, 180.0, etc.)
    Literal(f64),

    /// The "none" keyword (component is missing/undefined)
    None,

    /// A channel keyword reference (l, c, h, r, g, b, etc.)
    ColorChannel(ColorChannel),

    /// A calc() expression (may contain channel keywords)
    Calc(CalcNode),
}

impl ColorComponent {
    /// Resolve component to a concrete value with optional origin color
    ///
    /// # Arguments
    /// * `origin_color` - The origin color for channel keyword resolution
    /// * `params` - Runtime parameters for CSS variable substitution
    /// * `base_font_size` - Base font size for rem conversion
    ///
    /// # Returns
    /// The resolved numeric value, or an error if resolution fails
    pub fn resolve(
        &self,
        origin_color: Option<&AbsoluteColor>,
        params: &IndexMap<String, f64>,
        base_font_size: f32,
    ) -> Result<f64, String> {
        match self {
            ColorComponent::Literal(value) => Ok(*value),

            ColorComponent::None => {
                // "none" becomes 0.0 (could be enhanced to use origin value)
                Ok(0.0)
            }

            ColorComponent::ColorChannel(keyword) => {
                let origin = origin_color.ok_or_else(|| {
                    format!("Channel keyword {:?} requires origin color", keyword)
                })?;
                let value = origin.get_component_by_channel_keyword(*keyword)?;
                Ok(value as f64)
            }

            ColorComponent::Calc(node) => {
                // Resolve calc with BOTH runtime parameters AND channel keywords
                // The new resolve_with_params_and_origin handles the correct order:
                // 1. Substitute CSS variables (var(--x))
                // 2. Substitute channel keywords (l, c, h, etc.)
                // 3. Resolve to final value
                let resolved =
                    node.resolve_with_params_and_origin(params, base_font_size, origin_color)?;

                // Extract numeric value - handle both Number and Angle (angles are in degrees)
                resolved
                    .as_number()
                    .or_else(|| resolved.as_angle_degrees())
                    .ok_or_else(|| {
                        "Color component calc must resolve to number or angle".to_string()
                    })
            }
        }
    }

    /// Parse from ThemeValue
    ///
    /// Converts a ThemeValue into a ColorComponent. Handles numbers, percentages,
    /// angles, and calc expressions.
    pub fn from_theme_value(value: &ThemeValue) -> Result<Self, String> {
        match value {
            ThemeValue::Number(n) => Ok(ColorComponent::Literal(*n)),

            ThemeValue::Percentage(p) => Ok(ColorComponent::Literal(p / 100.0)),

            ThemeValue::Angle(val, unit) => {
                let deg = unit.to_degrees(*val);
                Ok(ColorComponent::Literal(deg))
            }

            ThemeValue::Length(val, _unit) => {
                // For now, just use the numeric value
                // In a full implementation, we'd handle unit conversion
                Ok(ColorComponent::Literal(*val))
            }

            ThemeValue::Calc(node) => Ok(ColorComponent::Calc(*node.clone())),

            // Handle CSS variables - wrap in a Calc node
            ThemeValue::Variable(name) => Ok(ColorComponent::Calc(CalcNode::Leaf(
                CalcLeaf::Variable(name.clone()),
            ))),

            _ => Err(format!("Cannot convert {:?} to ColorComponent", value)),
        }
    }

    /// Create a literal component
    pub fn literal(value: f64) -> Self {
        ColorComponent::Literal(value)
    }

    /// Create a color channel component
    pub fn channel(keyword: ColorChannel) -> Self {
        ColorComponent::ColorChannel(keyword)
    }

    /// Create a calc component
    pub fn calc(node: CalcNode) -> Self {
        ColorComponent::Calc(node)
    }

    /// Check if this is a literal value
    pub fn is_literal(&self) -> bool {
        matches!(self, ColorComponent::Literal(_))
    }

    /// Check if this is a color channel reference
    pub fn is_color_channel(&self) -> bool {
        matches!(self, ColorComponent::ColorChannel(_))
    }

    /// Check if this contains a calc expression
    pub fn is_calc(&self) -> bool {
        matches!(self, ColorComponent::Calc(_))
    }

    /// Check if this is "none"
    pub fn is_none(&self) -> bool {
        matches!(self, ColorComponent::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_color::ColorSpace;

    #[test]
    fn test_literal_component() {
        let comp = ColorComponent::Literal(0.5);
        let result = comp.resolve(None, &IndexMap::new(), 16.0).unwrap();
        assert_eq!(result, 0.5);
    }

    #[test]
    fn test_channel_keyword_without_origin_fails() {
        let comp = ColorComponent::ColorChannel(ColorChannel::L);
        let result = comp.resolve(None, &IndexMap::new(), 16.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_keyword_with_origin() {
        let comp = ColorComponent::ColorChannel(ColorChannel::L);
        let origin = AbsoluteColor::new(ColorSpace::Oklch, 0.6, 0.2, 180.0, 1.0);
        let result = comp.resolve(Some(&origin), &IndexMap::new(), 16.0).unwrap();
        assert!((result - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_none_component() {
        let comp = ColorComponent::None;
        let result = comp.resolve(None, &IndexMap::new(), 16.0).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn test_calc_with_channel_keyword() {
        use crate::theme::calc::{CalcLeaf, CalcNode};

        // Build calc(l - 0.2) manually
        let calc = CalcNode::Sum(vec![
            CalcNode::Leaf(CalcLeaf::ColorChannel(ColorChannel::L)),
            CalcNode::Leaf(CalcLeaf::Number(-0.2)),
        ]);
        let comp = ColorComponent::Calc(calc);

        // Origin color with L = 0.6
        let origin = AbsoluteColor::new(ColorSpace::Oklch, 0.6, 0.2, 180.0, 1.0);
        let result = comp.resolve(Some(&origin), &IndexMap::new(), 16.0).unwrap();

        // Should be 0.6 - 0.2 = 0.4
        assert!((result - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_calc_with_runtime_param() {
        use crate::theme::calc::{CalcLeaf, CalcNode};

        // Build calc(l * 0.5) manually (using literal instead of variable for unit test)
        let calc = CalcNode::Product(vec![
            CalcNode::Leaf(CalcLeaf::ColorChannel(ColorChannel::L)),
            CalcNode::Leaf(CalcLeaf::Number(0.5)),
        ]);
        let comp = ColorComponent::Calc(calc);

        // Origin color with L = 0.8
        let origin = AbsoluteColor::new(ColorSpace::Oklch, 0.8, 0.2, 180.0, 1.0);

        let result = comp.resolve(Some(&origin), &IndexMap::new(), 16.0).unwrap();

        // Should be 0.8 * 0.5 = 0.4
        assert!((result - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_calc_with_hue_channel() {
        use crate::theme::calc::{CalcLeaf, CalcNode};

        // Build calc(h + 120) manually (hue is substituted as unitless number in degrees)
        let calc = CalcNode::Sum(vec![
            CalcNode::Leaf(CalcLeaf::ColorChannel(ColorChannel::H)),
            CalcNode::Leaf(CalcLeaf::Number(120.0)),
        ]);
        let comp = ColorComponent::Calc(calc);

        // Origin color with H = 180deg
        let origin = AbsoluteColor::new(ColorSpace::Oklch, 0.6, 0.2, 180.0, 1.0);
        let result = comp.resolve(Some(&origin), &IndexMap::new(), 16.0).unwrap();

        // Should be 180 + 120 = 300
        assert!((result - 300.0).abs() < 0.001);
    }
}
