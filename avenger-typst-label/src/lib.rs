//! Typst-style single-line label typesetting for Avenger.
//!
//! This crate owns the lightweight Typst-label engine and artifact types.
//! Avenger-specific fallback, truncation, caching, and renderer integration
//! live in `avenger-text` and higher-level crates.

mod typst_diag;
mod typst_eval;
mod typst_label;
mod typst_layout;
mod typst_library;
mod typst_pdf;
mod typst_realize;
mod typst_render;
mod typst_svg;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "typst_syntax/lib.rs"]
mod typst_syntax;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "typst_timing/lib.rs"]
mod typst_timing;
#[allow(dead_code, unused_imports, unused_macros)]
#[path = "typst_utils/lib.rs"]
mod typst_utils;

pub use typst_label::{
    CacheOptions, CompiledLabel, EngineOptions, FontOptions, Glyph, GroupItem, ImageItem,
    LabelEngine, LabelError, LabelFlags, LabelFrame, LabelFrameItem, LabelInitError, LabelLimits,
    LabelMetrics, LabelOptions, LabelParamValue, LabelParams, LabelWarning, PdfDrawItem, PdfLabel,
    PdfOptions, PdfPathItem, Point, RasterImage, RasterOptions, ShapeItem, Size, SvgLabel,
    SvgOptions, TextItem, TextItemKind, TextStyle, escape_text, pdf_items, rasterize, svg_items,
};
pub use typst_library::{
    Color, FontStyle, FontWeight, MathDisplayStyle, MathFontBytesId, MathFontSpec, MathStyle,
};
pub use typst_pdf::{FontResource, FontResourceId, PdfGlyph, PdfGlyphRun, PdfTextLayer};
pub use typst_render::{RasterRequest, RgbaImageData};
pub use typst_svg::{
    PathArtifact, PathCommand, PathData, PathImageFormat, PathImageItem, PathItem, PathKind,
    Stroke, StrokeCap, StrokeJoin, Transform,
};
