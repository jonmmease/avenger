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
const SIDES: &[&str] = &["top", "bottom", "left", "right"];
const LINE_STYLE: Syntax = Syntax::Keywords(&["none", "solid", "dashed", "dotted"]);
const OUTPUT_OR_AUTO: Syntax = Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::OutputItem]);

/// Every element that is not a virtual node.
const NODES: &[Element] = &[
    Chart,
    Title,
    Subtitle,
    Footer,
    Description,
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
/// The nodes that draw a paint.
const PAINTED: &[Element] = &[
    Mark,
    Title,
    Subtitle,
    Footer,
    Description,
    Attribution,
    Label,
    Tick,
    Rule,
    Grid,
    Swatch,
    Ramp,
    Track,
    Thumb,
    Readout,
];
/// The nodes that draw text.
const TEXTS: &[Element] = &[
    Title,
    Subtitle,
    Footer,
    Description,
    Label,
    Mark,
    Attribution,
    Readout,
];
const LINES: &[Element] = &[Mark, Tick, Rule, Grid];
const BOXES: &[Element] = &[Chart, Plot, Legend, Attribution, Widget];
const GUIDES: &[Element] = &[Axis, Legend, Facet];
const FOCUS: &[Element] = &[Chart, Axis, Legend, Mark, Entry, Group, Item, Widget];
/// A widget and the parts of its control.
const WIDGET: &[Element] = &[Widget, Track, Thumb];
const MARK: &[Element] = &[Mark];
const PLOT: &[Element] = &[Plot];
const SCALE: &[Element] = &[Scale];
const AXIS: &[Element] = &[Axis];

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
    Axes = "axes", false, Syntax::Keywords(&["outer", "each"]), "each", PLOT;
    RowGap = "row-gap", false, SIZE, "0", &[Plot, Legend];
    ColumnGap = "column-gap", false, SIZE, "0", &[Plot, Legend];

    // Paint and marks.
    Color = "color", true, Syntax::Color, "black", NODES;
    AccentColor = "accent-color", true, Syntax::Any(&[Syntax::Keywords(&["auto"]), Syntax::Color]), "auto", WIDGET;
    Fill = "fill", true, PAINT, "auto", PAINTED;
    Stroke = "stroke", true, PAINT, "auto", PAINTED;
    Opacity = "opacity", false, UNIT, "1", MARK;
    FillOpacity = "fill-opacity", false, UNIT, "1", MARK;
    StrokeOpacity = "stroke-opacity", false, UNIT, "1", MARK;
    StrokeWidth = "stroke-width", false, SIZE, "1", LINES;
    StrokeDash = "stroke-dash", false, Syntax::Any(&[Syntax::Keywords(&["none"]), Syntax::Dash]), "none", LINES;
    StrokeDashoffset = "stroke-dashoffset", false, LENGTH, "0", MARK;
    StrokeLinecap = "stroke-linecap", false, Syntax::Keywords(&["butt", "round", "square"]), "butt", MARK;
    StrokeLinejoin = "stroke-linejoin", false, Syntax::Keywords(&["miter", "round", "bevel"]), "miter", MARK;
    StrokeMiterlimit = "stroke-miterlimit", false, Syntax::Number { min: Some(1.0), max: None }, "4", MARK;
    Shape = "shape", false, Syntax::Shape, "circle", &[Mark, Swatch];
    Angle = "angle", false, Syntax::Angle, "0", &[Mark, Title, Axis];
    Size = "size", false, Syntax::Any(&[NON_NEGATIVE, Syntax::Percentage]), "30", MARK;
    ContinuousSize = "continuous-size", false, SIZE, "5", MARK;
    BinSpacing = "bin-spacing", false, SIZE, "0", MARK;
    BandPosition = "band-position", false, UNIT, "0.5", &[Mark, Axis];
    BandPadding = "band-padding", false, UNIT, "0", MARK;
    CornerRadius = "corner-radius", false, SIZE, "0", MARK;
    CornerRadiusEnd = "corner-radius-end", false, SIZE_OR_AUTO, "auto", MARK;
    CornerRadiusTopLeft = "corner-radius-top-left", false, SIZE_OR_AUTO, "auto", MARK;
    CornerRadiusTopRight = "corner-radius-top-right", false, SIZE_OR_AUTO, "auto", MARK;
    CornerRadiusBottomLeft = "corner-radius-bottom-left", false, SIZE_OR_AUTO, "auto", MARK;
    CornerRadiusBottomRight = "corner-radius-bottom-right", false, SIZE_OR_AUTO, "auto", MARK;
    PadAngle = "pad-angle", false, Syntax::Angle, "0", MARK;
    Dx = "dx", false, LENGTH, "0", MARK;
    Dy = "dy", false, LENGTH, "0", MARK;
    Dx2 = "dx2", false, LENGTH, "0", MARK;
    Dy2 = "dy2", false, LENGTH, "0", MARK;
    DRadius = "d-radius", false, LENGTH, "0", MARK;
    DTheta = "d-theta", false, Syntax::Angle, "0", MARK;
    Inset = "inset", false, SIZE, "0", MARK;
    Baseline = "baseline", false, Syntax::Keywords(&["top", "middle", "bottom", "alphabetic", "line-top", "line-bottom"]), "middle", MARK;
    WidthMode = "width-mode", false, Syntax::Keywords(&["max", "fixed"]), "max", MARK;
    WhiteSpace = "white-space", false, Syntax::Keywords(&["normal", "nowrap"]), "normal", MARK;
    TextOverflow = "text-overflow", false, Syntax::Keywords(&["clip", "ellipsis"]), "clip", MARK;
    RadiusSide = "radius-side", false, Syntax::Keywords(&["outward", "middle", "inward"]), "outward", MARK;
    ThetaSide = "theta-side", false, Syntax::Keywords(&["clockwise", "middle", "counterclockwise"]), "middle", MARK;
    TextOrient = "text-orient", false, Syntax::Keywords(&["horizontal", "radial", "tangential"]), "horizontal", MARK;
    ObjectFit = "object-fit", false, Syntax::Keywords(&["contain", "fill"]), "contain", MARK;
    ImageRendering = "image-rendering", false, Syntax::Keywords(&["auto", "smooth", "pixelated"]), "auto", MARK;
    Pattern = "pattern", false, Syntax::Pattern, "none", &[Mark, Swatch];

    // Text.
    FontSize = "font-size", true, SIZE, "1rem", NODES;
    FontWeight = "font-weight", true, Syntax::FontWeight, "normal", NODES;
    FontFamily = "font-family", true, Syntax::FontFamilies, "sans-serif", NODES;
    FontStyle = "font-style", true, Syntax::Keywords(&["normal", "italic"]), "normal", NODES;
    LineHeight = "line-height", true, Syntax::Any(&[Syntax::Keywords(&["normal"]), Syntax::Number { min: Some(0.0), max: None }, SIZE]), "normal", NODES;
    MaxWidth = "max-width", false, SIZE_OR_NONE, "none", TEXTS;
    MaxLines = "max-lines", false, Syntax::Any(&[POSITIVE_INTEGER, Syntax::Keywords(&["none"])]), "1", TEXTS;
    TextAlign = "text-align", false, Syntax::Keywords(&["start", "center", "end", "auto"]), "auto", TEXTS;
    VerticalAlign = "vertical-align", false, Syntax::Keywords(&["top", "middle", "bottom", "alphabetic", "auto"]), "auto", TEXTS;
    TitleFrame = "title-frame", false, Syntax::Keywords(&["plot", "chart"]), "chart", &[Chart];

    // Guides.
    LabelAngle = "label-angle", false, Syntax::Any(&[Syntax::Angle, Syntax::Keywords(&["auto"])]), "0", &[Axis, Facet, Legend, Level];
    Orient = "orient", false, Syntax::Keywords(&["top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right"]), "bottom", &[Axis, Legend, Facet, Title, Attribution, Widget];
    LabelOrient = "label-orient", false, Syntax::Keywords(&["top", "bottom", "left", "right", "radial", "tangential"]), "top", &[Facet, Axis];
    TitleOrient = "title-orient", false, Syntax::Keywords(SIDES), "top", &[Facet];
    Direction = "direction", false, Syntax::Keywords(&["horizontal", "vertical"]), "vertical", &[Legend];
    Wrap = "wrap", false, Syntax::Keywords(&["flow", "grid"]), "flow", &[Legend];
    TickCount = "tick-count", false, Syntax::Any(&[Syntax::Number { min: Some(0.0), max: None }, Syntax::Keywords(&["auto"])]), "auto", &[Axis, Legend];
    TickSpacing = "tick-spacing", false, SIZE_OR_AUTO, "auto", &[Axis];
    ThinTicks = "thin-ticks", false, Syntax::Boolean, "false", &[Axis];
    MaxEntries = "max-entries", false, Syntax::Any(&[Syntax::Integer { min: 2 }, Syntax::Keywords(&["none"])]), "none", &[Legend];
    TickSize = "tick-size", false, SIZE, "5", &[Tick];
    LabelPadding = "label-padding", false, LENGTH, "2", &[Axis, Legend, Facet, Graticule, Mark];
    TitlePadding = "title-padding", false, LENGTH, "4", GUIDES;
    Offset = "offset", false, LENGTH, "0", &[Axis, Legend, Facet, Title, Attribution, Widget];
    LabelOffset = "label-offset", false, LENGTH, "0", &[Axis, Mark];
    MinExtent = "min-extent", false, SIZE_OR_NONE, "none", AXIS;
    MaxExtent = "max-extent", false, SIZE_OR_NONE, "none", AXIS;
    Thickness = "thickness", false, SIZE, "16", &[Ramp];
    Length = "length", false, SIZE_OR_AUTO, "auto", &[Ramp];
    MaxLength = "max-length", false, SIZE_OR_NONE, "none", &[Ramp];
    AxisGap = "axis-gap", false, SIZE, "0", &[Legend];
    LabelOverlap = "label-overlap", false, Syntax::Keywords(&["none", "parity", "greedy"]), "none", &[Axis, Legend, Mark];
    LabelSeparation = "label-separation", false, SIZE, "0", AXIS;
    LabelFlush = "label-flush", false, Syntax::Any(&[Syntax::Boolean, SIZE]), "false", AXIS;
    TickBand = "tick-band", false, Syntax::Keywords(&["center", "extent"]), "center", AXIS;
    ZIndex = "z-index", false, Syntax::Integer { min: 0 }, "0", AXIS;
    Separator = "separator", false, Syntax::String, "\" · \"", &[Attribution];
    SeparatorSpace = "separator-space", false, SIZE, "4", &[Attribution];
    LineGap = "line-gap", false, SIZE, "2", &[Attribution];

    // The labels of marks.
    LabelPosition = "label-position", false, Syntax::KeywordList(&["inside", "inside-start", "inside-end", "outside", "middle", "top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right"]), "inside", MARK;
    LabelOverflow = "label-overflow", false, Syntax::Keywords(&["hide", "outside", "shrink", "show"]), "show", MARK;
    LabelAvoid = "label-avoid", false, Syntax::Keywords(&["labels", "marks"]), "labels", MARK;
    LabelMinFontSize = "label-min-font-size", false, SIZE, "0.7rem", MARK;

    // Boxes, visibility, focus, and forced colors.
    Background = "background", false, Syntax::Color, "transparent", BOXES;
    BorderWidth = "border-width", false, SIZE, "1", BOXES;
    BorderStyle = "border-style", false, LINE_STYLE, "none", BOXES;
    BorderColor = "border-color", false, Syntax::Color, "currentColor", BOXES;
    BorderRadius = "border-radius", false, SIZE, "0", BOXES;
    Display = "display", false, Syntax::Keywords(&["none", "block", "inline", "inline-block", "flex", "inline-flex", "grid", "inline-grid", "contents", "flow-root", "list-item", "table"]), "block", NODES;
    OutlineWidth = "outline-width", false, SIZE, "3", FOCUS;
    OutlineStyle = "outline-style", false, LINE_STYLE, "none", FOCUS;
    OutlineColor = "outline-color", false, Syntax::Color, "currentColor", FOCUS;
    OutlineOffset = "outline-offset", false, LENGTH, "0", FOCUS;
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

    /// Whether a node of `element` accepts the property.
    #[inline]
    pub fn applies_to(self, element: Element) -> bool {
        self.element_mask() & (1 << element as u32) != 0
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
