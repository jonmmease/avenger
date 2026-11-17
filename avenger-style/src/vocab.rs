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
    Plot = "plot",
    Mark = "mark",
    Axis = "axis",
    Legend = "legend",
    Facet = "facet",
    Scale = "scale",
    Attribution = "attribution",
    Graticule = "graticule",
    Sphere = "sphere",
    Level = "level",
    Label = "label",
    Tick = "tick",
    Rule = "rule",
    Grid = "grid",
    Swatch = "swatch",
    Ramp = "ramp",
    Widget = "widget",
    Rail = "rail",
    Thumb = "thumb",
    Readout = "readout",
    Other = "other",
    Null = "null",
}

impl Element {
    /// Whether the element is a variant of its parent for a category that a chart styles apart
    /// from its scale: `other`, the Other group of a lump, or `null`, the NULL category. Every
    /// property applies to a variant, which keeps each value of its parent that no declaration
    /// changes.
    pub fn is_variant(self) -> bool {
        matches!(self, Element::Other | Element::Null)
    }

    /// The elements of the children that a node of the element can have, its parts. A chart is a
    /// part of a plot, as the charts of a composition stand in its plot, and the variants are
    /// parts of the marks and the labels whose categories they style.
    pub fn parts(self) -> &'static [Element] {
        use Element::*;
        match self {
            Chart => &[Title, Plot],
            Title => &[Subtitle],
            Plot => &[
                Mark,
                Axis,
                Legend,
                Facet,
                Scale,
                Attribution,
                Sphere,
                Graticule,
                Widget,
                Chart,
            ],
            Mark | Label => &[Other, Null],
            Axis => &[Title, Label, Tick, Rule, Grid, Level],
            Level => &[Label],
            Legend => &[Title, Swatch, Label, Ramp, Tick, Rule],
            Facet => &[Title, Label, Tick, Rule],
            Graticule => &[Grid, Label],
            Widget => &[Title, Label, Rail, Thumb, Readout],
            Subtitle | Scale | Attribution | Sphere | Tick | Rule | Grid | Swatch | Ramp | Rail
            | Thumb | Readout | Other | Null => &[],
        }
    }

    /// Whether a node of the element can have keyboard focus, so `:focus-visible` is valid on it:
    /// a widget, which Tab focuses and which draws a ring around its control while it has focus.
    pub fn can_focus(self) -> bool {
        self == Element::Widget
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
/// prefixed name.
pub const MARK_KINDS: &[&str] = &[
    "symbol", "line", "trail", "area", "bar", "rect", "rule", "text", "image", "path", "shape",
];

/// Whether `name` names a mark, as a kind such as `bar` or as a prefixed name, the kind with the
/// prefix of a coordinate system, such as `polar_bar`.
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
