//! Typst-style math fragment typesetting for Avenger.
//!
//! This crate is intentionally standalone. It owns the API and artifact types
//! for math fragments, but does not integrate with `avenger-text`, scenegraph
//! marks, chart APIs, or renderers yet.

mod api;
mod delimiter;
mod engine;
mod error;
mod limits;
mod paths;
mod pdf;
mod raster;
mod style;
mod types;
mod warnings;

pub use api::{AvengerTypst, TypstCacheConfig, TypstEngineBackend, TypstEngineConfig};
pub use delimiter::{
    MathDelimiterInfo, MathDelimiterOptions, MathDisplayHint, UnmatchedDelimiterPolicy,
};
pub use error::{MathTypesetError, TypstInitError};
pub use limits::MathLimits;
pub use paths::{
    MathPathArtifact, MathPathCommand, MathPathData, MathPathItem, MathPathKind, MathStroke,
    MathTransform,
};
pub use pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
pub use raster::{MathRasterArtifact, RasterRequest, RgbaImageData};
pub use style::{
    Color, FontStyle, FontWeight, MathDisplayStyle, MathFontBytesId, MathFontConfig, MathFontSpec,
    MathStrictness, MathStyle, PlainTextStyle,
};
pub use types::{
    MathFragmentOptions, MathOutputRequest, MathRun, MathRunArtifact, MathStringArtifact,
    MathStringOptions, MathStringRun, MathSyntaxMode, PlainTextRun, TypesetMetrics,
};
pub use warnings::MathTypesetWarning;
