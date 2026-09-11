use std::ops::Range;

use lyon_path::Path;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight, TextSyntaxMode},
};

use lyon_path::geom::point;

use crate::math::TextMarkupConfig;

use crate::text_line::{
    bounds_from_metrics, tight_bounds_from_metrics, typeset_line, TextLineMeasurer,
};

#[derive(Debug, Clone)]
pub struct TextPathExtractionConfig<'a> {
    pub text: &'a str,
    pub color: [f32; 4],
    pub font: &'a str,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    /// Positive finite width in logical pixels. Plain text uses grapheme-safe
    /// ellipsis; Typst markup is compiled intact and clipped at this width.
    /// Other values leave the label unconstrained.
    pub limit: f32,
    pub syntax_mode: TextSyntaxMode,
    pub params: &'a avenger_typst_label::LabelParams,
    pub number_locale: Option<&'a str>,
    pub number_locale_specs: Option<&'a crate::NumberLocaleSpecs>,
    pub datetime_locale: Option<&'a str>,
    pub datetime_timezone: Option<&'a str>,
    pub datetime_locale_specs: Option<&'a crate::DateTimeLocaleSpecs>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathDashPattern {
    pub array: Vec<f32>,
    pub phase: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathStroke {
    pub color: [f32; 4],
    pub width: f32,
    pub line_cap: TextPathLineCap,
    pub line_join: TextPathLineJoin,
    pub dash: Option<TextPathDashPattern>,
    pub miter_limit: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPathLineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPathLineJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathKind {
    PlainGlyph,
    MathGlyph,
    MathShape,
}

#[derive(Debug, Clone)]
pub struct TextPathItem {
    pub path: Path,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<TextPathStroke>,
    pub byte_range: Range<usize>,
    pub kind: TextPathKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathImageFormat {
    Png,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathImageItem {
    pub data: Vec<u8>,
    pub format: TextPathImageFormat,
    pub width: f32,
    pub height: f32,
    pub transform: [f32; 6],
    pub byte_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlainTextPathRun {
    pub text: String,
    pub byte_range: Range<usize>,
    /// Resolved faces used to shape this run. Resource IDs are local to the run.
    pub font_resources: Vec<avenger_typst_label::FontResource>,
    /// Resolved foreground color.
    pub color: [f32; 4],
    /// Whether the run uses right-to-left text direction.
    pub is_rtl: bool,
    pub font: String,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub x: f32,
    pub y_offset: f32,
    pub bounds: TextBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathDrawItem {
    PlainRun(usize),
    PathItem(usize),
    ImageItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    /// When set, the consumer must clip every draw item to x <= this cutoff in
    /// label coordinates, before applying the label's placement transform.
    pub clip_width: Option<f32>,
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub images: Vec<TextPathImageItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    pub draw_items: Vec<TextPathDrawItem>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            clip_width: None,
            bounds,
            items: Vec::new(),
            images: Vec::new(),
            plain_runs: Vec::new(),
            draw_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextPathExtractorImpl {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
}

impl TextPathExtractorImpl {
    pub(crate) fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
        Self { typst, math }
    }

    fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub(crate) fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        let math = self.math.with_syntax_mode(config.syntax_mode);
        let text = crate::measurement::prepare_text_to_limit_with(
            config.text,
            config.syntax_mode,
            config.limit,
            |candidate| {
                let measurement = TextMeasurementConfig {
                    text: candidate,
                    font: config.font,
                    font_size: config.font_size,
                    font_weight: config.font_weight,
                    font_style: config.font_style,
                    syntax_mode: config.syntax_mode,
                    params: config.params,
                    number_locale: config.number_locale,
                    number_locale_specs: config.number_locale_specs,
                    datetime_locale: config.datetime_locale,
                    datetime_timezone: config.datetime_timezone,
                    datetime_locale_specs: config.datetime_locale_specs,
                };
                self.measure_text_bounds(&measurement)
                    .map(|bounds| bounds.width)
            },
        )?;
        let result = typeset_line(
            &self.typst,
            &math,
            &text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            config.params,
            config.number_locale,
            config.number_locale_specs,
            config.datetime_locale,
            config.datetime_timezone,
            config.datetime_locale_specs,
        )?;
        let tight_bounds = tight_bounds_from_metrics(result.label.metrics);
        let mut bounds = bounds_from_metrics(
            result.label.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let clip_width =
            crate::measurement::apply_text_limit(&mut bounds, config.syntax_mode, config.limit);
        let y_offset = bounds.ascent - tight_bounds.ascent;
        let mut output = TextPathBuffer::new(bounds.clone());
        output.clip_width = clip_width;
        let svg = avenger_typst_label::svg_items(
            &result.label,
            &avenger_typst_label::SvgOptions::default(),
        )?;

        // Native SVG text cannot reproduce arbitrary OpenType substitutions in every viewer.
        // Retain the already shaped outlines for those runs, including subscripts and small caps.
        let outlined_ranges: Vec<_> = svg
            .items
            .iter()
            .filter_map(|(_, item)| match item {
                avenger_typst_label::LabelFrameItem::Text(text)
                    if !text.font_features.is_empty()
                        || text.font_resources.iter().any(|r| !r.variations.is_empty()) =>
                {
                    Some(text.byte_range.clone())
                }
                _ => None,
            })
            .collect();

        for (point, item) in svg.items {
            match item {
                avenger_typst_label::LabelFrameItem::Text(text) => {
                    if text.kind != avenger_typst_label::TextItemKind::Plain
                        || !text.font_features.is_empty()
                        || text.font_resources.iter().any(|r| !r.variations.is_empty())
                    {
                        continue;
                    }
                    let run_font = text
                        .style
                        .as_ref()
                        .map(|style| style.font_family.clone())
                        .unwrap_or_else(|| config.font.to_string());
                    let run_bounds = tight_bounds_from_metrics(text.metrics);
                    // A frame item can carry the whole label's font table. Keep only
                    // faces referenced by this run, in glyph order for fallback selection.
                    let mut font_resources = Vec::new();
                    if let Some(pdf) = &text.pdf_text {
                        for glyph_run in &pdf.glyph_runs {
                            if font_resources.iter().any(
                                |resource: &avenger_typst_label::FontResource| {
                                    resource.id == glyph_run.font
                                },
                            ) {
                                continue;
                            }
                            if let Some(resource) = text
                                .font_resources
                                .iter()
                                .find(|resource| resource.id == glyph_run.font)
                            {
                                font_resources.push(resource.clone());
                            }
                        }
                    }
                    let run_index = output.plain_runs.len();
                    output.plain_runs.push(PlainTextPathRun {
                        text: text.text,
                        byte_range: text.byte_range,
                        font_resources,
                        color: text
                            .style
                            .as_ref()
                            .map(|style| rgba_from_typst_color(style.fill))
                            .unwrap_or(config.color),
                        is_rtl: text.is_rtl,
                        font: run_font,
                        font_size: text
                            .style
                            .as_ref()
                            .map(|style| style.font_size)
                            .unwrap_or(config.font_size),
                        font_weight: text
                            .style
                            .as_ref()
                            .map(|style| typst_font_weight(&style.font_weight))
                            .unwrap_or(config.font_weight),
                        font_style: text
                            .style
                            .as_ref()
                            .map(|style| typst_font_style(style.font_style))
                            .unwrap_or(config.font_style),
                        x: point.x,
                        y_offset: y_offset + point.y - run_bounds.ascent,
                        bounds: run_bounds,
                    });
                    output
                        .draw_items
                        .push(TextPathDrawItem::PlainRun(run_index));
                }
                avenger_typst_label::LabelFrameItem::Shape(shape) => {
                    if shape.text_kind == Some(avenger_typst_label::TextItemKind::Plain)
                        && matches!(
                            shape.item.kind,
                            avenger_typst_label::PathKind::GlyphOutline { .. }
                        )
                        && !outlined_ranges.contains(&shape.byte_range)
                    {
                        continue;
                    }
                    let path_index = output.items.len();
                    let mut path_item = typst_path_item_to_text_path_item(
                        shape.item,
                        shape.byte_range,
                        0.0,
                        y_offset,
                    );
                    if shape.text_kind == Some(avenger_typst_label::TextItemKind::Plain)
                        && path_item.kind == TextPathKind::MathGlyph
                    {
                        path_item.kind = TextPathKind::PlainGlyph;
                    }
                    output.items.push(path_item);
                    output
                        .draw_items
                        .push(TextPathDrawItem::PathItem(path_index));
                }
                avenger_typst_label::LabelFrameItem::Image(image) => {
                    let image_index = output.images.len();
                    output.images.push(typst_image_item_to_text_path_image_item(
                        image.image,
                        image.byte_range,
                        0.0,
                        y_offset,
                    ));
                    output
                        .draw_items
                        .push(TextPathDrawItem::ImageItem(image_index));
                }
                avenger_typst_label::LabelFrameItem::Group(_) => {
                    return Err(AvengerTextError::InternalError(
                        "Typst grouped label frame items are not supported in SVG extraction yet"
                            .to_string(),
                    ));
                }
            }
        }

        Ok(output)
    }
}

pub(crate) fn typst_path_item_to_text_path_item(
    item: avenger_typst_label::PathItem,
    byte_range: Range<usize>,
    x_offset: f32,
    y_offset: f32,
) -> TextPathItem {
    let kind = match item.kind {
        avenger_typst_label::PathKind::GlyphOutline { .. } => TextPathKind::MathGlyph,
        avenger_typst_label::PathKind::MathShape => TextPathKind::MathShape,
    };
    TextPathItem {
        path: math_path_data_to_lyon_path(&item.path, item.transform, x_offset, y_offset),
        fill: item.fill.map(rgba_from_typst_color),
        stroke: item.stroke.map(|stroke| TextPathStroke {
            color: rgba_from_typst_color(stroke.color),
            width: stroke.width,
            line_cap: text_path_stroke_cap(stroke.line_cap),
            line_join: text_path_stroke_join(stroke.line_join),
            dash: stroke.dash.map(|dash| TextPathDashPattern {
                array: dash.array,
                phase: dash.phase,
            }),
            miter_limit: stroke.miter_limit,
        }),
        byte_range,
        kind,
    }
}

fn text_path_stroke_cap(cap: avenger_typst_label::LineCap) -> TextPathLineCap {
    match cap {
        avenger_typst_label::LineCap::Butt => TextPathLineCap::Butt,
        avenger_typst_label::LineCap::Round => TextPathLineCap::Round,
        avenger_typst_label::LineCap::Square => TextPathLineCap::Square,
    }
}

fn text_path_stroke_join(join: avenger_typst_label::LineJoin) -> TextPathLineJoin {
    match join {
        avenger_typst_label::LineJoin::Bevel => TextPathLineJoin::Bevel,
        avenger_typst_label::LineJoin::Miter => TextPathLineJoin::Miter,
        avenger_typst_label::LineJoin::Round => TextPathLineJoin::Round,
    }
}

fn typst_image_item_to_text_path_image_item(
    image: avenger_typst_label::PathImageItem,
    byte_range: Range<usize>,
    x_offset: f32,
    y_offset: f32,
) -> TextPathImageItem {
    let format = match image.format {
        avenger_typst_label::PathImageFormat::Png => TextPathImageFormat::Png,
    };
    TextPathImageItem {
        data: image.data,
        format,
        width: image.width,
        height: image.height,
        transform: [
            image.transform.sx,
            image.transform.ky,
            image.transform.kx,
            image.transform.sy,
            image.transform.tx + x_offset,
            image.transform.ty + y_offset,
        ],
        byte_range,
    }
}

fn math_path_data_to_lyon_path(
    path: &avenger_typst_label::PathData,
    transform: avenger_typst_label::Transform,
    x_offset: f32,
    y_offset: f32,
) -> Path {
    let mut builder = Path::builder();
    for command in &path.commands {
        match *command {
            avenger_typst_label::PathCommand::MoveTo { x, y } => {
                builder.begin(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst_label::PathCommand::LineTo { x, y } => {
                builder.line_to(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst_label::PathCommand::QuadTo { x1, y1, x, y } => {
                builder.quadratic_bezier_to(
                    transform_math_point(transform, x1, y1, x_offset, y_offset),
                    transform_math_point(transform, x, y, x_offset, y_offset),
                );
            }
            avenger_typst_label::PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                builder.cubic_bezier_to(
                    transform_math_point(transform, x1, y1, x_offset, y_offset),
                    transform_math_point(transform, x2, y2, x_offset, y_offset),
                    transform_math_point(transform, x, y, x_offset, y_offset),
                );
            }
            avenger_typst_label::PathCommand::Close => builder.close(),
        }
    }
    builder.build()
}

fn transform_math_point(
    transform: avenger_typst_label::Transform,
    x: f32,
    y: f32,
    x_offset: f32,
    y_offset: f32,
) -> lyon_path::math::Point {
    point(
        x_offset + transform.sx * x + transform.kx * y + transform.tx,
        y_offset + transform.ky * x + transform.sy * y + transform.ty,
    )
}

pub(crate) fn rgba_from_typst_color(color: avenger_typst_label::Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn typst_font_weight(weight: &avenger_typst_label::FontWeight) -> FontWeight {
    match weight {
        avenger_typst_label::FontWeight::Normal => {
            FontWeight::Name(crate::types::FontWeightNameSpec::Normal)
        }
        avenger_typst_label::FontWeight::Bold => {
            FontWeight::Name(crate::types::FontWeightNameSpec::Bold)
        }
        avenger_typst_label::FontWeight::Number(value) => FontWeight::Number(*value as f32),
    }
}

fn typst_font_style(style: avenger_typst_label::FontStyle) -> FontStyle {
    match style {
        avenger_typst_label::FontStyle::Normal => FontStyle::Normal,
        avenger_typst_label::FontStyle::Italic | avenger_typst_label::FontStyle::Oblique => {
            FontStyle::Italic
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

    fn config(text: &str) -> TextPathExtractionConfig<'_> {
        static COLOR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
        static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
        static STYLE: FontStyle = FontStyle::Normal;

        TextPathExtractionConfig {
            text,
            color: COLOR,
            font: "",
            font_size: 10.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            params: crate::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }

    fn math_config() -> crate::math::TextMarkupConfig {
        crate::math::TextMarkupConfig::default()
    }

    fn typst() -> avenger_typst_label::LabelEngine {
        avenger_typst_label::LabelEngine::new(Default::default()).unwrap()
    }

    fn lato_runs(source: &str) -> Vec<(avenger_typst_label::Point, avenger_typst_label::TextItem)> {
        use avenger_typst_label::{
            EngineOptions, FontOptions, LabelEngine, LabelFrameItem, LabelOptions,
        };
        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                registered_fonts: crate::fonts::registered_default_fonts(),
                ..Default::default()
            },
        })
        .unwrap();
        let mut options = LabelOptions::default();
        options.text.font_family = "Lato".into();
        options.text.font_size = 40.0;
        engine
            .compile(source, &options)
            .unwrap()
            .frame
            .items
            .into_iter()
            .filter_map(|(point, item)| match item {
                LabelFrameItem::Text(text) => Some((point, text)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typographic_scripts_keep_parent_size_and_baseline() {
        for (function, tag) in [("sub", *b"subs"), ("super", *b"sups")] {
            // Explicit size and baseline affect only synthesized scripts.
            for arguments in ["", "(size: 0.25em, baseline: 0.8em)"] {
                let runs = lato_runs(&format!("H#{function}{arguments}[2]O"));
                assert_eq!(runs.len(), 3);
                let (parent_position, parent) = &runs[0];
                let (script_position, script) = &runs[1];
                assert_eq!(script.text, "2");
                assert_eq!(script.style.as_ref().unwrap().font_size, 40.0);
                assert_eq!(script_position.y, parent_position.y);
                assert_eq!(
                    script.font_features,
                    vec![avenger_typst_label::FontFeature { tag, value: 1 }]
                );
                assert!(script.metrics.width < parent.metrics.width);
            }
        }
    }

    #[test]
    fn incomplete_script_features_synthesize_the_entire_run() {
        // Lato provides script digits but no script at sign.
        for function in ["sub", "super"] {
            let runs = lato_runs(&format!("H#{function}(size: 0.5em)[2@]O"));
            let (_, script) = runs.iter().find(|(_, run)| run.text == "2@").unwrap();
            assert_eq!(script.style.as_ref().unwrap().font_size, 20.0);
            assert!(script.font_features.is_empty());
            assert_eq!(script.font_resources[0].family, "Lato");
        }
    }

    #[test]
    fn svg_extraction_outlines_feature_runs_and_retains_ordinary_text() {
        let engine = crate::TextEngine::with_font_resolution(&crate::FontResolutionOptions {
            load_system_fonts: false,
            ..crate::default_font_resolution()
        })
        .unwrap();
        for source in [
            "H#sub[2]O",
            "H#super[2]O",
            "#smallcaps[Smallcaps]",
            "H#smallcaps(all: true)[CAPS]O",
        ] {
            let buffer = engine
                .extract_paths(&TextPathExtractionConfig {
                    font: "Lato",
                    font_size: 40.0,
                    ..config(source)
                })
                .unwrap();
            assert!(!buffer.items.is_empty(), "{source}");
            assert!(buffer
                .items
                .iter()
                .all(|item| item.kind == TextPathKind::PlainGlyph));
            let native: String = buffer
                .plain_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(native, if source.starts_with('H') { "HO" } else { "" });
        }
    }

    #[test]
    fn plain_runs_only_include_their_resolved_faces() {
        let engine = crate::TextEngine::with_font_resolution(&crate::FontResolutionOptions {
            load_system_fonts: false,
            ..crate::default_font_resolution()
        })
        .unwrap();
        let buffer = engine
            .extract_paths(&TextPathExtractionConfig {
                font: "Lato",
                ..config("Regular _Italic_ *Bold* $sqrt(x)$")
            })
            .unwrap();
        for run in &buffer.plain_runs {
            assert_eq!(run.font_resources.len(), 1, "{}", run.text);
            let resource = &run.font_resources[0];
            let face = ttf_parser::Face::parse(&resource.data, resource.face_index).unwrap();
            assert_eq!(face.is_italic(), run.font_style == FontStyle::Italic);
            if run.text.contains("Bold") {
                assert_eq!(face.weight().to_number(), 700);
            }
        }
    }

    #[test]
    fn text_line_extractor_returns_plain_runs_and_math_paths() {
        let typst = typst();
        let extractor = TextPathExtractorImpl::new(typst, math_config());
        let text = "speed $v^2$".to_string();
        let buffer = extractor.extract_text_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, "speed ");
        assert!(!buffer.items.is_empty());
        assert_eq!(buffer.items[0].kind, TextPathKind::MathGlyph);
        assert!(matches!(
            buffer.draw_items.first(),
            Some(TextPathDrawItem::PlainRun(0))
        ));
        assert!(buffer
            .draw_items
            .iter()
            .skip(1)
            .all(|item| matches!(item, TextPathDrawItem::PathItem(_))));
    }

    #[test]
    fn text_line_extractor_returns_plain_runs_and_static_decoration_paths() {
        let typst = typst();
        let extractor = TextPathExtractorImpl::new(typst, math_config());
        let text = "#underline[important]".to_string();
        let buffer = extractor.extract_text_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, "important");
        assert_eq!(buffer.items.len(), 1);
        assert_eq!(buffer.items[0].kind, TextPathKind::MathShape);
        assert!(buffer.items[0].stroke.is_some());
        assert!(matches!(
            buffer.draw_items.as_slice(),
            [TextPathDrawItem::PlainRun(0), TextPathDrawItem::PathItem(0)]
        ));
    }

    #[test]
    fn text_line_extractor_preserves_decoration_dash_phase_and_miter_limit() {
        let typst = typst();
        let extractor = TextPathExtractorImpl::new(typst, math_config());
        let text = "#underline(stroke: (thickness: 1pt, dash: (array: (2pt, 1pt), phase: 0.5pt), miter-limit: 2))[important]".to_string();
        let buffer = extractor.extract_text_paths(&config(&text)).unwrap();
        let stroke = buffer.items[0]
            .stroke
            .as_ref()
            .expect("decoration stroke should exist");
        let dash = stroke.dash.as_ref().expect("dash should resolve");

        assert_eq!(dash.array.as_slice(), [2.0, 1.0].as_slice());
        assert_eq!(dash.phase, 0.5);
        assert_eq!(stroke.miter_limit, 2.0);
    }

    #[test]
    fn text_line_extractor_returns_synthesized_script_runs_with_smaller_style() {
        let typst = typst();
        let extractor = TextPathExtractorImpl::new(typst, math_config());
        let text = "H#sub(typographic: false)[2]O #super(typographic: false)[\\*]".to_string();
        let buffer = extractor.extract_text_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 4);
        assert_eq!(buffer.plain_runs[0].text, "H");
        assert_eq!(buffer.plain_runs[1].text, "2");
        assert_eq!(buffer.plain_runs[2].text, "O ");
        assert_eq!(buffer.plain_runs[3].text, "*");
        assert!(buffer.plain_runs[1].font_size < buffer.plain_runs[0].font_size);
        assert!(buffer.plain_runs[3].font_size < buffer.plain_runs[0].font_size);
        assert!(buffer.plain_runs[1].y_offset > buffer.plain_runs[0].y_offset);
        assert!(buffer.plain_runs[3].y_offset < buffer.plain_runs[0].y_offset);
    }
}
