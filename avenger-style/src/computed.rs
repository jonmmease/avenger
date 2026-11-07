//! Computed values: the absolute value of each property of each node after the cascade.

use std::{ops::Index, sync::Arc};

use crate::{
    condition::Condition, host::HostInputs, palette::Palette, source::Span, tree::NodeId,
    values::Color, vocab::Element, Property,
};

/// A computed value: lengths in pixels, angles in degrees, percentages as fractions, and colors
/// as sRGB RGBA.
#[derive(Clone, Debug, PartialEq)]
pub enum Computed {
    /// A number, a length in pixels, or an angle in degrees.
    Number(f32),
    /// A percentage, as a fraction.
    Percentage(f32),
    Integer(i64),
    Color([f32; 4]),
    /// A color that reads `currentColor`, such as `currentColor` itself or a `color-mix()` of
    /// it, resolved against the node's `color`, with its specified form. A node that inherits it
    /// resolves the specified form against its own `color`, as CSS Color 4 specifies.
    CurrentColor {
        color: [f32; 4],
        specified: Color,
    },
    Keyword(&'static str),
    Boolean(bool),
    /// A string, which the nodes that inherit it share.
    String(Arc<str>),
    Ratio(f32),
    /// A list, such as the families of a font, which the nodes that inherit it share.
    List(Arc<[Computed]>),
    Sequence(Arc<[Computed]>),
}

/// The paint of a fill or a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    None,
    Color([f32; 4]),
    /// The contrast color of what the label stands on, which the renderer picks for the label of
    /// a mark inside its item, and the node's `color` otherwise.
    Contrast {
        otherwise: [f32; 4],
    },
}

/// How `fill: auto` and `stroke: auto` paint a node, by its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoPaint {
    /// A filled shape or a text: `fill: auto` is the color.
    Fill,
    /// A line: `stroke: auto` is the color.
    Stroke,
    /// The label of a mark, whose fill contrasts with its item.
    MarkLabel,
    /// A node that draws no paint.
    Nothing,
}

/// The computed values of one node.
#[derive(Clone, Debug)]
pub struct ComputedStyle {
    pub element: Element,
    pub(crate) values: Vec<Computed>,
    pub(crate) auto_paint: AutoPaint,
}

impl ComputedStyle {
    /// The computed value of `property`.
    pub fn get(&self, property: Property) -> &Computed {
        &self.values[property as usize]
    }

    /// A number, a length in pixels, or an angle in degrees, or `None` for a keyword such as
    /// `auto` or `none`.
    pub fn number(&self, property: Property) -> Option<f32> {
        match self.get(property) {
            Computed::Number(value) => Some(*value),
            Computed::Integer(value) => Some(*value as f32),
            _ => None,
        }
    }

    /// The numbers of a property that takes one number or several separated by spaces, such as
    /// the paddings of the levels of a nested band scale.
    pub fn numbers(&self, property: Property) -> Option<Vec<f32>> {
        match self.get(property) {
            Computed::Number(value) => Some(vec![*value]),
            Computed::Sequence(items) => (items.iter())
                .map(|item| match item {
                    Computed::Number(value) => Some(*value),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }

    pub fn integer(&self, property: Property) -> Option<i64> {
        match self.get(property) {
            Computed::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn percentage(&self, property: Property) -> Option<f32> {
        match self.get(property) {
            Computed::Percentage(value) => Some(*value),
            _ => None,
        }
    }

    pub fn keyword(&self, property: Property) -> Option<&'static str> {
        match self.get(property) {
            Computed::Keyword(keyword) => Some(keyword),
            _ => None,
        }
    }

    pub fn boolean(&self, property: Property) -> Option<bool> {
        match self.get(property) {
            Computed::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn string(&self, property: Property) -> Option<&str> {
        match self.get(property) {
            Computed::String(value) => Some(value.as_ref()),
            _ => None,
        }
    }

    /// A color, or `None` for a keyword.
    pub fn color(&self, property: Property) -> Option<[f32; 4]> {
        match self.get(property) {
            Computed::Color(color) | Computed::CurrentColor { color, .. } => Some(*color),
            _ => None,
        }
    }

    /// The paint of `fill` or `stroke`, with `auto` resolved by the kind of the node: the color
    /// for the fill of a shape or a text and for the stroke of a line, and none otherwise.
    pub fn paint(&self, property: Property) -> Paint {
        let color = self.color(Property::Color).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        match self.get(property) {
            Computed::Color(color) | Computed::CurrentColor { color, .. } => Paint::Color(*color),
            Computed::Keyword("auto") => match (property, self.auto_paint) {
                (Property::Fill, AutoPaint::Fill) | (Property::Stroke, AutoPaint::Stroke) => {
                    Paint::Color(color)
                }
                (Property::Fill, AutoPaint::MarkLabel) => Paint::Contrast { otherwise: color },
                _ => Paint::None,
            },
            _ => Paint::None,
        }
    }

    /// The font size in pixels.
    pub fn font_size(&self) -> f32 {
        self.number(Property::FontSize).unwrap_or(10.0)
    }

    /// The line height in pixels: `normal` is the font size plus 2 pixels, and a number multiplies
    /// the font size.
    pub fn line_height(&self) -> f32 {
        match self.get(Property::LineHeight) {
            Computed::Number(value) => *value,
            Computed::Ratio(factor) => factor * self.font_size(),
            _ => self.font_size() + 2.0,
        }
    }

    /// Whether `display` hides the node.
    pub fn is_hidden(&self) -> bool {
        self.keyword(Property::Display) == Some("none")
    }
}

/// A warning of the cascade: a value that does not draw as written.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleWarning {
    /// A value of `property` on `node`.
    Property {
        node: NodeId,
        property: String,
        message: String,
    },
    /// The colors of an `@palette` rule after substitution, which leave the palette undefined.
    Palette {
        name: String,
        /// The span of the rule's name.
        span: Span,
        message: String,
    },
}

impl StyleWarning {
    pub fn message(&self) -> &str {
        match self {
            StyleWarning::Property { message, .. } | StyleWarning::Palette { message, .. } => {
                message
            }
        }
    }
}

/// The computed values of every node of a tree, for one set of host inputs.
#[derive(Clone, Debug)]
pub struct ComputedStyles {
    /// The styles of the nodes, which siblings that selectors cannot tell apart share.
    pub(crate) styles: Vec<Arc<ComputedStyle>>,
    pub(crate) warnings: Vec<StyleWarning>,
    pub(crate) palettes: Vec<Arc<Palette>>,
    pub(crate) host: HostInputs,
    /// The conditions of the rules and the layer declarations, with the outcome of each at
    /// `host`.
    pub(crate) conditions: Vec<(Condition, bool)>,
}

impl ComputedStyles {
    /// The paint of `fill` or `stroke` of `node` with `current` as its `currentColor`, such as the
    /// color of one row of a mark: a `color-mix()` or a relative color of `currentColor` resolves
    /// against `current` at the host inputs of these styles, and `auto` paints `current` where
    /// the kind of the node paints its color. A paint that does not read `currentColor` is
    /// [`ComputedStyle::paint`].
    pub fn paint_with(&self, node: NodeId, property: Property, current: [f32; 4]) -> Paint {
        let style = &self[node];
        match style.get(property) {
            Computed::CurrentColor { specified, .. } => Paint::Color(
                crate::cascade::resolve_color(specified, current, &self.host),
            ),
            Computed::Keyword("auto") => match (property, style.auto_paint) {
                (Property::Fill, AutoPaint::Fill) | (Property::Stroke, AutoPaint::Stroke) => {
                    Paint::Color(current)
                }
                (Property::Fill, AutoPaint::MarkLabel) => Paint::Contrast { otherwise: current },
                _ => Paint::None,
            },
            _ => style.paint(property),
        }
    }
}

impl Index<NodeId> for ComputedStyles {
    type Output = ComputedStyle;

    fn index(&self, node: NodeId) -> &ComputedStyle {
        &self.styles[node.index()]
    }
}

impl ComputedStyles {
    pub fn warnings(&self) -> &[StyleWarning] {
        &self.warnings
    }

    /// The palette of `name`: one that an `@palette` rule defines, by cascade precedence, or a
    /// built-in palette. Names are case-insensitive.
    pub fn palette(&self, name: &str) -> Option<&Palette> {
        let name = name.to_ascii_lowercase();
        (self.palettes.iter())
            .find(|palette| palette.name == name)
            .map(AsRef::as_ref)
    }

    /// Whether a render at `host` needs new computed styles: when the preferences or the host
    /// theme change, or when the size changes and a condition changes its outcome. No value
    /// reads the size of the host box, so a resize within a breakpoint keeps the styles.
    pub fn needs_update(&self, host: &HostInputs) -> bool {
        if !self.host.same_preferences_and_theme(host) {
            return true;
        }
        if self.host.width == host.width && self.host.height == host.height {
            return false;
        }
        (self.conditions.iter())
            .any(|(condition, outcome)| crate::conditions::holds(condition, host) != *outcome)
    }
}
