//! Typst-style single-line label typesetting for Avenger.
//!
//! This crate owns the lightweight Typst-label engine and artifact types.
//! Avenger-specific fallback, truncation, caching, and renderer integration
//! live in `avenger-text` and higher-level crates.

mod api;
mod delimiter;
mod engine;
mod error;
mod fonts;
mod label;
mod limits;
mod paths;
mod pdf;
mod raster;
mod style;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "syntax/lib.rs"]
mod syntax;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "timing/lib.rs"]
mod timing;
mod types;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "utils/lib.rs"]
mod utils;
mod warnings;

pub use label::{
    CacheOptions, CompiledLabel, EngineOptions, FontOptions, Glyph, GroupItem, ImageItem,
    LabelEngine, LabelError, LabelFlags, LabelFrame, LabelFrameItem, LabelInitError, LabelLimits,
    LabelMetrics, LabelOptions, LabelParamValue, LabelParams, LabelWarning, PdfDrawItem, PdfLabel,
    PdfOptions, PdfPathItem, Point, RasterImage, RasterOptions, ShapeItem, Size, SvgLabel,
    SvgOptions, TextItem, TextItemKind, TextStyle, escape_text, pdf_items, rasterize, svg_items,
};
pub use paths::{
    MathImageFormat, MathImageItem, MathPathArtifact, MathPathCommand, MathPathData, MathPathItem,
    MathPathKind, MathStroke, MathTransform,
};
pub use pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
pub use raster::RgbaImageData;
pub use style::{
    Color, FontStyle, FontWeight, MathDisplayStyle, MathFontBytesId, MathFontSpec, MathStyle,
};
