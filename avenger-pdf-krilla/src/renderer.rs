use std::path::Path;

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::Clip, image::SceneImageMark,
        line::SceneLineMark, mark::SceneMark, path::ScenePathMark, rect::SceneRectMark,
        rule::SceneRuleMark, symbol::SceneSymbolMark, trail::SceneTrailMark,
    },
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use itertools::izip;
use krilla::{
    color::rgb,
    geom::{PathBuilder, Rect, Size, Transform},
    image::Image,
    num::NormalizedF32,
    page::PageSettings,
    paint::{
        Fill, FillRule, LineCap, LineJoin, LinearGradient, Paint, RadialGradient, SpreadMethod,
        Stop, Stroke, StrokeDash,
    },
    surface::Surface,
    Document, SerializeSettings,
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

        let mut settings = SerializeSettings::default();
        settings.compress_content_streams = self.options.compress;
        settings.render_svg_glyph_fn = krilla_svg::render_svg_glyph;

        let mut document = Document::new_with(settings);
        let mut page = document.start_page_with(page_settings);
        let mut surface = page.surface();
        self.draw_background(&mut surface, width, height)?;
        self.draw_scene_graph(&mut surface, scene_graph)?;
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
                    self.draw_scene_mark(surface, mark, item.origin)
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
            SceneMark::Text(_) => Err(AvengerPdfError::UnsupportedFeature(
                "text marks are not implemented in avenger-pdf-krilla yet".to_string(),
            )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
    use avenger_common::{
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_image::RgbaImage;
    use avenger_scenegraph::marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        rect::SceneRectMark,
        text::SceneTextMark,
    };

    fn empty_scene_graph(width: f32, height: f32) -> SceneGraph {
        SceneGraph {
            width,
            height,
            origin: [0.0, 0.0],
            marks: Vec::new(),
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
    fn text_marks_report_not_implemented_yet() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark::default().into()],
        };

        let err = PdfRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(err, AvengerPdfError::UnsupportedFeature(_)));
    }
}
