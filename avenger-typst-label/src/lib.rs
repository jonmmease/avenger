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
    FontMetrics, FontOptions, FontRef, FrameItem, Geometry, Glyph, GroupItem, Label,
    LabelAlign, LabelEngine, LabelError, LabelFlags, LabelFrame, LabelLimits,
    LabelLineHeight, LabelMetrics, LabelOptions, LabelSource, LabelValue, LabelValues,
    LabelWarning, LabelWidth, LineCap, LineJoin, LineMetrics, MathStyle,
    MissingFontPolicy, Point, RegisteredFont, Shape, Size, Stroke, TextBounds, TextDir,
    TextItem, TextStyle, Transform, bind, escape_text,
};
#[cfg(feature = "raster")]
pub use label::{TextRaster, TextRasterKey};
#[cfg(feature = "bundled-fonts")]
pub use label::{bundled_font_options, bundled_label_engine};
pub use typst_library::text::{FontStyle, FontWeight, Lang, Region};
pub use typst_pdf::{PdfGlyph, PdfItem, PdfLabel, PdfOptions, PdfText, pdf_items};
#[cfg(feature = "raster")]
pub use typst_render::{
    RasterError, RasterImage, RasterOptions, RgbaImageData, rasterize,
};
pub use typst_svg::{
    GlyphRef, ImageItem, PathItem, PathKind, SvgItem, SvgLabel, SvgOptions, TextRun,
    svg_items,
};
