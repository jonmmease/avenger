use std::{
    collections::HashMap,
    fmt,
    hash::{DefaultHasher, Hash, Hasher},
    io::Cursor,
    sync::Arc,
};

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::pattern::{PatternFill, PatternLayerOperation};
use avenger_common::types::{FillRule, StrokeCap, StrokeJoin, SCENE_MITER_LIMIT};
use avenger_image::{ImageResolver, RgbaImage};
use avenger_scenegraph::{
    marks::{
        group::Clip,
        image::SceneImageMark,
        mark::SceneMark,
        mesh_image::SceneMeshImageMark,
        text::{text_origin, SceneTextMark},
    },
    render::{
        items::{path_items, path_mark_items, PathItem, StrokeItem},
        order::{SceneDisplayList, SceneDisplayMark},
        paint::{resolve_paint, GradientBounds, ResolvedPaint},
        pattern::{
            build_layered_pattern_geometry, LayeredPatternGeometry, PatternCoverageLayer,
            PatternCoveragePrimitive, PatternRect, PatternRenderContext,
        },
        stroke::dash_pattern,
        text::TextShape,
    },
    scene_graph::{ResolvedImages, SceneGraph},
};
use avenger_typst_label::{FontStyle, ImageItem, LabelEngine, SvgItem, TextRun};
use base64::{prelude::BASE64_STANDARD, Engine};
use lyon_algorithms::aabb::bounding_box;

use crate::{
    error::AvengerSvgError,
    fonts::SvgFontCollector,
    options::SvgRenderOptions,
    path::{format_number, lyon_path_to_svg_d, push_number, push_numbers},
    style::{escape_attr, escape_text, push_color_attrs, push_stop_color_attrs},
};

/// Export scene graphs as self-contained SVG documents.
#[derive(Clone, Default)]
pub struct SvgRenderer {
    options: SvgRenderOptions,
    images: Option<Arc<dyn ImageResolver>>,
}

impl fmt::Debug for SvgRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SvgRenderer")
            .field("options", &self.options)
            .field("images", &self.images.is_some())
            .finish()
    }
}

impl SvgRenderer {
    /// Create a renderer with the default options, which include a white background.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set output options.
    pub fn with_options(mut self, options: SvgRenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Draw image resources with the images that `images` holds when the export runs. A
    /// resource that isn't ready fails the export, or draws its placeholder under
    /// [`SvgRenderOptions::allow_unready_images`]. Without a resolver, no resource is ready.
    pub fn with_images(mut self, images: Arc<dyn ImageResolver>) -> Self {
        self.images = Some(images);
        self
    }

    /// Export one scene, laying out its text with `text_engine`, which should be the engine that
    /// laid out the scene.
    pub fn render_scene_graph(
        &self,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<String, AvengerSvgError> {
        let [width, height] = [scene_graph.width, scene_graph.height];
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return Err(AvengerSvgError::InvalidGeometry(
                "SVG dimensions must be positive and finite".into(),
            ));
        }
        let images = scene_graph.resolve_images(self.images.as_deref());
        if !self.options.allow_unready_images && !images.unready.is_empty() {
            return Err(AvengerSvgError::ImagesNotReady(images.unready));
        }
        let mut writer = SvgWriter {
            options: &self.options,
            images: &images,
            text_engine,
            defs: SvgDefs::default(),
            fonts: SvgFontCollector::default(),
            body: String::new(),
        };
        if let Some(color) = self.options.background {
            writer.body.push_str(r#"<rect x="0" y="0" width=""#);
            push_number(&mut writer.body, width, self.options.precision)?;
            writer.body.push_str(r#"" height=""#);
            push_number(&mut writer.body, height, self.options.precision)?;
            writer.body.push('"');
            push_color_attrs(&mut writer.body, "fill", color, self.options.precision)?;
            writer.body.push_str("/>\n");
        }
        for item in SceneDisplayList::from_scene_graph(scene_graph).ordered_items() {
            let clip = writer.defs.clip_id(&item.clip, self.options.precision)?;
            let clip = clip.as_deref();
            let paths = match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => path_mark_items(mark, item.origin),
                SceneDisplayMark::Borrowed(SceneMark::Text(mark)) => {
                    writer.write_text_mark(mark, item.origin, clip)?;
                    continue;
                }
                SceneDisplayMark::Borrowed(SceneMark::Image(mark)) => {
                    writer.write_image_mark(mark, item.origin, clip)?;
                    continue;
                }
                SceneDisplayMark::Borrowed(SceneMark::MeshImage(mark)) => {
                    writer.write_mesh_image_mark(mark, item.origin, clip)?;
                    continue;
                }
                SceneDisplayMark::Borrowed(mark) => {
                    path_items(mark, item.origin).map_err(geometry_error)?
                }
            };
            for path in &paths {
                writer.write_path_item(path, clip, item.pattern_origin)?;
            }
        }
        writer.defs.font_css = writer.fonts.font_face_css()?;

        let precision = self.options.precision;
        let mut output = String::new();
        output.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" width=""#);
        output.push_str(&format_number(width, precision)?);
        output.push_str(r#"" height=""#);
        output.push_str(&format_number(height, precision)?);
        output.push_str(r#"" viewBox="0 0 "#);
        output.push_str(&format_number(width, precision)?);
        output.push(' ');
        output.push_str(&format_number(height, precision)?);
        output.push_str(&format!("\" stroke-miterlimit=\"{SCENE_MITER_LIMIT}\">\n"));
        writer.defs.write_defs(&mut output);
        output.push_str(&writer.body);
        output.push_str("</svg>\n");
        Ok(output)
    }
}

/// The document as it is written: its definitions, the fonts its text uses, and its body.
struct SvgWriter<'a> {
    options: &'a SvgRenderOptions,
    /// The ready images of the scene's resources.
    images: &'a ResolvedImages,
    text_engine: &'a LabelEngine,
    defs: SvgDefs,
    fonts: SvgFontCollector,
    body: String,
}

impl SvgWriter<'_> {
    fn write_path_item(
        &mut self,
        item: &PathItem,
        clip: Option<&str>,
        group_origin: [f32; 2],
    ) -> Result<(), AvengerSvgError> {
        let d = lyon_path_to_svg_d(&item.path, self.options.precision)?;
        if d.is_empty() {
            return Ok(());
        }
        let fill = item.fill.map(|fill| fill.paint);
        let outline = item.stroke_outline().map_err(geometry_error)?;
        // Strokes whose caps native strokes would draw differently draw as their filled outline.
        let stroke = item.stroke.filter(|_| outline.is_none());
        match item.pattern(group_origin) {
            None => self.write_path(&d, fill, stroke.as_ref(), item, clip)?,
            Some((pattern, context)) => {
                // The pattern draws between the fill and the stroke.
                self.write_path(&d, fill, None, item, clip)?;
                self.write_pattern(item, &d, pattern, &context, clip)?;
                self.write_path(&d, None, stroke.as_ref(), item, clip)?;
            }
        }
        if let (Some(outline), Some(stroke)) = (outline, item.stroke) {
            let d = lyon_path_to_svg_d(&outline, self.options.precision)?;
            self.write_path(&d, Some(stroke.paint), None, item, clip)?;
        }
        Ok(())
    }

    /// Writes a path element, unless neither its fill nor its stroke draws.
    fn write_path(
        &mut self,
        d: &str,
        fill: Option<&ColorOrGradient>,
        stroke: Option<&StrokeItem>,
        item: &PathItem,
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let mut attrs = String::new();
        let fills = match fill {
            Some(paint) => self.push_paint(&mut attrs, "fill", paint, item)?,
            None => false,
        };
        if !fills {
            attrs.push_str(r#" fill="none""#);
        }
        let strokes = match stroke {
            Some(stroke) => self.push_paint(&mut attrs, "stroke", stroke.paint, item)?,
            None => false,
        };
        if !fills && !strokes {
            return Ok(());
        }
        if let Some(stroke) = stroke.filter(|_| strokes) {
            let precision = self.options.precision;
            attrs.push_str(r#" stroke-width=""#);
            push_number(&mut attrs, stroke.width, precision)?;
            attrs.push('"');
            if stroke.cap != StrokeCap::Butt {
                attrs.push_str(&format!(r#" stroke-linecap="{}""#, cap_value(stroke.cap)));
            }
            if stroke.join != StrokeJoin::Miter {
                attrs.push_str(&format!(
                    r#" stroke-linejoin="{}""#,
                    join_value(stroke.join)
                ));
            }
            if let Some(dash) = stroke.dash.and_then(dash_pattern) {
                attrs.push_str(r#" stroke-dasharray=""#);
                push_numbers(&mut attrs, &dash, precision)?;
                attrs.push('"');
            }
        }
        self.body.push_str(r#"<path d=""#);
        self.body.push_str(d);
        self.body.push('"');
        self.body.push_str(&attrs);
        push_clip_attr(&mut self.body, clip);
        self.body.push_str("/>\n");
        Ok(())
    }

    /// Writes the attributes that paint `attr` with `paint`, and returns whether it draws.
    fn push_paint(
        &mut self,
        attrs: &mut String,
        attr: &str,
        paint: &ColorOrGradient,
        item: &PathItem,
    ) -> Result<bool, AvengerSvgError> {
        let precision = self.options.precision;
        match resolve_paint(paint, item.gradients, item.bounds)? {
            None => Ok(false),
            Some(ResolvedPaint::Solid(color)) => {
                push_color_attrs(attrs, attr, color, precision)?;
                Ok(true)
            }
            Some(gradient) => {
                let id = self.defs.gradient_id(gradient, precision)?;
                attrs.push_str(&format!(r#" {attr}="url(#{id})""#));
                Ok(true)
            }
        }
    }

    /// Writes a pattern's ink over the host path, under the host's clip.
    fn write_pattern(
        &mut self,
        item: &PathItem,
        host_d: &str,
        pattern: &PatternFill,
        context: &PatternRenderContext,
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let Some(geometry) = build_layered_pattern_geometry(pattern, context)? else {
            return Ok(());
        };
        let [r, g, b, opacity] = geometry.ink;
        if opacity <= 0.0 {
            return Ok(());
        }
        let precision = self.options.precision;
        if geometry
            .layers
            .iter()
            .all(|layer| layer.operation == PatternLayerOperation::Add)
        {
            // Overlapping primitives paint opaque ink, and the group applies the ink's opacity once.
            let host_clip = self
                .defs
                .clip_id(&Clip::Path(item.path.clone()), precision)?;
            if clip.is_some() {
                self.body.push_str("<g");
                push_clip_attr(&mut self.body, clip);
                self.body.push_str(">\n");
            }
            self.body.push_str("<g");
            push_clip_attr(&mut self.body, host_clip.as_deref());
            if opacity < 1.0 {
                self.body.push_str(r#" opacity=""#);
                push_number(&mut self.body, opacity, precision)?;
                self.body.push('"');
            }
            self.body.push_str(">\n");
            for layer in &geometry.layers {
                push_pattern_primitives(&mut self.body, layer, [r, g, b, 1.0], precision)?;
            }
            self.body.push_str("</g>\n");
            if clip.is_some() {
                self.body.push_str("</g>\n");
            }
        } else {
            let mask = self
                .defs
                .write_pattern_mask(&geometry, context.host_bounds, precision)?;
            self.body.push_str(r#"<path d=""#);
            self.body.push_str(host_d);
            self.body.push('"');
            push_color_attrs(&mut self.body, "fill", geometry.ink, precision)?;
            self.body.push_str(&format!(r#" mask="url(#{mask})""#));
            push_clip_attr(&mut self.body, clip);
            self.body.push_str("/>\n");
        }
        Ok(())
    }

    fn write_text_mark(
        &mut self,
        mark: &SceneTextMark,
        origin: [f32; 2],
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let precision = self.options.precision;
        // The halos draw before the labels, so no halo covers a label of the mark.
        if let Some(halo) = mark.halo_mark(self.text_engine)? {
            for path in path_mark_items(&halo, origin) {
                self.write_path_item(&path, clip, origin)?;
            }
        }
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
            let (bounds, svg) = self.text_engine.svg(&label.label)?;
            let [x, top] = text_origin(&bounds, anchor, label.align, label.baseline);
            // The scene clip stays in scene coordinates outside the label's rotation.
            self.body.push_str("<g");
            push_clip_attr(&mut self.body, clip);
            self.body.push_str(">\n<g transform=\"rotate(");
            push_numbers(
                &mut self.body,
                &[label.angle, anchor[0], anchor[1]],
                precision,
            )?;
            self.body.push_str(") translate(");
            push_numbers(&mut self.body, &[x, top], precision)?;
            self.body.push_str(")\">\n");

            // A run and the color glyph images of its text item draw the same glyphs: as the
            // images when those are enabled, and as the run otherwise. Images of items without a
            // run always draw.
            let has_item = |text_item: usize, image: bool| {
                svg.items.iter().any(|item| match item {
                    SvgItem::Image(item) => image && item.glyph.text == text_item,
                    SvgItem::Text(run) => !image && run.text_item == text_item,
                    SvgItem::Path(_) => false,
                })
            };
            let images = self.options.color_glyph_images;
            for item in &svg.items {
                match item {
                    SvgItem::Text(run) if !(images && has_item(run.text_item, true)) => {
                        self.write_text_run(run)?
                    }
                    SvgItem::Path(path) => self.write_text_shape(&TextShape::new(path))?,
                    SvgItem::Image(image) if images || !has_item(image.glyph.text, false) => {
                        self.write_text_image(image)?
                    }
                    _ => {}
                }
            }
            self.body.push_str("</g>\n</g>\n");
        }
        Ok(())
    }

    fn write_text_run(&mut self, run: &TextRun) -> Result<(), AvengerSvgError> {
        let precision = self.options.precision;
        let font = if self.options.embed_fonts {
            self.fonts.collect_run(run)
        } else {
            run.font.family().to_string()
        };
        let x = run.x + if run.rtl { run.width } else { 0.0 };
        self.body.push_str(r#"<text x=""#);
        push_number(&mut self.body, x, precision)?;
        self.body.push_str(r#"" y=""#);
        push_number(&mut self.body, run.baseline, precision)?;
        self.body.push('"');
        push_color_attrs(&mut self.body, "fill", run.fill.to_rgba(), precision)?;
        if run.rtl {
            self.body
                .push_str(r#" direction="rtl" unicode-bidi="isolate""#);
        }
        self.body.push_str(
            r#" style="font-synthesis:none" text-anchor="start" dominant-baseline="alphabetic" font-family=""#,
        );
        self.body.push_str(&escape_attr(&font));
        self.body.push_str(r#"" font-size=""#);
        push_number(&mut self.body, run.size, precision)?;
        self.body.push_str(&format!(
            r#"" font-weight="{}" font-style="{}" xml:space="preserve">"#,
            run.weight.to_number(),
            font_style_value(run.style)
        ));
        self.body.push_str(&escape_text(&run.text));
        self.body.push_str("</text>\n");
        Ok(())
    }

    fn write_text_shape(&mut self, shape: &TextShape) -> Result<(), AvengerSvgError> {
        let precision = self.options.precision;
        let d = lyon_path_to_svg_d(&shape.path, precision)?;
        if d.is_empty() {
            return Ok(());
        }
        self.body.push_str(r#"<path d=""#);
        self.body.push_str(&d);
        self.body.push('"');
        match shape.fill {
            Some(fill) => {
                push_color_attrs(&mut self.body, "fill", fill, precision)?;
                if shape.fill_rule == FillRule::EvenOdd {
                    self.body.push_str(r#" fill-rule="evenodd""#);
                }
            }
            None => self.body.push_str(r#" fill="none""#),
        }
        if let Some(stroke) = &shape.stroke {
            push_color_attrs(&mut self.body, "stroke", stroke.color, precision)?;
            self.body.push_str(r#" stroke-width=""#);
            push_number(&mut self.body, stroke.width, precision)?;
            self.body.push_str(&format!(
                r#"" stroke-linecap="{}" stroke-linejoin="{}" stroke-miterlimit=""#,
                cap_value(stroke.cap),
                join_value(stroke.join)
            ));
            push_number(&mut self.body, stroke.miter_limit, precision)?;
            self.body.push('"');
            if let Some(dash) = stroke.dash.as_deref().and_then(dash_pattern) {
                self.body.push_str(r#" stroke-dashoffset=""#);
                push_number(&mut self.body, stroke.dash_offset, precision)?;
                self.body.push_str(r#"" stroke-dasharray=""#);
                push_numbers(&mut self.body, &dash, precision)?;
                self.body.push('"');
            }
        }
        self.body.push_str("/>\n");
        Ok(())
    }

    fn write_text_image(&mut self, image: &ImageItem) -> Result<(), AvengerSvgError> {
        let precision = self.options.precision;
        let t = image.transform;
        self.body.push_str(r#"<image x="0" y="0" width=""#);
        push_number(&mut self.body, image.size.x, precision)?;
        self.body.push_str(r#"" height=""#);
        push_number(&mut self.body, image.size.y, precision)?;
        self.body
            .push_str(r#"" preserveAspectRatio="none" href="data:image/png;base64,"#);
        self.body.push_str(&BASE64_STANDARD.encode(&image.data));
        self.body.push_str(r#"" transform="matrix("#);
        push_numbers(
            &mut self.body,
            &[t.sx, t.ky, t.kx, t.sy, t.tx, t.ty],
            precision,
        )?;
        self.body.push_str(")\"/>\n");
        Ok(())
    }

    fn write_image_mark(
        &mut self,
        mark: &SceneImageMark,
        origin: [f32; 2],
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (source, path) in mark.image_iter().zip(mark.transformed_path_iter(origin)) {
            let bbox = bounding_box(&path);
            let placement = [bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y];
            // An image with a non-finite placement, or without pixels to draw, draws nothing.
            let Some(image) = source.pixels(&|url| self.images.get(url)) else {
                continue;
            };
            if placement.iter().all(|value| value.is_finite()) {
                self.write_image(&image, placement, mark.smooth, clip)?;
            }
        }
        Ok(())
    }

    fn write_mesh_image_mark(
        &mut self,
        mark: &SceneMeshImageMark,
        origin: [f32; 2],
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        // SVG has no textured mesh, so the mark's instances draw as one image, rasterized at
        // twice the scene's scale on the page's pixel grid.
        if let Some((raster, placement)) = mark.rasterize(origin, 2.0, &|url| self.images.get(url))
        {
            self.write_image(&raster, placement, mark.smooth, clip)?;
        }
        Ok(())
    }

    /// Writes an image stretched over `[min_x, min_y, max_x, max_y]`.
    fn write_image(
        &mut self,
        image: &RgbaImage,
        [min_x, min_y, max_x, max_y]: [f32; 4],
        smooth: bool,
        clip: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let rgba = image.to_image().ok_or_else(|| {
            AvengerSvgError::ImageEncoding("invalid RGBA image buffer".to_string())
        })?;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|err| AvengerSvgError::ImageEncoding(err.to_string()))?;
        let precision = self.options.precision;
        self.body.push_str(r#"<image x=""#);
        push_number(&mut self.body, min_x, precision)?;
        self.body.push_str(r#"" y=""#);
        push_number(&mut self.body, min_y, precision)?;
        self.body.push_str(r#"" width=""#);
        push_number(&mut self.body, max_x - min_x, precision)?;
        self.body.push_str(r#"" height=""#);
        push_number(&mut self.body, max_y - min_y, precision)?;
        self.body
            .push_str(r#"" preserveAspectRatio="none" href="data:image/png;base64,"#);
        self.body
            .push_str(&BASE64_STANDARD.encode(png.into_inner()));
        self.body.push('"');
        if !smooth {
            self.body.push_str(r#" style="image-rendering:pixelated""#);
        }
        push_clip_attr(&mut self.body, clip);
        self.body.push_str("/>\n");
        Ok(())
    }
}

/// Reusable definitions: gradients, clip paths, pattern masks, and font faces.
#[derive(Default)]
struct SvgDefs {
    font_css: String,
    body: String,
    gradient_ids: DefIds<(Gradient, GradientBounds)>,
    clip_ids: DefIds<Clip>,
    mask_count: usize,
}

/// The ids of written definitions, found by their content's hash.
struct DefIds<K> {
    ids: HashMap<u64, Vec<(K, String)>>,
    len: usize,
}

impl<K> Default for DefIds<K> {
    fn default() -> Self {
        Self {
            ids: HashMap::new(),
            len: 0,
        }
    }
}

impl<K> DefIds<K> {
    /// The id of the written definition with this hash whose content `matches`.
    fn find(&self, hash: u64, matches: impl Fn(&K) -> bool) -> Option<String> {
        self.ids
            .get(&hash)?
            .iter()
            .find(|(key, _)| matches(key))
            .map(|(_, id)| id.clone())
    }

    /// Records a new definition, and returns its id: `prefix` and the definition's number.
    fn insert(&mut self, hash: u64, key: K, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.len);
        self.len += 1;
        self.ids.entry(hash).or_default().push((key, id.clone()));
        id
    }
}

fn content_hash(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

impl SvgDefs {
    fn write_defs(&self, output: &mut String) {
        if self.body.is_empty() && self.font_css.is_empty() {
            return;
        }
        output.push_str("<defs>\n");
        if !self.font_css.is_empty() {
            output.push_str("<style><![CDATA[\n");
            output.push_str(&self.font_css);
            output.push_str("]]></style>\n");
        }
        output.push_str(&self.body);
        output.push_str("</defs>\n");
    }

    fn gradient_id(
        &mut self,
        paint: ResolvedPaint,
        precision: usize,
    ) -> Result<String, AvengerSvgError> {
        let (gradient, bounds) = match paint {
            ResolvedPaint::Linear(g, bounds) => (Gradient::LinearGradient(g.clone()), bounds),
            ResolvedPaint::Radial(g, bounds) => (Gradient::RadialGradient(g.clone()), bounds),
            ResolvedPaint::Solid(_) => unreachable!("solid paint needs no gradient"),
        };
        let bits = |point: [f32; 2]| point.map(f32::to_bits);
        let hash = content_hash(&(&gradient, bits(bounds.min), bits(bounds.max)));
        if let Some(id) = self
            .gradient_ids
            .find(hash, |(g, b)| *g == gradient && *b == bounds)
        {
            return Ok(id);
        }
        let id = self
            .gradient_ids
            .insert(hash, (gradient.clone(), bounds), "svg-gradient");
        let (tag, controls): (_, Vec<(&str, f32)>) = match &gradient {
            Gradient::LinearGradient(g) => (
                "linearGradient",
                vec![("x1", g.x0), ("y1", g.y0), ("x2", g.x1), ("y2", g.y1)],
            ),
            Gradient::RadialGradient(g) => (
                "radialGradient",
                vec![
                    ("fx", g.x0),
                    ("fy", g.y0),
                    ("cx", g.x1),
                    ("cy", g.y1),
                    ("fr", g.r0),
                    ("r", g.r1),
                ],
            ),
        };
        // Controls are in the unit box, which the transform maps onto the bounds.
        self.body.push_str(&format!(
            "<{tag} id=\"{id}\" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"matrix("
        ));
        push_numbers(
            &mut self.body,
            &[
                bounds.max[0] - bounds.min[0],
                0.0,
                0.0,
                bounds.max[1] - bounds.min[1],
                bounds.min[0],
                bounds.min[1],
            ],
            precision,
        )?;
        self.body.push_str(")\"");
        for (name, value) in controls {
            if !value.is_finite() {
                return Err(AvengerSvgError::InvalidGeometry(
                    "gradient controls must be finite".into(),
                ));
            }
            // Unit-box controls keep their precision independently of scene-coordinate rounding.
            self.body.push_str(&format!(" {name}=\"{value}\""));
        }
        self.body.push_str(">\n");
        for stop in gradient.stops() {
            self.body.push_str(r#"<stop offset=""#);
            push_number(&mut self.body, stop.offset, precision)?;
            self.body.push('"');
            push_stop_color_attrs(&mut self.body, stop.color, precision)?;
            self.body.push_str("/>\n");
        }
        self.body.push_str(&format!("</{tag}>\n"));
        Ok(id)
    }

    fn clip_id(
        &mut self,
        clip: &Clip,
        precision: usize,
    ) -> Result<Option<String>, AvengerSvgError> {
        let hash = content_hash(clip);
        if let Some(id) = self.clip_ids.find(hash, |existing| existing == clip) {
            return Ok(Some(id));
        }
        let shape = match clip {
            Clip::None => return Ok(None),
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                let mut shape = String::from(r#"<rect x=""#);
                push_number(&mut shape, *x, precision)?;
                shape.push_str(r#"" y=""#);
                push_number(&mut shape, *y, precision)?;
                shape.push_str(r#"" width=""#);
                push_number(&mut shape, *width, precision)?;
                shape.push_str(r#"" height=""#);
                push_number(&mut shape, *height, precision)?;
                shape.push_str(r#""/>"#);
                shape
            }
            Clip::Path(path) => {
                format!(r#"<path d="{}"/>"#, lyon_path_to_svg_d(path, precision)?)
            }
        };
        let id = self.clip_ids.insert(hash, clip.clone(), "svg-clip");
        self.body.push_str(&format!(
            "<clipPath id=\"{id}\" clipPathUnits=\"userSpaceOnUse\">{shape}</clipPath>\n"
        ));
        Ok(Some(id))
    }

    /// Writes the luminance masks that combine a pattern's layers, and returns the last one's id.
    fn write_pattern_mask(
        &mut self,
        geometry: &LayeredPatternGeometry,
        bounds: PatternRect,
        precision: usize,
    ) -> Result<String, AvengerSvgError> {
        let mut previous: Option<String> = None;
        for layer in &geometry.layers {
            let layer_mask = self.start_mask(bounds, precision)?;
            push_pattern_primitives(&mut self.body, layer, [1.0; 4], precision)?;
            self.body.push_str("</mask>\n");

            let mask = self.start_mask(bounds, precision)?;
            if let Some(previous) = &previous {
                push_masked_rect(&mut self.body, bounds, "white", previous, precision)?;
            }
            let layer_color = if layer.operation == PatternLayerOperation::Subtract {
                "black"
            } else {
                "white"
            };
            push_masked_rect(&mut self.body, bounds, layer_color, &layer_mask, precision)?;
            if let (PatternLayerOperation::Xor, Some(previous)) = (layer.operation, &previous) {
                self.body
                    .push_str(&format!("<g mask=\"url(#{previous})\">\n"));
                push_masked_rect(&mut self.body, bounds, "black", &layer_mask, precision)?;
                self.body.push_str("</g>\n");
            }
            self.body.push_str("</mask>\n");
            previous = Some(mask);
        }
        Ok(previous.expect("pattern geometry has a layer"))
    }

    /// Opens a luminance mask that starts black over `bounds`, and returns its id.
    fn start_mask(
        &mut self,
        bounds: PatternRect,
        precision: usize,
    ) -> Result<String, AvengerSvgError> {
        let id = format!("svg-mask-{}", self.mask_count);
        self.mask_count += 1;
        self.body.push_str(&format!(
            "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" maskContentUnits=\"userSpaceOnUse\" mask-type=\"luminance\">\n"
        ));
        push_rect(&mut self.body, bounds, "black", precision)?;
        Ok(id)
    }
}

fn push_clip_attr(output: &mut String, clip: Option<&str>) {
    if let Some(clip) = clip {
        output.push_str(&format!(r#" clip-path="url(#{clip})""#));
    }
}

fn push_rect(
    output: &mut String,
    bounds: PatternRect,
    fill: &str,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    output.push_str(r#"<rect x=""#);
    push_number(output, bounds.min_x(), precision)?;
    output.push_str(r#"" y=""#);
    push_number(output, bounds.min_y(), precision)?;
    output.push_str(r#"" width=""#);
    push_number(output, bounds.width, precision)?;
    output.push_str(r#"" height=""#);
    push_number(output, bounds.height, precision)?;
    output.push_str(&format!("\" fill=\"{fill}\"/>\n"));
    Ok(())
}

fn push_masked_rect(
    output: &mut String,
    bounds: PatternRect,
    fill: &str,
    mask: &str,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    output.push_str(&format!("<g mask=\"url(#{mask})\">\n"));
    push_rect(output, bounds, fill, precision)?;
    output.push_str("</g>\n");
    Ok(())
}

fn push_pattern_primitives(
    output: &mut String,
    layer: &PatternCoverageLayer,
    ink: [f32; 4],
    precision: usize,
) -> Result<(), AvengerSvgError> {
    for primitive in &layer.primitives {
        output.push_str(r#"<path d=""#);
        output.push_str(&lyon_path_to_svg_d(primitive.path(), precision)?);
        output.push('"');
        match primitive {
            PatternCoveragePrimitive::Filled(_) => {
                push_color_attrs(output, "fill", ink, precision)?;
            }
            PatternCoveragePrimitive::Stroked { stroke_width, .. } => {
                output.push_str(r#" fill="none""#);
                push_color_attrs(output, "stroke", ink, precision)?;
                output.push_str(r#" stroke-width=""#);
                push_number(output, *stroke_width, precision)?;
                output.push('"');
            }
        }
        output.push_str("/>\n");
    }
    Ok(())
}

fn cap_value(cap: StrokeCap) -> &'static str {
    match cap {
        StrokeCap::Butt => "butt",
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
    }
}

fn join_value(join: StrokeJoin) -> &'static str {
    match join {
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
        StrokeJoin::Bevel => "bevel",
    }
}

pub(crate) fn font_style_value(style: FontStyle) -> &'static str {
    match style {
        FontStyle::Normal => "normal",
        FontStyle::Italic => "italic",
        FontStyle::Oblique => "oblique",
    }
}

fn geometry_error(error: impl std::fmt::Display) -> AvengerSvgError {
    AvengerSvgError::InvalidGeometry(error.to_string())
}

#[cfg(test)]
mod tests {
    use avenger_color::ColorOrGradient;
    use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
    use avenger_common::value::ScalarOrArray;
    use avenger_image::RgbaImage;
    use avenger_scenegraph::{
        marks::{
            group::{Clip, SceneGroup},
            image::SceneImageMark,
            mesh_image::{MeshImage, SceneMeshImageMark},
            rect::SceneRectMark,
            text::SceneTextMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_typst_label::{EngineOptions, FontOptions, MissingFontPolicy};
    use base64::{prelude::BASE64_STANDARD, Engine};
    use font_subset::FontReader;
    use resvg::usvg;

    use super::*;

    fn test_engine() -> LabelEngine {
        LabelEngine::new(EngineOptions {
            fonts: test_fonts(),
        })
    }

    fn test_fonts() -> FontOptions {
        FontOptions {
            load_system_fonts: false,
            ..avenger_typst_label::bundled_font_options()
        }
    }

    #[test]
    fn renders_document_background_with_explicit_dimensions() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(r##"<rect x="0" y="0" width="20" height="10" fill="#ffffff"/>"##));
    }

    #[test]
    fn renders_typst_text_decoration_dash_phase_and_miter_limit() {
        let scene_graph = SceneGraph {
            width: 160.0,
            height: 60.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar(
                    "#underline(stroke: (thickness: 1pt, dash: (array: (2pt, 1pt), phase: 0.5pt), miter-limit: 2))[x]"
                        .to_string(),
                ),
                x: ScalarOrArray::new_scalar(10.0),
                y: ScalarOrArray::new_scalar(30.0),
                font_size: ScalarOrArray::new_scalar(14.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(r#"stroke-dasharray="2 1""#));
        assert!(svg.contains(r#"stroke-dashoffset="0.5""#));
        assert!(svg.contains(r#"stroke-miterlimit="2""#));
    }

    #[test]
    fn renders_clip_paths_for_clipped_descendants() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                origin: [1.0, 2.0],
                clip: Clip::Rect {
                    x: 3.0,
                    y: 4.0,
                    width: 5.0,
                    height: 6.0,
                },
                marks: vec![SceneRectMark {
                    clip: true,
                    len: 1,
                    x: ScalarOrArray::new_scalar(0.0),
                    y: ScalarOrArray::new_scalar(0.0),
                    width: Some(ScalarOrArray::new_scalar(10.0)),
                    height: Some(ScalarOrArray::new_scalar(10.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(r#"<clipPath id="svg-clip-0" clipPathUnits="userSpaceOnUse"><rect x="4" y="6" width="5" height="6"/></clipPath>"#));
        assert!(svg.contains(r#"clip-path="url(#svg-clip-0)""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn equal_gradients_and_clips_share_one_definition() {
        use avenger_color::{GradientStop, LinearGradient};
        // Two of the three rects have the same bounds, so they share a gradient.
        let rects = SceneRectMark {
            len: 3,
            x: ScalarOrArray::new_array(vec![0.0, 0.0, 20.0]),
            y: ScalarOrArray::new_scalar(0.0),
            width: Some(ScalarOrArray::new_scalar(10.0)),
            height: Some(ScalarOrArray::new_scalar(10.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
            gradients: vec![Gradient::LinearGradient(LinearGradient {
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
            })],
            ..Default::default()
        };
        let group = || {
            SceneGroup {
                clip: Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 30.0,
                    height: 30.0,
                },
                marks: vec![rects.clone().into()],
                ..Default::default()
            }
            .into()
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 40.0,
            origin: [0.0, 0.0],
            marks: vec![group(), group()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert_eq!(svg.matches("<linearGradient").count(), 2);
        assert_eq!(svg.matches("<clipPath").count(), 1);
    }

    #[test]
    fn renders_image_marks_as_embedded_png_images() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [1.0, 2.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(
                    RgbaImage {
                        width: 1,
                        height: 1,
                        data: vec![255, 0, 0, 255],
                    }
                    .into(),
                ),
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: ScalarOrArray::new_scalar(4.0),
                height: ScalarOrArray::new_scalar(5.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(r#"<image x="3" y="5" width="4" height="5""#));
        assert!(svg.contains(r#"preserveAspectRatio="none""#));
        assert!(svg.contains(r#"href="data:image/png;base64,"#));
        assert!(svg.contains(r#"style="image-rendering:pixelated""#));
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let image = first_usvg_image(tree.root().children()).expect("SVG should contain an image");
        assert_eq!(image.rendering_mode(), usvg::ImageRendering::Pixelated);
    }

    /// A mesh image mark's instances export as one image on the half-unit grid of twice the
    /// scene's scale, so adjacent instances meet without a seam.
    #[test]
    fn renders_mesh_image_instances_as_one_image_on_the_pixel_grid() {
        let quad = |x: f32| MeshImage {
            image: RgbaImage {
                width: 1,
                height: 1,
                data: vec![255, 0, 0, 255],
            }
            .into(),
            positions: vec![[x, 0.2], [x + 10.0, 0.2], [x + 10.0, 10.2], [x, 10.2]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        let scene_graph = SceneGraph {
            width: 30.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneMeshImageMark {
                instances: vec![quad(0.3), quad(10.3)],
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert_eq!(svg.matches("<image").count(), 1);
        assert!(svg.contains(r#"<image x="0" y="0" width="20.5" height="10.5""#));
    }

    #[test]
    fn renders_text_marks_as_native_svg_text() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [1.0, 2.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("<A&B>".to_string()),
                x: ScalarOrArray::new_scalar(10.0),
                y: ScalarOrArray::new_scalar(12.0),
                align: ScalarOrArray::new_scalar(TextAlign::Center),
                baseline: ScalarOrArray::new_scalar(TextBaseline::Middle),
                angle: ScalarOrArray::new_scalar(45.0),
                color: ScalarOrArray::new_scalar([1.0, 0.0, 0.0, 0.5]),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                font_weight: ScalarOrArray::new_scalar(FontWeight::BOLD),
                font_style: ScalarOrArray::new_scalar(FontStyle::Italic),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(r##" fill="#ff0000" fill-opacity="0.5""##));
        assert!(svg.contains("<style><![CDATA[\n@font-face"));
        assert!(svg.contains(r#"font-family: "avenger-font-0";"#));
        assert!(svg.contains("data:font/woff2;base64,"));
        assert!(svg.contains(r#"font-family="avenger-font-0""#));
        assert!(svg.contains(r#"font-size="12""#));
        // The bundled fonts have no bold italic face, so the text draws in Lato Italic.
        assert!(svg.contains(r#"font-weight="400""#));
        assert!(svg.contains(r#"font-style="italic""#));
        assert!(svg.contains(r#"transform="rotate(45 11 14) translate("#));
        assert!(svg.contains("&lt;A&amp;B&gt;</text>"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_typst_text_as_native_text_and_math_paths() {
        let scene_graph = SceneGraph {
            width: 120.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("speed $v^2$ now".to_string()),
                x: ScalarOrArray::new_scalar(6.0),
                y: ScalarOrArray::new_scalar(18.0),
                color: ScalarOrArray::new_scalar([0.0, 0.25, 1.0, 0.75]),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let math_svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(math_svg.contains("<path "));
        assert!(math_svg.contains("<text "));
        assert!(math_svg.contains("speed "));
        assert!(math_svg.contains(" now"));
        assert!(!math_svg.contains("$v^2$"));
        assert!(math_svg.contains(r##"fill="#0040ff""##));
        assert!(usvg::Tree::from_str(&math_svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_typst_named_emoji_as_native_svg_text() {
        let scene_graph = SceneGraph {
            width: 120.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Mood #emoji.face".to_string()),
                x: ScalarOrArray::new_scalar(6.0),
                y: ScalarOrArray::new_scalar(18.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let svg = SvgRenderer::new()
            .with_options(SvgRenderOptions {
                color_glyph_images: false,
                embed_fonts: false,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains("<text "));
        assert!(svg.contains("Mood "));
        assert!(svg.contains("😀"));
        assert!(!svg.contains("#emoji.face"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    /// A bound string's glyphs all come from its literal, so the run of its text and the image
    /// of its emoji share a source range. They pair by text item, so the run still draws.
    #[test]
    #[cfg(target_os = "macos")]
    fn generated_text_keeps_its_run_beside_an_emoji_image() {
        use avenger_typst_label::{bind, LabelValue, LabelValues};
        let values = LabelValues::from([("name".into(), LabelValue::Str("Sales 😀".into()))]);
        let mark = SceneTextMark {
            text: ScalarOrArray::new_scalar(bind("#name", &values).unwrap()),
            x: ScalarOrArray::new_scalar(6.0),
            y: ScalarOrArray::new_scalar(18.0),
            font: ScalarOrArray::new_scalar("Lato".to_string()),
            font_size: ScalarOrArray::new_scalar(12.0),
            text_syntax: TextSyntaxMode::TypstMarkup,
            ..Default::default()
        };
        let scene_graph = SceneGraph {
            width: 120.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![mark.into()],
        };
        // With the system's fonts, Apple Color Emoji draws the emoji, as a bitmap image.
        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        assert!(svg.contains(">Sales </text>"), "{svg}");
        assert!(svg.contains("<image"), "{svg}");
    }

    #[test]
    fn renders_synthesized_sub_super_as_smaller_native_svg_text() {
        let scene_graph = SceneGraph {
            width: 120.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar(
                    "H#sub(typographic: false)[2]O #super(typographic: false)[\\*]".to_string(),
                ),
                x: ScalarOrArray::new_scalar(6.0),
                y: ScalarOrArray::new_scalar(18.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(svg.contains(">H</text>"));
        assert!(svg.contains(">2</text>"));
        assert!(svg.contains(">O </text>"));
        assert!(svg.contains(">*</text>"));
        assert!(svg.contains(r#"font-size="12""#));
        assert!(svg
            .lines()
            .any(|line| line.contains(">2</text>") && !line.contains(r#"font-size="12""#)));
        assert!(svg
            .lines()
            .any(|line| line.contains(">*</text>") && !line.contains(r#"font-size="12""#)));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn missing_font_fallback_emits_resolved_text_family() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Fallback".to_string()),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Definitely Missing Font".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                missing_font: MissingFontPolicy::Fallback,
                ..test_fonts()
            },
        });
        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &engine)
            .unwrap();

        assert!(svg.contains(r#"font-family="avenger-font-0""#));
        assert!(!svg.contains(r#"font-family="Definitely Missing Font""#));
        assert!(svg.contains(r#"font-family: "avenger-font-0";"#));
    }

    #[test]
    fn cuts_text_marks_to_their_width() {
        let source_text = "Long label text";
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar(source_text.to_string()),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(10.0),
                width: ScalarOrArray::new_scalar(avenger_typst_label::LabelWidth::Max(35.0)),
                wrap: false,
                ellipsis: true,
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        // The ellipsis is a run of its own, after the kept text.
        assert!(svg.contains(">\u{2026}</text>"));
        assert!(!svg.contains("Long label text</text>"));
        let rendered_text = first_text_body(&svg);
        assert!(!rendered_text.contains('x'));

        let woff2 = first_woff2_payload(&svg);
        let reader = FontReader::new(&woff2).unwrap();
        let font = reader.read().unwrap();
        assert!(font.contains_char('\u{2026}'));
        // Shaping tables require the complete face even for a shortened label.
        assert!(font.contains_char('x'));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn writes_font_families_without_embedding_fonts() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Label".to_string()),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(10.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .with_options(SvgRenderOptions {
                embed_fonts: false,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph, &test_engine())
            .unwrap();

        assert!(!svg.contains("@font-face"));
        assert!(svg.contains(r#"font-family="Lato""#));
    }

    fn first_text_body(svg: &str) -> &str {
        let text_start = svg.find("<text ").expect("SVG should contain text");
        let body_start = svg[text_start..]
            .find('>')
            .expect("text element should have an opening tag")
            + text_start
            + 1;
        let body_end = svg[body_start..]
            .find("</text>")
            .expect("text element should have a closing tag")
            + body_start;
        &svg[body_start..body_end]
    }

    fn first_woff2_payload(svg: &str) -> Vec<u8> {
        let prefix = "data:font/woff2;base64,";
        let start = svg.find(prefix).expect("SVG should contain WOFF2 data URI") + prefix.len();
        let rest = &svg[start..];
        let end = rest.find('"').expect("WOFF2 data URI should be quoted");
        BASE64_STANDARD.decode(&rest[..end]).unwrap()
    }

    fn first_usvg_image(nodes: &[usvg::Node]) -> Option<&usvg::Image> {
        for node in nodes {
            match node {
                usvg::Node::Image(image) => return Some(image),
                usvg::Node::Group(group) => {
                    if let Some(image) = first_usvg_image(group.children()) {
                        return Some(image);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
