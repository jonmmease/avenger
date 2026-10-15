use std::{collections::HashMap, sync::Arc};

use avenger_color::{ColorOrGradient, GradientStop};
use avenger_common::types::{FillRule, StrokeCap, StrokeJoin, SCENE_MITER_LIMIT};
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        group::Clip,
        image::SceneImageMark,
        mark::SceneMark,
        mesh_image::SceneMeshImageMark,
        pattern::{PatternFill, PatternLayerOperation},
        text::{text_origin, SceneTextMark},
    },
    render::{
        items::{path_items, path_mark_items, PathItem},
        order::{SceneDisplayList, SceneDisplayMark},
        paint::{resolve_paint, GradientBounds, ResolvedPaint},
        pattern::{
            build_layered_pattern_geometry, LayeredPatternGeometry, PatternCoverageLayer,
            PatternCoveragePrimitive, PatternRect, PatternRenderContext,
        },
        stroke::dash_pattern,
        text::TextShape,
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::{FontEmbedding, FontRef, LabelEngine, PdfItem, PdfText};
use krilla::{
    color::rgb,
    geom::{Path as PdfPath, PathBuilder, Point, Rect, Size, Transform},
    image::{BitsPerComponent, CustomImage, Image, ImageColorspace},
    mask::{Mask, MaskType},
    num::NormalizedF32,
    page::PageSettings,
    paint::{Fill, LineCap, LineJoin, Paint, SpreadMethod, Stop, Stroke, StrokeDash},
    stream::Stream,
    surface::Surface,
    text::{Font, GlyphId, KrillaGlyph, Tag},
    Data, Document,
};
use lyon_algorithms::aabb::bounding_box;
use lyon_path::{Event, Path};

use crate::{error::AvengerPdfError, options::PdfRenderOptions};

/// Export scene graphs as PDF documents with embedded fonts and selectable text.
#[derive(Debug, Clone, Default)]
pub struct PdfRenderer {
    options: PdfRenderOptions,
}

impl PdfRenderer {
    /// Create a renderer with the default options, which include a white background.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set output options.
    pub fn with_options(mut self, options: PdfRenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Export one scene as a one-page document, where a scene unit is a point, laying out its text
    /// with `text_engine`, which should be the engine that laid out the scene.
    pub fn render_scene_graph(
        &self,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<Vec<u8>, AvengerPdfError> {
        let [width, height] = [scene_graph.width, scene_graph.height];
        let page_settings = PageSettings::from_wh(width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;
        let mut document = Document::new();
        let mut page = document.start_page_with(page_settings);
        PdfWriter {
            surface: page.surface(),
            text_engine,
            fonts: HashMap::new(),
        }
        .draw_scene(scene_graph, self.options.background)?;
        page.finish();
        Ok(document.finish()?)
    }
}

/// The page as it is drawn, and the fonts its text has loaded.
struct PdfWriter<'a, 'b> {
    surface: Surface<'b>,
    text_engine: &'a LabelEngine,
    fonts: HashMap<FontRef, Font>,
}

impl PdfWriter<'_, '_> {
    fn draw_scene(
        mut self,
        scene_graph: &SceneGraph,
        background: Option<[f32; 4]>,
    ) -> Result<(), AvengerPdfError> {
        if let Some(color) = background.filter(|color| color[3] > 0.0) {
            let page = Rect::from_xywh(0.0, 0.0, scene_graph.width, scene_graph.height)
                .expect("the page size is valid");
            let mut builder = PathBuilder::new();
            builder.push_rect(page);
            if let Some(page) = builder.finish() {
                let fill = color_fill(color, FillRule::NonZero);
                draw(&mut self.surface, &page, Some(fill), None);
            }
        }
        for item in SceneDisplayList::from_scene_graph(scene_graph).ordered_items() {
            let clip = match &item.clip {
                Clip::None => None,
                // A clip that encloses nothing hides the item.
                clip => match clip_path(clip) {
                    Some(clip) => Some(clip),
                    None => continue,
                },
            };
            if let Some(path) = &clip {
                self.surface
                    .push_clip_path(path, &krilla::paint::FillRule::NonZero);
            }
            let result = self.draw_item(&item.mark, item.origin, item.pattern_origin);
            if clip.is_some() {
                self.surface.pop();
            }
            result?;
        }
        self.surface.finish();
        Ok(())
    }

    fn draw_item(
        &mut self,
        mark: &SceneDisplayMark,
        origin: [f32; 2],
        pattern_origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        let paths = match mark {
            SceneDisplayMark::OwnedGroupPath(mark) => path_mark_items(mark, origin),
            SceneDisplayMark::Borrowed(SceneMark::Text(mark)) => {
                return self.draw_text_mark(mark, origin);
            }
            SceneDisplayMark::Borrowed(SceneMark::Image(mark)) => {
                return self.draw_image_mark(mark, origin);
            }
            SceneDisplayMark::Borrowed(SceneMark::MeshImage(mark)) => {
                return self.draw_mesh_image_mark(mark, origin);
            }
            SceneDisplayMark::Borrowed(mark) => path_items(mark, origin).map_err(geometry_error)?,
        };
        for path in &paths {
            self.draw_path_item(path, pattern_origin)?;
        }
        Ok(())
    }

    fn draw_path_item(
        &mut self,
        item: &PathItem,
        group_origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        let Some(path) = pdf_path(&item.path) else {
            return Ok(());
        };
        let fill = match item.fill {
            Some(fill) => item_paint(fill.paint, item)?.map(|(paint, opacity)| Fill {
                paint,
                opacity,
                rule: krilla::paint::FillRule::NonZero,
            }),
            None => None,
        };
        let outline = item.stroke_outline().map_err(geometry_error)?;
        // Strokes whose caps native strokes would draw differently draw as their filled outline.
        let stroke = match item.stroke.filter(|_| outline.is_none()) {
            Some(stroke) => item_paint(stroke.paint, item)?.map(|(paint, opacity)| Stroke {
                paint,
                width: stroke.width,
                miter_limit: SCENE_MITER_LIMIT,
                line_cap: line_cap(stroke.cap),
                line_join: line_join(stroke.join),
                opacity,
                dash: stroke
                    .dash
                    .and_then(dash_pattern)
                    .map(|array| StrokeDash { array, offset: 0.0 }),
            }),
            None => None,
        };
        match item.pattern(group_origin) {
            None => draw(&mut self.surface, &path, fill, stroke),
            Some((pattern, context)) => {
                // The pattern draws between the fill and the stroke.
                draw(&mut self.surface, &path, fill, None);
                self.draw_pattern(&path, pattern, &context)?;
                draw(&mut self.surface, &path, None, stroke);
            }
        }
        if let (Some(outline), Some(stroke)) = (outline, item.stroke) {
            let fill = item_paint(stroke.paint, item)?;
            if let (Some(outline), Some((paint, opacity))) = (pdf_path(&outline), fill) {
                let fill = Fill {
                    paint,
                    opacity,
                    rule: krilla::paint::FillRule::NonZero,
                };
                draw(&mut self.surface, &outline, Some(fill), None);
            }
        }
        Ok(())
    }

    /// Draws a pattern's ink over the host path.
    fn draw_pattern(
        &mut self,
        host: &PdfPath,
        pattern: &PatternFill,
        context: &PatternRenderContext,
    ) -> Result<(), AvengerPdfError> {
        let Some(geometry) = build_layered_pattern_geometry(pattern, context)? else {
            return Ok(());
        };
        let [r, g, b, opacity] = geometry.ink;
        if opacity <= 0.0 {
            return Ok(());
        }
        let Some(bounds) = rect_path(context.host_bounds) else {
            return Ok(());
        };
        let ink = [r, g, b, 1.0];
        let surface = &mut self.surface;
        surface.push_clip_path(host, &krilla::paint::FillRule::NonZero);
        // Primitives paint opaque ink, and the pattern applies the ink's opacity once.
        surface.push_opacity(normalized(opacity));
        if geometry
            .layers
            .iter()
            .all(|layer| layer.operation == PatternLayerOperation::Add)
        {
            for layer in &geometry.layers {
                draw_pattern_primitives(surface, layer, ink);
            }
        } else {
            let mask = pattern_mask(surface, &geometry, &bounds);
            fill_rect(surface, &bounds, ink, Some(mask));
        }
        surface.pop();
        surface.pop();
        Ok(())
    }

    fn draw_text_mark(
        &mut self,
        mark: &SceneTextMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for label in mark.labels() {
            let anchor = [label.position[0] + origin[0], label.position[1] + origin[1]];
            // A label with a non-finite position or angle draws nothing.
            if !anchor
                .iter()
                .chain([&label.angle])
                .all(|value| value.is_finite())
            {
                continue;
            }
            let (bounds, pdf) = self.text_engine.pdf(&label.label)?;
            let [x, top] = text_origin(&bounds, anchor, label.align, label.baseline);
            // The label's origin moves to `[x, top]`, and the label rotates about its anchor.
            let (sin, cos) = label.angle.to_radians().sin_cos();
            let [dx, dy] = [x - anchor[0], top - anchor[1]];
            self.surface.push_transform(&Transform::from_row(
                cos,
                sin,
                -sin,
                cos,
                anchor[0] + cos * dx - sin * dy,
                anchor[1] + sin * dx + cos * dy,
            ));
            let result = pdf.items.iter().try_for_each(|item| match item {
                PdfItem::Text(run) => self.draw_glyph_run(run, &pdf.fonts[run.font]),
                PdfItem::Path(path) => {
                    draw_text_shape(&mut self.surface, &TextShape::new(path));
                    Ok(())
                }
            });
            self.surface.pop();
            result?;
        }
        Ok(())
    }

    fn draw_glyph_run(&mut self, run: &PdfText, face: &FontRef) -> Result<(), AvengerPdfError> {
        if run.glyphs.is_empty() {
            return Ok(());
        }
        let font = self.font(face)?;
        // The run's transform places its origin, and may rotate or mirror it in math.
        let t = run.transform;
        self.surface
            .push_transform(&Transform::from_row(t.sx, t.ky, t.kx, t.sy, t.tx, t.ty));
        // Without a fill, the glyphs draw black, so a transparent run still sets one.
        self.surface
            .set_fill(Some(color_fill(run.fill.to_rgba(), FillRule::NonZero)));
        self.surface.set_stroke(None);
        self.surface.draw_glyphs(
            Point::from_xy(0.0, 0.0),
            &glyphs(run),
            font,
            &run.text,
            run.size,
            false,
        );
        self.surface.pop();
        Ok(())
    }

    fn font(&mut self, face: &FontRef) -> Result<Font, AvengerPdfError> {
        if let Some(font) = self.fonts.get(face) {
            return Ok(font.clone());
        }
        // Krilla embeds a subset of each face.
        let forbidden = match face.embedding() {
            FontEmbedding::Allowed => None,
            FontEmbedding::NoSubsetting => Some("subsetting"),
            FontEmbedding::Forbidden => Some("embedding"),
        };
        if let Some(action) = forbidden {
            return Err(AvengerPdfError::Font(format!(
                "the license of font {} forbids {action} it",
                face.family()
            )));
        }
        let coordinates: Vec<_> = face
            .variations()
            .into_iter()
            .map(|(tag, value)| (Tag::new(&tag), value))
            .collect();
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(face.data().clone());
        let font =
            Font::new_variable(Data::from(data), face.index(), &coordinates).ok_or_else(|| {
                AvengerPdfError::Font(format!("failed to load font {}", face.family()))
            })?;
        self.fonts.insert(face.clone(), font.clone());
        Ok(font)
    }

    fn draw_image_mark(
        &mut self,
        mark: &SceneImageMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        for (image, path) in mark.image_iter().zip(mark.transformed_path_iter(origin)) {
            let bbox = bounding_box(&path);
            let placement = [bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y];
            // An image with a non-finite placement draws nothing.
            if placement.iter().all(|value| value.is_finite()) {
                self.draw_image(image, placement, mark.smooth)?;
            }
        }
        Ok(())
    }

    fn draw_mesh_image_mark(
        &mut self,
        mark: &SceneMeshImageMark,
        origin: [f32; 2],
    ) -> Result<(), AvengerPdfError> {
        // PDF has no textured mesh, so the mesh draws as an image rasterized at twice its size.
        if let Some((raster, placement)) = mark.rasterize(origin, 2.0) {
            self.draw_image(&raster, placement, mark.smooth)?;
        }
        Ok(())
    }

    /// Draws an image stretched over `[min_x, min_y, max_x, max_y]`.
    fn draw_image(
        &mut self,
        image: &RgbaImage,
        [min_x, min_y, max_x, max_y]: [f32; 4],
        smooth: bool,
    ) -> Result<(), AvengerPdfError> {
        let image = pdf_image(image, smooth)?;
        let Some(size) = Size::from_wh(max_x - min_x, max_y - min_y) else {
            return Ok(());
        };
        self.surface
            .push_transform(&Transform::from_translate(min_x, min_y));
        self.surface.draw_image(image, size);
        self.surface.pop();
        Ok(())
    }
}

/// Draws a path, unless neither its fill nor its stroke draws.
fn draw(surface: &mut Surface, path: &PdfPath, fill: Option<Fill>, stroke: Option<Stroke>) {
    if fill.is_none() && stroke.is_none() {
        return;
    }
    surface.set_fill(fill);
    surface.set_stroke(stroke);
    surface.draw_path(path);
}

fn draw_text_shape(surface: &mut Surface, shape: &TextShape) {
    let Some(path) = pdf_path(&shape.path) else {
        return;
    };
    let fill = shape
        .fill
        .filter(|color| color[3] > 0.0)
        .map(|color| color_fill(color, shape.fill_rule));
    let stroke = shape
        .stroke
        .as_ref()
        .filter(|stroke| stroke.width > 0.0 && stroke.color[3] > 0.0)
        .map(|stroke| Stroke {
            paint: rgb_color(stroke.color).into(),
            width: stroke.width,
            miter_limit: stroke.miter_limit,
            line_cap: line_cap(stroke.cap),
            line_join: line_join(stroke.join),
            opacity: normalized(stroke.color[3]),
            dash: stroke
                .dash
                .as_deref()
                .and_then(dash_pattern)
                .map(|array| StrokeDash {
                    array,
                    offset: stroke.dash_offset,
                }),
        });
    draw(surface, &path, fill, stroke);
}

/// The luminosity mask that combines a pattern's layers over `bounds`.
fn pattern_mask(
    surface: &mut Surface,
    geometry: &LayeredPatternGeometry,
    bounds: &PdfPath,
) -> Stream {
    let [black, white] = [[0.0, 0.0, 0.0, 1.0], [1.0; 4]];
    let mut previous: Option<Stream> = None;
    for layer in &geometry.layers {
        let layer_mask = mask_stream(surface, |surface| {
            fill_rect(surface, bounds, black, None);
            draw_pattern_primitives(surface, layer, white);
        });
        let mask = mask_stream(surface, |surface| {
            fill_rect(surface, bounds, black, None);
            if let Some(previous) = &previous {
                fill_rect(surface, bounds, white, Some(previous.clone()));
            }
            let layer_color = if layer.operation == PatternLayerOperation::Subtract {
                black
            } else {
                white
            };
            fill_rect(surface, bounds, layer_color, Some(layer_mask.clone()));
            if let (PatternLayerOperation::Xor, Some(previous)) = (layer.operation, &previous) {
                surface.push_mask(Mask::new(previous.clone(), MaskType::Luminosity));
                fill_rect(surface, bounds, black, Some(layer_mask.clone()));
                surface.pop();
            }
        });
        previous = Some(mask);
    }
    previous.expect("pattern geometry has a layer")
}

fn mask_stream(surface: &mut Surface, draw: impl FnOnce(&mut Surface)) -> Stream {
    let mut builder = surface.stream_builder();
    let mut mask_surface = builder.surface();
    draw(&mut mask_surface);
    mask_surface.finish();
    builder.finish()
}

/// Fills `bounds` with an opaque color, under a luminosity mask when one is given.
fn fill_rect(surface: &mut Surface, bounds: &PdfPath, color: [f32; 4], mask: Option<Stream>) {
    let masked = mask.is_some();
    if let Some(mask) = mask {
        surface.push_mask(Mask::new(mask, MaskType::Luminosity));
    }
    draw(
        surface,
        bounds,
        Some(color_fill(color, FillRule::NonZero)),
        None,
    );
    if masked {
        surface.pop();
    }
}

fn draw_pattern_primitives(surface: &mut Surface, layer: &PatternCoverageLayer, ink: [f32; 4]) {
    for primitive in &layer.primitives {
        let Some(path) = pdf_path(primitive.path()) else {
            continue;
        };
        match primitive {
            PatternCoveragePrimitive::Filled(_) => draw(
                surface,
                &path,
                Some(color_fill(ink, FillRule::NonZero)),
                None,
            ),
            PatternCoveragePrimitive::Stroked { stroke_width, .. } => {
                let stroke = Stroke {
                    paint: rgb_color(ink).into(),
                    width: *stroke_width,
                    miter_limit: SCENE_MITER_LIMIT,
                    line_cap: LineCap::Butt,
                    line_join: LineJoin::Miter,
                    opacity: normalized(ink[3]),
                    dash: None,
                };
                draw(surface, &path, None, Some(stroke))
            }
        }
    }
}

/// How `paint` draws over the item, or `None` when it draws nothing.
fn item_paint(
    paint: &ColorOrGradient,
    item: &PathItem,
) -> Result<Option<(Paint, NormalizedF32)>, AvengerPdfError> {
    Ok(
        resolve_paint(paint, item.gradients, item.bounds)?.map(|paint| match paint {
            ResolvedPaint::Solid(color) => (rgb_color(color).into(), normalized(color[3])),
            ResolvedPaint::Linear(gradient, bounds) => {
                let [width, height] = size(bounds);
                let paint = krilla::paint::LinearGradient {
                    x1: gradient.x0,
                    y1: gradient.y0,
                    x2: gradient.x1,
                    y2: gradient.y1,
                    // Controls are in the unit box, which the transform maps onto the bounds.
                    transform: Transform::from_row(
                        width,
                        0.0,
                        0.0,
                        height,
                        bounds.min[0],
                        bounds.min[1],
                    ),
                    spread_method: SpreadMethod::Pad,
                    stops: stops(&gradient.stops),
                    anti_alias: false,
                };
                (paint.into(), NormalizedF32::ONE)
            }
            ResolvedPaint::Radial(gradient, bounds) => {
                let [width, height] = size(bounds).map(f64::from);
                let dx = gradient.x1 as f64 - gradient.x0 as f64;
                let dy = gradient.y1 as f64 - gradient.y0 as f64;
                let dr = gradient.r1 as f64 - gradient.r0 as f64;
                // PDF viewers use absolute zero thresholds in the radial solver, so the circles
                // scale until their deltas reach one unit, and the transform undoes the scale.
                let scale = width.max(1.0 / dx.abs().max(dy.abs()).max(dr.abs()));
                let paint = krilla::paint::RadialGradient {
                    fx: 0.0,
                    fy: 0.0,
                    fr: (gradient.r0 as f64 * scale) as f32,
                    cx: (dx * scale) as f32,
                    cy: (dy * scale) as f32,
                    cr: (gradient.r1 as f64 * scale) as f32,
                    transform: Transform::from_row(
                        (width / scale) as f32,
                        0.0,
                        0.0,
                        (height / scale) as f32,
                        (bounds.min[0] as f64 + gradient.x0 as f64 * width) as f32,
                        (bounds.min[1] as f64 + gradient.y0 as f64 * height) as f32,
                    ),
                    spread_method: SpreadMethod::Pad,
                    stops: stops(&gradient.stops),
                    anti_alias: false,
                };
                (paint.into(), NormalizedF32::ONE)
            }
        }),
    )
}

fn size(bounds: GradientBounds) -> [f32; 2] {
    [bounds.max[0] - bounds.min[0], bounds.max[1] - bounds.min[1]]
}

fn stops(stops: &[GradientStop]) -> Vec<Stop> {
    stops
        .iter()
        .map(|stop| Stop {
            offset: normalized(stop.offset),
            color: rgb_color(stop.color).into(),
            opacity: normalized(stop.color[3]),
        })
        .collect()
}

fn color_fill(color: [f32; 4], rule: FillRule) -> Fill {
    Fill {
        paint: rgb_color(color).into(),
        opacity: normalized(color[3]),
        rule: pdf_fill_rule(rule),
    }
}

/// A straight-alpha color's RGB channels.
fn rgb_color(color: [f32; 4]) -> rgb::Color {
    let [r, g, b] =
        [color[0], color[1], color[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    rgb::Color::new(r, g, b)
}

fn normalized(value: f32) -> NormalizedF32 {
    NormalizedF32::new(value.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

fn pdf_fill_rule(rule: FillRule) -> krilla::paint::FillRule {
    match rule {
        FillRule::NonZero => krilla::paint::FillRule::NonZero,
        FillRule::EvenOdd => krilla::paint::FillRule::EvenOdd,
    }
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
        StrokeJoin::Miter => LineJoin::Miter,
        StrokeJoin::Round => LineJoin::Round,
        StrokeJoin::Bevel => LineJoin::Bevel,
    }
}

/// A clip's area, which it encloses under the nonzero rule, or `None` when it encloses nothing.
fn clip_path(clip: &Clip) -> Option<PdfPath> {
    match clip {
        Clip::None => None,
        Clip::Rect {
            x,
            y,
            width,
            height,
        } => {
            let mut builder = PathBuilder::new();
            builder.push_rect(Rect::from_xywh(*x, *y, *width, *height)?);
            builder.finish()
        }
        Clip::Path(path) => pdf_path(path),
    }
}

fn rect_path(bounds: PatternRect) -> Option<PdfPath> {
    let mut builder = PathBuilder::new();
    builder.push_rect(Rect::from_xywh(
        bounds.min_x(),
        bounds.min_y(),
        bounds.width,
        bounds.height,
    )?);
    builder.finish()
}

/// A lyon path as a PDF path, or `None` when it has no segments.
fn pdf_path(path: &Path) -> Option<PdfPath> {
    let mut builder = PathBuilder::new();
    for event in path.iter() {
        match event {
            Event::Begin { at } => builder.move_to(at.x, at.y),
            Event::Line { to, .. } => builder.line_to(to.x, to.y),
            Event::Quadratic { ctrl, to, .. } => builder.quad_to(ctrl.x, ctrl.y, to.x, to.y),
            Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => builder.cubic_to(ctrl1.x, ctrl1.y, ctrl2.x, ctrl2.y, to.x, to.y),
            Event::End { close: true, .. } => builder.close(),
            Event::End { close: false, .. } => {}
        }
    }
    builder.finish()
}

/// An RGBA image with its alpha as a soft mask.
// Dimensions are part of the identity, so images with equal pixel bytes stay distinct.
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

fn pdf_image(image: &RgbaImage, smooth: bool) -> Result<Image, AvengerPdfError> {
    let len = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if image.width == 0 || image.height == 0 || len != Some(image.data.len()) {
        return Err(AvengerPdfError::Image("invalid RGBA image buffer".into()));
    }
    let alpha: Vec<_> = image.data.chunks_exact(4).map(|pixel| pixel[3]).collect();
    Image::from_custom(
        PdfRgbaImage {
            width: image.width,
            height: image.height,
            rgb: color_channel(image, smooth).into(),
            alpha: alpha.into(),
        },
        smooth,
    )
    .map_err(AvengerPdfError::Image)
}

/// The image's RGB channels. Viewers filter a smooth image's color apart from its soft mask, so its
/// transparent pixels take the alpha-weighted color of their neighbors, and their hidden color
/// doesn't tint the visible pixels beside them.
fn color_channel(image: &RgbaImage, smooth: bool) -> Vec<u8> {
    let [width, height] = [image.width as usize, image.height as usize];
    let pixel = |x: usize, y: usize| &image.data[(y * width + x) * 4..][..4];
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let own = pixel(x, y);
            if !smooth || own[3] > 0 {
                rgb.extend_from_slice(&own[..3]);
                continue;
            }
            let (mut sum, mut weight) = ([0u32; 3], 0u32);
            for neighbor_y in y.saturating_sub(1)..(y + 2).min(height) {
                for neighbor_x in x.saturating_sub(1)..(x + 2).min(width) {
                    let neighbor = pixel(neighbor_x, neighbor_y);
                    let alpha = u32::from(neighbor[3]);
                    for (channel, value) in sum.iter_mut().zip(neighbor) {
                        *channel += u32::from(*value) * alpha;
                    }
                    weight += alpha;
                }
            }
            if weight == 0 {
                rgb.extend_from_slice(&own[..3]);
            } else {
                rgb.extend(sum.map(|channel| ((channel + weight / 2) / weight) as u8));
            }
        }
    }
    rgb
}

/// A run's glyphs, relative to its origin.
fn glyphs(run: &PdfText) -> Vec<KrillaGlyph> {
    let size = run.size.max(0.0001);
    // Outside ASCII, shaping can reorder and merge clusters, so every glyph maps to the run's whole
    // text.
    let whole_text = !run.text.is_ascii();
    let mut cursor = 0.0;
    run.glyphs
        .iter()
        .map(|glyph| {
            let krilla_glyph = KrillaGlyph::new(
                GlyphId::new(u32::from(glyph.id)),
                glyph.x_advance / size,
                (glyph.position.x - cursor) / size,
                -glyph.position.y / size,
                0.0,
                if whole_text {
                    0..run.text.len()
                } else {
                    glyph.range.clone()
                },
                None,
            );
            cursor += glyph.x_advance;
            krilla_glyph
        })
        .collect()
}

fn geometry_error(error: impl std::fmt::Display) -> AvengerPdfError {
    AvengerPdfError::InvalidGeometry(error.to_string())
}

#[cfg(test)]
mod tests {
    use avenger_typst_label::{PdfGlyph, Point, Transform};

    use super::*;

    #[test]
    fn glyph_offsets_are_relative_to_the_pen_and_y_up() {
        // A glyph 3pt right of its run's origin and 12pt below it, which is y-down.
        let run = PdfText {
            font: 0,
            size: 10.0,
            fill: avenger_color::AbsoluteColor::from_rgba([0.0, 0.0, 0.0, 1.0]),
            transform: Transform::translate(5.0, 7.0),
            text: "A".to_string(),
            glyphs: vec![PdfGlyph {
                id: 1,
                position: Point::new(3.0, 12.0),
                x_advance: 10.0,
                range: 0..1,
                source: 0..1,
            }],
        };
        let glyphs = glyphs(&run);
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].x_offset, 0.3);
        assert_eq!(glyphs[0].y_offset, -1.2);
    }
}
