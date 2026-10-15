//! The label typesetter: Avenger's engine around the ported Typst pipeline.
//!
//! A [`LabelEngine`] compiles a label's markup, or literal text, into a [`CompiledLabel`]:
//! its line as a [`LabelFrame`], with metrics and warnings. The SVG, PDF and raster lowerers
//! turn compiled labels into drawing items for those outputs.

mod bind;
mod bounds;
mod boxed;
#[cfg(feature = "bundled-fonts")]
mod bundled;
mod datetime;
mod engine;
mod error;
mod format;
mod frame;
mod lower;
mod memo;
mod options;
#[cfg(test)]
pub(crate) mod oracle;
#[cfg(feature = "raster")]
mod raster;
mod source;
mod styles;
mod values;
mod world;

pub use self::bind::bind;
pub use self::bounds::TextBounds;
#[cfg(feature = "bundled-fonts")]
pub use self::bundled::{bundled_font_options, bundled_label_engine};
pub use self::engine::{
    CompiledLabel, FontMetrics, LabelEngine, LabelFlags, LabelMetrics, LineMetrics,
    escape_text,
};
pub use self::error::{LabelError, LabelWarning};
pub(crate) use self::format::{FormattingCache, define};
pub use self::frame::{
    Curve, CurveItem, DashPattern, FillRule, FontRef, FrameItem, Geometry, Glyph,
    GroupItem, LabelFrame, LineCap, LineJoin, Point, Shape, Size, Stroke, TextItem,
    Transform,
};
#[cfg(feature = "raster")]
pub use self::memo::TextRasterKey;
pub use self::options::{
    Em, EngineOptions, FontOptions, Label, LabelAlign, LabelLimits, LabelLineHeight,
    LabelOptions, LabelSource, LabelWidth, MathStyle, MissingFontPolicy, RegisteredFont,
    TextDir, TextStyle,
};
#[cfg(feature = "raster")]
pub use self::raster::TextRaster;
pub(crate) use self::source::{label_file, label_span};
pub use self::values::{LabelValue, LabelValues};

#[cfg(test)]
pub(crate) use self::world::fixtures;
