use crate::limits::LabelLimits;
use crate::paths::PathArtifact;
use crate::pdf::{FontResource, PdfTextLayer};
use crate::raster::{RasterImage, RasterRequest};
use crate::style::{MathStyle, PlainTextStyle};
use crate::warnings::LabelWarning;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathOutputRequest {
    pub paths: bool,
    pub raster: Option<RasterRequest>,
    pub pdf_text_layer: bool,
}

impl Default for MathOutputRequest {
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
pub struct MathFragmentOptions {
    pub style: MathStyle,
    pub outputs: MathOutputRequest,
    pub limits: LabelLimits,
}

impl Default for MathFragmentOptions {
    fn default() -> Self {
        Self {
            style: MathStyle::default(),
            outputs: MathOutputRequest::default(),
            limits: LabelLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextLineOutputRequest {
    pub paths: bool,
    pub raster: Option<RasterRequest>,
    pub pdf_text_layer: bool,
    pub positioned_runs: bool,
}

impl Default for TextLineOutputRequest {
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
pub struct TextLineOptions {
    pub text_style: PlainTextStyle,
    pub math_style: MathStyle,
    pub outputs: TextLineOutputRequest,
    pub limits: LabelLimits,
}

impl Default for TextLineOptions {
    fn default() -> Self {
        Self {
            text_style: PlainTextStyle::default(),
            math_style: MathStyle::default(),
            outputs: TextLineOutputRequest::default(),
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
pub struct TextLineArtifact {
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
