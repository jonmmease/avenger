use std::{collections::HashMap, path::Path, sync::Arc};

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::Clip,
        image::SceneImageMark,
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::{PatternFill, PatternLayerOperation, PatternReferenceFrame},
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        text_leader::{
            compute_text_leader_geometry, TextLeaderArrowhead, TextLeaderGeometry,
            TextLeaderGeometryInput, TextLeaderPath,
        },
        trail::SceneTrailMark,
    },
    pattern_geometry::{
        build_layered_pattern_geometry, LayeredPatternGeometry, PatternGeometryError, PatternRect,
        PatternRenderContext,
    },
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_text::{
    path::{TextPathItem, TextPathKind, TextPathLineCap, TextPathLineJoin},
    pdf::{TextPdfBuffer, TextPdfDrawItem, TextPdfExtractionConfig},
    types::{TextAlign, TextBaseline},
    TextEngine,
};
use avenger_typst_label::{FontResource, FontResourceId, PdfGlyphRun};
use itertools::izip;
use krilla::{
    color::rgb,
    geom::{PathBuilder, Point, Rect, Size, Transform},
    image::{BitsPerComponent, CustomImage, Image, ImageColorspace},
    mask::{Mask, MaskType},
    num::NormalizedF32,
    page::PageSettings,
    paint::{
        Fill, FillRule, LineCap, LineJoin, LinearGradient, Paint, RadialGradient, SpreadMethod,
        Stop, Stroke, StrokeDash,
    },
    stream::Stream,
    surface::Surface,
    text::{Font as KrillaFont, GlyphId, KrillaGlyph},
    Data, Document, SerializeSettings,
};
use lyon_algorithms::aabb::bounding_box;
use lyon_path::{Event, Path as LyonPath};

use crate::{
    error::AvengerPdfError,
    options::{PdfBackground, PdfRenderOptions},
};

/// Export scene graphs as PDF documents with embedded fonts and selectable text.
#[derive(Debug, Clone, Default)]
pub struct PdfRenderer {
    options: PdfRenderOptions,
    text_engine: Option<TextEngine>,
}

impl PdfRenderer {
    /// Create a renderer with bundled fonts and a white background.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set output options. A supplied text engine takes precedence over font options.
    pub fn with_options(mut self, options: PdfRenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Use the same engine and resolved fonts as scene layout.
    pub fn with_text_engine(mut self, text_engine: TextEngine) -> Self {
        self.text_engine = Some(text_engine);
        self
    }

    /// Export a single PDF page. Image resources must be resolved before export.
    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<Vec<u8>, AvengerPdfError> {
        let (width, height) = (scene_graph.width, scene_graph.height);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(AvengerPdfError::InvalidPageSize { width, height });
        }
        let page_settings = PageSettings::from_wh(width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;

        let settings = SerializeSettings {
            compress_content_streams: self.options.compress,
            ..Default::default()
        };

        let mut document = Document::new_with(settings);
        let mut page = document.start_page_with(page_settings);
        let mut surface = page.surface();
        let text_engine = match &self.text_engine {
            Some(engine) => engine.clone(),
            None => {
                TextEngine::with_font_resolution(&self.options.font_resolution).map_err(|err| {
                    AvengerPdfError::TextBuffer(format!("failed to initialize text engine: {err}"))
                })?
            }
        };
        let mut font_cache = PdfFontCache::default();
        self.draw_background(&mut surface, width, height)?;
        self.draw_scene_graph(&mut surface, scene_graph, &text_engine, &mut font_cache)?;
        surface.finish();
        page.finish();

        Ok(document.finish()?)
    }

    /// Render and write a PDF file, creating parent directories when needed.
    pub fn write_scene_graph_pdf<P: AsRef<Path>>(
        &self,
        scene_graph: &SceneGraph,
        output: P,
    ) -> Result<(), AvengerPdfError> {
        let bytes = self.render_scene_graph(scene_graph)?;
        let output = output.as_ref();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, bytes)?;
        Ok(())
    }

    fn draw_background(
        &self,
        surface: &mut Surface<'_>,
        width: f32,
        height: f32,
    ) -> Result<(), AvengerPdfError> {
        let color = match self.options.background {
            PdfBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            PdfBackground::Transparent => None,
            PdfBackground::Color(color) => Some(color),
        };
        let Some(color) = color else {
            return Ok(());
        };
        let [r, g, b, a] = color.map(|value| value.clamp(0.0, 1.0));
        if a <= 0.0 {
            return Ok(());
        }

        let rect = Rect::from_xywh(0.0, 0.0, width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;
        let mut builder = PathBuilder::new();
        builder.push_rect(rect);
        let Some(path) = builder.finish() else {
            return Ok(());
        };

        surface.set_fill(Some(Fill {
            paint: rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
            opacity: NormalizedF32::new(a).unwrap_or(NormalizedF32::ONE),
            rule: FillRule::NonZero,
        }));
        surface.set_stroke(None);
        surface.draw_path(&path);
        Ok(())
    }

    fn draw_scene_graph(
        &self,
        surface: &mut Surface<'_>,
        scene_graph: &SceneGraph,
        text_engine: &TextEngine,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        let chart_bounds = PatternRect::new(0.0, 0.0, scene_graph.width, scene_graph.height);

        for item in display_list.ordered_items() {
            let clip_path = clip_to_krilla_path(&item.clip)?;
            if !matches!(item.clip, Clip::None) && clip_path.is_none() {
                continue;
            }
            if let Some(path) = clip_path.as_ref() {
                surface.push_clip_path(path, &FillRule::NonZero);
            }

            let result = match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => self.draw_path_mark(
                    surface,
                    mark,
                    item.origin,
                    item.pattern_reference_frame.as_ref(),
                    chart_bounds,
                ),
                SceneDisplayMark::Borrowed(mark) => self.draw_scene_mark(
                    surface,
                    mark,
                    item.origin,
                    item.pattern_reference_frame.as_ref(),
                    chart_bounds,
                    text_engine,
                    font_cache,
                ),
            };

            if clip_path.is_some() {
                surface.pop();
            }

            result?;
        }

        Ok(())
    }

    // Keep the display item geometry and shared text context together.
    #[allow(clippy::too_many_arguments)]
    fn draw_scene_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
        text_engine: &TextEngine,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        match mark {
            SceneMark::Rect(mark) => {
                self.draw_rect_mark(surface, mark, origin, pattern_reference_frame, chart_bounds)
            }
            SceneMark::Path(mark) => {
                self.draw_path_mark(surface, mark, origin, pattern_reference_frame, chart_bounds)
            }
            SceneMark::Rule(mark) => self.draw_rule_mark(surface, mark, origin),
            SceneMark::Line(mark) => self.draw_line_mark(surface, mark, origin),
            SceneMark::Area(mark) => {
                self.draw_area_mark(surface, mark, origin, pattern_reference_frame, chart_bounds)
            }
            SceneMark::Symbol(mark) => {
                self.draw_symbol_mark(surface, mark, origin, pattern_reference_frame, chart_bounds)
            }
            SceneMark::Arc(mark) => {
                self.draw_arc_mark(surface, mark, origin, pattern_reference_frame, chart_bounds)
            }
            SceneMark::Trail(mark) => self.draw_trail_mark(surface, mark, origin),
            SceneMark::Image(mark) => self.draw_image_mark(surface, mark, origin),
            SceneMark::WarpedImage(mark) => self.draw_warped_image_mark(surface, mark, origin),
            SceneMark::Text(mark) => {
                self.draw_text_mark(surface, mark, origin, chart_bounds, text_engine, font_cache)
            }
            SceneMark::Group(_) => Ok(()),
        }
    }

    fn draw_rect_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneRectMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, fill_pattern, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.draw_filled_path_with_optional_pattern(
                surface,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    stroke_dash_offset: 0.0,
                    stroke_miter_limit: None,
                    gradients: &mark.gradients,
                },
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn draw_path_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &ScenePathMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, fill_pattern, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter()
        ) {
            self.draw_filled_path_with_optional_pattern(
                surface,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: Some(mark.stroke_cap),
                    stroke_join: Some(mark.stroke_join),
                    stroke_dash: None,
                    stroke_dash_offset: 0.0,
                    stroke_miter_limit: None,
                    gradients: &mark.gradients,
                },
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn draw_symbol_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, fill_pattern, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter()
        ) {
            self.draw_filled_path_with_optional_pattern(
                surface,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    stroke_dash_offset: 0.0,
                    stroke_miter_limit: None,
                    gradients: &mark.gradients,
                },
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn draw_arc_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneArcMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, fill_pattern, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.draw_filled_path_with_optional_pattern(
                surface,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    stroke_dash_offset: 0.0,
                    stroke_miter_limit: None,
                    gradients: &mark.gradients,
                },
                pattern_reference_frame,
                chart_bounds,
            )?;
        }

        Ok(())
    }

    fn draw_area_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneAreaMark,
        origin: [f32; 2],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        self.draw_filled_path_with_optional_pattern(
            surface,
            &mark.transformed_path(origin),
            &mark.fill,
            mark.fill_pattern.as_ref(),
            PathStyle {
                fill: None,
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                stroke_dash_offset: 0.0,
                stroke_miter_limit: None,
                gradients: &mark.gradients,
            },
            pattern_reference_frame,
            chart_bounds,
        )
    }

    fn draw_line_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneLineMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        self.draw_path_with_style(
            surface,
            &line_mark_path(mark, origin),
            PathStyle {
                fill: None,
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                stroke_dash_offset: 0.0,
                stroke_miter_limit: None,
                gradients: &mark.gradients,
            },
        )
    }

    fn draw_rule_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneRuleMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        let stroke_dashes = mark
            .stroke_dash_iter()
            .map(|iter| iter.collect::<Vec<_>>())
            .unwrap_or_default();

        for (index, (x1, y1, x2, y2, stroke, stroke_width, stroke_cap)) in izip!(
            mark.x_iter(),
            mark.y_iter(),
            mark.x2_iter(),
            mark.y2_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter(),
            mark.stroke_cap_iter(),
        )
        .enumerate()
        {
            let mut builder = LyonPath::builder();
            builder.begin(lyon_path::math::point(*x1 + origin[0], *y1 + origin[1]));
            builder.line_to(lyon_path::math::point(*x2 + origin[0], *y2 + origin[1]));
            builder.end(false);
            let path = builder.build();
            self.draw_path_with_style(
                surface,
                &path,
                PathStyle {
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: Some(*stroke_cap),
                    stroke_join: None,
                    stroke_dash: stroke_dashes.get(index).map(|dash| dash.as_slice()),
                    stroke_dash_offset: 0.0,
                    stroke_miter_limit: None,
                    gradients: &mark.gradients,
                },
            )?;
        }

        Ok(())
    }

    fn draw_trail_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneTrailMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        self.draw_path_with_style(
            surface,
            &trail_outline_path(mark, origin),
            PathStyle {
                fill: Some(&mark.stroke),
                stroke: None,
                stroke_width: None,
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
                stroke_dash_offset: 0.0,
                stroke_miter_limit: None,
                gradients: &mark.gradients,
            },
        )
    }

    fn draw_text_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneTextMark,
        origin: [f32; 2],
        chart_bounds: PatternRect,
        text_engine: &TextEngine,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        let leader_stroke_dash_values = mark
            .leader_stroke_dash
            .as_ref()
            .map(|dash| dash.as_vec(mark.len as usize, mark.indices.as_ref()));

        for (
            index,
            (
                text,
                target,
                label,
                defined,
                align,
                baseline,
                angle,
                color,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
                leader,
                leader_stroke,
                leader_stroke_width,
                leader_stroke_cap,
                leader_stroke_join,
                leader_label_padding,
                leader_target_radius,
                leader_min_length,
                leader_shape,
                leader_arrow,
                leader_arrow_length,
                leader_arrow_width,
            ),
        ) in izip!(
            mark.text_iter(),
            mark.target_position_iter(),
            mark.label_position_iter(),
            mark.defined_iter(),
            mark.align_iter(),
            mark.baseline_iter(),
            mark.angle_iter(),
            mark.color_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
            mark.leader_iter(),
            mark.leader_stroke_iter(),
            mark.leader_stroke_width_iter(),
            mark.leader_stroke_cap_iter(),
            mark.leader_stroke_join_iter(),
            mark.leader_label_padding_iter(),
            mark.leader_target_radius_iter(),
            mark.leader_min_length_iter(),
            mark.leader_shape_iter(),
            mark.leader_arrow_iter(),
            mark.leader_arrow_length_iter(),
            mark.leader_arrow_width_iter(),
        )
        .enumerate()
        {
            if !*defined {
                continue;
            }

            let ColorOrGradient::Color(text_color) = color else {
                return Err(AvengerPdfError::UnsupportedFeature(
                    "gradient text paint is not supported by avenger-pdf".to_string(),
                ));
            };
            let target = [target[0] + origin[0], target[1] + origin[1]];
            let label = [label[0] + origin[0], label[1] + origin[1]];
            let buffer = text_engine.extract_pdf_with_plain_fallback(&TextPdfExtractionConfig {
                text,
                color: *text_color,
                font,
                font_size: *font_size,
                font_weight: *font_weight,
                font_style: *font_style,
                limit: *limit,
                syntax_mode: mark.text_syntax,
                params: &mark.text_params,
                number_locale: mark.number_locale.as_deref(),
                number_locale_specs: Some(&mark.number_locale_specs),
                datetime_locale: mark.datetime_locale.as_deref(),
                datetime_timezone: mark.datetime_timezone.as_deref(),
                datetime_locale_specs: Some(&mark.datetime_locale_specs),
            })?;

            if *leader {
                if let Some(geometry) = compute_text_leader_geometry(TextLeaderGeometryInput {
                    target,
                    label_anchor: label,
                    angle_degrees: *angle,
                    text_bounds: &buffer.bounds,
                    align,
                    baseline,
                    label_padding: *leader_label_padding,
                    target_radius: *leader_target_radius,
                    min_length: *leader_min_length,
                    shape: *leader_shape,
                    arrow: *leader_arrow,
                    arrow_length: *leader_arrow_length,
                    arrow_width: *leader_arrow_width,
                }) {
                    self.draw_text_leader(
                        surface,
                        &geometry,
                        LeaderStrokeStyle {
                            paint: leader_stroke,
                            width: *leader_stroke_width,
                            cap: *leader_stroke_cap,
                            join: *leader_stroke_join,
                            dash: leader_stroke_dash_values
                                .as_ref()
                                .and_then(|values| values.get(index).map(Vec::as_slice)),
                        },
                    )?;
                }
            }

            self.draw_text_pdf_buffer(
                surface,
                &buffer,
                TextPdfPlacement {
                    label,
                    align,
                    baseline,
                    angle: *angle,
                    viewport: [chart_bounds.width, chart_bounds.height],
                },
                font_cache,
            )?;
        }

        Ok(())
    }

    fn draw_text_pdf_buffer(
        &self,
        surface: &mut Surface<'_>,
        buffer: &TextPdfBuffer,
        placement: TextPdfPlacement<'_>,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        let [x, text_top] =
            buffer
                .bounds
                .calculate_origin(placement.label, placement.align, placement.baseline);
        if placement.angle != 0.0 {
            surface.push_transform(&Transform::from_rotate_at(
                placement.angle,
                placement.label[0],
                placement.label[1],
            ));
        }

        if let Some(width) = buffer.clip_width {
            // Other clip edges lie outside the viewport after label rotation.
            let margin = placement.viewport[0]
                + placement.viewport[1]
                + 2.0 * (placement.label[0].abs() + placement.label[1].abs())
                + x.abs()
                + text_top.abs()
                + width;
            let rect = Rect::from_xywh(x - margin, text_top - margin, margin + width, 2.0 * margin)
                .ok_or_else(|| AvengerPdfError::TextBuffer("invalid text clip geometry".into()))?;
            let mut path = PathBuilder::new();
            path.push_rect(rect);
            surface.push_clip_path(
                &path.finish().ok_or_else(|| {
                    AvengerPdfError::TextBuffer("empty text clip geometry".into())
                })?,
                &FillRule::NonZero,
            );
        }

        let result = (|| {
            for draw_item in &buffer.draw_items {
                match *draw_item {
                    TextPdfDrawItem::GlyphRun(index) => {
                        let Some(run) = buffer.glyph_runs.get(index) else {
                            return Err(AvengerPdfError::TextBuffer(
                                "PDF text buffer referenced a missing glyph run".to_string(),
                            ));
                        };
                        self.draw_pdf_glyph_run(surface, buffer, run, x, text_top, font_cache)?;
                    }
                    TextPdfDrawItem::PathItem(index) => {
                        let Some(item) = buffer.items.get(index) else {
                            return Err(AvengerPdfError::TextBuffer(
                                "PDF text buffer referenced a missing path item".to_string(),
                            ));
                        };
                        self.draw_text_path_item(surface, item, x, text_top)?;
                    }
                }
            }
            Ok(())
        })();

        if buffer.clip_width.is_some() {
            surface.pop();
        }
        if placement.angle != 0.0 {
            surface.pop();
        }
        result
    }

    fn draw_pdf_glyph_run(
        &self,
        surface: &mut Surface<'_>,
        buffer: &TextPdfBuffer,
        run: &PdfGlyphRun,
        x: f32,
        y: f32,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        let resource = font_resource(buffer, run.font)?;
        let font = font_cache.font_for(resource)?;
        let glyphs = krilla_glyphs_from_run(run)?;
        if glyphs.is_empty() {
            return Ok(());
        }

        surface.set_fill(color_fill([run.fill.r, run.fill.g, run.fill.b, run.fill.a]));
        surface.set_stroke(run.stroke.as_ref().and_then(|stroke| {
            color_stroke(
                [
                    stroke.color.r,
                    stroke.color.g,
                    stroke.color.b,
                    stroke.color.a,
                ],
                stroke.width,
            )
        }));
        surface.draw_glyphs(
            Point::from_xy(x, y),
            &glyphs,
            font,
            &run.text,
            run.font_size,
            false,
        );
        Ok(())
    }

    fn draw_text_path_item(
        &self,
        surface: &mut Surface<'_>,
        item: &TextPathItem,
        x: f32,
        y: f32,
    ) -> Result<(), AvengerPdfError> {
        if item.kind != TextPathKind::MathShape {
            return Ok(());
        }
        let fill = item.fill.map(ColorOrGradient::Color);
        let stroke = item
            .stroke
            .as_ref()
            .map(|stroke| ColorOrGradient::Color(stroke.color));

        surface.push_transform(&Transform::from_translate(x, y));
        let result = self.draw_path_with_style(
            surface,
            &item.path,
            PathStyle {
                fill: fill.as_ref(),
                stroke: stroke.as_ref(),
                stroke_width: item.stroke.as_ref().map(|stroke| stroke.width),
                stroke_cap: item
                    .stroke
                    .as_ref()
                    .map(|stroke| text_path_stroke_cap(stroke.line_cap)),
                stroke_join: item
                    .stroke
                    .as_ref()
                    .map(|stroke| text_path_stroke_join(stroke.line_join)),
                stroke_dash: item
                    .stroke
                    .as_ref()
                    .and_then(|stroke| stroke.dash.as_ref().map(|dash| dash.array.as_slice())),
                stroke_dash_offset: item
                    .stroke
                    .as_ref()
                    .and_then(|stroke| stroke.dash.as_ref().map(|dash| dash.phase))
                    .unwrap_or(0.0),
                stroke_miter_limit: item.stroke.as_ref().map(|stroke| stroke.miter_limit),
                gradients: &[],
            },
        );
        surface.pop();
        result
    }

    fn draw_text_leader(
        &self,
        surface: &mut Surface<'_>,
        geometry: &TextLeaderGeometry,
        style: LeaderStrokeStyle<'_>,
    ) -> Result<(), AvengerPdfError> {
        let spine = text_leader_path(&geometry.spine);
        self.draw_path_with_style(
            surface,
            &spine,
            PathStyle {
                fill: None,
                stroke: Some(style.paint),
                stroke_width: Some(style.width.max(0.0)),
                stroke_cap: Some(style.cap),
                stroke_join: Some(style.join),
                stroke_dash: style.dash,
                stroke_dash_offset: 0.0,
                stroke_miter_limit: None,
                gradients: &[],
            },
        )?;

        if let Some(arrowhead) = &geometry.arrowhead {
            let path = text_leader_arrowhead_path(arrowhead);
            match arrowhead {
                TextLeaderArrowhead::Open { .. } => self.draw_path_with_style(
                    surface,
                    &path,
                    PathStyle {
                        fill: None,
                        stroke: Some(style.paint),
                        stroke_width: Some(style.width.max(0.0)),
                        stroke_cap: Some(style.cap),
                        stroke_join: Some(style.join),
                        stroke_dash: None,
                        stroke_dash_offset: 0.0,
                        stroke_miter_limit: None,
                        gradients: &[],
                    },
                )?,
                TextLeaderArrowhead::Triangle { .. } => self.draw_path_with_style(
                    surface,
                    &path,
                    PathStyle {
                        fill: Some(style.paint),
                        stroke: None,
                        stroke_width: None,
                        stroke_cap: None,
                        stroke_join: None,
                        stroke_dash: None,
                        stroke_dash_offset: 0.0,
                        stroke_miter_limit: None,
                        gradients: &[],
                    },
                )?,
            }
        }

        Ok(())
    }

    fn draw_image_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneImageMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (image_source, path) in
            izip!(mark.image_source_iter(), mark.transformed_path_iter(origin))
        {
            let Some(image) = image_source.inline_image() else {
                return Err(AvengerPdfError::UnsupportedFeature(
                    "resource-backed image marks require resolution before PDF rendering"
                        .to_string(),
                ));
            };
            let bbox = bounding_box(&path);
            let width = bbox.max.x - bbox.min.x;
            let height = bbox.max.y - bbox.min.y;
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            let Some(size) = Size::from_wh(width, height) else {
                continue;
            };

            let image = rgba_image(image, mark.smooth)?;
            surface.push_transform(&Transform::from_translate(bbox.min.x, bbox.min.y));
            surface.draw_image(image, size);
            surface.pop();
        }

        Ok(())
    }

    fn draw_warped_image_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &avenger_scenegraph::marks::warped_image::SceneWarpedImageMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        if mark.image.inline_image().is_none() {
            return Err(AvengerPdfError::UnsupportedFeature(
                "resource-backed warped image marks require resolution before PDF rendering"
                    .to_string(),
            ));
        }
        // PDF has no textured-mesh primitive; rasterize the mesh at 2x
        // supersampling and place the result at its bounding box.
        let Some((raster, [min_x, min_y, max_x, max_y])) = mark.rasterize(origin, 2.0) else {
            return Ok(());
        };
        let width = max_x - min_x;
        let height = max_y - min_y;
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }
        let Some(size) = Size::from_wh(width, height) else {
            return Ok(());
        };
        let image = rgba_image(&raster, mark.smooth)?;
        surface.push_transform(&Transform::from_translate(min_x, min_y));
        surface.draw_image(image, size);
        surface.pop();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_filled_path_with_optional_pattern(
        &self,
        surface: &mut Surface<'_>,
        path: &LyonPath,
        fill: &ColorOrGradient,
        fill_pattern: Option<&PatternFill>,
        stroke_style: PathStyle<'_>,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        let Some(fill_pattern) = fill_pattern else {
            return self.draw_path_with_style(
                surface,
                path,
                PathStyle {
                    fill: Some(fill),
                    ..stroke_style
                },
            );
        };

        self.draw_path_with_style(
            surface,
            path,
            PathStyle {
                fill: Some(fill),
                stroke: None,
                stroke_width: None,
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
                stroke_dash_offset: 0.0,
                stroke_miter_limit: None,
                gradients: stroke_style.gradients,
            },
        )?;

        self.draw_pattern_overlay(
            surface,
            path,
            fill,
            fill_pattern,
            stroke_style.gradients,
            pattern_reference_frame,
            chart_bounds,
        )?;

        self.draw_path_with_style(
            surface,
            path,
            PathStyle {
                fill: None,
                ..stroke_style
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pattern_overlay(
        &self,
        surface: &mut Surface<'_>,
        host_path: &LyonPath,
        host_fill: &ColorOrGradient,
        fill_pattern: &PatternFill,
        gradients: &[Gradient],
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerPdfError> {
        let Some(host_clip_path) = lyon_path_to_krilla(host_path) else {
            return Ok(());
        };

        let bbox = bounding_box(host_path);
        let host_bounds = PatternRect::new(
            bbox.min.x,
            bbox.min.y,
            bbox.max.x - bbox.min.x,
            bbox.max.y - bbox.min.y,
        );
        let plot_bounds = pattern_reference_frame
            .map(|frame| PatternRect::new(frame.x, frame.y, frame.width, frame.height));
        let pattern_context = PatternRenderContext {
            chart_bounds,
            plot_bounds,
            host_bounds,
            host_fill,
            gradients,
        };
        let Some(geometry) = build_layered_pattern_geometry(fill_pattern, &pattern_context)
            .map_err(pattern_geometry_error_to_pdf_error)?
        else {
            return Ok(());
        };

        if geometry.ink[3] <= 0.0 {
            return Ok(());
        }

        surface.push_clip_path(&host_clip_path, &FillRule::NonZero);
        if geometry
            .layers
            .iter()
            .all(|layer| matches!(layer.operation, PatternLayerOperation::Add))
        {
            let mut opaque_ink = geometry.ink;
            let opacity = opaque_ink[3].clamp(0.0, 1.0);
            opaque_ink[3] = 1.0;
            let Some(fill) = color_fill(opaque_ink) else {
                surface.pop();
                return Ok(());
            };

            surface.push_opacity(normalized(opacity));
            surface.set_fill(Some(fill));
            surface.set_stroke(None);
            for layer in &geometry.layers {
                let Some(coverage_path) = lyon_path_to_krilla(&layer.coverage_path) else {
                    continue;
                };
                surface.draw_path(&coverage_path);
            }
            surface.pop();
        } else {
            let Some(mask_stream) =
                self.build_pattern_operation_mask(surface, &geometry, host_bounds)?
            else {
                surface.pop();
                return Ok(());
            };
            let mut opaque_ink = geometry.ink;
            let opacity = opaque_ink[3].clamp(0.0, 1.0);
            opaque_ink[3] = 1.0;
            let Some(fill) = color_fill(opaque_ink) else {
                surface.pop();
                return Ok(());
            };

            surface.push_mask(Mask::new(mask_stream, MaskType::Luminosity));
            surface.push_opacity(normalized(opacity));
            surface.set_fill(Some(fill));
            surface.set_stroke(None);
            surface.draw_path(&host_clip_path);
            surface.pop();
            surface.pop();
        }
        surface.pop();
        Ok(())
    }

    fn build_pattern_operation_mask(
        &self,
        surface: &mut Surface<'_>,
        geometry: &LayeredPatternGeometry,
        bounds: PatternRect,
    ) -> Result<Option<Stream>, AvengerPdfError> {
        let mut previous_mask: Option<Stream> = None;

        for layer in &geometry.layers {
            let Some(layer_path) = lyon_path_to_krilla(&layer.coverage_path) else {
                continue;
            };

            let mut stream_builder = surface.stream_builder();
            let mut mask_surface = stream_builder.surface();
            draw_luminosity_mask_rect(&mut mask_surface, bounds, false);

            match layer.operation {
                PatternLayerOperation::Add => {
                    if let Some(previous) = &previous_mask {
                        draw_luminosity_previous_mask(
                            &mut mask_surface,
                            bounds,
                            previous.clone(),
                            true,
                        );
                    }
                    draw_luminosity_mask_path(&mut mask_surface, &layer_path, true);
                }
                PatternLayerOperation::Subtract => {
                    if let Some(previous) = &previous_mask {
                        draw_luminosity_previous_mask(
                            &mut mask_surface,
                            bounds,
                            previous.clone(),
                            true,
                        );
                        draw_luminosity_mask_path(&mut mask_surface, &layer_path, false);
                    }
                }
                PatternLayerOperation::Xor => {
                    if let Some(previous) = &previous_mask {
                        mask_surface.push_mask(Mask::new(previous.clone(), MaskType::Luminosity));
                        draw_luminosity_mask_rect(&mut mask_surface, bounds, true);
                        draw_luminosity_mask_path(&mut mask_surface, &layer_path, false);
                        mask_surface.pop();

                        draw_luminosity_mask_path(&mut mask_surface, &layer_path, true);
                        mask_surface.push_clip_path(&layer_path, &FillRule::NonZero);
                        mask_surface.push_mask(Mask::new(previous.clone(), MaskType::Luminosity));
                        draw_luminosity_mask_rect(&mut mask_surface, bounds, false);
                        mask_surface.pop();
                        mask_surface.pop();
                    } else {
                        draw_luminosity_mask_path(&mut mask_surface, &layer_path, true);
                    }
                }
            }

            mask_surface.finish();
            previous_mask = Some(stream_builder.finish());
        }

        Ok(previous_mask)
    }

    fn draw_path_with_style(
        &self,
        surface: &mut Surface<'_>,
        path: &LyonPath,
        style: PathStyle<'_>,
    ) -> Result<(), AvengerPdfError> {
        let Some(krilla_path) = lyon_path_to_krilla(path) else {
            return Ok(());
        };
        let bbox = bounding_box(path);
        let fill = style
            .fill
            .map(|fill| fill_from_paint(fill, style.gradients, &bbox))
            .transpose()?
            .flatten();
        let stroke = stroke_from_style(&style, &bbox)?;
        if fill.is_none() && stroke.is_none() {
            return Ok(());
        }

        surface.set_fill(fill);
        surface.set_stroke(stroke);
        surface.draw_path(&krilla_path);
        Ok(())
    }
}

fn text_path_stroke_cap(cap: TextPathLineCap) -> StrokeCap {
    match cap {
        TextPathLineCap::Butt => StrokeCap::Butt,
        TextPathLineCap::Round => StrokeCap::Round,
        TextPathLineCap::Square => StrokeCap::Square,
    }
}

fn text_path_stroke_join(join: TextPathLineJoin) -> StrokeJoin {
    match join {
        TextPathLineJoin::Bevel => StrokeJoin::Bevel,
        TextPathLineJoin::Miter => StrokeJoin::Miter,
        TextPathLineJoin::Round => StrokeJoin::Round,
    }
}

fn color_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn pattern_geometry_error_to_pdf_error(error: PatternGeometryError) -> AvengerPdfError {
    match error {
        PatternGeometryError::InvalidPattern => {
            AvengerPdfError::InvalidGeometry("invalid pattern fill".to_string())
        }
        PatternGeometryError::MissingPlotReferenceFrame => AvengerPdfError::InvalidGeometry(
            "pattern plot anchor requires an active pattern reference frame".to_string(),
        ),
    }
}

#[derive(Clone, Copy)]
struct PathStyle<'a> {
    fill: Option<&'a ColorOrGradient>,
    stroke: Option<&'a ColorOrGradient>,
    stroke_width: Option<f32>,
    stroke_cap: Option<StrokeCap>,
    stroke_join: Option<StrokeJoin>,
    stroke_dash: Option<&'a [f32]>,
    stroke_dash_offset: f32,
    stroke_miter_limit: Option<f32>,
    gradients: &'a [Gradient],
}

struct TextPdfPlacement<'a> {
    label: [f32; 2],
    align: &'a TextAlign,
    baseline: &'a TextBaseline,
    angle: f32,
    viewport: [f32; 2],
}

struct LeaderStrokeStyle<'a> {
    paint: &'a ColorOrGradient,
    width: f32,
    cap: StrokeCap,
    join: StrokeJoin,
    dash: Option<&'a [f32]>,
}

#[derive(Default)]
struct PdfFontCache {
    fonts: HashMap<FontCacheKey, KrillaFont>,
}

impl PdfFontCache {
    fn font_for(&mut self, resource: &FontResource) -> Result<KrillaFont, AvengerPdfError> {
        let key = FontCacheKey::from_resource(resource);
        if let Some(font) = self.fonts.get(&key) {
            return Ok(font.clone());
        }

        let font = KrillaFont::new(Data::from(resource.data.to_vec()), resource.face_index)
            .ok_or_else(|| {
                AvengerPdfError::Font(format!(
                    "failed to load font resource {} ({})",
                    resource.id.0, resource.family
                ))
            })?;
        self.fonts.insert(key, font.clone());
        Ok(font)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontCacheKey {
    face_index: u32,
    data: Arc<[u8]>,
}

impl FontCacheKey {
    fn from_resource(resource: &FontResource) -> Self {
        Self {
            face_index: resource.face_index,
            data: resource.data.clone(),
        }
    }
}

// Dimensions participate in identity so equal pixel bytes cannot alias different images.
#[derive(Clone, Hash)]
struct PdfRgbaImage {
    width: u32,
    height: u32,
    rgb: Arc<[u8]>,
    alpha: Arc<[u8]>,
}

impl CustomImage for PdfRgbaImage {
    fn color_channel(&self) -> &[u8] {
        &self.rgb
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        Some(&self.alpha)
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Rgb
    }
}

fn rgba_image(image: &avenger_image::RgbaImage, smooth: bool) -> Result<Image, AvengerPdfError> {
    let expected_len = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(4));
    if image.width == 0 || image.height == 0 || expected_len != Some(image.data.len()) {
        return Err(AvengerPdfError::Image("invalid RGBA image buffer".into()));
    }
    let mut rgb = Vec::with_capacity(image.data.len() / 4 * 3);
    let mut alpha = Vec::with_capacity(image.data.len() / 4);
    for pixel in image.data.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
        alpha.push(pixel[3]);
    }
    Image::from_custom(
        PdfRgbaImage {
            width: image.width,
            height: image.height,
            rgb: rgb.into(),
            alpha: alpha.into(),
        },
        smooth,
    )
    .map_err(AvengerPdfError::Image)
}

fn lyon_path_to_krilla(path: &LyonPath) -> Option<krilla::geom::Path> {
    let mut builder = PathBuilder::new();
    let mut has_segments = false;
    for event in path.iter() {
        match event {
            Event::Begin { at } => {
                builder.move_to(at.x, at.y);
                has_segments = true;
            }
            Event::Line { to, .. } => {
                builder.line_to(to.x, to.y);
                has_segments = true;
            }
            Event::Quadratic { ctrl, to, .. } => {
                builder.quad_to(ctrl.x, ctrl.y, to.x, to.y);
                has_segments = true;
            }
            Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                builder.cubic_to(ctrl1.x, ctrl1.y, ctrl2.x, ctrl2.y, to.x, to.y);
                has_segments = true;
            }
            Event::End { close, .. } => {
                if close {
                    builder.close();
                }
            }
        }
    }

    has_segments.then(|| builder.finish()).flatten()
}

fn pattern_rect_to_krilla(bounds: PatternRect) -> Option<krilla::geom::Path> {
    if bounds.is_empty() {
        return None;
    }

    let mut builder = PathBuilder::new();
    builder.move_to(bounds.min_x(), bounds.min_y());
    builder.line_to(bounds.max_x(), bounds.min_y());
    builder.line_to(bounds.max_x(), bounds.max_y());
    builder.line_to(bounds.min_x(), bounds.max_y());
    builder.close();
    builder.finish()
}

fn draw_luminosity_previous_mask(
    surface: &mut Surface<'_>,
    bounds: PatternRect,
    mask_stream: Stream,
    white: bool,
) {
    surface.push_mask(Mask::new(mask_stream, MaskType::Luminosity));
    draw_luminosity_mask_rect(surface, bounds, white);
    surface.pop();
}

fn draw_luminosity_mask_rect(surface: &mut Surface<'_>, bounds: PatternRect, white: bool) {
    let Some(path) = pattern_rect_to_krilla(bounds) else {
        return;
    };
    draw_luminosity_mask_path(surface, &path, white);
}

fn draw_luminosity_mask_path(surface: &mut Surface<'_>, path: &krilla::geom::Path, white: bool) {
    surface.set_fill(Some(luminosity_mask_fill(white)));
    surface.set_stroke(None);
    surface.draw_path(path);
}

fn luminosity_mask_fill(white: bool) -> Fill {
    let value = if white { 1.0 } else { 0.0 };
    color_fill([value, value, value, 1.0]).expect("opaque grayscale fill is valid")
}

fn clip_to_krilla_path(clip: &Clip) -> Result<Option<krilla::geom::Path>, AvengerPdfError> {
    let path = match clip {
        Clip::None => return Ok(None),
        Clip::Rect {
            x,
            y,
            width,
            height,
        } => {
            let Some(rect) = Rect::from_xywh(*x, *y, *width, *height) else {
                return Ok(None);
            };
            let mut builder = PathBuilder::new();
            builder.push_rect(rect);
            builder.finish()
        }
        Clip::Path(path) => lyon_path_to_krilla(path),
    };

    Ok(path)
}

fn fill_from_paint(
    paint: &ColorOrGradient,
    gradients: &[Gradient],
    bbox: &lyon_path::geom::Box2D<f32>,
) -> Result<Option<Fill>, AvengerPdfError> {
    let Some((paint, opacity)) = paint_and_opacity(paint, gradients, bbox)? else {
        return Ok(None);
    };
    Ok(Some(Fill {
        paint,
        opacity,
        rule: FillRule::NonZero,
    }))
}

fn color_fill(color: [f32; 4]) -> Option<Fill> {
    let [r, g, b, a] = color.map(|value| value.clamp(0.0, 1.0));
    if a <= 0.0 {
        return None;
    }
    Some(Fill {
        paint: rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
        opacity: normalized(a),
        rule: FillRule::NonZero,
    })
}

fn color_stroke(color: [f32; 4], width: f32) -> Option<Stroke> {
    if width <= 0.0 {
        return None;
    }
    let [r, g, b, a] = color.map(|value| value.clamp(0.0, 1.0));
    if a <= 0.0 {
        return None;
    }
    Some(Stroke {
        paint: rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
        width,
        miter_limit: 10.0,
        line_cap: LineCap::Butt,
        line_join: LineJoin::Miter,
        opacity: normalized(a),
        dash: None,
    })
}

fn stroke_from_style(
    style: &PathStyle<'_>,
    bbox: &lyon_path::geom::Box2D<f32>,
) -> Result<Option<Stroke>, AvengerPdfError> {
    let width = style.stroke_width.unwrap_or(0.0);
    if width <= 0.0 {
        return Ok(None);
    }
    let Some(stroke) = style.stroke else {
        return Ok(None);
    };
    let Some((paint, opacity)) = paint_and_opacity(stroke, style.gradients, bbox)? else {
        return Ok(None);
    };

    Ok(Some(Stroke {
        paint,
        width,
        miter_limit: style.stroke_miter_limit.unwrap_or(10.0),
        line_cap: style.stroke_cap.map(line_cap).unwrap_or_default(),
        line_join: style.stroke_join.map(line_join).unwrap_or_default(),
        opacity,
        dash: style
            .stroke_dash
            .filter(|dash| !dash.is_empty())
            .map(|dash| StrokeDash {
                array: dash.to_vec(),
                offset: style.stroke_dash_offset,
            }),
    }))
}

fn paint_and_opacity(
    paint: &ColorOrGradient,
    gradients: &[Gradient],
    bbox: &lyon_path::geom::Box2D<f32>,
) -> Result<Option<(Paint, NormalizedF32)>, AvengerPdfError> {
    match paint {
        ColorOrGradient::Color(color) => {
            let [r, g, b, a] = color.map(|value| value.clamp(0.0, 1.0));
            if a <= 0.0 {
                return Ok(None);
            }
            Ok(Some((
                rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
                normalized(a),
            )))
        }
        ColorOrGradient::GradientIndex(index) => {
            let gradient = gradients.get(*index as usize).ok_or_else(|| {
                AvengerPdfError::UnsupportedFeature(format!(
                    "gradient index {index} is out of range"
                ))
            })?;
            gradient_paint(gradient, bbox)
                .map(|paint| paint.map(|paint| (paint, NormalizedF32::ONE)))
        }
    }
}

fn gradient_paint(
    gradient: &Gradient,
    bbox: &lyon_path::geom::Box2D<f32>,
) -> Result<Option<Paint>, AvengerPdfError> {
    let stops = gradient_stops(gradient.stops());
    if stops.is_empty() {
        return Ok(None);
    }

    let left = bbox.min.x;
    let top = bbox.min.y;
    let width = (bbox.max.x - bbox.min.x).max(0.0);
    let height = (bbox.max.y - bbox.min.y).max(0.0);

    // Object-bounding-box paint is undefined on zero-area geometry, as in SVG.
    // Avoid passing a singular gradient transform to the PDF writer.
    if width == 0.0 || height == 0.0 {
        return Ok(None);
    }

    Ok(Some(match gradient {
        Gradient::LinearGradient(gradient) => LinearGradient {
            x1: gradient.x0.clamp(0.0, 1.0),
            y1: gradient.y0.clamp(0.0, 1.0),
            x2: gradient.x1.clamp(0.0, 1.0),
            y2: gradient.y1.clamp(0.0, 1.0),
            // Gradient geometry is defined in the unit bounding box, including its normals.
            transform: Transform::from_row(width, 0.0, 0.0, height, left, top),
            spread_method: SpreadMethod::Pad,
            stops,
            anti_alias: false,
        }
        .into(),
        Gradient::RadialGradient(gradient) => RadialGradient {
            fx: gradient.x0.clamp(0.0, 1.0),
            fy: gradient.y0.clamp(0.0, 1.0),
            fr: gradient.r0.clamp(0.0, 1.0),
            cx: gradient.x1.clamp(0.0, 1.0),
            cy: gradient.y1.clamp(0.0, 1.0),
            cr: gradient.r1.clamp(0.0, 1.0),
            // Core radial gradients use a centered square enclosing the mark bounds.
            transform: Transform::from_row(
                width.max(height),
                0.0,
                0.0,
                width.max(height),
                left - (height - width).max(0.0) / 2.0,
                top - (width - height).max(0.0) / 2.0,
            ),
            spread_method: SpreadMethod::Pad,
            stops,
            anti_alias: false,
        }
        .into(),
    }))
}

fn gradient_stops(stops: &[avenger_color::GradientStop]) -> Vec<Stop> {
    stops
        .iter()
        .map(|stop| {
            let [r, g, b, a] = stop.color.map(|value| value.clamp(0.0, 1.0));
            Stop {
                offset: normalized(stop.offset.clamp(0.0, 1.0)),
                color: rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
                opacity: normalized(a),
            }
        })
        .collect()
}

fn normalized(value: f32) -> NormalizedF32 {
    NormalizedF32::new(value.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn line_cap(cap: StrokeCap) -> LineCap {
    match cap {
        StrokeCap::Butt => LineCap::Butt,
        StrokeCap::Round => LineCap::Round,
        StrokeCap::Square => LineCap::Square,
    }
}

fn line_join(join: StrokeJoin) -> LineJoin {
    match join {
        StrokeJoin::Bevel => LineJoin::Bevel,
        StrokeJoin::Miter => LineJoin::Miter,
        StrokeJoin::Round => LineJoin::Round,
    }
}

fn line_mark_path(mark: &SceneLineMark, origin: [f32; 2]) -> LyonPath {
    let mut builder = LyonPath::builder();
    let mut path_len = 0usize;
    let mut last = None;

    for (x, y, defined) in izip!(mark.x_iter(), mark.y_iter(), mark.defined_iter()) {
        if *defined {
            let point = lyon_path::math::point(*x + origin[0], *y + origin[1]);
            if path_len == 0 {
                builder.begin(point);
            } else {
                builder.line_to(point);
            }
            path_len += 1;
            last = Some(point);
        } else {
            close_single_point_subpath(&mut builder, path_len, last);
            path_len = 0;
            last = None;
        }
    }

    close_single_point_subpath(&mut builder, path_len, last);
    builder.build()
}

fn close_single_point_subpath(
    builder: &mut lyon_path::path::Builder,
    path_len: usize,
    last: Option<lyon_path::math::Point>,
) {
    match (path_len, last) {
        (0, _) => {}
        (1, Some(point)) => {
            builder.line_to(point);
            builder.end(false);
        }
        _ => builder.end(false),
    }
}

fn trail_outline_path(mark: &SceneTrailMark, origin: [f32; 2]) -> LyonPath {
    let mut builder = LyonPath::builder();
    let mut prev = None;
    let mut run_len = 0usize;

    for (x, y, size, defined) in izip!(
        mark.x_iter(),
        mark.y_iter(),
        mark.size_iter(),
        mark.defined_iter()
    ) {
        if *defined {
            let point = [*x + origin[0], *y + origin[1]];
            let radius = (*size).max(0.0) / 2.0;
            if let Some((prev_point, prev_radius)) = prev {
                push_trail_segment_path(&mut builder, prev_point, prev_radius, point, radius);
            }
            prev = Some((point, radius));
            run_len += 1;
        } else {
            if run_len == 1 {
                if let Some((point, radius)) = prev {
                    push_trail_circle_path(&mut builder, point, radius);
                }
            }
            prev = None;
            run_len = 0;
        }
    }

    if run_len == 1 {
        if let Some((point, radius)) = prev {
            push_trail_circle_path(&mut builder, point, radius);
        }
    }

    builder.build()
}

fn push_trail_segment_path(
    builder: &mut lyon_path::path::Builder,
    p0: [f32; 2],
    r0: f32,
    p1: [f32; 2],
    r1: f32,
) {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let len = (dx * dx + dy * dy).sqrt();

    if len <= f32::EPSILON {
        push_trail_circle_path(builder, p0, r0.max(r1));
        return;
    }

    if r0 <= 0.0 && r1 <= 0.0 {
        return;
    }

    let nx = -dy / len;
    let ny = dx / len;
    let normal_angle = ny.atan2(nx);
    let p0_left = [p0[0] + nx * r0, p0[1] + ny * r0];
    let p1_left = [p1[0] + nx * r1, p1[1] + ny * r1];
    let p0_right = [p0[0] - nx * r0, p0[1] - ny * r0];

    builder.begin(lyon_path::math::point(p0_left[0], p0_left[1]));
    builder.line_to(lyon_path::math::point(p1_left[0], p1_left[1]));
    push_circular_arc_path(
        builder,
        p1,
        r1,
        normal_angle,
        normal_angle - std::f32::consts::PI,
    );
    builder.line_to(lyon_path::math::point(p0_right[0], p0_right[1]));
    push_circular_arc_path(
        builder,
        p0,
        r0,
        normal_angle - std::f32::consts::PI,
        normal_angle - 2.0 * std::f32::consts::PI,
    );
    builder.end(true);
}

fn push_trail_circle_path(builder: &mut lyon_path::path::Builder, point: [f32; 2], radius: f32) {
    if radius <= 0.0 {
        return;
    }

    builder.begin(lyon_path::math::point(point[0] + radius, point[1]));
    push_circular_arc_path(builder, point, radius, 0.0, -std::f32::consts::FRAC_PI_2);
    push_circular_arc_path(
        builder,
        point,
        radius,
        -std::f32::consts::FRAC_PI_2,
        -std::f32::consts::PI,
    );
    push_circular_arc_path(
        builder,
        point,
        radius,
        -std::f32::consts::PI,
        -3.0 * std::f32::consts::FRAC_PI_2,
    );
    push_circular_arc_path(
        builder,
        point,
        radius,
        -3.0 * std::f32::consts::FRAC_PI_2,
        -2.0 * std::f32::consts::PI,
    );
    builder.end(true);
}

fn push_circular_arc_path(
    builder: &mut lyon_path::path::Builder,
    center: [f32; 2],
    radius: f32,
    start_angle: f32,
    end_angle: f32,
) {
    if radius <= 0.0 {
        return;
    }

    let sweep = end_angle - start_angle;
    let segments = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / segments as f32;
    let mut angle0 = start_angle;

    for _ in 0..segments {
        let angle1 = angle0 + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let p0 = [
            center[0] + radius * angle0.cos(),
            center[1] + radius * angle0.sin(),
        ];
        let p1 = [
            center[0] + radius * angle1.cos(),
            center[1] + radius * angle1.sin(),
        ];
        let c0 = [
            p0[0] - k * radius * angle0.sin(),
            p0[1] + k * radius * angle0.cos(),
        ];
        let c1 = [
            p1[0] + k * radius * angle1.sin(),
            p1[1] - k * radius * angle1.cos(),
        ];
        builder.cubic_bezier_to(
            lyon_path::math::point(c0[0], c0[1]),
            lyon_path::math::point(c1[0], c1[1]),
            lyon_path::math::point(p1[0], p1[1]),
        );
        angle0 = angle1;
    }
}

fn font_resource(
    buffer: &TextPdfBuffer,
    id: FontResourceId,
) -> Result<&FontResource, AvengerPdfError> {
    buffer
        .font_resources
        .iter()
        .find(|resource| resource.id == id)
        .ok_or_else(|| {
            AvengerPdfError::TextBuffer(format!("PDF text buffer referenced missing font {}", id.0))
        })
}

fn krilla_glyphs_from_run(run: &PdfGlyphRun) -> Result<Vec<KrillaGlyph>, AvengerPdfError> {
    let font_size = run.font_size.max(0.0001);
    let use_run_actual_text = !run.text.is_ascii();
    let mut cursor_x = 0.0;
    let mut cursor_y = 0.0;
    let mut glyphs = Vec::with_capacity(run.glyphs.len());

    for glyph in &run.glyphs {
        if !is_translation_only(glyph.transform) {
            return Err(AvengerPdfError::UnsupportedFeature(
                "non-translation PDF glyph transforms are not supported".to_string(),
            ));
        }
        let glyph_x = glyph.transform.tx + glyph.x;
        let glyph_y = glyph.transform.ty + glyph.y;
        glyphs.push(KrillaGlyph::new(
            GlyphId::new(glyph.glyph_id as u32),
            glyph.x_advance / font_size,
            (glyph_x - cursor_x) / font_size,
            (cursor_y - glyph_y) / font_size,
            -glyph.y_advance / font_size,
            if use_run_actual_text {
                0..run.text.len()
            } else {
                glyph.text_range.clone()
            },
            None,
        ));
        cursor_x += glyph.x_advance;
        cursor_y += glyph.y_advance;
    }

    Ok(glyphs)
}

fn is_translation_only(transform: avenger_typst_label::Transform) -> bool {
    const EPSILON: f32 = 1.0e-5;
    (transform.sx - 1.0).abs() < EPSILON
        && transform.ky.abs() < EPSILON
        && transform.kx.abs() < EPSILON
        && (transform.sy - 1.0).abs() < EPSILON
}

fn text_leader_path(path: &TextLeaderPath) -> LyonPath {
    let mut builder = LyonPath::builder();
    match path {
        TextLeaderPath::Line { start, end } => {
            builder.begin(lyon_path::math::point(start[0], start[1]));
            builder.line_to(lyon_path::math::point(end[0], end[1]));
            builder.end(false);
        }
        TextLeaderPath::Polyline { points } => {
            if let Some(first) = points.first() {
                builder.begin(lyon_path::math::point(first[0], first[1]));
                for point in points.iter().skip(1) {
                    builder.line_to(lyon_path::math::point(point[0], point[1]));
                }
                builder.end(false);
            }
        }
        TextLeaderPath::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => {
            builder.begin(lyon_path::math::point(start[0], start[1]));
            builder.cubic_bezier_to(
                lyon_path::math::point(ctrl1[0], ctrl1[1]),
                lyon_path::math::point(ctrl2[0], ctrl2[1]),
                lyon_path::math::point(end[0], end[1]),
            );
            builder.end(false);
        }
    }
    builder.build()
}

fn text_leader_arrowhead_path(arrowhead: &TextLeaderArrowhead) -> LyonPath {
    let mut builder = LyonPath::builder();
    match arrowhead {
        TextLeaderArrowhead::Open { left, right } => {
            builder.begin(lyon_path::math::point(left[0][0], left[0][1]));
            builder.line_to(lyon_path::math::point(left[1][0], left[1][1]));
            builder.end(false);
            builder.begin(lyon_path::math::point(right[0][0], right[0][1]));
            builder.line_to(lyon_path::math::point(right[1][0], right[1][1]));
            builder.end(false);
        }
        TextLeaderArrowhead::Triangle { points } => {
            builder.begin(lyon_path::math::point(points[0][0], points[0][1]));
            builder.line_to(lyon_path::math::point(points[1][0], points[1][1]));
            builder.line_to(lyon_path::math::point(points[2][0], points[2][1]));
            builder.close();
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_text::types::TextSyntaxMode;
    use std::path::PathBuf;

    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
    use avenger_common::{
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_image::RgbaImage;
    use avenger_scenegraph::marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        pattern::{
            PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation,
            PatternReferenceFrame, PatternSymbol, StripePatternLayer, SymbolLattice2d, SymbolPaint,
            SymbolPatternLayer,
        },
        rect::SceneRectMark,
        text::SceneTextMark,
        trail::SceneTrailMark,
    };
    use avenger_text::types::{TextAlign, TextBaseline};
    use avenger_text::{FontResolutionOptions, MissingFontPolicy};

    fn empty_scene_graph(width: f32, height: f32) -> SceneGraph {
        SceneGraph {
            width,
            height,
            origin: [0.0, 0.0],
            marks: Vec::new(),
        }
    }

    fn text_scene_graph(text: &str) -> SceneGraph {
        text_scene_graph_with_font(text, "sans-serif")
    }

    fn text_scene_graph_with_font(text: &str, font: &str) -> SceneGraph {
        text_scene_graph_with_font_and_syntax(text, font, TextSyntaxMode::Plain)
    }

    fn typst_text_scene_graph(text: &str) -> SceneGraph {
        text_scene_graph_with_font_and_syntax(text, "sans-serif", TextSyntaxMode::TypstMarkup)
    }

    fn text_scene_graph_with_font_and_syntax(
        text: &str,
        font: &str,
        text_syntax: TextSyntaxMode,
    ) -> SceneGraph {
        SceneGraph {
            width: 240.0,
            height: 80.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar(text.to_string()),
                x: ScalarOrArray::new_scalar(12.0),
                y: ScalarOrArray::new_scalar(36.0),
                font: ScalarOrArray::new_scalar(font.to_string()),
                align: ScalarOrArray::new_scalar(TextAlign::Left),
                baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
                font_size: ScalarOrArray::new_scalar(18.0),
                limit: ScalarOrArray::new_scalar(f32::INFINITY),
                text_syntax,
                ..Default::default()
            }
            .into()],
        }
    }

    fn caveat_font_resolution() -> FontResolutionOptions {
        FontResolutionOptions {
            extra_font_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../avenger-vega-test-data/fonts/Caveat/static")],
            ..Default::default()
        }
    }

    fn missing_font_scene_graph() -> SceneGraph {
        SceneGraph {
            width: 160.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Missing font".to_string()),
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(24.0),
                font: ScalarOrArray::new_scalar("Definitely Missing Avenger Font".to_string()),
                font_size: ScalarOrArray::new_scalar(14.0),
                limit: ScalarOrArray::new_scalar(f32::INFINITY),
                ..Default::default()
            }
            .into()],
        }
    }

    #[test]
    fn renders_empty_scene_graph_as_pdf() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&empty_scene_graph(80.0, 40.0))
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn honors_transparent_background() {
        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                background: PdfBackground::Transparent,
                compress: false,
                ..Default::default()
            })
            .render_scene_graph(&empty_scene_graph(20.0, 20.0))
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn stroke_from_style_preserves_dash_offset_and_miter_limit() {
        let paint = ColorOrGradient::Color([0.1, 0.2, 0.3, 1.0]);
        let dash = [2.0, 1.0];
        let style = PathStyle {
            fill: None,
            stroke: Some(&paint),
            stroke_width: Some(1.5),
            stroke_cap: Some(StrokeCap::Round),
            stroke_join: Some(StrokeJoin::Miter),
            stroke_dash: Some(&dash),
            stroke_dash_offset: 0.5,
            stroke_miter_limit: Some(2.0),
            gradients: &[],
        };
        let bbox = lyon_path::geom::Box2D::new(
            lyon_path::geom::point(0.0, 0.0),
            lyon_path::geom::point(10.0, 10.0),
        );

        let stroke = stroke_from_style(&style, &bbox)
            .unwrap()
            .expect("stroke should resolve");

        assert_eq!(stroke.miter_limit, 2.0);
        let dash = stroke.dash.expect("dash should resolve");
        assert_eq!(dash.array.as_slice(), [2.0, 1.0].as_slice());
        assert_eq!(dash.offset, 0.5);
    }

    #[test]
    fn writes_scene_graph_pdf_to_disk() {
        let tempdir = tempfile::tempdir().unwrap();
        let output = tempdir.path().join("nested").join("chart.pdf");

        PdfRenderer::new()
            .write_scene_graph_pdf(&empty_scene_graph(20.0, 20.0), &output)
            .unwrap();

        let pdf = std::fs::read(output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn renders_rect_marks() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                x: ScalarOrArray::new_scalar(10.0),
                y: ScalarOrArray::new_scalar(8.0),
                width: Some(ScalarOrArray::new_scalar(30.0)),
                height: Some(ScalarOrArray::new_scalar(20.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 0.8])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: ScalarOrArray::new_scalar(2.0),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn renders_rect_fill_pattern_overlay() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.25,
            },
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0))],
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(4.0),
                width: Some(ScalarOrArray::new_scalar(24.0)),
                height: Some(ScalarOrArray::new_scalar(16.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn renders_mixed_pattern_layer_operation_masks() {
        let mut xor_stripe = StripePatternLayer::new(90.0, 8.0, 2.0);
        xor_stripe.operation = PatternLayerOperation::Xor;
        let subtract_symbol = SymbolPatternLayer {
            operation: PatternLayerOperation::Subtract,
            lattice: SymbolLattice2d {
                u_spacing: 12.0,
                u_angle: 0.0,
                v_spacing: 12.0,
                v_angle: 90.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: "circle".to_string(),
                size: 16.0,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        };
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.25,
            },
            layers: vec![
                PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0)),
                PatternLayer::Stripe(xor_stripe),
                PatternLayer::Symbol(subtract_symbol),
            ],
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(4.0),
                width: Some(ScalarOrArray::new_scalar(24.0)),
                height: Some(ScalarOrArray::new_scalar(16.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn plot_anchored_fill_pattern_requires_reference_frame() {
        let pattern = PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0))],
            ..Default::default()
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(4.0),
                width: Some(ScalarOrArray::new_scalar(24.0)),
                height: Some(ScalarOrArray::new_scalar(16.0)),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                ..Default::default()
            }
            .into()],
        };

        let err = PdfRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(err, AvengerPdfError::InvalidGeometry(_)));
        assert!(err.to_string().contains("pattern plot anchor"));
    }

    #[test]
    fn plot_anchored_fill_pattern_uses_group_reference_frame() {
        let pattern = PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0))],
            ..Default::default()
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                pattern_reference_frame: Some(PatternReferenceFrame {
                    x: 4.0,
                    y: 4.0,
                    width: 24.0,
                    height: 16.0,
                }),
                marks: vec![SceneRectMark {
                    len: 1,
                    x: ScalarOrArray::new_scalar(4.0),
                    y: ScalarOrArray::new_scalar(4.0),
                    width: Some(ScalarOrArray::new_scalar(24.0)),
                    height: Some(ScalarOrArray::new_scalar(16.0)),
                    fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn renders_symbol_pattern_layer_overlay() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            layers: vec![PatternLayer::Symbol(SymbolPatternLayer {
                operation: Default::default(),
                lattice: SymbolLattice2d {
                    u_spacing: 8.0,
                    u_angle: 0.0,
                    v_spacing: 8.0,
                    v_angle: 90.0,
                    u_phase: 0.0,
                    v_phase: 0.0,
                },
                symbol: PatternSymbol {
                    shape: "circle".to_string(),
                    size: 4.0,
                    rotation: 0.0,
                },
                paint: SymbolPaint::Filled,
            })],
            ..Default::default()
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(4.0),
                width: Some(ScalarOrArray::new_scalar(24.0)),
                height: Some(ScalarOrArray::new_scalar(16.0)),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn renders_clipped_gradient_groups() {
        let gradient = Gradient::LinearGradient(LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 0.0,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: [1.0, 0.0, 0.0, 1.0],
                },
                GradientStop {
                    offset: 1.0,
                    color: [0.0, 0.0, 1.0, 1.0],
                },
            ],
        });
        let rect = SceneRectMark {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            width: Some(ScalarOrArray::new_scalar(80.0)),
            height: Some(ScalarOrArray::new_scalar(40.0)),
            gradients: vec![gradient],
            fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: Clip::Rect {
                    x: 5.0,
                    y: 5.0,
                    width: 70.0,
                    height: 30.0,
                },
                marks: vec![rect.into()],
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn renders_inline_image_marks() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            data: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                image: ScalarOrArray::new_scalar(SceneImageSource::inline(image)),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(5.0),
                width: ScalarOrArray::new_scalar(30.0),
                height: ScalarOrArray::new_scalar(30.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn trail_outline_path_uses_size_as_geometry() {
        let path = trail_outline_path(
            &SceneTrailMark {
                len: 2,
                x: ScalarOrArray::new_array(vec![10.0, 30.0]),
                y: ScalarOrArray::new_array(vec![20.0, 20.0]),
                size: ScalarOrArray::new_array(vec![10.0, 20.0]),
                ..Default::default()
            },
            [0.0, 0.0],
        );
        let bbox = bounding_box(&path);

        assert!((bbox.min.x - 5.0).abs() < 0.001, "{bbox:?}");
        assert!((bbox.min.y - 10.0).abs() < 0.001, "{bbox:?}");
        assert!((bbox.max.x - 40.0).abs() < 0.001, "{bbox:?}");
        assert!((bbox.max.y - 30.0).abs() < 0.001, "{bbox:?}");
    }

    #[test]
    fn renders_plain_text_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&text_scene_graph("Hello PDF"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("Hello PDF"), "{extracted:?}");
    }

    #[test]
    fn converts_pdf_glyph_y_offsets_to_krilla_coordinates() {
        let run = PdfGlyphRun {
            font: FontResourceId(0),
            font_size: 10.0,
            fill: avenger_typst_label::Color::BLACK,
            stroke: None,
            text: "A".to_string(),
            glyphs: vec![avenger_typst_label::PdfGlyph {
                glyph_id: 1,
                unicode: "A".to_string(),
                text_range: 0..1,
                x: 0.0,
                y: 0.0,
                x_advance: 10.0,
                y_advance: 0.0,
                transform: avenger_typst_label::Transform {
                    tx: 3.0,
                    ty: 12.0,
                    ..avenger_typst_label::Transform::IDENTITY
                },
            }],
        };

        let glyphs = krilla_glyphs_from_run(&run).unwrap();

        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].x_offset, 0.3);
        assert_eq!(glyphs[0].y_offset, -1.2);
    }

    #[test]
    fn renders_math_text_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&typst_text_scene_graph("score $R^2$ = 0.94"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("score"), "{extracted:?}");
        assert!(extracted.contains("0.94"), "{extracted:?}");
        assert!(!extracted.contains('$'), "{extracted:?}");
    }

    #[test]
    fn renders_named_emoji_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&typst_text_scene_graph("Mood #emoji.face"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("Mood"), "{extracted:?}");
        assert!(extracted.contains('😀'), "{extracted:?}");
    }

    #[test]
    fn renders_complex_script_text_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&text_scene_graph("שלום नमस्ते"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("שלום"), "{extracted:?}");
        assert!(extracted.contains("नमस्ते"), "{extracted:?}");
    }

    #[test]
    fn missing_requested_text_font_errors_when_policy_is_error() {
        let err = PdfRenderer::new()
            .render_scene_graph(&missing_font_scene_graph())
            .unwrap_err();

        assert!(matches!(err, AvengerPdfError::Text(_)));
        assert!(err.to_string().contains("Definitely Missing Avenger Font"));
    }

    #[test]
    fn missing_requested_text_font_can_fallback_when_policy_allows() {
        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                font_resolution: FontResolutionOptions {
                    missing_font: MissingFontPolicy::Fallback,
                    ..Default::default()
                },
                ..Default::default()
            })
            .render_scene_graph(&missing_font_scene_graph())
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn extra_font_dirs_are_used_for_pdf_text_rendering() {
        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                compress: false,
                font_resolution: caveat_font_resolution(),
                ..Default::default()
            })
            .render_scene_graph(&text_scene_graph_with_font("Caveat", "Caveat"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(extracted.contains("Caveat"), "{extracted:?}");
    }
}
