use crate::label::LabelParams;
use crate::typst_eval::LabelLimits;
use crate::typst_library::{MathStyle, PlainTextStyle};
use crate::typst_pdf::{FontResource, PdfTextLayer};
use crate::typst_render::{RasterImage, RasterRequest};
use crate::typst_svg::PathArtifact;
use crate::warnings::LabelWarning;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathOutputOptions {
    pub paths: bool,
    pub raster: Option<RasterRequest>,
    pub pdf_text_layer: bool,
}

impl Default for MathOutputOptions {
    fn default() -> Self {
        Self {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathLayoutOptions {
    pub style: MathStyle,
    pub outputs: MathOutputOptions,
    pub limits: LabelLimits,
}

impl Default for MathLayoutOptions {
    fn default() -> Self {
        Self {
            style: MathStyle::default(),
            outputs: MathOutputOptions::default(),
            limits: LabelLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LineOutputOptions {
    pub paths: bool,
    pub raster: Option<RasterRequest>,
    pub pdf_text_layer: bool,
    pub positioned_runs: bool,
}

impl Default for LineOutputOptions {
    fn default() -> Self {
        Self {
            paths: true,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LineLayoutOptions {
    pub text_style: PlainTextStyle,
    pub math_style: MathStyle,
    pub params: LabelParams,
    pub outputs: LineOutputOptions,
    pub limits: LabelLimits,
}

impl Default for LineLayoutOptions {
    fn default() -> Self {
        Self {
            text_style: PlainTextStyle::default(),
            math_style: MathStyle::default(),
            params: LabelParams::default(),
            outputs: LineOutputOptions::default(),
            limits: LabelLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypesetMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathRunArtifact {
    pub metrics: TypesetMetrics,
    pub paths: Option<PathArtifact>,
    pub raster: Option<RasterImage>,
    pub pdf_text: Option<PdfTextLayer>,
    pub font_resources: Vec<FontResource>,
    pub warnings: Vec<LabelWarning>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LineLayoutArtifact {
    pub source: String,
    pub metrics: TypesetMetrics,
    pub paths: Option<PathArtifact>,
    pub raster: Option<RasterImage>,
    pub pdf_text: Option<PdfTextLayer>,
    pub positioned_runs: Vec<PositionedTextLineRun>,
    pub font_resources: Vec<FontResource>,
    pub warnings: Vec<LabelWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PositionedTextLineRunKind {
    Plain,
    Math,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PositionedTextLineRun {
    pub kind: PositionedTextLineRunKind,
    pub text: String,
    pub byte_range: std::ops::Range<usize>,
    /// Style for native plain-text output. Math runs leave this empty because
    /// their SVG/PDF representation is carried by paths/PDF glyph metadata.
    pub text_style: Option<PlainTextStyle>,
    /// X coordinate of the run start in the tight Typst line frame.
    pub x: f32,
    /// Baseline coordinate for plain text in the tight Typst line frame.
    pub y: f32,
    pub metrics: TypesetMetrics,
    pub paths: Option<PathArtifact>,
    pub pdf_text: Option<PdfTextLayer>,
    pub font_resources: Vec<FontResource>,
}
