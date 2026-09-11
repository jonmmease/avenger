//! Public frame-first label API.
//!
//! This is the Avenger-facing boundary for the lightweight Typst label engine:
//! compile markup or literal text into a `CompiledLabel`, then lower that
//! compiled frame into raster, SVG, or PDF artifacts. Avenger fallback,
//! truncation, cache keys, and renderer policy live outside this crate.

mod error;
pub(crate) mod fonts;
mod params;
mod pdf;
mod warnings;

use std::{
    hash::{Hash, Hasher},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

use avenger_format_datetime::DateTimeLocaleRegistry;
use avenger_format_number::NumberLocaleRegistry;
use indexmap::IndexMap;

use crate::typst_eval::markup::{
    DateTimeFormatMarkupContext, MarkupFormatContext, NumberFormatMarkupContext,
    parse_line_with_format_context,
};
use crate::typst_eval::math::is_retained_math_name;
use crate::typst_eval::math::parse_math_with_params;
use crate::typst_layout::frame::{
    LineLayoutArtifact, LineLayoutOptions, PositionedTextLineRun, PositionedTextLineRunKind,
    TypesetMetrics,
};
use crate::typst_layout::line::TypstEngineCore;
use crate::typst_library::MathStyle;
use crate::typst_library::foundations::{Dict, Scope, Value};
use crate::typst_library::text::call::is_retained_markup_name;
use crate::typst_library::text::content::{LabelContent, LineNode};
pub use crate::typst_render::RasterImage;
use crate::typst_svg::{PathArtifact, PathImageItem, PathItem, PathKind, Transform};

#[cfg(feature = "raster")]
use crate::typst_render::RasterRequest;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use crate::typst_layout::frame::FontFeature;
pub use crate::typst_library::{MathFontBytesId, TextStyle};
pub use error::{LabelError, LabelInitError};
pub use pdf::{
    FontResource, FontResourceId, PdfDrawItem, PdfGlyph, PdfGlyphRun, PdfLabel, PdfOptions,
    PdfPathItem, PdfTextLayer,
};
pub use warnings::LabelWarning;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub struct EngineOptions {
    pub fonts: FontOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontOptions {
    pub load_system_fonts: bool,
    /// Policy for requested text font families; warnings are returned in CompiledLabel.
    #[cfg_attr(feature = "serde", serde(default))]
    pub missing_font: MissingFontPolicy,
    pub extra_font_dirs: Vec<PathBuf>,
    pub extra_font_families: Vec<String>,
    #[cfg_attr(feature = "serde", serde(skip))]
    pub registered_fonts: Vec<RegisteredFont>,
    pub default_sans_serif_family: Option<String>,
    pub default_monospace_family: Option<String>,
    pub default_math_family: Option<String>,
}

impl Default for FontOptions {
    fn default() -> Self {
        Self {
            load_system_fonts: true,
            missing_font: MissingFontPolicy::Fallback,
            extra_font_dirs: Vec::new(),
            extra_font_families: Vec::new(),
            registered_fonts: Vec::new(),
            default_sans_serif_family: None,
            default_monospace_family: None,
            default_math_family: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFont {
    pub id: MathFontBytesId,
    pub data: Arc<[u8]>,
    pub face_index: u32,
}

impl RegisteredFont {
    pub fn new(id: MathFontBytesId, data: impl Into<Arc<[u8]>>) -> Self {
        Self {
            id,
            data: data.into(),
            face_index: 0,
        }
    }

    pub fn with_face_index(mut self, face_index: u32) -> Self {
        self.face_index = face_index;
        self
    }
}

/// Behavior when none of the requested font families is available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MissingFontPolicy {
    Error,
    Warn,
    #[default]
    Fallback,
}

/// Vertical metrics from the resolved face, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelLimits {
    pub max_source_bytes: usize,
    pub max_math_spans: usize,
    pub max_math_depth: usize,
}

impl Default for LabelLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024,
            max_math_spans: 64,
            max_math_depth: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub struct LabelOptions {
    pub text: TextStyle,
    pub math: MathStyle,
    pub params: LabelParams,
    pub number_locale: Option<String>,
    #[cfg_attr(feature = "serde", serde(skip, default))]
    pub number_locale_registry: Option<Arc<NumberLocaleRegistry>>,
    pub datetime_locale: Option<String>,
    pub datetime_timezone: Option<String>,
    #[cfg_attr(feature = "serde", serde(skip, default))]
    pub datetime_locale_registry: Option<Arc<DateTimeLocaleRegistry>>,
    pub limits: LabelLimits,
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
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
    UtcDateTime(chrono::DateTime<chrono::Utc>),
    Array(Vec<LabelParamValue>),
    Dict(IndexMap<String, LabelParamValue>),
}

impl Hash for LabelParamValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::None => {}
            Self::Bool(value) => value.hash(state),
            Self::Int(value) => value.hash(state),
            Self::Float(value) => value.to_bits().hash(state),
            Self::Str(value) => value.hash(state),
            Self::Date(value) => value.hash(state),
            Self::DateTime(value) => value.hash(state),
            Self::UtcDateTime(value) => value.hash(state),
            Self::Array(values) => values.hash(state),
            Self::Dict(values) => {
                values.len().hash(state);
                for (key, value) in values {
                    key.hash(state);
                    value.hash(state);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct LabelEngine {
    inner: TypstEngineCore,
}

impl LabelEngine {
    pub fn new(options: EngineOptions) -> Result<Self, LabelInitError> {
        Ok(Self {
            inner: TypstEngineCore::new(&options)?,
        })
    }

    pub fn compile(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError> {
        validate_source_limits(source, options.limits)?;
        validate_label_params(&options.params)?;
        let layout_options = line_layout_options(options);
        let line = parse_line_with_format_context(
            source,
            &layout_options.params,
            MarkupFormatContext {
                number: NumberFormatMarkupContext {
                    locale_id: options.number_locale.as_deref(),
                    registry: options.number_locale_registry.as_deref(),
                },
                datetime: DateTimeFormatMarkupContext {
                    locale_id: options.datetime_locale.as_deref(),
                    timezone: options.datetime_timezone.as_deref(),
                    registry: options.datetime_locale_registry.as_deref(),
                },
            },
        )?;
        validate_line_math(
            &line,
            options.limits,
            &options.params,
            &layout_options.params,
        )?;
        let artifact = self
            .inner
            .typeset_parsed_line(source, &line, &layout_options)?;
        self.validate_fonts(
            CompiledLabel::from_artifact(artifact, label_has_markup(source)),
            &options.text,
        )
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
            .typeset_plain_line(text, &line_layout_options(options))?;
        self.validate_fonts(CompiledLabel::from_artifact(artifact, false), &options.text)
    }

    pub fn measure_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<LabelMetrics, LabelError> {
        self.compile_text(text, options).map(|label| label.metrics)
    }

    /// Resolve the same primary face used for plain text and read its font tables.
    pub fn font_metrics(&self, style: &TextStyle) -> Result<FontMetrics, LabelError> {
        self.inner.font_metrics(style)
    }

    fn validate_fonts(
        &self,
        mut label: CompiledLabel,
        base: &TextStyle,
    ) -> Result<CompiledLabel, LabelError> {
        fn styles<'a>(items: &'a [(Point, LabelFrameItem)], output: &mut Vec<&'a TextStyle>) {
            for (_, item) in items {
                match item {
                    LabelFrameItem::Text(text) => output.extend(text.style.as_ref()),
                    LabelFrameItem::Group(group) => styles(&group.items, output),
                    _ => {}
                }
            }
        }
        let mut requested = vec![base];
        styles(&label.frame.items, &mut requested);
        let mut warnings = Vec::new();
        for style in requested {
            if let Some(warning) = self.inner.check_font(style)?
                && !warnings.contains(&warning)
            {
                warnings.push(warning);
            }
        }
        label.warnings.extend(warnings);
        Ok(label)
    }

    pub fn referenced_params(&self, source: &str) -> Result<Vec<String>, LabelError> {
        referenced_params(source)
    }
}

pub fn referenced_params(source: &str) -> Result<Vec<String>, LabelError> {
    params::referenced_params(source)
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CompiledLabel {
    pub source: String,
    pub frame: LabelFrame,
    pub metrics: LabelMetrics,
    pub flags: LabelFlags,
    pub warnings: Vec<LabelWarning>,
}

impl CompiledLabel {
    fn from_artifact(artifact: LineLayoutArtifact, has_markup: bool) -> Self {
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
        }
    }

    pub fn semantic_text(&self) -> String {
        let mut text = String::new();
        collect_semantic_text(&self.frame.items, &mut text);
        if text.is_empty() {
            self.source.clone()
        } else {
            text
        }
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
    fn from_artifact(artifact: &LineLayoutArtifact) -> Self {
        let mut items = Vec::new();
        for run in &artifact.positioned_runs {
            let paths = paths_for_run(run, &artifact.paths);
            let (pdf_text, font_resources) =
                pdf_text_for_run(run, &artifact.pdf_text, &artifact.font_resources);
            match run.kind {
                PositionedTextLineRunKind::Plain => {
                    push_plain_run_items(&mut items, run, paths.as_ref(), pdf_text, font_resources);
                }
                PositionedTextLineRunKind::Math => {
                    push_text_item(
                        &mut items,
                        run,
                        TextItemKind::Math,
                        pdf_text,
                        font_resources,
                    );
                    push_path_items(
                        &mut items,
                        run.byte_range.clone(),
                        TextItemKind::Math,
                        paths.as_ref(),
                    );
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

fn push_plain_run_items(
    items: &mut Vec<(Point, LabelFrameItem)>,
    run: &PositionedTextLineRun,
    paths: Option<&PathArtifact>,
    pdf_text: Option<PdfTextLayer>,
    font_resources: Vec<FontResource>,
) {
    use crate::typst_svg::PathDrawItem;
    let mut pending = Some((pdf_text, font_resources));
    if let Some(paths) = paths {
        for draw in paths.ordered_items() {
            let glyph = match draw {
                PathDrawItem::Path(i) => {
                    matches!(paths.items[i].kind, PathKind::GlyphOutline { .. })
                }
                PathDrawItem::Image(_) => true,
            };
            if glyph && let Some((pdf, resources)) = pending.take() {
                push_text_item(items, run, TextItemKind::Plain, pdf, resources);
            }
            push_path_draw_item(items, &run.byte_range, TextItemKind::Plain, paths, draw);
        }
    }
    if let Some((pdf, resources)) = pending {
        push_text_item(items, run, TextItemKind::Plain, pdf, resources);
    }
}

fn push_text_item(
    items: &mut Vec<(Point, LabelFrameItem)>,
    run: &PositionedTextLineRun,
    kind: TextItemKind,
    pdf_text: Option<PdfTextLayer>,
    font_resources: Vec<FontResource>,
) {
    let point = Point { x: run.x, y: run.y };
    items.push((
        point,
        LabelFrameItem::Text(TextItem {
            kind,
            text: run.text.clone(),
            byte_range: run.byte_range.clone(),
            is_rtl: run.is_rtl,
            style: run.text_style.clone(),
            font_features: run.font_features.clone(),
            metrics: LabelMetrics::from(run.metrics),
            glyphs: glyphs_from_pdf_text(pdf_text.as_ref(), run.byte_range.start),
            pdf_text,
            font_resources,
        }),
    ));
}

fn push_path_items(
    items: &mut Vec<(Point, LabelFrameItem)>,
    byte_range: Range<usize>,
    text_kind: TextItemKind,
    paths: Option<&PathArtifact>,
) {
    if let Some(paths) = paths {
        for draw in paths.ordered_items() {
            push_path_draw_item(items, &byte_range, text_kind, paths, draw);
        }
    }
}

fn push_path_draw_item(
    items: &mut Vec<(Point, LabelFrameItem)>,
    range: &Range<usize>,
    kind: TextItemKind,
    paths: &PathArtifact,
    draw: crate::typst_svg::PathDrawItem,
) {
    use crate::typst_svg::PathDrawItem;
    let item = match draw {
        PathDrawItem::Path(i) => LabelFrameItem::Shape(ShapeItem {
            byte_range: range.clone(),
            text_kind: Some(kind),
            item: paths.items[i].clone(),
        }),
        PathDrawItem::Image(i) => LabelFrameItem::Image(ImageItem {
            byte_range: range.clone(),
            image: paths.images[i].clone(),
        }),
    };
    items.push((Point::ZERO, item));
}

fn paths_for_run(run: &PositionedTextLineRun, aggregate: &PathArtifact) -> Option<PathArtifact> {
    if let Some(paths) = &run.paths {
        return Some(paths.clone());
    }
    let mut paths = run.paths.clone().unwrap_or_else(|| PathArtifact {
        logical_width: run.metrics.width,
        logical_height: run.metrics.height,
        items: Vec::new(),
        images: Vec::new(),
        draw_order: Vec::new(),
    });

    for item in &aggregate.items {
        if path_item_belongs_to_run(item, run)
            && !paths.items.iter().any(|existing| existing == item)
        {
            paths.items.push(item.clone());
        }
    }
    for image in &aggregate.images {
        if image_item_belongs_to_run(image, run)
            && !paths.images.iter().any(|existing| existing == image)
        {
            paths.images.push(image.clone());
        }
    }

    (!paths.items.is_empty() || !paths.images.is_empty()).then_some(paths)
}

fn pdf_text_for_run(
    run: &PositionedTextLineRun,
    aggregate: &PdfTextLayer,
    aggregate_resources: &[FontResource],
) -> (Option<PdfTextLayer>, Vec<FontResource>) {
    if run.pdf_text.is_some() {
        return (run.pdf_text.clone(), run.font_resources.clone());
    }
    let glyph_runs = aggregate
        .glyph_runs
        .iter()
        .filter(|glyph_run| glyph_run_belongs_to_run(glyph_run, run))
        .cloned()
        .collect::<Vec<_>>();
    if !glyph_runs.is_empty() {
        return (
            Some(PdfTextLayer {
                logical_width: run.metrics.width,
                logical_height: run.metrics.height,
                semantic_text: run.text.clone(),
                glyph_runs,
            }),
            aggregate_resources.to_vec(),
        );
    }

    (run.pdf_text.clone(), run.font_resources.clone())
}

fn glyph_run_belongs_to_run(glyph_run: &PdfGlyphRun, run: &PositionedTextLineRun) -> bool {
    glyph_run.glyphs.iter().any(|glyph| {
        let center_x = glyph.transform.tx + glyph.x_advance / 2.0;
        value_is_in_run_x_range(center_x, run)
    })
}

fn path_item_belongs_to_run(item: &PathItem, run: &PositionedTextLineRun) -> bool {
    let center_x = transformed_path_center_x(item).unwrap_or(item.transform.tx);
    value_is_in_run_x_range(center_x, run)
}

fn image_item_belongs_to_run(image: &PathImageItem, run: &PositionedTextLineRun) -> bool {
    value_is_in_run_x_range(image.transform.tx + image.width / 2.0, run)
}

fn value_is_in_run_x_range(x: f32, run: &PositionedTextLineRun) -> bool {
    let left = run.x - 0.5;
    let right = run.x + run.metrics.width + 0.5;
    x >= left && x <= right
}

fn transformed_path_center_x(item: &PathItem) -> Option<f32> {
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for command in &item.path.commands {
        for (x, y) in command_points(command) {
            let x = item.transform.sx * x + item.transform.kx * y + item.transform.tx;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }
    }
    min_x.is_finite().then_some((min_x + max_x) / 2.0)
}

fn command_points(command: &crate::typst_svg::PathCommand) -> Vec<(f32, f32)> {
    match *command {
        crate::typst_svg::PathCommand::MoveTo { x, y }
        | crate::typst_svg::PathCommand::LineTo { x, y } => {
            vec![(x, y)]
        }
        crate::typst_svg::PathCommand::QuadTo { x1, y1, x, y } => vec![(x1, y1), (x, y)],
        crate::typst_svg::PathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => vec![(x1, y1), (x2, y2), (x, y)],
        crate::typst_svg::PathCommand::Close => Vec::new(),
    }
}

fn collect_semantic_text(items: &[(Point, LabelFrameItem)], output: &mut String) {
    fn collect<'a>(items: &'a [(Point, LabelFrameItem)], runs: &mut Vec<(usize, &'a str)>) {
        for (_, item) in items {
            match item {
                LabelFrameItem::Text(text) => runs.push((text.byte_range.start, &text.text)),
                LabelFrameItem::Group(group) => collect(&group.items, runs),
                _ => {}
            }
        }
    }
    let mut runs = Vec::new();
    collect(items, &mut runs);
    runs.sort_by_key(|(start, _)| *start);
    for (_, text) in runs {
        output.push_str(text);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TextItemKind {
    Plain,
    Math,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextItem {
    pub kind: TextItemKind,
    pub text: String,
    pub byte_range: Range<usize>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub is_rtl: bool,
    pub style: Option<TextStyle>,
    /// OpenType features used to produce the positioned glyphs.
    #[cfg_attr(feature = "serde", serde(default))]
    pub font_features: Vec<FontFeature>,
    pub metrics: LabelMetrics,
    pub glyphs: Vec<Glyph>,
    pub pdf_text: Option<PdfTextLayer>,
    pub font_resources: Vec<FontResource>,
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
    pub transform: Transform,
}

impl From<&PdfGlyph> for Glyph {
    fn from(glyph: &PdfGlyph) -> Self {
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
    pub text_kind: Option<TextItemKind>,
    pub item: PathItem,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageItem {
    pub byte_range: Range<usize>,
    pub image: PathImageItem,
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
    pub font_resources: Vec<FontResource>,
}

pub fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if should_escape_text_char(ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn should_escape_text_char(ch: char) -> bool {
    matches!(
        ch,
        '\\' | '/'
            | '['
            | ']'
            | '~'
            | '-'
            | '+'
            | '='
            | '.'
            | '\''
            | '"'
            | '*'
            | '_'
            | ':'
            | '`'
            | '$'
            | '<'
            | '>'
            | '@'
            | '#'
            | 'h'
    )
}

pub fn rasterize(
    label: &CompiledLabel,
    options: &RasterOptions,
) -> Result<RasterImage, LabelError> {
    let paths = frame_path_artifact(&label.frame);
    rasterize_paths(&paths, *options)
}

#[cfg(feature = "raster")]
fn rasterize_paths(
    paths: &PathArtifact,
    options: RasterOptions,
) -> Result<RasterImage, LabelError> {
    crate::typst_render::rasterize_path_artifact(
        paths,
        RasterRequest {
            scale: options.scale,
        },
    )
}

#[cfg(not(feature = "raster"))]
fn rasterize_paths(
    _paths: &PathArtifact,
    _options: RasterOptions,
) -> Result<RasterImage, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "the avenger-typst-label raster feature is disabled",
    ))
}

pub fn svg_items(label: &CompiledLabel, _options: &SvgOptions) -> Result<SvgLabel, LabelError> {
    Ok(SvgLabel {
        metrics: label.metrics,
        items: label.frame.items.clone(),
        font_resources: frame_font_resources(&label.frame),
    })
}

pub fn pdf_items(label: &CompiledLabel, _options: &PdfOptions) -> Result<PdfLabel, LabelError> {
    let mut output = PdfLabel {
        metrics: label.metrics,
        semantic_text: label.semantic_text(),
        font_resources: Vec::new(),
        glyph_runs: Vec::new(),
        path_items: Vec::new(),
        draw_items: Vec::new(),
    };
    collect_pdf_items(&label.frame.items, &mut output);
    output.font_resources.sort_by_key(|resource| resource.id.0);
    output.font_resources.dedup_by_key(|resource| resource.id.0);
    Ok(output)
}

fn frame_path_artifact(frame: &LabelFrame) -> PathArtifact {
    let mut artifact = PathArtifact {
        logical_width: frame.size.x,
        logical_height: frame.size.y,
        items: Vec::new(),
        images: Vec::new(),
        draw_order: Vec::new(),
    };
    collect_frame_paths(&frame.items, &mut artifact);
    artifact
}

fn collect_frame_paths(items: &[(Point, LabelFrameItem)], artifact: &mut PathArtifact) {
    for (_, item) in items {
        match item {
            LabelFrameItem::Shape(shape) => {
                artifact
                    .draw_order
                    .push(crate::typst_svg::PathDrawItem::Path(artifact.items.len()));
                artifact.items.push(shape.item.clone());
            }
            LabelFrameItem::Image(image) => {
                artifact
                    .draw_order
                    .push(crate::typst_svg::PathDrawItem::Image(artifact.images.len()));
                artifact.images.push(image.image.clone());
            }
            LabelFrameItem::Group(group) => collect_frame_paths(&group.items, artifact),
            LabelFrameItem::Text(_) => {}
        }
    }
}

fn frame_font_resources(frame: &LabelFrame) -> Vec<FontResource> {
    let mut resources = Vec::new();
    collect_frame_font_resources(&frame.items, &mut resources);
    resources.sort_by_key(|resource| resource.id.0);
    resources.dedup_by_key(|resource| resource.id.0);
    resources
}

fn collect_frame_font_resources(
    items: &[(Point, LabelFrameItem)],
    resources: &mut Vec<FontResource>,
) {
    for (_, item) in items {
        match item {
            LabelFrameItem::Text(text) => resources.extend(text.font_resources.iter().cloned()),
            LabelFrameItem::Group(group) => collect_frame_font_resources(&group.items, resources),
            LabelFrameItem::Shape(_) | LabelFrameItem::Image(_) => {}
        }
    }
}

fn collect_pdf_items(items: &[(Point, LabelFrameItem)], output: &mut PdfLabel) {
    for (_, item) in items {
        match item {
            LabelFrameItem::Text(text) => {
                output
                    .font_resources
                    .extend(text.font_resources.iter().cloned());
                if let Some(pdf_text) = &text.pdf_text {
                    for run in &pdf_text.glyph_runs {
                        let index = output.glyph_runs.len();
                        output.glyph_runs.push(run.clone());
                        output.draw_items.push(PdfDrawItem::GlyphRun(index));
                    }
                }
            }
            LabelFrameItem::Shape(shape) => {
                if matches!(shape.item.kind, PathKind::MathShape) || shape.item.stroke.is_some() {
                    let index = output.path_items.len();
                    output.path_items.push(PdfPathItem {
                        byte_range: shape.byte_range.clone(),
                        item: shape.item.clone(),
                    });
                    output.draw_items.push(PdfDrawItem::PathItem(index));
                }
            }
            LabelFrameItem::Group(group) => collect_pdf_items(&group.items, output),
            LabelFrameItem::Image(_) => {}
        }
    }
}

fn line_layout_options(options: &LabelOptions) -> LineLayoutOptions {
    LineLayoutOptions {
        text_style: options.text.clone(),
        math_style: options.math.clone(),
        params: scope_from_label_params(&options.params),
        limits: options.limits,
    }
}

fn scope_from_label_params(params: &LabelParams) -> Scope {
    Scope::new(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value_from_label_param(value)))
            .collect::<Dict>(),
    )
}

fn value_from_label_param(value: &LabelParamValue) -> Value {
    match value {
        LabelParamValue::None => Value::None,
        LabelParamValue::Bool(value) => Value::Bool(*value),
        LabelParamValue::Int(value) => Value::Int(*value),
        LabelParamValue::Float(value) => Value::Float(*value),
        LabelParamValue::Str(value) => Value::Str(value.clone()),
        LabelParamValue::Date(value) => Value::Date(*value),
        LabelParamValue::DateTime(value) => Value::DateTime(*value),
        LabelParamValue::UtcDateTime(value) => Value::UtcDateTime(*value),
        LabelParamValue::Array(values) => {
            Value::Array(values.iter().map(value_from_label_param).collect())
        }
        LabelParamValue::Dict(values) => Value::Dict(
            values
                .iter()
                .map(|(name, value)| (name.clone(), value_from_label_param(value)))
                .collect(),
        ),
    }
}

fn glyphs_from_pdf_text(pdf_text: Option<&PdfTextLayer>, source_offset: usize) -> Vec<Glyph> {
    let Some(pdf_text) = pdf_text else {
        return Vec::new();
    };
    let mut run_search_start = 0;
    let mut glyphs = Vec::new();
    for run in &pdf_text.glyph_runs {
        let relative_run_start = pdf_text
            .semantic_text
            .get(run_search_start..)
            .and_then(|tail| tail.find(&run.text))
            .map(|offset| run_search_start + offset)
            .or_else(|| pdf_text.semantic_text.find(&run.text))
            .unwrap_or(run_search_start);
        run_search_start = relative_run_start.saturating_add(run.text.len());
        glyphs.extend(run.glyphs.iter().map(|glyph| {
            let mut glyph = Glyph::from(glyph);
            let offset = source_offset + relative_run_start;
            glyph.text_range.start += offset;
            glyph.text_range.end += offset;
            glyph
        }));
    }
    glyphs
}

fn validate_source_limits(source: &str, limits: LabelLimits) -> Result<(), LabelError> {
    if source.len() > limits.max_source_bytes {
        return Err(LabelError::SourceTooLarge {
            actual: source.len(),
            limit: limits.max_source_bytes,
        });
    }
    Ok(())
}

fn validate_label_params(params: &LabelParams) -> Result<(), LabelError> {
    for name in params.keys() {
        if is_retained_markup_name(name) {
            return Err(LabelError::ParameterNameCollision {
                name: name.clone(),
                namespace: "text",
            });
        }
        if is_retained_math_name(name) {
            return Err(LabelError::ParameterNameCollision {
                name: name.clone(),
                namespace: "math",
            });
        }
    }
    Ok(())
}

fn validate_line_math(
    line: &LabelContent,
    limits: LabelLimits,
    params: &LabelParams,
    scope: &Scope,
) -> Result<(), LabelError> {
    let math_span_count = line
        .nodes
        .iter()
        .filter(|node| matches!(node, LineNode::Math(_)))
        .count();
    if math_span_count > limits.max_math_spans {
        return Err(LabelError::TooManyMathSpans {
            actual: math_span_count,
            limit: limits.max_math_spans,
        });
    }

    for math in line.nodes.iter().filter_map(|node| match node {
        LineNode::Math(math) => Some(math),
        _ => None,
    }) {
        if math.source.trim().is_empty() {
            return Err(LabelError::EmptyMathFragment {
                start: math.source_range.start,
                end: math.source_range.end,
            });
        }

        let depth = max_grouping_depth(&math.source);
        if depth > limits.max_math_depth {
            return Err(LabelError::MathDepthExceeded {
                actual: depth,
                limit: limits.max_math_depth,
            });
        }

        strict_hash_precheck(&math.source, math.source_range.start, params)?;
        parse_math_with_params(&math.source, math.source_range.start, scope)?;
    }
    Ok(())
}

fn strict_hash_precheck(
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<(), LabelError> {
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
            if allowed_param_ident_end(source, idx, params).is_some() {
                continue;
            }
            if !is_embedded_literal_allowed_in_math(source, idx) {
                return Err(LabelError::UnsupportedSyntax {
                    position: offset + idx,
                    message: "embedded Typst code is not allowed in math fragments",
                });
            }
        }
    }
    Ok(())
}

fn allowed_param_ident_end(source: &str, idx: usize, params: &LabelParams) -> Option<usize> {
    let rest = source.get(idx + 1..)?;
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first != '_' && !unicode_ident::is_xid_start(first) {
        return None;
    }

    let mut end = idx + 1 + first.len_utf8();
    for (relative_idx, ch) in chars {
        if ch == '_' || unicode_ident::is_xid_continue(ch) {
            end = idx + 1 + relative_idx + ch.len_utf8();
        } else {
            break;
        }
    }

    let name = &source[idx + 1..end];
    params.contains_key(name).then_some(end)
}

fn is_embedded_literal_allowed_in_math(source: &str, idx: usize) -> bool {
    let Some(rest) = source.get(idx + 1..) else {
        return false;
    };
    rest.starts_with("true")
        || rest.starts_with("false")
        || rest.starts_with("auto")
        || rest.starts_with('(')
        || starts_with_math_numeric_literal(rest)
}

fn starts_with_math_numeric_literal(rest: &str) -> bool {
    let mut chars = rest.char_indices().peekable();
    if matches!(chars.peek(), Some((_, '+' | '-'))) {
        chars.next();
    }

    let mut saw_digit = false;
    let mut saw_dot = false;
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            chars.next();
        } else if ch == '.' && !saw_dot {
            saw_dot = true;
            chars.next();
        } else {
            break;
        }
    }
    if !saw_digit {
        return false;
    }

    let unit_start = chars.peek().map_or(rest.len(), |(idx, _)| *idx);
    let unit = &rest[unit_start..];
    ["%", "em", "pt", "deg", "rad"]
        .iter()
        .any(|suffix| starts_with_literal_unit(unit, suffix))
}

fn starts_with_literal_unit(unit_and_tail: &str, unit: &str) -> bool {
    let Some(tail) = unit_and_tail.strip_prefix(unit) else {
        return false;
    };
    tail.chars()
        .next()
        .is_none_or(|ch| !matches!(ch, '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
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
