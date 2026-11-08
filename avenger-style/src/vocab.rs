//! The vocabulary of the style tree: its elements and the attributes that each element has.

use std::fmt;

macro_rules! elements {
    ($($variant:ident = $name:literal,)+) => {
        /// An element of the style tree: the kind of a node.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Element {
            $($variant,)+
        }

        impl Element {
            pub const ALL: &'static [Element] = &[$(Element::$variant,)+];

            /// The element's name in selectors.
            pub fn name(self) -> &'static str {
                match self {
                    $(Element::$variant => $name,)+
                }
            }

            /// The element of `name`, which selectors match case-sensitively.
            pub fn from_name(name: &str) -> Option<Element> {
                match name {
                    $($name => Some(Element::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

elements! {
    Chart = "chart",
    Title = "title",
    Subtitle = "subtitle",
    Footer = "footer",
    Description = "description",
    Plot = "plot",
    Mark = "mark",
    Axis = "axis",
    Legend = "legend",
    Facet = "facet",
    Scale = "scale",
    Attribution = "attribution",
    Graticule = "graticule",
    Level = "level",
    Label = "label",
    Tick = "tick",
    Rule = "rule",
    Grid = "grid",
    Swatch = "swatch",
    Ramp = "ramp",
    Item = "item",
    Group = "group",
    Entry = "entry",
    Widget = "widget",
    Track = "track",
    Thumb = "thumb",
    Readout = "readout",
}

impl Element {
    /// Whether the element is a virtual node, which keyboard focus reaches and which a selector
    /// matches only through `:focus-visible`.
    pub fn is_virtual(self) -> bool {
        matches!(self, Element::Item | Element::Group | Element::Entry)
    }

    /// Whether a node of the element can have keyboard focus, so `:focus-visible` is valid on it.
    pub fn can_focus(self) -> bool {
        matches!(
            self,
            Element::Chart
                | Element::Widget
                | Element::Axis
                | Element::Legend
                | Element::Mark
                | Element::Entry
                | Element::Group
                | Element::Item
        )
    }

    /// The names of the attributes that nodes of the element have.
    pub fn attributes(self) -> &'static [&'static str] {
        match self {
            Element::Plot => &["type", "projection", "discrete"],
            Element::Mark => &["type", "store"],
            Element::Axis => &["channel", "scale", "kind"],
            Element::Legend => &["type", "channel", "scale", "kind"],
            Element::Scale => &["type", "channel", "kind", "marks"],
            Element::Facet => &["type"],
            Element::Widget => &["type", "key"],
            _ => &[],
        }
    }

    pub fn has_attribute(self, name: &str) -> bool {
        self.attributes().contains(&name)
    }
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The kinds of the marks of avenger-chart, which the `type` of a mark names along with its
/// function name.
pub const MARK_KINDS: &[&str] = &[
    "symbol", "line", "trail", "area", "bar", "rect", "rule", "text", "image", "path", "shape",
];

/// Whether `name` names a mark, as a kind such as `bar` or as a function name such as
/// `polar_bar`.
pub fn is_mark_name(name: &str) -> bool {
    let kind = name
        .strip_prefix("polar_")
        .or_else(|| name.strip_prefix("geo_"))
        .unwrap_or(name);
    MARK_KINDS.contains(&kind)
}

/// Whether any element has an attribute named `name`.
pub fn is_attribute(name: &str) -> bool {
    Element::ALL
        .iter()
        .any(|element| element.has_attribute(name))
}
