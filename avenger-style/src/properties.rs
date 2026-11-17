//! The properties of the style language: their names, grammars, inheritance, initial values, and
//! the elements that accept them, which cover the style fields of the avenger-chart format.

use cssparser::Parser;

use crate::{
    values::{self, Syntax, Value},
    vocab::Element::{self, *},
};

const LENGTH: Syntax = Syntax::Length { min: None };
const SIZE: Syntax = Syntax::Length { min: Some(0.0) };
const UNIT: Syntax = Syntax::Number {
    min: Some(0.0),
    max: Some(1.0),
};
const NON_NEGATIVE: Syntax = Syntax::Number {
    min: Some(0.0),
    max: None,
};
/// The paddings of a scale: one number, or one for each level of a nested band scale, outermost
/// first.
const PADDINGS: Syntax = Syntax::Any(&[NON_NEGATIVE, Syntax::Repeat(&NON_NEGATIVE, 2, 8)]);
const POSITIVE_INTEGER: Syntax = Syntax::Integer { min: 1 };
const SIZE_OR_AUTO: Syntax = Syntax::Any(&[SIZE, Syntax::Keywords(&["auto"])]);
const SIZE_OR_NONE: Syntax = Syntax::Any(&[SIZE, Syntax::Keywords(&["none"])]);
const PAINT: Syntax = Syntax::Any(&[Syntax::Keywords(&["auto", "none"]), Syntax::Color]);
const LINE_STYLE: Syntax = Syntax::Keywords(&["none", "solid", "dashed", "dotted"]);
const OUTPUT_OR_AUTO: Syntax = Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::OutputItem]);

/// Every element except the variants, `other` and `null`.
const NODES: &[Element] = &[
    Chart,
    Title,
    Subtitle,
    Plot,
    Mark,
    Axis,
    Legend,
    Facet,
    Scale,
    Attribution,
    Graticule,
    Level,
    Label,
    Tick,
    Rule,
    Grid,
    Swatch,
    Ramp,
    Widget,
    Track,
    Thumb,
    Readout,
];
/// The nodes that read `color`: through a text, a paint, `currentColor`, or the nodes below them.
const COLORED: &[Element] = &[
    Chart,
    Title,
    Subtitle,
    Plot,
    Mark,
    Axis,
    Legend,
    Facet,
    Scale,
    Attribution,
    Graticule,
    Label,
    Tick,
    Rule,
    Grid,
    Widget,
    Track,
    Thumb,
    Readout,
];
/// The nodes that draw text, and the nodes above them, which pass a font to them.
const FONTED: &[Element] = &[
    Chart,
    Title,
    Subtitle,
    Plot,
    Mark,
    Axis,
    Legend,
    Facet,
    Attribution,
    Graticule,
    Label,
    Widget,
    Readout,
];
/// The nodes that draw text in a style of their own, and the nodes above them.
const STYLED_TEXTS: &[Element] = &[Chart, Title, Subtitle, Plot, Mark, Widget, Readout];
/// The nodes that lay out text in a box of their own: text marks, and the labels and titles of
/// guides.
const TEXT_BOXES: &[Element] = &[Mark, Title, Subtitle, Label];
/// The nodes that fill a shape or a text.
const FILLED: &[Element] = &[
    Mark,
    Title,
    Subtitle,
    Attribution,
    Label,
    Track,
    Thumb,
    Readout,
];
/// The nodes that stroke a line or an outline.
const STROKED: &[Element] = &[Mark, Tick, Rule, Grid, Track, Thumb];
const LINES: &[Element] = &[Mark, Tick, Rule, Grid];
const BOXES: &[Element] = &[Chart, Plot, Legend, Attribution, Widget];
/// A widget and the parts of its control.
const WIDGET: &[Element] = &[Widget, Track, Thumb];
/// The nodes that can have keyboard focus, which draw a focus ring.
const FOCUS: &[Element] = &[Widget];
const MARK: &[Element] = &[Mark];
const PLOT: &[Element] = &[Plot];
const SCALE: &[Element] = &[Scale];

// The sets of elements of the properties are bits of a `u32`.
const _: () = assert!(Element::ALL.len() < 32);

/// A set of elements as bits, by their positions in `Element`, which has fewer than 32 elements.
const fn element_mask(elements: &[Element]) -> u32 {
    let mut mask = 0;
    let mut index = 0;
    while index < elements.len() {
        mask |= 1 << elements[index] as u32;
        index += 1;
    }
    mask
}

macro_rules! properties {
    ($($variant:ident = $name:literal, $inherited:literal, $syntax:expr, $initial:literal, $elements:expr;)+) => {
        /// A longhand property.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Property {
            $($variant,)+
        }

        impl Property {
            pub const ALL: &'static [Property] = &[$(Property::$variant,)+];

            /// The property's CSS name.
            pub fn name(self) -> &'static str {
                match self {
                    $(Property::$variant => $name,)+
                }
            }

            /// The property of a CSS name.
            pub fn from_name(name: &str) -> Option<Property> {
                match name {
                    $($name => Some(Property::$variant),)+
                    _ => None,
                }
            }

            /// The grammar of the property's values: the union of its grammars on every element
            /// that accepts it.
            pub fn syntax(self) -> &'static Syntax {
                match self {
                    $(Property::$variant => {
                        const SYNTAX: Syntax = $syntax;
                        &SYNTAX
                    })+
                }
            }

            /// Whether a node without a declaration of the property takes its parent's value.
            #[inline]
            pub fn is_inherited(self) -> bool {
                const INHERITED: &[bool] = &[$($inherited,)+];
                INHERITED[self as usize]
            }

            /// The elements of `elements` as bits, by their positions in `Element`.
            #[inline]
            fn element_mask(self) -> u32 {
                const MASKS: &[u32] = &[$(element_mask($elements),)+];
                MASKS[self as usize]
            }

            /// The initial value, as CSS text. Some properties have an initial value that depends
            /// on the node, which the cascade gives instead.
            pub fn initial(self) -> &'static str {
                match self {
                    $(Property::$variant => $initial,)+
                }
            }

            /// The elements whose nodes accept the property. A declaration on another node has no
            /// effect.
            pub fn elements(self) -> &'static [Element] {
                match self {
                    $(Property::$variant => $elements,)+
                }
            }
        }
    };
}

properties! {
    // Size and composition.
    Width = "width", false, SIZE_OR_AUTO, "auto", &[Plot, Mark];
    Height = "height", false, SIZE_OR_AUTO, "auto", &[Plot, Mark];
    ContainIntrinsicWidth = "contain-intrinsic-width", false, SIZE_OR_NONE, "none", PLOT;
    ContainIntrinsicHeight = "contain-intrinsic-height", false, SIZE_OR_NONE, "none", PLOT;
    AspectRatio = "aspect-ratio", false, Syntax::Any(&[Syntax::Ratio, Syntax::Keywords(&["auto"])]), "auto", PLOT;
    Step = "step", false, SIZE_OR_NONE, "none", SCALE;
    PaddingTop = "padding-top", false, SIZE, "0", BOXES;
    PaddingRight = "padding-right", false, SIZE, "0", BOXES;
    PaddingBottom = "padding-bottom", false, SIZE, "0", BOXES;
    PaddingLeft = "padding-left", false, SIZE, "0", BOXES;
    Columns = "columns", false, Syntax::Any(&[POSITIVE_INTEGER, Syntax::Keywords(&["auto", "none"])]), "auto", &[Plot, Legend];
    MaxColumns = "max-columns", false, Syntax::Any(&[POSITIVE_INTEGER, Syntax::Keywords(&["none"])]), "none", PLOT;
    MinColumnWidth = "min-column-width", false, SIZE_OR_AUTO, "auto", PLOT;
    Align = "align", false, Syntax::Any(&[Syntax::Boolean, Syntax::Keywords(&["auto", "left", "center", "right"])]), "auto", &[Plot, Mark];
    RowGap = "row-gap", false, SIZE, "0", &[Plot, Legend];
    ColumnGap = "column-gap", false, SIZE, "0", &[Plot, Legend];

    // Paint and marks.
    Color = "color", true, Syntax::Color, "black", COLORED;
    AccentColor = "accent-color", true, Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::Color]), "auto", WIDGET;
    Fill = "fill", true, PAINT, "auto", FILLED;
    Stroke = "stroke", true, PAINT, "auto", STROKED;
    Opacity = "opacity", false, UNIT, "1", MARK;
    FillOpacity = "fill-opacity", false, UNIT, "1", MARK;
    StrokeOpacity = "stroke-opacity", false, UNIT, "1", MARK;
    StrokeWidth = "stroke-width", false, SIZE, "1", LINES;
    StrokeDasharray = "stroke-dasharray", false, Syntax::Any(&[Syntax::Keywords(&["none"]), Syntax::Dash]), "none", LINES;
    StrokeLinecap = "stroke-linecap", false, Syntax::Keywords(&["butt", "round", "square"]), "butt", MARK;
    StrokeLinejoin = "stroke-linejoin", false, Syntax::Keywords(&["miter", "round", "bevel"]), "miter", MARK;
    Shape = "shape", false, Syntax::Shape, "circle", &[Mark, Swatch];
    Angle = "angle", false, Syntax::Any(&[Syntax::Angle, Syntax::Keywords(&["auto"])]), "0", &[Mark, Title, Axis, Label];
    Size = "size", false, Syntax::Any(&[NON_NEGATIVE, Syntax::Percentage]), "30", MARK;
    ContinuousSize = "continuous-size", false, SIZE, "5", MARK;
    BinSpacing = "bin-spacing", false, SIZE, "0", MARK;
    BandPosition = "band-position", false, UNIT, "0.5", &[Mark, Axis];
    CornerRadius = "corner-radius", false, SIZE, "0", MARK;
    PadAngle = "pad-angle", false, Syntax::Angle, "0", MARK;
    Inset = "inset", false, SIZE, "0", MARK;
    Baseline = "baseline", false, Syntax::Keywords(&["top", "middle", "bottom", "alphabetic", "line-top", "line-bottom"]), "middle", MARK;
    WidthMode = "width-mode", false, Syntax::Keywords(&["max", "fixed"]), "max", MARK;
    WhiteSpace = "white-space", false, Syntax::Keywords(&["normal", "nowrap"]), "normal", TEXT_BOXES;
    TextOverflow = "text-overflow", false, Syntax::Keywords(&["clip", "ellipsis"]), "clip", TEXT_BOXES;
    RadiusSide = "radius-side", false, Syntax::Keywords(&["outward", "middle", "inward"]), "outward", MARK;
    ThetaSide = "theta-side", false, Syntax::Keywords(&["clockwise", "middle", "counterclockwise"]), "middle", MARK;
    TextOrient = "text-orient", false, Syntax::Keywords(&["horizontal", "radial", "tangential"]), "horizontal", &[Mark, Label];
    ObjectFit = "object-fit", false, Syntax::Keywords(&["contain", "fill"]), "contain", MARK;
    ImageRendering = "image-rendering", false, Syntax::Keywords(&["auto", "smooth", "pixelated"]), "auto", MARK;
    Pattern = "pattern", false, Syntax::Pattern, "none", &[Mark, Swatch];

    // Text.
    FontSize = "font-size", true, SIZE, "1rem", FONTED;
    FontWeight = "font-weight", true, Syntax::FontWeight, "normal", FONTED;
    FontFamily = "font-family", true, Syntax::FontFamilies, "sans-serif", FONTED;
    FontStyle = "font-style", true, Syntax::Keywords(&["normal", "italic"]), "normal", STYLED_TEXTS;
    LineHeight = "line-height", true, Syntax::Any(&[Syntax::Keywords(&["normal"]), Syntax::Number { min: Some(0.0), max: None }, SIZE]), "normal", &[Chart, Plot, Mark, Axis, Legend, Facet, Graticule, Title, Subtitle, Label];
    MaxWidth = "max-width", false, SIZE_OR_NONE, "none", TEXT_BOXES;
    MaxLines = "max-lines", false, Syntax::Any(&[POSITIVE_INTEGER, Syntax::Keywords(&["none"])]), "none", TEXT_BOXES;
    TextAlign = "text-align", false, Syntax::Keywords(&["start", "center", "end", "auto"]), "auto", &[Title, Label, Attribution];
    VerticalAlign = "vertical-align", false, Syntax::Keywords(&["top", "middle", "bottom", "alphabetic", "auto"]), "auto", &[Label];

    // Guides.
    Orient = "orient", false, Syntax::Keywords(&["top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right"]), "bottom", &[Axis, Legend, Facet, Title, Attribution, Widget];
    Wrap = "wrap", false, Syntax::Keywords(&["flow", "grid"]), "flow", &[Legend];
    ValueCount = "value-count", false, Syntax::Any(&[Syntax::Number { min: Some(0.0), max: None }, Syntax::Keywords(&["auto"])]), "auto", &[Axis, Legend];
    ValueSpacing = "value-spacing", false, SIZE_OR_AUTO, "auto", &[Axis];
    ValueThinning = "value-thinning", false, Syntax::Boolean, "false", &[Axis];
    MaxEntries = "max-entries", false, Syntax::Any(&[Syntax::Integer { min: 2 }, Syntax::Keywords(&["none"])]), "none", &[Legend];
    Length = "length", false, SIZE, "5", &[Tick];
    Margin = "margin", false, LENGTH, "0", &[Label, Title, Ramp];
    Offset = "offset", false, LENGTH, "0", &[Axis, Legend, Facet, Title, Attribution, Widget];
    Thickness = "thickness", false, SIZE, "16", &[Ramp];
    MaxLength = "max-length", false, SIZE_OR_NONE, "none", &[Ramp];
    Overlap = "overlap", false, Syntax::Keywords(&["none", "parity", "greedy"]), "none", &[Label];
    Separator = "separator", false, Syntax::String, "\" · \"", &[Attribution];
    SeparatorSpace = "separator-space", false, SIZE, "4", &[Attribution];
    LineGap = "line-gap", false, SIZE, "2", &[Attribution];

    // Boxes, visibility, focus, and forced colors.
    Background = "background", false, Syntax::Color, "transparent", BOXES;
    BorderWidth = "border-width", false, SIZE, "1", BOXES;
    BorderStyle = "border-style", false, LINE_STYLE, "none", BOXES;
    BorderColor = "border-color", false, Syntax::Color, "currentColor", BOXES;
    BorderRadius = "border-radius", false, SIZE, "0", BOXES;
    // The focus ring of a widget, which applies only through `:focus-visible`. `auto` keeps the
    // widget's own style and color.
    OutlineWidth = "outline-width", false, SIZE, "2", FOCUS;
    OutlineStyle = "outline-style", false, Syntax::Keywords(&["auto", "none", "solid"]), "auto", FOCUS;
    OutlineColor = "outline-color", false, Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::Color]), "auto", FOCUS;
    OutlineOffset = "outline-offset", false, LENGTH, "2", FOCUS;
    Display = "display", false, Syntax::Keywords(&["none", "block", "inline", "inline-block", "flex", "inline-flex", "grid", "inline-grid", "contents", "flow-root", "list-item", "table"]), "block", NODES;
    ForcedColorAdjust = "forced-color-adjust", true, Syntax::Keywords(&["auto", "none"]), "auto", NODES;

    // Scales.
    Scheme = "scheme", false, Syntax::Scheme, "auto", SCALE;
    Range = "range", false, Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::OutputList]), "auto", SCALE;
    Unknown = "unknown", false, OUTPUT_OR_AUTO, "auto", SCALE;
    Other = "other", false, OUTPUT_OR_AUTO, "auto", SCALE;
    PaddingInner = "padding-inner", false, PADDINGS, "0.1", SCALE;
    PaddingOuter = "padding-outer", false, PADDINGS, "0.05", SCALE;
}

impl Property {
    /// Whether the property draws the focus ring, which applies only through `:focus-visible`.
    pub fn is_outline(self) -> bool {
        matches!(
            self,
            Property::OutlineWidth
                | Property::OutlineStyle
                | Property::OutlineColor
                | Property::OutlineOffset
        )
    }

    #[inline]
    /// Whether the property applies to `element`. Every property applies to a variant of its
    /// parent, `other` or `null`, which takes what its parent takes.
    pub fn applies_to(self, element: Element) -> bool {
        element.is_variant() || self.element_mask() & (1 << element as u32) != 0
    }

    /// Parses a value of the property from CSS text without `var()`, `env()`, or a CSS-wide
    /// keyword, such as the value of a style field of a chart.
    pub fn parse_value(self, text: &str) -> Option<Value> {
        let mut input = Parser::new(text);
        values::parse_all(self.syntax(), &mut input).ok()
    }
}

/// A shorthand property, which a stylesheet writes and which sets its longhands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shorthand {
    /// One to four lengths for the sides, and with one number the paddings of a scale.
    Padding,
    Border,
    Outline,
    /// One length for rows and columns, or two for rows and then columns.
    Gap,
    /// `none`, or one length for the width and the height, or two for the width and then the
    /// height, that a plot counts at when nothing gives it a size.
    ContainIntrinsicSize,
    /// Every property except the outline properties and the custom properties. It takes only a
    /// CSS-wide keyword.
    All,
}

impl Shorthand {
    pub fn from_name(name: &str) -> Option<Shorthand> {
        Some(match name {
            "padding" => Shorthand::Padding,
            "border" => Shorthand::Border,
            "outline" => Shorthand::Outline,
            "gap" => Shorthand::Gap,
            "contain-intrinsic-size" => Shorthand::ContainIntrinsicSize,
            "all" => Shorthand::All,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Shorthand::Padding => "padding",
            Shorthand::Border => "border",
            Shorthand::Outline => "outline",
            Shorthand::Gap => "gap",
            Shorthand::ContainIntrinsicSize => "contain-intrinsic-size",
            Shorthand::All => "all",
        }
    }

    /// The longhands that the shorthand sets.
    pub fn longhands(self) -> Vec<Property> {
        match self {
            Shorthand::Padding => vec![
                Property::PaddingTop,
                Property::PaddingRight,
                Property::PaddingBottom,
                Property::PaddingLeft,
                Property::PaddingInner,
                Property::PaddingOuter,
            ],
            Shorthand::Border => vec![
                Property::BorderWidth,
                Property::BorderStyle,
                Property::BorderColor,
            ],
            Shorthand::Outline => vec![
                Property::OutlineWidth,
                Property::OutlineStyle,
                Property::OutlineColor,
            ],
            Shorthand::Gap => vec![Property::RowGap, Property::ColumnGap],
            Shorthand::ContainIntrinsicSize => vec![
                Property::ContainIntrinsicWidth,
                Property::ContainIntrinsicHeight,
            ],
            Shorthand::All => (Property::ALL.iter())
                .copied()
                .filter(|property| !property.is_outline())
                .collect(),
        }
    }

    /// Whether the shorthand sets outline properties, which apply only through `:focus-visible`.
    pub fn is_outline(self) -> bool {
        self == Shorthand::Outline
    }
}
