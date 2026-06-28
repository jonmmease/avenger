use std::{ops::Range, path::PathBuf};

use indexmap::IndexMap;

use crate::api::{TypstCacheConfig, TypstEngineConfig};
use crate::delimiter::{ParsedSegment, parse_segments};
use crate::engine::engine::TypstEngineCore;
use crate::error::{MathTypesetError, TypstInitError};
use crate::limits::MathLimits;
use crate::paths::{MathImageItem, MathPathArtifact, MathTransform};
use crate::pdf::{MathFontResource, MathPdfGlyph, MathPdfTextLayer};
use crate::raster::MathRasterArtifact;
use crate::style::{MathFontConfig, MathStrictness, MathStyle, PlainTextStyle};
use crate::types::{
    MathSyntaxMode, PositionedTextLineRun, PositionedTextLineRunKind, TextLineArtifact,
    TextLineOptions, TextLineOutputRequest, TypesetMetrics,
};
use crate::warnings::MathTypesetWarning;

#[cfg(feature = "raster")]
use crate::raster::RasterRequest;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub type LabelInitError = TypstInitError;
pub type LabelError = MathTypesetError;
pub type LabelWarning = MathTypesetWarning;
pub type LabelLimits = MathLimits;
pub type TextStyle = PlainTextStyle;
pub type RasterImage = MathRasterArtifact;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EngineOptions {
    pub fonts: FontOptions,
    pub cache: CacheOptions,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            fonts: FontOptions::default(),
            cache: CacheOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontOptions {
    pub load_system_fonts: bool,
    pub extra_font_dirs: Vec<PathBuf>,
    pub extra_font_families: Vec<String>,
}

impl Default for FontOptions {
    fn default() -> Self {
        let config = MathFontConfig::default();
        Self {
            load_system_fonts: config.load_system_fonts,
            extra_font_dirs: config.extra_font_dirs,
            extra_font_families: config.extra_font_families,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CacheOptions {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelOptions {
    pub text: TextStyle,
    pub math: MathStyle,
    pub params: LabelParams,
    pub limits: LabelLimits,
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            text: TextStyle::default(),
            math: MathStyle::default(),
            params: LabelParams::default(),
            limits: LabelLimits::default(),
        }
    }
}

pub type LabelParams = IndexMap<String, LabelParamValue>;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelParamValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<LabelParamValue>),
    Dict(IndexMap<String, LabelParamValue>),
}

#[derive(Debug, Clone)]
pub struct LabelEngine {
    inner: TypstEngineCore,
}

impl LabelEngine {
    pub fn new(options: EngineOptions) -> Result<Self, LabelInitError> {
        let config = TypstEngineConfig::from(options);
        Ok(Self {
            inner: TypstEngineCore::new(&config)?,
        })
    }

    pub fn compile(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        validate_source_limits(source, options.limits)?;
        let segments = parse_segments(source, &Default::default())?;
        validate_math_segments(&segments, options.limits)?;
        let artifact = self.inner.typeset_text_line(
            source,
            &text_line_options(options, MathSyntaxMode::TypstFragmentStrict),
        )?;
        Ok(CompiledLabel::from_artifact(
            artifact,
            label_has_markup(source),
        ))
    }

    pub fn measure(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        self.compile(source, options).map(|label| label.metrics)
    }

    pub fn compile_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        validate_source_limits(text, options.limits)?;
        let artifact = self
            .inner
            .typeset_text_line(text, &text_line_options(options, MathSyntaxMode::PlainText))?;
        Ok(CompiledLabel::from_artifact(artifact, false))
    }

    pub fn measure_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        self.compile_text(text, options).map(|label| label.metrics)
    }
}

impl From<EngineOptions> for TypstEngineConfig {
    fn from(options: EngineOptions) -> Self {
        Self {
            font_config: MathFontConfig {
                extra_font_families: options.fonts.extra_font_families,
                load_system_fonts: options.fonts.load_system_fonts,
                extra_font_dirs: options.fonts.extra_font_dirs,
            },
            cache: TypstCacheConfig {
                enabled: options.cache.enabled,
            },
            strictness: MathStrictness::Strict,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CompiledLabel {
    pub source: String,
    pub frame: LabelFrame,
    pub metrics: LabelMetrics,
    pub flags: LabelFlags,
    pub warnings: Vec<LabelWarning>,
    artifact: TextLineArtifact,
}

impl CompiledLabel {
    fn from_artifact(artifact: TextLineArtifact, has_markup: bool) -> Self {
        let metrics = LabelMetrics::from(artifact.metrics);
        let flags = LabelFlags {
            has_math: artifact
                .positioned_runs
                .iter()
                .any(|run| run.kind == PositionedTextLineRunKind::Math),
            has_markup,
        };
        Self {
            source: artifact.source.clone(),
            frame: LabelFrame::from_artifact(&artifact),
            metrics,
            flags,
            warnings: artifact.warnings.clone(),
            artifact,
        }
    }

    pub fn semantic_text(&self) -> String {
        if !self.artifact.positioned_runs.is_empty() {
            return self
                .artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect();
        }
        self.artifact
            .pdf_text
            .as_ref()
            .map(|pdf_text| pdf_text.semantic_text.clone())
            .unwrap_or_else(|| self.source.clone())
    }

    pub(crate) fn artifact(&self) -> &TextLineArtifact {
        &self.artifact
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl From<TypesetMetrics> for LabelMetrics {
    fn from(metrics: TypesetMetrics) -> Self {
        Self {
            width: metrics.width,
            height: metrics.height,
            baseline: metrics.baseline,
            ascent: metrics.ascent,
            descent: metrics.descent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelFlags {
    pub has_math: bool,
    pub has_markup: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelFrame {
    pub size: Size,
    pub baseline: f32,
    pub items: Vec<(Point, LabelFrameItem)>,
}

impl LabelFrame {
    fn from_artifact(artifact: &TextLineArtifact) -> Self {
        let mut items = Vec::new();
        for run in &artifact.positioned_runs {
            let point = Point { x: run.x, y: run.y };
            match run.kind {
                PositionedTextLineRunKind::Plain => {
                    items.push((
                        point,
                        LabelFrameItem::Text(TextItem {
                            text: run.text.clone(),
                            byte_range: run.byte_range.clone(),
                            style: run.text_style.clone(),
                            metrics: LabelMetrics::from(run.metrics),
                            glyphs: glyphs_from_pdf_text(run.pdf_text.as_ref()),
                        }),
                    ));
                }
                PositionedTextLineRunKind::Math => {
                    if let Some(paths) = run.paths.clone() {
                        items.push((
                            point,
                            LabelFrameItem::Shape(ShapeItem {
                                byte_range: run.byte_range.clone(),
                                paths,
                            }),
                        ));
                    }
                }
            }
        }

        Self {
            size: Size {
                x: artifact.metrics.width,
                y: artifact.metrics.height,
            },
            baseline: artifact.metrics.baseline,
            items,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Size {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelFrameItem {
    Text(TextItem),
    Shape(ShapeItem),
    Image(ImageItem),
    Group(GroupItem),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextItem {
    pub text: String,
    pub byte_range: Range<usize>,
    pub style: Option<TextStyle>,
    pub metrics: LabelMetrics,
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Glyph {
    pub glyph_id: u16,
    pub unicode: String,
    pub text_range: Range<usize>,
    pub x: f32,
    pub y: f32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub transform: MathTransform,
}

impl From<&MathPdfGlyph> for Glyph {
    fn from(glyph: &MathPdfGlyph) -> Self {
        Self {
            glyph_id: glyph.glyph_id,
            unicode: glyph.unicode.clone(),
            text_range: glyph.text_range.clone(),
            x: glyph.x,
            y: glyph.y,
            x_advance: glyph.x_advance,
            y_advance: glyph.y_advance,
            transform: glyph.transform,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ShapeItem {
    pub byte_range: Range<usize>,
    pub paths: MathPathArtifact,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageItem {
    pub byte_range: Range<usize>,
    pub image: MathImageItem,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GroupItem {
    pub items: Vec<(Point, LabelFrameItem)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RasterOptions {
    pub scale: f32,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SvgOptions {}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SvgLabel {
    pub metrics: LabelMetrics,
    pub items: Vec<(Point, LabelFrameItem)>,
    pub positioned_runs: Vec<PositionedTextLineRun>,
    pub paths: Option<MathPathArtifact>,
    pub font_resources: Vec<MathFontResource>,
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfOptions {}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfLabel {
    pub metrics: LabelMetrics,
    pub text: Option<MathPdfTextLayer>,
    pub paths: Option<MathPathArtifact>,
    pub font_resources: Vec<MathFontResource>,
}

pub fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '$' | '#' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn rasterize(
    label: &CompiledLabel,
    options: &RasterOptions,
) -> Result<RasterImage, LabelError> {
    let paths = label
        .artifact()
        .paths
        .as_ref()
        .ok_or(LabelError::UnsupportedOutput(
            "compiled label does not contain path data for rasterization",
        ))?;
    rasterize_paths(paths, *options)
}

#[cfg(feature = "raster")]
fn rasterize_paths(
    paths: &MathPathArtifact,
    options: RasterOptions,
) -> Result<RasterImage, LabelError> {
    crate::raster::rasterize_path_artifact(
        paths,
        RasterRequest {
            scale: options.scale,
        },
    )
}

#[cfg(not(feature = "raster"))]
fn rasterize_paths(
    _paths: &MathPathArtifact,
    _options: RasterOptions,
) -> Result<RasterImage, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "the avenger-typst-label raster feature is disabled",
    ))
}

pub fn svg_items(label: &CompiledLabel, _options: &SvgOptions) -> Result<SvgLabel, LabelError> {
    let artifact = label.artifact();
    Ok(SvgLabel {
        metrics: label.metrics,
        items: label.frame.items.clone(),
        positioned_runs: artifact.positioned_runs.clone(),
        paths: artifact.paths.clone(),
        font_resources: artifact.font_resources.clone(),
    })
}

pub fn pdf_items(label: &CompiledLabel, _options: &PdfOptions) -> Result<PdfLabel, LabelError> {
    let artifact = label.artifact();
    Ok(PdfLabel {
        metrics: label.metrics,
        text: artifact.pdf_text.clone(),
        paths: artifact.paths.clone(),
        font_resources: artifact.font_resources.clone(),
    })
}

fn text_line_options(options: &LabelOptions, syntax: MathSyntaxMode) -> TextLineOptions {
    TextLineOptions {
        text_style: options.text.clone(),
        math_style: options.math.clone(),
        outputs: TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        },
        delimiters: Default::default(),
        syntax,
        limits: options.limits,
    }
}

fn glyphs_from_pdf_text(pdf_text: Option<&MathPdfTextLayer>) -> Vec<Glyph> {
    pdf_text
        .into_iter()
        .flat_map(|pdf_text| pdf_text.glyph_runs.iter())
        .flat_map(|run| run.glyphs.iter().map(Glyph::from))
        .collect()
}

fn validate_source_limits(source: &str, limits: MathLimits) -> Result<(), MathTypesetError> {
    if source.len() > limits.max_source_bytes {
        return Err(MathTypesetError::SourceTooLarge {
            actual: source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

fn validate_math_segments(
    segments: &[ParsedSegment],
    limits: MathLimits,
) -> Result<(), MathTypesetError> {
    let math_span_count = segments
        .iter()
        .filter(|segment| matches!(segment, ParsedSegment::Math { .. }))
        .count();
    if math_span_count > limits.max_math_spans {
        return Err(MathTypesetError::TooManyMathSpans {
            actual: math_span_count,
            limit: limits.max_math_spans,
        });
    }

    for segment in segments {
        if let ParsedSegment::Math {
            source,
            source_range,
            ..
        } = segment
        {
            validate_math_fragment(source, limits, source_range.clone())?;
        }
    }

    Ok(())
}

fn validate_math_fragment(
    source: &str,
    limits: MathLimits,
    range: Range<usize>,
) -> Result<(), MathTypesetError> {
    if source.trim().is_empty() {
        return Err(MathTypesetError::EmptyMathFragment {
            start: range.start,
            end: range.end,
        });
    }

    strict_hash_precheck(source, range.start)?;
    let depth = max_grouping_depth(source);
    if depth > limits.max_math_depth {
        return Err(MathTypesetError::MathDepthExceeded {
            actual: depth,
            limit: limits.max_math_depth,
        });
    }
    validate_math_parse(source, range.start)?;

    Ok(())
}

fn strict_hash_precheck(source: &str, offset: usize) -> Result<(), MathTypesetError> {
    let mut escaped = false;
    for (idx, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '#' {
            return Err(MathTypesetError::UnsupportedSyntax {
                position: offset + idx,
                message: "embedded Typst code is not allowed in math fragments",
            });
        }
    }
    Ok(())
}

fn validate_math_parse(source: &str, offset: usize) -> Result<(), MathTypesetError> {
    crate::engine::math::syntax::parse_math(source, offset).map(|_| ())
}

fn max_grouping_depth(source: &str) -> usize {
    let mut escaped = false;
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max_depth
}

fn label_has_markup(source: &str) -> bool {
    source
        .chars()
        .any(|ch| matches!(ch, '$' | '#' | '\\' | '_' | '*' | '`'))
}
