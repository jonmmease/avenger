//! The label typesetter: Avenger's engine around the ported Typst pipeline.
//!
//! A [`LabelEngine`] compiles a label's markup, or literal text, into a [`CompiledLabel`]:
//! its line as a [`LabelFrame`], with metrics and warnings. The SVG, PDF and raster lowerers
//! turn compiled labels into drawing items for those outputs.

mod engine;
mod error;
mod format;
mod frame;
mod lower;
mod options;
#[cfg(test)]
pub(crate) mod oracle;
mod params;
mod styles;
mod world;

pub use self::engine::{
    CompiledLabel, FontMetrics, LabelEngine, LabelFlags, LabelMetrics, escape_text,
};
pub use self::error::{LabelError, LabelWarning};
pub(crate) use self::format::{FormattingCache, define};
pub use self::frame::{
    Curve, CurveItem, DashPattern, FillRule, FontRef, FrameItem, Geometry, Glyph,
    GroupItem, LabelFrame, LineCap, LineJoin, Point, Shape, Size, Stroke, TextItem,
    Transform,
};
pub use self::options::{
    Em, EngineOptions, FontOptions, LabelFormatting, LabelLimits, LabelOptions,
    MathStyle, MissingFontPolicy, RegisteredFont, TextDir, TextStyle,
};
pub use self::params::{LabelParamValue, LabelParams, referenced_params};

#[cfg(test)]
pub(crate) use self::world::fixtures;
