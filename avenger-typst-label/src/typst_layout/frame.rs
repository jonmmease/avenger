use crate::typst_diag::LabelWarning;
use crate::typst_eval::LabelLimits;
use crate::typst_label::LabelParams;
use crate::typst_library::{MathStyle, PlainTextStyle};
use crate::typst_pdf::{FontResource, PdfTextLayer};
use crate::typst_svg::PathArtifact;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathLayoutOptions {
    pub style: MathStyle,
    pub limits: LabelLimits,
}

impl Default for MathLayoutOptions {
    fn default() -> Self {
        Self {
            style: MathStyle::default(),
            limits: LabelLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LineLayoutOptions {
    pub text_style: PlainTextStyle,
    pub math_style: MathStyle,
    pub params: LabelParams,
    pub limits: LabelLimits,
}

impl Default for LineLayoutOptions {
    fn default() -> Self {
        Self {
            text_style: PlainTextStyle::default(),
            math_style: MathStyle::default(),
            params: LabelParams::default(),
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
    pub paths: PathArtifact,
    pub pdf_text: PdfTextLayer,
    pub font_resources: Vec<FontResource>,
    pub warnings: Vec<LabelWarning>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LineLayoutArtifact {
    pub source: String,
    pub metrics: TypesetMetrics,
    pub paths: PathArtifact,
    pub pdf_text: PdfTextLayer,
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
