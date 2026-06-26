use std::ops::Range;

use lyon_path::Path;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight},
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
    pub limit: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathStroke {
    pub color: [f32; 4],
    pub width: f32,
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

#[derive(Debug, Clone, PartialEq)]
pub struct PlainTextPathRun {
    pub text: String,
    pub byte_range: Range<usize>,
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
}

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    pub draw_items: Vec<TextPathDrawItem>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            bounds,
            items: Vec::new(),
            plain_runs: Vec::new(),
            draw_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextPathExtractorImpl {
    typst: avenger_typst::AvengerTypst,
    math: TextMarkupConfig,
}

impl TextPathExtractorImpl {
    pub(crate) fn new(typst: avenger_typst::AvengerTypst, math: TextMarkupConfig) -> Self {
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
        let text = crate::measurement::truncate_text_to_limit_with(
            config.text,
            config.limit,
            |candidate| {
                let measurement = TextMeasurementConfig {
                    text: candidate,
                    font: config.font,
                    font_size: config.font_size,
                    font_weight: config.font_weight,
                    font_style: config.font_style,
                };
                self.measure_text_bounds(&measurement)
                    .map(|bounds| bounds.width)
            },
        )?;
        let result = typeset_line(
            &self.typst,
            &self.math,
            &text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            avenger_typst::TextLineOutputRequest {
                paths: false,
                raster: None,
                pdf_text_layer: false,
                positioned_runs: true,
            },
        )?;
        let tight_bounds = tight_bounds_from_metrics(result.artifact.metrics);
        let bounds = bounds_from_metrics(
            result.artifact.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let y_offset = bounds.ascent - tight_bounds.ascent;
        let mut output = TextPathBuffer::new(bounds.clone());

        for run in result.artifact.positioned_runs {
            match run.kind {
                avenger_typst::PositionedTextLineRunKind::Plain => {
                    let mut after_text_items = Vec::new();
                    if let Some(paths) = run.paths {
                        for item in paths.items {
                            let text_item = typst_path_item_to_text_path_item(
                                item,
                                run.byte_range.clone(),
                                0.0,
                                y_offset,
                            );
                            if text_item.fill.is_some() && text_item.stroke.is_none() {
                                let path_index = output.items.len();
                                output.items.push(text_item);
                                output
                                    .draw_items
                                    .push(TextPathDrawItem::PathItem(path_index));
                            } else {
                                after_text_items.push(text_item);
                            }
                        }
                    }

                    let run_bounds = tight_bounds_from_metrics(run.metrics);
                    let run_index = output.plain_runs.len();
                    output.plain_runs.push(PlainTextPathRun {
                        text: run.text,
                        byte_range: run.byte_range,
                        font: run
                            .text_style
                            .as_ref()
                            .map(|style| style.font_family.clone())
                            .unwrap_or_else(|| config.font.to_string()),
                        font_size: run
                            .text_style
                            .as_ref()
                            .map(|style| style.font_size)
                            .unwrap_or(config.font_size),
                        font_weight: run
                            .text_style
                            .as_ref()
                            .map(|style| typst_font_weight(&style.font_weight))
                            .unwrap_or(config.font_weight),
                        font_style: run
                            .text_style
                            .as_ref()
                            .map(|style| typst_font_style(style.font_style))
                            .unwrap_or(config.font_style),
                        x: run.x,
                        y_offset: y_offset + run.y - run_bounds.ascent,
                        bounds: run_bounds,
                    });
                    output
                        .draw_items
                        .push(TextPathDrawItem::PlainRun(run_index));
                    for item in after_text_items {
                        let path_index = output.items.len();
                        output.items.push(item);
                        output
                            .draw_items
                            .push(TextPathDrawItem::PathItem(path_index));
                    }
                }
                avenger_typst::PositionedTextLineRunKind::Math => {
                    let paths = run.paths.ok_or_else(|| {
                        AvengerTextError::InternalError(
                            "Typst positioned math path output was requested but missing"
                                .to_string(),
                        )
                    })?;
                    for item in paths.items {
                        let path_index = output.items.len();
                        output.items.push(typst_path_item_to_text_path_item(
                            item,
                            run.byte_range.clone(),
                            0.0,
                            y_offset,
                        ));
                        output
                            .draw_items
                            .push(TextPathDrawItem::PathItem(path_index));
                    }
                }
            }
        }

        Ok(output)
    }
}

fn typst_path_item_to_text_path_item(
    item: avenger_typst::MathPathItem,
    byte_range: Range<usize>,
    x_offset: f32,
    y_offset: f32,
) -> TextPathItem {
    let kind = match item.kind {
        avenger_typst::MathPathKind::GlyphOutline { .. } => TextPathKind::MathGlyph,
        avenger_typst::MathPathKind::MathShape => TextPathKind::MathShape,
    };
    TextPathItem {
        path: math_path_data_to_lyon_path(&item.path, item.transform, x_offset, y_offset),
        fill: item.fill.map(rgba_from_typst_color),
        stroke: item.stroke.map(|stroke| TextPathStroke {
            color: rgba_from_typst_color(stroke.color),
            width: stroke.width,
        }),
        byte_range,
        kind,
    }
}

fn math_path_data_to_lyon_path(
    path: &avenger_typst::MathPathData,
    transform: avenger_typst::MathTransform,
    x_offset: f32,
    y_offset: f32,
) -> Path {
    let mut builder = Path::builder();
    for command in &path.commands {
        match *command {
            avenger_typst::MathPathCommand::MoveTo { x, y } => {
                builder.begin(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst::MathPathCommand::LineTo { x, y } => {
                builder.line_to(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst::MathPathCommand::QuadTo { x1, y1, x, y } => {
                builder.quadratic_bezier_to(
                    transform_math_point(transform, x1, y1, x_offset, y_offset),
                    transform_math_point(transform, x, y, x_offset, y_offset),
                );
            }
            avenger_typst::MathPathCommand::CubicTo {
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
            avenger_typst::MathPathCommand::Close => builder.close(),
        }
    }
    builder.build()
}

fn transform_math_point(
    transform: avenger_typst::MathTransform,
    x: f32,
    y: f32,
    x_offset: f32,
    y_offset: f32,
) -> lyon_path::math::Point {
    point(
        x_offset + transform.xx * x + transform.xy * y + transform.dx,
        y_offset + transform.yx * x + transform.yy * y + transform.dy,
    )
}

fn rgba_from_typst_color(color: avenger_typst::Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn typst_font_weight(weight: &avenger_typst::FontWeight) -> FontWeight {
    match weight {
        avenger_typst::FontWeight::Normal => {
            FontWeight::Name(crate::types::FontWeightNameSpec::Normal)
        }
        avenger_typst::FontWeight::Bold => FontWeight::Name(crate::types::FontWeightNameSpec::Bold),
        avenger_typst::FontWeight::Number(value) => FontWeight::Number(*value as f32),
    }
}

fn typst_font_style(style: avenger_typst::FontStyle) -> FontStyle {
    match style {
        avenger_typst::FontStyle::Normal => FontStyle::Normal,
        avenger_typst::FontStyle::Italic | avenger_typst::FontStyle::Oblique => FontStyle::Italic,
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
        }
    }

    fn math_config() -> crate::math::TextMarkupConfig {
        crate::math::TextMarkupConfig::default()
    }

    fn typst() -> avenger_typst::AvengerTypst {
        avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig::default()).unwrap()
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
    fn text_line_extractor_returns_script_runs_with_smaller_style() {
        let typst = typst();
        let extractor = TextPathExtractorImpl::new(typst, math_config());
        let text = "H#sub[2]O #super[*]".to_string();
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
