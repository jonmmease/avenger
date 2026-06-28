use std::{ops::Range, path::PathBuf};

use indexmap::IndexMap;

use crate::engine::engine::TypstEngineCore;
use crate::engine::math::syntax::is_retained_math_name;
use crate::engine::syntax::is_retained_markup_name;
use crate::style::{MathStyle, PlainTextStyle};
use crate::types::{
    LineLayoutArtifact, LineLayoutOptions, LineOutputOptions, PositionedTextLineRun,
    PositionedTextLineRunKind, TypesetMetrics,
};
use crate::typst_pdf::{FontResource, PdfGlyph, PdfGlyphRun, PdfTextLayer};
pub use crate::typst_render::RasterImage;
use crate::typst_svg::{PathArtifact, PathImageItem, PathItem, PathKind, Transform};

#[cfg(feature = "raster")]
use crate::typst_render::RasterRequest;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use crate::error::{LabelError, LabelInitError};
pub use crate::limits::LabelLimits;
pub use crate::warnings::LabelWarning;
pub type TextStyle = PlainTextStyle;

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
        Self {
            load_system_fonts: true,
            extra_font_dirs: Vec::new(),
            extra_font_families: Vec::new(),
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
        let artifact = self
            .inner
            .typeset_markup_line(source, &line_layout_options(options))?;
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
            .typeset_plain_line(text, &line_layout_options(options))?;
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
            let paths = paths_for_run(run, artifact.paths.as_ref());
            let (pdf_text, font_resources) =
                pdf_text_for_run(run, artifact.pdf_text.as_ref(), &artifact.font_resources);
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
        push_missing_aggregate_shape_items(
            &mut items,
            0..artifact.source.len(),
            artifact.paths.as_ref(),
        );

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

fn push_missing_aggregate_shape_items(
    items: &mut Vec<(Point, LabelFrameItem)>,
    byte_range: Range<usize>,
    paths: Option<&PathArtifact>,
) {
    let Some(paths) = paths else {
        return;
    };
    for item in &paths.items {
        if !matches!(item.kind, PathKind::MathShape) {
            continue;
        }
        if frame_contains_path_item(items, item) {
            continue;
        }
        items.push((
            Point::ZERO,
            LabelFrameItem::Shape(ShapeItem {
                byte_range: byte_range.clone(),
                text_kind: None,
                item: item.clone(),
            }),
        ));
    }
}

fn frame_contains_path_item(items: &[(Point, LabelFrameItem)], path: &PathItem) -> bool {
    items.iter().any(|(_, item)| match item {
        LabelFrameItem::Shape(shape) => &shape.item == path,
        LabelFrameItem::Group(group) => frame_contains_path_item(&group.items, path),
        LabelFrameItem::Text(_) | LabelFrameItem::Image(_) => false,
    })
}

fn push_plain_run_items(
    items: &mut Vec<(Point, LabelFrameItem)>,
    run: &PositionedTextLineRun,
    paths: Option<&PathArtifact>,
    pdf_text: Option<PdfTextLayer>,
    font_resources: Vec<FontResource>,
) {
    let mut foreground_shapes = Vec::new();
    let mut images = Vec::new();

    if let Some(paths) = paths {
        for item in &paths.items {
            let shape = (
                Point::ZERO,
                LabelFrameItem::Shape(ShapeItem {
                    byte_range: run.byte_range.clone(),
                    text_kind: Some(TextItemKind::Plain),
                    item: item.clone(),
                }),
            );
            if item.fill.is_some() && item.stroke.is_none() {
                items.push(shape);
            } else {
                foreground_shapes.push(shape);
            }
        }
        for image in &paths.images {
            images.push((
                Point::ZERO,
                LabelFrameItem::Image(ImageItem {
                    byte_range: run.byte_range.clone(),
                    image: image.clone(),
                }),
            ));
        }
    }

    push_text_item(items, run, TextItemKind::Plain, pdf_text, font_resources);
    items.extend(images);
    items.extend(foreground_shapes);
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
            style: run.text_style.clone(),
            metrics: LabelMetrics::from(run.metrics),
            glyphs: glyphs_from_pdf_text(pdf_text.as_ref()),
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
        for item in &paths.items {
            items.push((
                Point::ZERO,
                LabelFrameItem::Shape(ShapeItem {
                    byte_range: byte_range.clone(),
                    text_kind: Some(text_kind),
                    item: item.clone(),
                }),
            ));
        }
        for image in &paths.images {
            items.push((
                Point::ZERO,
                LabelFrameItem::Image(ImageItem {
                    byte_range: byte_range.clone(),
                    image: image.clone(),
                }),
            ));
        }
    }
}

fn paths_for_run(
    run: &PositionedTextLineRun,
    aggregate: Option<&PathArtifact>,
) -> Option<PathArtifact> {
    let mut paths = run.paths.clone().unwrap_or_else(|| PathArtifact {
        logical_width: run.metrics.width,
        logical_height: run.metrics.height,
        items: Vec::new(),
        images: Vec::new(),
    });

    if let Some(aggregate) = aggregate {
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
    }

    (!paths.items.is_empty() || !paths.images.is_empty()).then_some(paths)
}

fn pdf_text_for_run(
    run: &PositionedTextLineRun,
    aggregate: Option<&PdfTextLayer>,
    aggregate_resources: &[FontResource],
) -> (Option<PdfTextLayer>, Vec<FontResource>) {
    if let Some(aggregate) = aggregate {
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
    }

    (run.pdf_text.clone(), run.font_resources.clone())
}

fn glyph_run_belongs_to_run(glyph_run: &PdfGlyphRun, run: &PositionedTextLineRun) -> bool {
    glyph_run.glyphs.iter().any(|glyph| {
        let center_x = glyph.transform.dx + glyph.x_advance / 2.0;
        value_is_in_run_x_range(center_x, run)
    })
}

fn path_item_belongs_to_run(item: &PathItem, run: &PositionedTextLineRun) -> bool {
    let center_x = transformed_path_center_x(item).unwrap_or(item.transform.dx);
    value_is_in_run_x_range(center_x, run)
}

fn image_item_belongs_to_run(image: &PathImageItem, run: &PositionedTextLineRun) -> bool {
    value_is_in_run_x_range(image.transform.dx + image.width / 2.0, run)
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
            let x = item.transform.xx * x + item.transform.xy * y + item.transform.dx;
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
    for (_, item) in items {
        match item {
            LabelFrameItem::Text(text) => output.push_str(&text.text),
            LabelFrameItem::Group(group) => collect_semantic_text(&group.items, output),
            LabelFrameItem::Shape(_) | LabelFrameItem::Image(_) => {}
        }
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
    pub style: Option<TextStyle>,
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

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfOptions {}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfLabel {
    pub metrics: LabelMetrics,
    pub semantic_text: String,
    pub font_resources: Vec<FontResource>,
    pub glyph_runs: Vec<PdfGlyphRun>,
    pub path_items: Vec<PdfPathItem>,
    pub draw_items: Vec<PdfDrawItem>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PdfPathItem {
    pub byte_range: Range<usize>,
    pub item: PathItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PdfDrawItem {
    GlyphRun(usize),
    PathItem(usize),
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
    };
    collect_frame_paths(&frame.items, &mut artifact);
    artifact
}

fn collect_frame_paths(items: &[(Point, LabelFrameItem)], artifact: &mut PathArtifact) {
    for (_, item) in items {
        match item {
            LabelFrameItem::Shape(shape) => artifact.items.push(shape.item.clone()),
            LabelFrameItem::Image(image) => artifact.images.push(image.image.clone()),
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
        params: options.params.clone(),
        outputs: LineOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        },
        limits: options.limits,
    }
}

fn glyphs_from_pdf_text(pdf_text: Option<&PdfTextLayer>) -> Vec<Glyph> {
    pdf_text
        .into_iter()
        .flat_map(|pdf_text| pdf_text.glyph_runs.iter())
        .flat_map(|run| run.glyphs.iter().map(Glyph::from))
        .collect()
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

fn label_has_markup(source: &str) -> bool {
    source
        .chars()
        .any(|ch| matches!(ch, '$' | '#' | '\\' | '_' | '*' | '`'))
}
