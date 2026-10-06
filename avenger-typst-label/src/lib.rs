//! Typst-style single-line label typesetting for Avenger.
//!
//! This crate owns the lightweight Typst-label engine and artifact types.
//! Avenger-specific fallback, truncation, caching, and renderer integration
//! live in `avenger-text` and higher-level crates.

mod legacy;
#[path = "typst_syntax/lib.rs"]
mod typst_syntax;
#[path = "typst_timing/lib.rs"]
mod typst_timing;
#[rustfmt::skip]
#[expect(unused_imports, reason = "grouped upstream imports keep names of removed items")]
#[path = "typst_utils/lib.rs"]
mod typst_utils;

pub use legacy::label::{
    CompiledLabel, EngineOptions, FontFeature, FontMetrics, FontOptions, FontResource,
    FontResourceId, Glyph, GroupItem, ImageItem, LabelEngine, LabelError, LabelFlags,
    LabelFormatting, LabelFrame, LabelFrameItem, LabelInitError, LabelLimits, LabelMetrics,
    LabelOptions, LabelParamValue, LabelParams, LabelWarning, MissingFontPolicy, PdfDrawItem,
    PdfGlyph, PdfGlyphRun, PdfLabel, PdfOptions, PdfPathItem, PdfTextLayer, Point, RasterImage,
    RasterOptions, RegisteredFont, ShapeItem, Size, SvgLabel, SvgOptions, TextItem, TextItemKind,
    TextStyle, escape_text, pdf_items, rasterize, referenced_params, svg_items,
};
pub use legacy::typst_library::{
    Color, FontStyle, FontWeight, MathFontBytesId, MathFontSpec, MathStyle,
};
pub use legacy::typst_render::{RasterRequest, RgbaImageData};
pub use legacy::typst_svg::{
    LineCap, LineJoin, PathArtifact, PathCommand, PathData, PathDrawItem, PathImageFormat,
    PathImageItem, PathItem, PathKind, Stroke, Transform,
};
