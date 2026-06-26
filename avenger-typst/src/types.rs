use crate::delimiter::{MathDelimiterInfo, MathDelimiterOptions};
use crate::limits::MathLimits;
use crate::paths::MathPathArtifact;
use crate::pdf::{MathFontResource, MathPdfTextLayer};
use crate::raster::{MathRasterArtifact, RasterRequest};
use crate::style::{MathStyle, PlainTextStyle};
use crate::warnings::MathTypesetWarning;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathSyntaxMode {
    TypstFragmentStrict,
}

impl Default for MathSyntaxMode {
    fn default() -> Self {
        Self::TypstFragmentStrict
    }
}

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
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
}

impl Default for MathFragmentOptions {
    fn default() -> Self {
        Self {
            style: MathStyle::default(),
            outputs: MathOutputRequest::default(),
            syntax: MathSyntaxMode::TypstFragmentStrict,
            limits: MathLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStringOptions {
    pub text_style: PlainTextStyle,
    pub math_style: MathStyle,
    pub outputs: MathOutputRequest,
    pub delimiters: MathDelimiterOptions,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
}

impl Default for MathStringOptions {
    fn default() -> Self {
        Self {
            text_style: PlainTextStyle::default(),
            math_style: MathStyle::default(),
            outputs: MathOutputRequest::default(),
            delimiters: MathDelimiterOptions::default(),
            syntax: MathSyntaxMode::TypstFragmentStrict,
            limits: MathLimits::default(),
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
    pub delimiters: MathDelimiterOptions,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
}

impl Default for TextLineOptions {
    fn default() -> Self {
        Self {
            text_style: PlainTextStyle::default(),
            math_style: MathStyle::default(),
            outputs: TextLineOutputRequest::default(),
            delimiters: MathDelimiterOptions::default(),
            syntax: MathSyntaxMode::TypstFragmentStrict,
            limits: MathLimits::default(),
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
pub struct PlainTextRun {
    pub text: String,
    pub byte_range: std::ops::Range<usize>,
    pub style: PlainTextStyle,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathRun {
    pub source: String,
    pub byte_range: std::ops::Range<usize>,
    pub delimiter: MathDelimiterInfo,
    pub artifact: MathRunArtifact,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathStringRun {
    Plain(PlainTextRun),
    Math(MathRun),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathRunArtifact {
    pub metrics: TypesetMetrics,
    pub paths: Option<MathPathArtifact>,
    pub raster: Option<MathRasterArtifact>,
    pub pdf_text: Option<MathPdfTextLayer>,
    pub font_resources: Vec<MathFontResource>,
    pub warnings: Vec<MathTypesetWarning>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStringArtifact {
    pub source: String,
    pub runs: Vec<MathStringRun>,
    pub font_resources: Vec<MathFontResource>,
    pub warnings: Vec<MathTypesetWarning>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextLineArtifact {
    pub source: String,
    pub metrics: TypesetMetrics,
    pub paths: Option<MathPathArtifact>,
    pub raster: Option<MathRasterArtifact>,
    pub pdf_text: Option<MathPdfTextLayer>,
    pub positioned_runs: Vec<PositionedTextLineRun>,
    pub font_resources: Vec<MathFontResource>,
    pub warnings: Vec<MathTypesetWarning>,
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
    pub paths: Option<MathPathArtifact>,
    pub pdf_text: Option<MathPdfTextLayer>,
    pub font_resources: Vec<MathFontResource>,
}
