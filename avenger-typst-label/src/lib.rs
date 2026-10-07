//! Typesets labels written in Typst markup, with inline math, for Avenger's charts.
//!
//! A [`LabelEngine`] compiles a label to a [`CompiledLabel`]: its frame of positioned glyphs
//! and shapes, with its metrics. [`svg_items`], [`pdf_items`] and, with the `raster` feature,
//! `rasterize` lower it for output. The typesetting is a port of Typst 0.15.1's pipeline;
//! `UPSTREAM.md` maps the ported files and lists the deliberate divergences.

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

pub use label::{
    CompiledLabel, Curve, CurveItem, DashPattern, Em, EngineOptions, FillRule,
    FontMetrics, FontOptions, FontRef, FrameItem, Geometry, Glyph, GroupItem,
    LabelEngine, LabelError, LabelFlags, LabelFormatting, LabelFrame, LabelLimits,
    LabelMetrics, LabelOptions, LabelParamValue, LabelParams, LabelWarning, LabelWidth,
    LineCap, LineJoin, MathStyle, MissingFontPolicy, Point, RegisteredFont, Shape, Size,
    Stroke, TextDir, TextItem, TextStyle, Transform, escape_text, referenced_params,
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
