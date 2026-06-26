use std::{collections::HashMap, path::Path};

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
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_text::{
    path::{TextPathItem, TextPathKind},
    pdf::{TextPdfBuffer, TextPdfDrawItem, TextPdfExtractionConfig},
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
    FontResolutionOptions, MissingFontPolicy, TextEngine,
};
use avenger_typst::{MathFontResource, MathFontResourceId, MathPdfGlyphRun};
use itertools::izip;
use krilla::{
    color::rgb,
    geom::{PathBuilder, Point, Rect, Size, Transform},
    image::Image,
    num::NormalizedF32,
    page::PageSettings,
    paint::{
        Fill, FillRule, LineCap, LineJoin, LinearGradient, Paint, RadialGradient, SpreadMethod,
        Stop, Stroke, StrokeDash,
    },
    surface::Surface,
    text::{Font as KrillaFont, GlyphId, KrillaGlyph, Tag},
    Data, Document, SerializeSettings,
};
use lyon_algorithms::aabb::bounding_box;
use lyon_path::{Event, Path as LyonPath};

use crate::{
    error::AvengerPdfError,
    options::{PdfBackground, PdfRenderOptions},
};

#[derive(Debug, Clone, Default)]
pub struct PdfRenderer {
    options: PdfRenderOptions,
}

impl PdfRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(mut self, options: PdfRenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<Vec<u8>, AvengerPdfError> {
        let width = scene_graph.width.max(3.0);
        let height = scene_graph.height.max(3.0);
        let page_settings = PageSettings::from_wh(width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;
        let font_resolution = effective_font_resolution(scene_graph, &self.options.font_resolution);
        if matches!(font_resolution.missing_font, MissingFontPolicy::Error) {
            let fontdb = avenger_text::fonts::build_fontdb(&font_resolution);
            validate_scene_graph_text_fonts(scene_graph, &fontdb)?;
        }

        let mut settings = SerializeSettings::default();
        settings.compress_content_streams = self.options.compress;

        let mut document = Document::new_with(settings);
        let mut page = document.start_page_with(page_settings);
        let mut surface = page.surface();
        let text_engine = TextEngine::with_font_resolution(&font_resolution).map_err(|err| {
            AvengerPdfError::TextBuffer(format!("failed to initialize text engine: {err}"))
        })?;
        let mut font_cache = PdfFontCache::default();
        self.draw_background(&mut surface, width, height)?;
        self.draw_scene_graph(&mut surface, scene_graph, &text_engine, &mut font_cache)?;
        surface.finish();
        page.finish();

        Ok(document.finish()?)
    }

    pub fn write_scene_graph_pdf<P: AsRef<Path>>(
        &self,
        scene_graph: &SceneGraph,
        output: P,
    ) -> Result<(), AvengerPdfError> {
        let output = output.as_ref();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, self.render_scene_graph(scene_graph)?)?;
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

        for item in display_list.ordered_items() {
            let clip_path = clip_to_krilla_path(&item.clip)?;
            if let Some(path) = clip_path.as_ref() {
                surface.push_clip_path(path, &FillRule::NonZero);
            }

            let result = match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.draw_path_mark(surface, mark, item.origin)
                }
                SceneDisplayMark::Borrowed(mark) => {
                    self.draw_scene_mark(surface, mark, item.origin, text_engine, font_cache)
                }
            };

            if clip_path.is_some() {
                surface.pop();
            }

            result?;
        }

        Ok(())
    }

    fn draw_scene_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneMark,
        origin: [f32; 2],
        text_engine: &TextEngine,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        match mark {
            SceneMark::Rect(mark) => self.draw_rect_mark(surface, mark, origin),
            SceneMark::Path(mark) => self.draw_path_mark(surface, mark, origin),
            SceneMark::Rule(mark) => self.draw_rule_mark(surface, mark, origin),
            SceneMark::Line(mark) => self.draw_line_mark(surface, mark, origin),
            SceneMark::Area(mark) => self.draw_area_mark(surface, mark, origin),
            SceneMark::Symbol(mark) => self.draw_symbol_mark(surface, mark, origin),
            SceneMark::Arc(mark) => self.draw_arc_mark(surface, mark, origin),
            SceneMark::Trail(mark) => self.draw_trail_mark(surface, mark, origin),
            SceneMark::Image(mark) => self.draw_image_mark(surface, mark, origin),
            SceneMark::Text(mark) => {
                self.draw_text_mark(surface, mark, origin, text_engine, font_cache)
            }
            SceneMark::Group(_) => Ok(()),
        }
    }

    fn draw_rect_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneRectMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.draw_path_with_style(
                surface,
                &path,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
            )?;
        }

        Ok(())
    }

    fn draw_path_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &ScenePathMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter()
        ) {
            self.draw_path_with_style(
                surface,
                &path,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: Some(mark.stroke_cap),
                    stroke_join: Some(mark.stroke_join),
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
            )?;
        }

        Ok(())
    }

    fn draw_symbol_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter()
        ) {
            self.draw_path_with_style(
                surface,
                &path,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
            )?;
        }

        Ok(())
    }

    fn draw_arc_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneArcMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (path, fill, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.draw_path_with_style(
                surface,
                &path,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
            )?;
        }

        Ok(())
    }

    fn draw_area_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneAreaMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        self.draw_path_with_style(
            surface,
            &mark.transformed_path(origin),
            PathStyle {
                fill: Some(&mark.fill),
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                gradients: &mark.gradients,
            },
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
            &mark.transformed_path(origin),
            PathStyle {
                fill: Some(&mark.stroke),
                stroke: None,
                stroke_width: None,
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
                gradients: &mark.gradients,
            },
        )
    }

    fn draw_text_mark(
        &self,
        surface: &mut Surface<'_>,
        mark: &SceneTextMark,
        origin: [f32; 2],
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
                    "gradient text paint is not supported by avenger-pdf-krilla".to_string(),
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
                        leader_stroke,
                        *leader_stroke_width,
                        *leader_stroke_cap,
                        *leader_stroke_join,
                        leader_stroke_dash_values
                            .as_ref()
                            .and_then(|values| values.get(index).map(Vec::as_slice)),
                    )?;
                }
            }

            self.draw_text_pdf_buffer(
                surface, &buffer, label, align, baseline, *angle, font_cache,
            )?;
        }

        Ok(())
    }

    fn draw_text_pdf_buffer(
        &self,
        surface: &mut Surface<'_>,
        buffer: &TextPdfBuffer,
        label: [f32; 2],
        align: &TextAlign,
        baseline: &TextBaseline,
        angle: f32,
        font_cache: &mut PdfFontCache,
    ) -> Result<(), AvengerPdfError> {
        let [x, text_top] = buffer.bounds.calculate_origin(label, align, baseline);
        if angle != 0.0 {
            surface.push_transform(&Transform::from_rotate_at(angle, label[0], label[1]));
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

        if angle != 0.0 {
            surface.pop();
        }
        result
    }

    fn draw_pdf_glyph_run(
        &self,
        surface: &mut Surface<'_>,
        buffer: &TextPdfBuffer,
        run: &MathPdfGlyphRun,
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
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
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
        stroke: &ColorOrGradient,
        stroke_width: f32,
        stroke_cap: StrokeCap,
        stroke_join: StrokeJoin,
        stroke_dash: Option<&[f32]>,
    ) -> Result<(), AvengerPdfError> {
        let spine = text_leader_path(&geometry.spine);
        self.draw_path_with_style(
            surface,
            &spine,
            PathStyle {
                fill: None,
                stroke: Some(stroke),
                stroke_width: Some(stroke_width.max(0.0)),
                stroke_cap: Some(stroke_cap),
                stroke_join: Some(stroke_join),
                stroke_dash,
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
                        stroke: Some(stroke),
                        stroke_width: Some(stroke_width.max(0.0)),
                        stroke_cap: Some(stroke_cap),
                        stroke_join: Some(stroke_join),
                        stroke_dash: None,
                        gradients: &[],
                    },
                )?,
                TextLeaderArrowhead::Triangle { .. } => self.draw_path_with_style(
                    surface,
                    &path,
                    PathStyle {
                        fill: Some(stroke),
                        stroke: None,
                        stroke_width: None,
                        stroke_cap: None,
                        stroke_join: None,
                        stroke_dash: None,
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

            let image = Image::from_rgba8(image.data.clone(), image.width, image.height);
            surface.push_transform(&Transform::from_translate(bbox.min.x, bbox.min.y));
            surface.draw_image(image, size);
            surface.pop();
        }

        Ok(())
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

fn color_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

struct PathStyle<'a> {
    fill: Option<&'a ColorOrGradient>,
    stroke: Option<&'a ColorOrGradient>,
    stroke_width: Option<f32>,
    stroke_cap: Option<StrokeCap>,
    stroke_join: Option<StrokeJoin>,
    stroke_dash: Option<&'a [f32]>,
    gradients: &'a [Gradient],
}

#[derive(Default)]
struct PdfFontCache {
    fonts: HashMap<FontCacheKey, KrillaFont>,
}

impl PdfFontCache {
    fn font_for(&mut self, resource: &MathFontResource) -> Result<KrillaFont, AvengerPdfError> {
        let key = FontCacheKey::from_resource(resource);
        if let Some(font) = self.fonts.get(&key) {
            return Ok(font.clone());
        }

        let variation_coords = resource
            .variations
            .iter()
            .map(|variation| (Tag::new(&variation.tag), variation.value))
            .collect::<Vec<_>>();
        let font = KrillaFont::new_variable(
            Data::from(resource.data.to_vec()),
            resource.face_index,
            &variation_coords,
        )
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
    data_ptr: usize,
    data_len: usize,
    variations: Vec<FontVariationKey>,
}

impl FontCacheKey {
    fn from_resource(resource: &MathFontResource) -> Self {
        Self {
            face_index: resource.face_index,
            data_ptr: resource.data.as_ref().as_ptr() as usize,
            data_len: resource.data.len(),
            variations: resource
                .variations
                .iter()
                .map(|variation| FontVariationKey {
                    tag: variation.tag,
                    value_bits: variation.value.to_bits(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FontVariationKey {
    tag: [u8; 4],
    value_bits: u32,
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
        miter_limit: 10.0,
        line_cap: style.stroke_cap.map(line_cap).unwrap_or_default(),
        line_join: style.stroke_join.map(line_join).unwrap_or_default(),
        opacity,
        dash: style
            .stroke_dash
            .filter(|dash| !dash.is_empty())
            .map(|dash| StrokeDash {
                array: dash.to_vec(),
                offset: 0.0,
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

    Ok(Some(match gradient {
        Gradient::LinearGradient(gradient) => LinearGradient {
            x1: left + gradient.x0.clamp(0.0, 1.0) * width,
            y1: top + gradient.y0.clamp(0.0, 1.0) * height,
            x2: left + gradient.x1.clamp(0.0, 1.0) * width,
            y2: top + gradient.y1.clamp(0.0, 1.0) * height,
            transform: Transform::identity(),
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
            transform: Transform::from_row(width, 0.0, 0.0, height, left, top),
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

fn font_resource(
    buffer: &TextPdfBuffer,
    id: MathFontResourceId,
) -> Result<&MathFontResource, AvengerPdfError> {
    buffer
        .font_resources
        .iter()
        .find(|resource| resource.id == id)
        .ok_or_else(|| {
            AvengerPdfError::TextBuffer(format!("PDF text buffer referenced missing font {}", id.0))
        })
}

fn krilla_glyphs_from_run(run: &MathPdfGlyphRun) -> Result<Vec<KrillaGlyph>, AvengerPdfError> {
    let font_size = run.font_size.max(0.0001);
    let use_run_actual_text = run.text.chars().any(|ch| !ch.is_ascii());
    let mut cursor_x = 0.0;
    let mut cursor_y = 0.0;
    let mut glyphs = Vec::with_capacity(run.glyphs.len());

    for glyph in &run.glyphs {
        if !is_translation_only(glyph.transform) {
            return Err(AvengerPdfError::UnsupportedFeature(
                "non-translation PDF glyph transforms are not supported".to_string(),
            ));
        }
        let glyph_x = glyph.transform.dx + glyph.x;
        let glyph_y = glyph.transform.dy + glyph.y;
        glyphs.push(KrillaGlyph::new(
            GlyphId::new(glyph.glyph_id as u32),
            glyph.x_advance / font_size,
            (glyph_x - cursor_x) / font_size,
            (glyph_y - cursor_y) / font_size,
            glyph.y_advance / font_size,
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

fn is_translation_only(transform: avenger_typst::MathTransform) -> bool {
    const EPSILON: f32 = 1.0e-5;
    (transform.xx - 1.0).abs() < EPSILON
        && transform.yx.abs() < EPSILON
        && transform.xy.abs() < EPSILON
        && (transform.yy - 1.0).abs() < EPSILON
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

fn effective_font_resolution(
    scene_graph: &SceneGraph,
    options: &FontResolutionOptions,
) -> FontResolutionOptions {
    let mut options = options.clone();
    if scene_graph_contains_system_fallback_text(scene_graph) {
        options.load_system_fonts = true;
    }
    options
}

fn scene_graph_contains_system_fallback_text(scene_graph: &SceneGraph) -> bool {
    let display_list = SceneDisplayList::from_scene_graph(scene_graph);
    display_list.ordered_items().iter().any(|item| {
        let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
            return false;
        };
        mark.text_iter()
            .any(|text| text_needs_system_font_fallback(text))
    })
}

fn text_needs_system_font_fallback(text: &str) -> bool {
    text.contains("#emoji.")
        || text
            .chars()
            .any(|ch| !ch.is_ascii() || is_color_emoji_char(ch))
}

fn is_color_emoji_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1F000..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x2300..=0x23FF
            | 0xFE0F
            | 0x200D
    )
}

fn validate_scene_graph_text_fonts(
    scene_graph: &SceneGraph,
    fontdb: &fontdb::Database,
) -> Result<(), AvengerPdfError> {
    let display_list = SceneDisplayList::from_scene_graph(scene_graph);
    for item in display_list.ordered_items() {
        let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
            continue;
        };

        for (((text, font), font_weight), font_style) in mark
            .text_iter()
            .zip(mark.font_iter())
            .zip(mark.font_weight_iter())
            .zip(mark.font_style_iter())
        {
            if text.chars().all(char::is_whitespace) {
                continue;
            }

            let family = font.trim();
            if family.is_empty() || is_generic_font_family(family) {
                continue;
            }

            if !fontdb_has_scene_family(fontdb, family, font_weight, font_style) {
                return Err(AvengerPdfError::Font(format!(
                    "missing requested text font family {family}"
                )));
            }
        }
    }

    Ok(())
}

fn is_generic_font_family(family: &str) -> bool {
    matches!(
        family,
        "serif" | "sans-serif" | "cursive" | "fantasy" | "monospace"
    )
}

fn fontdb_has_scene_family(
    fontdb: &fontdb::Database,
    family: &str,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> bool {
    let families = [fontdb::Family::Name(family)];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight(font_weight_number(font_weight)),
        stretch: fontdb::Stretch::Normal,
        style: scene_font_style(font_style),
    };
    fontdb.query(&query).is_some()
}

fn font_weight_number(font_weight: &FontWeight) -> u16 {
    match font_weight {
        FontWeight::Name(name) => font_weight_name_number(name),
        FontWeight::Number(weight) => weight.clamp(1.0, 1000.0).round() as u16,
    }
}

fn font_weight_name_number(name: &FontWeightNameSpec) -> u16 {
    match name {
        FontWeightNameSpec::Normal => 400,
        FontWeightNameSpec::Bold => 700,
    }
}

fn scene_font_style(font_style: &FontStyle) -> fontdb::Style {
    match font_style {
        FontStyle::Normal => fontdb::Style::Normal,
        FontStyle::Italic => fontdb::Style::Italic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
    use avenger_common::{
        types::{
            ImageAlign, ImageBaseline, SceneTextLeaderArrow, SceneTextLeaderShape, StrokeCap,
            StrokeJoin, SymbolShape,
        },
        value::ScalarOrArray,
    };
    use avenger_image::RgbaImage;
    use avenger_scenegraph::marks::rule::SceneRuleMark;
    use avenger_scenegraph::marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        rect::SceneRectMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
    };
    use avenger_text::types::{TextAlign, TextBaseline};
    use avenger_text::{FontResolutionOptions, MissingFontPolicy};
    use pdfium_render::prelude::{PdfRenderConfig, Pdfium, PdfiumError};

    const PARITY_ENV: &str = "AVENGER_PDF_KRILLA_PARITY";
    const PDFIUM_LIBRARY_PATH_ENV: &str = "AVENGER_PDF_KRILLA_PDFIUM_LIBRARY_PATH";
    const CHART_PDFIUM_LIBRARY_PATH_ENV: &str = "AVENGER_CHART_PDFIUM_LIBRARY_PATH";
    const PARITY_SCALE: f32 = 2.0;
    static PDFIUM_RENDER_LOCK: Mutex<()> = Mutex::new(());

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
    fn renders_rect_marks_directly() {
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
    fn renders_plain_text_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&text_scene_graph("Hello PDF"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("Hello PDF"), "{extracted:?}");
    }

    #[test]
    fn renders_math_text_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&text_scene_graph("score $R^2$ = 0.94"))
            .unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();

        assert!(extracted.contains("score"), "{extracted:?}");
        assert!(extracted.contains("0.94"), "{extracted:?}");
        assert!(!extracted.contains('$'), "{extracted:?}");
    }

    #[test]
    fn renders_named_emoji_as_extractable_pdf_text() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&text_scene_graph("Mood #emoji.face"))
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

        assert!(matches!(
            err,
            AvengerPdfError::Font(message)
                if message.contains("missing requested text font family")
                    && message.contains("Definitely Missing Avenger Font")
        ));
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

    #[test]
    fn side_by_side_pdfium_parity_fixtures_when_enabled() {
        if !parity_enabled() {
            eprintln!("skipping PDF krilla parity fixtures; set {PARITY_ENV}=1 to run");
            return;
        }

        let fixtures = parity_fixtures();
        for fixture in fixtures {
            let direct_pdf = PdfRenderer::new()
                .with_options(fixture.direct_options())
                .render_scene_graph(&fixture.scene_graph)
                .unwrap_or_else(|err| {
                    panic!(
                        "direct krilla PDF render failed for {}: {err}",
                        fixture.name
                    )
                });
            let legacy_pdf = avenger_pdf::PdfRenderer::new()
                .with_options(fixture.legacy_options())
                .render_scene_graph(&fixture.scene_graph)
                .unwrap_or_else(|err| {
                    panic!("legacy svg2pdf render failed for {}: {err}", fixture.name)
                });

            assert_pdf_extracts(fixture.name, "direct", &direct_pdf, fixture.direct_text);
            assert_pdf_extracts(fixture.name, "legacy", &legacy_pdf, fixture.legacy_text);

            let direct_image = rasterize_pdf_with_pdfium(
                &direct_pdf,
                fixture.scene_graph.width,
                fixture.scene_graph.height,
            )
            .unwrap_or_else(|err| {
                panic!("direct PDFium raster failed for {}: {err}", fixture.name)
            });
            let legacy_image = rasterize_pdf_with_pdfium(
                &legacy_pdf,
                fixture.scene_graph.width,
                fixture.scene_graph.height,
            )
            .unwrap_or_else(|err| {
                panic!("legacy PDFium raster failed for {}: {err}", fixture.name)
            });

            let diff = image_diff_summary(&direct_image, &legacy_image);
            if !diff.within(fixture.tolerance) {
                save_parity_artifacts(
                    fixture.name,
                    &direct_pdf,
                    &legacy_pdf,
                    &direct_image,
                    &legacy_image,
                )
                .unwrap_or_else(|err| {
                    panic!(
                        "failed to save parity failure artifacts for {}: {err}",
                        fixture.name
                    )
                });
                panic!(
                    "PDF krilla parity fixture '{}' exceeded tolerance: {:?}, tolerance {:?}. Artifacts saved under {}",
                    fixture.name,
                    diff,
                    fixture.tolerance,
                    parity_output_dir(fixture.name).display()
                );
            }
        }
    }

    #[derive(Debug, Clone)]
    struct ParityFixture {
        name: &'static str,
        scene_graph: SceneGraph,
        tolerance: DiffTolerance,
        direct_text: &'static [&'static str],
        legacy_text: &'static [&'static str],
    }

    impl ParityFixture {
        fn direct_options(&self) -> PdfRenderOptions {
            PdfRenderOptions {
                compress: false,
                font_resolution: parity_font_resolution(),
                ..Default::default()
            }
        }

        fn legacy_options(&self) -> avenger_pdf::PdfRenderOptions {
            avenger_pdf::PdfRenderOptions {
                compress: false,
                font_resolution: parity_font_resolution(),
                ..Default::default()
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct DiffTolerance {
        max_mean_channel_abs_diff: f64,
        max_changed_pixel_fraction: f64,
        max_channel_abs_diff: u8,
    }

    #[derive(Debug, Clone, Copy)]
    struct ImageDiffSummary {
        mean_channel_abs_diff: f64,
        changed_pixel_fraction: f64,
        max_channel_abs_diff: u8,
    }

    impl ImageDiffSummary {
        fn within(self, tolerance: DiffTolerance) -> bool {
            self.mean_channel_abs_diff <= tolerance.max_mean_channel_abs_diff
                && self.changed_pixel_fraction <= tolerance.max_changed_pixel_fraction
                && self.max_channel_abs_diff <= tolerance.max_channel_abs_diff
        }
    }

    fn parity_enabled() -> bool {
        std::env::var(PARITY_ENV)
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    }

    fn parity_font_resolution() -> FontResolutionOptions {
        FontResolutionOptions {
            load_system_fonts: true,
            missing_font: MissingFontPolicy::Fallback,
            ..Default::default()
        }
    }

    fn parity_fixtures() -> Vec<ParityFixture> {
        vec![
            ParityFixture {
                name: "vector_gradient_clip_image",
                scene_graph: vector_gradient_clip_image_scene_graph(),
                tolerance: DiffTolerance {
                    max_mean_channel_abs_diff: 4.0,
                    max_changed_pixel_fraction: 0.25,
                    max_channel_abs_diff: 255,
                },
                direct_text: &[],
                legacy_text: &[],
            },
            ParityFixture {
                name: "mixed_text_math_emoji_markup",
                scene_graph: mixed_text_math_emoji_markup_scene_graph(),
                tolerance: DiffTolerance {
                    max_mean_channel_abs_diff: 10.0,
                    max_changed_pixel_fraction: 0.22,
                    max_channel_abs_diff: 255,
                },
                direct_text: &["Price", "0.94", "😀"],
                legacy_text: &["Price", "0.94"],
            },
            ParityFixture {
                name: "rotated_leader_zorder",
                scene_graph: rotated_leader_zorder_scene_graph(),
                tolerance: DiffTolerance {
                    max_mean_channel_abs_diff: 10.0,
                    max_changed_pixel_fraction: 0.22,
                    max_channel_abs_diff: 255,
                },
                direct_text: &["covered"],
                legacy_text: &["covered"],
            },
        ]
    }

    fn vector_gradient_clip_image_scene_graph() -> SceneGraph {
        let gradient = Gradient::LinearGradient(LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: [0.1, 0.45, 0.9, 1.0],
                },
                GradientStop {
                    offset: 1.0,
                    color: [0.9, 0.35, 0.1, 1.0],
                },
            ],
        });
        let image = RgbaImage {
            width: 3,
            height: 3,
            data: vec![
                20, 60, 160, 255, 80, 190, 120, 255, 250, 220, 80, 255, 210, 45, 45, 255, 245, 245,
                245, 255, 45, 130, 210, 255, 80, 190, 120, 255, 35, 35, 35, 255, 220, 90, 160, 255,
            ],
        };
        let clipped_rect = SceneRectMark {
            x: ScalarOrArray::new_scalar(12.0),
            y: ScalarOrArray::new_scalar(12.0),
            width: Some(ScalarOrArray::new_scalar(92.0)),
            height: Some(ScalarOrArray::new_scalar(54.0)),
            gradients: vec![gradient],
            fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.05, 0.05, 0.05, 0.9])),
            stroke_width: ScalarOrArray::new_scalar(1.5),
            ..Default::default()
        };
        let image_mark = SceneImageMark {
            image: ScalarOrArray::new_scalar(SceneImageSource::inline(image)),
            x: ScalarOrArray::new_scalar(116.0),
            y: ScalarOrArray::new_scalar(16.0),
            width: ScalarOrArray::new_scalar(42.0),
            height: ScalarOrArray::new_scalar(42.0),
            align: ScalarOrArray::new_scalar(ImageAlign::Left),
            baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
            ..Default::default()
        };

        SceneGraph {
            width: 180.0,
            height: 90.0,
            origin: [0.0, 0.0],
            marks: vec![
                SceneGroup {
                    clip: Clip::Rect {
                        x: 8.0,
                        y: 8.0,
                        width: 100.0,
                        height: 62.0,
                    },
                    marks: vec![clipped_rect.into()],
                    ..Default::default()
                }
                .into(),
                SceneRuleMark {
                    x: ScalarOrArray::new_scalar(10.0),
                    y: ScalarOrArray::new_scalar(76.0),
                    x2: ScalarOrArray::new_scalar(166.0),
                    y2: ScalarOrArray::new_scalar(76.0),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.1, 0.1, 0.1, 1.0])),
                    stroke_width: ScalarOrArray::new_scalar(2.0),
                    stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Round),
                    ..Default::default()
                }
                .into(),
                SceneSymbolMark {
                    shapes: vec![SymbolShape::Circle],
                    x: ScalarOrArray::new_scalar(88.0),
                    y: ScalarOrArray::new_scalar(76.0),
                    size: ScalarOrArray::new_scalar(160.0),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.05, 0.65, 0.45, 1.0,
                    ])),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
                    stroke_width: Some(1.2),
                    ..Default::default()
                }
                .into(),
                image_mark.into(),
            ],
        }
    }

    fn mixed_text_math_emoji_markup_scene_graph() -> SceneGraph {
        SceneGraph {
            width: 320.0,
            height: 92.0,
            origin: [0.0, 0.0],
            marks: vec![
                SceneTextMark {
                    text: ScalarOrArray::new_scalar(
                        "Price \\$7, score $R^2$ = 0.94 #emoji.face".to_string(),
                    ),
                    x: ScalarOrArray::new_scalar(12.0),
                    y: ScalarOrArray::new_scalar(32.0),
                    font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                    font_size: ScalarOrArray::new_scalar(18.0),
                    limit: ScalarOrArray::new_scalar(f32::INFINITY),
                    color: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.02, 0.04, 0.09, 1.0,
                    ])),
                    ..Default::default()
                }
                .into(),
                SceneTextMark {
                    text: ScalarOrArray::new_scalar(
                        "#underline[important] #strike[old] H#sub[2]O".to_string(),
                    ),
                    x: ScalarOrArray::new_scalar(12.0),
                    y: ScalarOrArray::new_scalar(66.0),
                    font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                    font_size: ScalarOrArray::new_scalar(16.0),
                    limit: ScalarOrArray::new_scalar(f32::INFINITY),
                    color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.1, 0.1, 0.1, 1.0])),
                    ..Default::default()
                }
                .into(),
            ],
        }
    }

    fn rotated_leader_zorder_scene_graph() -> SceneGraph {
        SceneGraph {
            width: 190.0,
            height: 120.0,
            origin: [0.0, 0.0],
            marks: vec![
                SceneRectMark {
                    x: ScalarOrArray::new_scalar(6.0),
                    y: ScalarOrArray::new_scalar(6.0),
                    width: Some(ScalarOrArray::new_scalar(178.0)),
                    height: Some(ScalarOrArray::new_scalar(108.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.98, 0.98, 0.96, 1.0,
                    ])),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.78, 0.78, 0.72, 1.0,
                    ])),
                    stroke_width: ScalarOrArray::new_scalar(1.0),
                    ..Default::default()
                }
                .into(),
                SceneTextMark {
                    text: ScalarOrArray::new_scalar("covered label".to_string()),
                    x: ScalarOrArray::new_scalar(30.0),
                    y: ScalarOrArray::new_scalar(86.0),
                    dx: ScalarOrArray::new_scalar(70.0),
                    dy: ScalarOrArray::new_scalar(-44.0),
                    font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                    font_size: ScalarOrArray::new_scalar(15.0),
                    limit: ScalarOrArray::new_scalar(f32::INFINITY),
                    angle: ScalarOrArray::new_scalar(-18.0),
                    leader: ScalarOrArray::new_scalar(true),
                    leader_stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.0, 0.25, 0.7, 1.0,
                    ])),
                    leader_stroke_width: ScalarOrArray::new_scalar(1.4),
                    leader_stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Round),
                    leader_stroke_join: ScalarOrArray::new_scalar(StrokeJoin::Round),
                    leader_shape: ScalarOrArray::new_scalar(SceneTextLeaderShape::Curved),
                    leader_arrow: ScalarOrArray::new_scalar(SceneTextLeaderArrow::Triangle),
                    color: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.03, 0.03, 0.04, 1.0,
                    ])),
                    zindex: Some(1),
                    ..Default::default()
                }
                .into(),
                SceneRectMark {
                    x: ScalarOrArray::new_scalar(104.0),
                    y: ScalarOrArray::new_scalar(30.0),
                    width: Some(ScalarOrArray::new_scalar(54.0)),
                    height: Some(ScalarOrArray::new_scalar(20.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                        0.85, 0.12, 0.12, 0.86,
                    ])),
                    stroke_width: ScalarOrArray::new_scalar(0.0),
                    zindex: Some(2),
                    ..Default::default()
                }
                .into(),
            ],
        }
    }

    fn assert_pdf_extracts(
        fixture: &str,
        renderer: &str,
        pdf: &[u8],
        expected_substrings: &[&str],
    ) {
        if expected_substrings.is_empty() {
            return;
        }

        let extracted = pdf_extract::extract_text_from_mem(pdf).unwrap_or_else(|err| {
            panic!("{renderer} text extraction failed for {fixture}: {err}");
        });
        let compact_extracted = extracted
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        for expected in expected_substrings {
            let compact_expected = expected
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            assert!(
                extracted.contains(expected) || compact_extracted.contains(&compact_expected),
                "{renderer} PDF extraction for {fixture} missing {expected:?}; extracted {extracted:?}"
            );
        }
    }

    fn bind_pdfium() -> Result<Pdfium, String> {
        for env_var in [PDFIUM_LIBRARY_PATH_ENV, CHART_PDFIUM_LIBRARY_PATH_ENV] {
            if let Some(path) = std::env::var_os(env_var) {
                return bind_pdfium_path(absolute_pdfium_path(PathBuf::from(path)), env_var);
            }
        }

        let scratch_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("scratch/pdfium/lib/libpdfium.dylib");
        if scratch_path.exists() {
            return bind_pdfium_path(scratch_path, "scratch/pdfium");
        }

        match Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")) {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(local_err) => match Pdfium::bind_to_system_library() {
                Ok(bindings) => Ok(Pdfium::new(bindings)),
                Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
                Err(system_err) => Err(format!(
                    "failed to bind PDFium beside the test binary: {local_err}; failed to bind system PDFium: {system_err}; set {PDFIUM_LIBRARY_PATH_ENV}=/path/to/libpdfium.dylib"
                )),
            },
        }
    }

    fn bind_pdfium_path(path: PathBuf, source: &str) -> Result<Pdfium, String> {
        match Pdfium::bind_to_library(&path) {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(err) => Err(format!(
                "failed to bind PDFium from {source} path {}: {err}",
                path.display()
            )),
        }
    }

    fn absolute_pdfium_path(path: PathBuf) -> PathBuf {
        if path.is_absolute() {
            return path;
        }
        let workspace_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&path);
        if workspace_path.exists() {
            return workspace_path;
        }
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }

    fn scaled_pdf_dimension(value: f32, axis: &str) -> Result<i32, String> {
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("invalid PDF {axis} dimension: {value}"));
        }

        let pixels = (value * PARITY_SCALE).ceil();
        if pixels > i32::MAX as f32 {
            return Err(format!(
                "PDF {axis} dimension {pixels} exceeds PDFium limits"
            ));
        }

        Ok(pixels as i32)
    }

    fn rasterize_pdf_with_pdfium(
        pdf: &[u8],
        scene_width: f32,
        scene_height: f32,
    ) -> Result<image::RgbaImage, String> {
        let width = scaled_pdf_dimension(scene_width, "width")?;
        let height = scaled_pdf_dimension(scene_height, "height")?;
        let _guard = PDFIUM_RENDER_LOCK
            .lock()
            .map_err(|_| "PDFium render lock was poisoned".to_string())?;
        let pdfium = bind_pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_vec(pdf.to_vec(), None)
            .map_err(|err| format!("failed to load generated PDF with PDFium: {err}"))?;
        let page_count = document.pages().len();
        if page_count != 1 {
            return Err(format!(
                "expected generated PDF to contain one page, found {page_count}"
            ));
        }
        let page = document
            .pages()
            .get(0)
            .map_err(|err| format!("failed to access generated PDF page: {err}"))?;
        let bitmap = page
            .render_with_config(&PdfRenderConfig::new().set_fixed_size(width, height))
            .map_err(|err| format!("failed to rasterize generated PDF with PDFium: {err}"))?;
        if bitmap.width() != width || bitmap.height() != height {
            return Err(format!(
                "PDFium raster dimensions differ. Expected ({width}, {height}), got ({}, {}).",
                bitmap.width(),
                bitmap.height()
            ));
        }
        Ok(bitmap
            .as_image()
            .map_err(|err| format!("failed to convert PDFium bitmap to image: {err}"))?
            .into_rgba8())
    }

    fn image_diff_summary(a: &image::RgbaImage, b: &image::RgbaImage) -> ImageDiffSummary {
        assert_eq!(a.dimensions(), b.dimensions(), "PDFium raster dimensions");

        let mut total_abs_diff = 0u64;
        let mut changed_pixels = 0u64;
        let mut max_channel_abs_diff = 0u8;
        for (a_pixel, b_pixel) in a.as_raw().chunks_exact(4).zip(b.as_raw().chunks_exact(4)) {
            let mut pixel_changed = false;
            for (a_channel, b_channel) in a_pixel.iter().zip(b_pixel.iter()) {
                let diff = a_channel.abs_diff(*b_channel);
                total_abs_diff += u64::from(diff);
                max_channel_abs_diff = max_channel_abs_diff.max(diff);
                if diff > 8 {
                    pixel_changed = true;
                }
            }
            if pixel_changed {
                changed_pixels += 1;
            }
        }

        let total_pixels = u64::from(a.width()) * u64::from(a.height());
        let total_channels = total_pixels * 4;
        ImageDiffSummary {
            mean_channel_abs_diff: total_abs_diff as f64 / total_channels as f64,
            changed_pixel_fraction: changed_pixels as f64 / total_pixels as f64,
            max_channel_abs_diff,
        }
    }

    fn save_parity_artifacts(
        fixture: &str,
        direct_pdf: &[u8],
        legacy_pdf: &[u8],
        direct_image: &image::RgbaImage,
        legacy_image: &image::RgbaImage,
    ) -> Result<(), String> {
        let output_dir = parity_output_dir(fixture);
        std::fs::create_dir_all(&output_dir)
            .map_err(|err| format!("failed to create {}: {err}", output_dir.display()))?;
        std::fs::write(output_dir.join("direct-krilla.pdf"), direct_pdf)
            .map_err(|err| format!("failed to write direct PDF: {err}"))?;
        std::fs::write(output_dir.join("legacy-svg2pdf.pdf"), legacy_pdf)
            .map_err(|err| format!("failed to write legacy PDF: {err}"))?;
        direct_image
            .save(output_dir.join("direct-krilla.png"))
            .map_err(|err| format!("failed to write direct PNG: {err}"))?;
        legacy_image
            .save(output_dir.join("legacy-svg2pdf.png"))
            .map_err(|err| format!("failed to write legacy PNG: {err}"))?;
        diff_image(direct_image, legacy_image)
            .save(output_dir.join("diff.png"))
            .map_err(|err| format!("failed to write diff PNG: {err}"))?;
        Ok(())
    }

    fn parity_output_dir(fixture: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("target/tests/pdf_krilla_parity")
            .join(fixture)
    }

    fn diff_image(a: &image::RgbaImage, b: &image::RgbaImage) -> image::RgbaImage {
        assert_eq!(a.dimensions(), b.dimensions(), "PDFium raster dimensions");
        let mut output = image::RgbaImage::new(a.width(), a.height());
        for ((out, a_pixel), b_pixel) in output
            .as_mut()
            .chunks_exact_mut(4)
            .zip(a.as_raw().chunks_exact(4))
            .zip(b.as_raw().chunks_exact(4))
        {
            out[0] = a_pixel[0].abs_diff(b_pixel[0]).saturating_mul(4);
            out[1] = a_pixel[1].abs_diff(b_pixel[1]).saturating_mul(4);
            out[2] = a_pixel[2].abs_diff(b_pixel[2]).saturating_mul(4);
            out[3] = 255;
        }
        output
    }
}
