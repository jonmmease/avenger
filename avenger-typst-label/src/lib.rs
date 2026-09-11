//! Typst-style single-line label typesetting for Avenger.
//!
//! This crate owns the lightweight Typst-label engine and artifact types.
//! Avenger-specific fallback, truncation, caching, and renderer integration
//! live in `avenger-text` and higher-level crates.

mod label;
mod typst_eval;
mod typst_layout;
mod typst_library;
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

pub use label::{
    CompiledLabel, EngineOptions, FontMetrics, FontOptions, FontResource, FontResourceId, Glyph,
    GroupItem, ImageItem, LabelEngine, LabelError, LabelFlags, LabelFrame, LabelFrameItem,
    LabelInitError, LabelLimits, LabelMetrics, LabelOptions, LabelParamValue, LabelParams,
    LabelWarning, MissingFontPolicy, PdfDrawItem, PdfGlyph, PdfGlyphRun, PdfLabel, PdfOptions,
    PdfPathItem, PdfTextLayer, Point, RasterImage, RasterOptions, RegisteredFont, ShapeItem, Size,
    SvgLabel, SvgOptions, TextItem, TextItemKind, TextStyle, escape_text, pdf_items, rasterize,
    referenced_params, svg_items,
};
pub use typst_library::{Color, FontStyle, FontWeight, MathFontBytesId, MathFontSpec, MathStyle};
pub use typst_render::{RasterRequest, RgbaImageData};
pub use typst_svg::{
    LineCap, LineJoin, PathArtifact, PathCommand, PathData, PathImageFormat, PathImageItem,
    PathItem, PathKind, Stroke, Transform,
};
