//! The marks of units: their kinds and types, their channels, and a view of their properties
//! that the marks of each type share across kinds.
//!
//! build.rs generates the channels and the encodings of each mark message, and the kind, the type,
//! and the name of each member of `Mark`, from the descriptors of the format.

use crate::{
    proto::{
        mark::Mark, Channel, Encoding, ImageAlign, ImageBaseline, Orient, Route, TextAlign,
        TextBaseline, TextSyntax,
    },
    CoordKind,
};

include!(concat!(env!("OUT_DIR"), "/marks.rs"));

/// The type of a mark, which its messages in each coordinate system share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkType {
    Symbol,
    Line,
    Trail,
    Area,
    Bar,
    Rect,
    Rule,
    Text,
    Image,
    Path,
    GeoShape,
}

impl Mark {
    /// The encodings of the mark with their channels, in the order of `Channel`.
    pub fn encodings(&self) -> impl Iterator<Item = (Channel, &Encoding)> {
        self.channels()
            .iter()
            .filter_map(|channel| Some((*channel, self.encoding(*channel)?)))
    }

    /// The properties of the mark, alike for the marks of one type in every coordinate system.
    pub fn view(&self) -> MarkView<'_> {
        match self {
            Self::Symbol(mark) => MarkView::Symbol(SymbolProps {
                filled: mark.filled,
                stroke_width: mark.stroke_width,
            }),
            Self::PolarSymbol(mark) => MarkView::Symbol(SymbolProps {
                filled: mark.filled,
                stroke_width: mark.stroke_width,
            }),
            Self::GeoSymbol(mark) => MarkView::Symbol(SymbolProps {
                filled: mark.filled,
                stroke_width: mark.stroke_width,
            }),
            Self::Line(_) => MarkView::Line(LineProps { route: None }),
            Self::PolarLine(mark) => MarkView::Line(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::GeoLine(mark) => MarkView::Line(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::Trail(_) => MarkView::Trail(LineProps { route: None }),
            Self::PolarTrail(mark) => MarkView::Trail(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::GeoTrail(mark) => MarkView::Trail(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::Area(mark) => MarkView::Area(AreaProps {
                orient: orient(mark.orient),
                route: None,
            }),
            Self::PolarArea(mark) => MarkView::Area(AreaProps {
                orient: orient(mark.orient),
                route: Some(route(mark.route)),
            }),
            Self::Bar(mark) => MarkView::Bar(BarProps {
                orient: orient(mark.orient),
                corner_radius: mark.corner_radius,
                route: None,
            }),
            Self::PolarBar(mark) => MarkView::Bar(BarProps {
                orient: orient(mark.orient),
                corner_radius: mark.corner_radius,
                route: Some(route(mark.route)),
            }),
            Self::Rect(mark) => MarkView::Rect(RectProps {
                corner_radius: mark.corner_radius,
                inset: mark.inset,
                route: None,
            }),
            Self::PolarRect(mark) => MarkView::Rect(RectProps {
                corner_radius: mark.corner_radius,
                inset: mark.inset,
                route: Some(route(mark.route)),
            }),
            Self::Rule(_) => MarkView::Rule(LineProps { route: None }),
            Self::PolarRule(mark) => MarkView::Rule(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::GeoRule(mark) => MarkView::Rule(LineProps {
                route: Some(route(mark.route)),
            }),
            Self::Text(mark) => MarkView::Text(TextProps {
                format: &mark.format,
                font_family: &mark.font_family,
                syntax: syntax(mark.syntax),
                placement: TextPlacement::Aligned {
                    align: align(mark.align),
                    baseline: baseline(mark.baseline),
                },
            }),
            Self::PolarText(mark) => MarkView::Text(TextProps {
                format: &mark.format,
                font_family: &mark.font_family,
                syntax: syntax(mark.syntax),
                placement: TextPlacement::Polar,
            }),
            Self::GeoText(mark) => MarkView::Text(TextProps {
                format: &mark.format,
                font_family: &mark.font_family,
                syntax: syntax(mark.syntax),
                placement: TextPlacement::Aligned {
                    align: align(mark.align),
                    baseline: baseline(mark.baseline),
                },
            }),
            Self::Image(mark) => MarkView::Image(ImageProps {
                align: ImageAlign::try_from(mark.align).unwrap_or_default(),
                baseline: ImageBaseline::try_from(mark.baseline).unwrap_or_default(),
                aspect: mark.aspect,
                smooth: mark.smooth,
            }),
            Self::GeoImage(mark) => MarkView::Image(ImageProps {
                align: ImageAlign::try_from(mark.align).unwrap_or_default(),
                baseline: ImageBaseline::try_from(mark.baseline).unwrap_or_default(),
                aspect: mark.aspect,
                smooth: mark.smooth,
            }),
            Self::Path(mark) => MarkView::Path(StrokeProps {
                stroke_width: mark.stroke_width,
            }),
            Self::GeoPath(mark) => MarkView::Path(StrokeProps {
                stroke_width: mark.stroke_width,
            }),
            Self::GeoShape(mark) => MarkView::GeoShape(StrokeProps {
                stroke_width: mark.stroke_width,
            }),
        }
    }
}

/// The properties of a mark, by its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MarkView<'a> {
    Symbol(SymbolProps),
    Line(LineProps),
    Trail(LineProps),
    Area(AreaProps),
    Bar(BarProps),
    Rect(RectProps),
    Rule(LineProps),
    Text(TextProps<'a>),
    Image(ImageProps),
    Path(StrokeProps),
    GeoShape(StrokeProps),
}

/// The properties of a symbol mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolProps {
    pub filled: Option<bool>,
    pub stroke_width: Option<f32>,
}

/// The properties of a line, a trail, or a rule mark: its route, or `None` in a Cartesian plot,
/// whose segments are straight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineProps {
    pub route: Option<Route>,
}

/// The properties of an area mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaProps {
    pub orient: Orient,
    pub route: Option<Route>,
}

/// The properties of a bar mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarProps {
    pub orient: Orient,
    pub corner_radius: Option<f32>,
    pub route: Option<Route>,
}

/// The properties of a rect mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectProps {
    pub corner_radius: Option<f32>,
    pub inset: Option<f32>,
    pub route: Option<Route>,
}

/// The properties of a text mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextProps<'a> {
    pub format: &'a str,
    pub font_family: &'a str,
    pub syntax: TextSyntax,
    pub placement: TextPlacement,
}

/// How a text lies at its position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextPlacement {
    /// As its `align` and its `baseline` say, in a Cartesian or a map plot.
    Aligned {
        align: TextAlign,
        baseline: TextBaseline,
    },
    /// Beside its point in a polar plot.
    Polar,
}

/// The properties of an image mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageProps {
    pub align: ImageAlign,
    pub baseline: ImageBaseline,
    pub aspect: Option<bool>,
    pub smooth: Option<bool>,
}

/// The properties of a path or a geo-shape mark: the width of the stroke of every one of its
/// shapes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeProps {
    pub stroke_width: Option<f32>,
}

fn route(value: i32) -> Route {
    Route::try_from(value).unwrap_or_default()
}

fn orient(value: i32) -> Orient {
    Orient::try_from(value).unwrap_or_default()
}

fn syntax(value: i32) -> TextSyntax {
    TextSyntax::try_from(value).unwrap_or_default()
}

fn align(value: i32) -> TextAlign {
    TextAlign::try_from(value).unwrap_or_default()
}

fn baseline(value: i32) -> TextBaseline {
    TextBaseline::try_from(value).unwrap_or_default()
}
