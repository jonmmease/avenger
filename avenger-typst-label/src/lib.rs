//! Typst-style single-line label typesetting for Avenger.
//!
//! This crate owns the lightweight Typst-label engine and artifact types.
//! Avenger-specific fallback, truncation, caching, and renderer integration
//! live in `avenger-text` and higher-level crates.

// `elem!` takes upstream's element definitions as written and works through their fields and
// attributes recursively. `TextElem`, with 41 fields and their documentation, needs about 300
// levels.
#![recursion_limit = "512"]

mod label;
mod typst_eval;
mod typst_layout;
mod typst_library;
mod typst_pdf;
mod typst_realize;
#[cfg(feature = "raster")]
mod typst_render;
mod typst_svg;
#[path = "typst_syntax/lib.rs"]
mod typst_syntax;
#[path = "typst_timing/lib.rs"]
mod typst_timing;
mod typst_utils;

pub use label::{
    CompiledLabel, Curve, CurveItem, DashPattern, Em, EngineOptions, FillRule,
    FontMetrics, FontOptions, FontRef, FrameItem, Geometry, Glyph, GroupItem,
    LabelEngine, LabelError, LabelFlags, LabelFormatting, LabelFrame, LabelLimits,
    LabelMetrics, LabelOptions, LabelParamValue, LabelParams, LabelWarning, LineCap,
    LineJoin, MathStyle, MissingFontPolicy, Point, RegisteredFont, Shape, Size, Stroke,
    TextDir, TextItem, TextStyle, Transform, escape_text, referenced_params,
};
pub use typst_library::text::{FontStyle, FontWeight, Lang, Region};
pub use typst_pdf::{PdfGlyph, PdfItem, PdfLabel, PdfOptions, PdfText, pdf_items};
#[cfg(feature = "raster")]
pub use typst_render::{
    RasterError, RasterImage, RasterOptions, RgbaImageData, rasterize,
};
pub use typst_svg::{
    GlyphRef, ImageItem, PathItem, PathKind, SvgItem, SvgLabel, SvgOptions, svg_items,
};
