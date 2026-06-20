use std::io::Cursor;

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::Clip, image::SceneImageMark,
        line::SceneLineMark, mark::SceneMark, path::ScenePathMark, rect::SceneRectMark,
        rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark, trail::SceneTrailMark,
    },
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use base64::{prelude::BASE64_STANDARD, Engine};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;

use crate::{
    error::AvengerSvgError,
    options::{SvgBackground, SvgRenderOptions},
    path::{format_number, lyon_path_to_svg_d, push_number, push_point},
    style::{
        push_color_attrs, push_fill_attrs, push_stop_color_attrs, push_stroke_attrs, PaintResolver,
    },
};

#[derive(Debug, Clone, Default)]
pub struct SvgRenderer {
    options: SvgRenderOptions,
}

impl SvgRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(mut self, options: SvgRenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<String, AvengerSvgError> {
        let mut document = SvgDocument::default();
        let precision = self.options.precision;
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);

        document
            .body
            .push_str("<g fill=\"none\" stroke-miterlimit=\"10\">\n");
        self.write_background(&mut document)?;

        for item in display_list.ordered_items() {
            let clip_id = document.defs.clip_id(&item.clip, precision)?;

            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.write_path_mark(&mut document, mark, item.origin, clip_id.as_deref())?;
                }
                SceneDisplayMark::Borrowed(mark) => {
                    self.write_scene_mark(&mut document, mark, item.origin, clip_id.as_deref())?;
                }
            }
        }

        document.body.push_str("</g>\n");

        let mut output = String::new();
        output.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" width=""#);
        output.push_str(&format_number(scene_graph.width, precision)?);
        output.push_str(r#"" height=""#);
        output.push_str(&format_number(scene_graph.height, precision)?);
        output.push_str(r#"" viewBox="0 0 "#);
        output.push_str(&format_number(scene_graph.width, precision)?);
        output.push(' ');
        output.push_str(&format_number(scene_graph.height, precision)?);
        output.push_str("\">\n");
        document.defs.write_defs(&mut output);
        output.push_str(&document.body);
        output.push_str("</svg>\n");
        Ok(output)
    }

    fn write_background(&self, document: &mut SvgDocument) -> Result<(), AvengerSvgError> {
        let color = match self.options.background {
            SvgBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            SvgBackground::Transparent => None,
            SvgBackground::Color(color) => Some(color),
        };

        let Some(color) = color else {
            return Ok(());
        };

        let output = &mut document.body;
        output.push_str(r#"<rect width="100%" height="100%""#);
        push_color_attrs(output, "fill", color, self.options.precision)?;
        output.push_str(r#" stroke="none"/>"#);
        output.push('\n');
        Ok(())
    }

    fn write_scene_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        match mark {
            SceneMark::Rect(mark) => self.write_rect_mark(document, mark, origin, clip_id),
            SceneMark::Path(mark) => self.write_path_mark(document, mark, origin, clip_id),
            SceneMark::Rule(mark) => self.write_rule_mark(document, mark, origin, clip_id),
            SceneMark::Line(mark) => self.write_line_mark(document, mark, origin, clip_id),
            SceneMark::Area(mark) => self.write_area_mark(document, mark, origin, clip_id),
            SceneMark::Symbol(mark) => self.write_symbol_mark(document, mark, origin, clip_id),
            SceneMark::Arc(mark) => self.write_arc_mark(document, mark, origin, clip_id),
            SceneMark::Trail(mark) => self.write_trail_mark(document, mark, origin, clip_id),
            SceneMark::Text(mark) => self.write_text_mark(document, mark, origin, clip_id),
            SceneMark::Image(mark) => self.write_image_mark(document, mark, origin, clip_id),
            SceneMark::Group(_) => Ok(()),
        }
    }

    fn write_rect_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneRectMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.write_path_element(
                document,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
            )?;
        }

        Ok(())
    }

    fn write_path_mark(
        &self,
        document: &mut SvgDocument,
        mark: &ScenePathMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter()
        ) {
            self.write_path_element(
                document,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: Some(mark.stroke_cap),
                    stroke_join: Some(mark.stroke_join),
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
            )?;
        }

        Ok(())
    }

    fn write_symbol_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneSymbolMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter()
        ) {
            self.write_path_element(
                document,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
            )?;
        }

        Ok(())
    }

    fn write_trail_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneTrailMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let d = trail_path_d(mark, origin, self.options.precision)?;
        self.write_path_element(
            document,
            &d,
            PathStyle {
                fill: Some(&mark.stroke),
                stroke: None,
                stroke_width: None,
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
                gradients: &mark.gradients,
            },
            clip_id,
        )
    }

    fn write_text_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneTextMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (
            text,
            x,
            y,
            align,
            baseline,
            angle,
            color,
            font,
            font_size,
            font_weight,
            font_style,
            limit,
        ) in izip!(
            mark.text_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.align_iter(),
            mark.baseline_iter(),
            mark.angle_iter(),
            mark.color_iter(),
            mark.font_iter(),
            mark.font_size_iter(),
            mark.font_weight_iter(),
            mark.font_style_iter(),
            mark.limit_iter(),
        ) {
            if *limit > 0.0 {
                return Err(AvengerSvgError::UnsupportedFeature(
                    "SVG text limit truncation is not implemented yet".to_string(),
                ));
            }

            let x = *x + origin[0];
            let y = *y + origin[1];
            document.body.push_str(r#"<text x=""#);
            push_number(&mut document.body, x, self.options.precision)?;
            document.body.push_str(r#"" y=""#);
            push_number(&mut document.body, y, self.options.precision)?;
            document.body.push('"');
            push_color_or_text_paint(&mut document.body, color, self.options.precision)?;
            document.body.push_str(r#" text-anchor=""#);
            document.body.push_str(text_anchor(align));
            document.body.push('"');
            document.body.push_str(r#" dominant-baseline=""#);
            document.body.push_str(dominant_baseline(baseline));
            document.body.push('"');
            document.body.push_str(r#" font-family=""#);
            document.body.push_str(&crate::style::escape_attr(font));
            document.body.push('"');
            document.body.push_str(r#" font-size=""#);
            push_number(&mut document.body, *font_size, self.options.precision)?;
            document.body.push('"');
            document.body.push_str(r#" font-weight=""#);
            document
                .body
                .push_str(&font_weight_value(font_weight, self.options.precision)?);
            document.body.push('"');
            document.body.push_str(r#" font-style=""#);
            document.body.push_str(font_style_value(font_style));
            document.body.push('"');
            if *angle != 0.0 {
                document.body.push_str(r#" transform="rotate("#);
                push_number(&mut document.body, *angle, self.options.precision)?;
                document.body.push(' ');
                push_number(&mut document.body, x, self.options.precision)?;
                document.body.push(' ');
                push_number(&mut document.body, y, self.options.precision)?;
                document.body.push(')');
                document.body.push('"');
            }
            push_clip_attr(&mut document.body, clip_id);
            document.body.push('>');
            document.body.push_str(&crate::style::escape_text(text));
            document.body.push_str("</text>\n");
        }

        Ok(())
    }

    fn write_image_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneImageMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (image, path) in izip!(mark.image_iter(), mark.transformed_path_iter(origin)) {
            let data_uri = rgba_image_to_png_data_uri(image)?;
            let bbox = bounding_box(&path);
            let x = bbox.min.x;
            let y = bbox.min.y;
            let width = bbox.max.x - bbox.min.x;
            let height = bbox.max.y - bbox.min.y;

            document.body.push_str(r#"<image x=""#);
            push_number(&mut document.body, x, self.options.precision)?;
            document.body.push_str(r#"" y=""#);
            push_number(&mut document.body, y, self.options.precision)?;
            document.body.push_str(r#"" width=""#);
            push_number(&mut document.body, width, self.options.precision)?;
            document.body.push_str(r#"" height=""#);
            push_number(&mut document.body, height, self.options.precision)?;
            document
                .body
                .push_str(r#"" preserveAspectRatio="none" href=""#);
            document.body.push_str(&data_uri);
            document.body.push('"');
            if !mark.smooth {
                document.body.push_str(r#" image-rendering="pixelated""#);
            }
            push_clip_attr(&mut document.body, clip_id);
            document.body.push_str("/>\n");
        }

        Ok(())
    }

    fn write_arc_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneArcMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.write_path_element(
                document,
                &lyon_path_to_svg_d(&path, self.options.precision)?,
                PathStyle {
                    fill: Some(fill),
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
            )?;
        }

        Ok(())
    }

    fn write_area_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneAreaMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let path = mark.transformed_path(origin);
        self.write_path_element(
            document,
            &lyon_path_to_svg_d(&path, self.options.precision)?,
            PathStyle {
                fill: Some(&mark.fill),
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                gradients: &mark.gradients,
            },
            clip_id,
        )
    }

    fn write_line_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneLineMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let d = line_path_d(mark, origin, self.options.precision)?;
        self.write_path_element(
            document,
            &d,
            PathStyle {
                fill: None,
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                gradients: &mark.gradients,
            },
            clip_id,
        )
    }

    fn write_rule_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneRuleMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
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
            let defs = &mut document.defs;
            let body = &mut document.body;
            let mut resolver = PaintContext {
                defs,
                gradients: &mark.gradients,
            };

            body.push_str("<line x1=\"");
            push_number(body, *x1 + origin[0], self.options.precision)?;
            body.push_str("\" y1=\"");
            push_number(body, *y1 + origin[1], self.options.precision)?;
            body.push_str("\" x2=\"");
            push_number(body, *x2 + origin[0], self.options.precision)?;
            body.push_str("\" y2=\"");
            push_number(body, *y2 + origin[1], self.options.precision)?;
            body.push('"');
            push_fill_attrs(body, None, &mut resolver, self.options.precision)?;
            push_stroke_attrs(
                body,
                Some(stroke),
                Some(*stroke_width),
                Some(*stroke_cap),
                None,
                stroke_dashes.get(index).map(|dash| dash.as_slice()),
                &mut resolver,
                self.options.precision,
            )?;
            push_clip_attr(body, clip_id);
            body.push_str("/>\n");
        }

        Ok(())
    }

    fn write_path_element(
        &self,
        document: &mut SvgDocument,
        d: &str,
        style: PathStyle<'_>,
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let defs = &mut document.defs;
        let body = &mut document.body;
        let mut resolver = PaintContext {
            defs,
            gradients: style.gradients,
        };

        body.push_str(r#"<path d=""#);
        body.push_str(d);
        body.push('"');
        push_fill_attrs(body, style.fill, &mut resolver, self.options.precision)?;
        push_stroke_attrs(
            body,
            style.stroke,
            style.stroke_width,
            style.stroke_cap,
            style.stroke_join,
            style.stroke_dash,
            &mut resolver,
            self.options.precision,
        )?;
        push_clip_attr(body, clip_id);
        body.push_str("/>\n");
        Ok(())
    }
}

#[derive(Default)]
struct SvgDocument {
    defs: SvgDefs,
    body: String,
}

#[derive(Default)]
struct SvgDefs {
    body: String,
    gradient_ids: Vec<(Gradient, String)>,
    clip_ids: Vec<(Clip, String)>,
    next_gradient_id: usize,
    next_clip_id: usize,
}

impl SvgDefs {
    fn write_defs(&self, output: &mut String) {
        if self.body.is_empty() {
            output.push_str("<defs/>\n");
        } else {
            output.push_str("<defs>\n");
            output.push_str(&self.body);
            output.push_str("</defs>\n");
        }
    }

    fn gradient_id(
        &mut self,
        gradients: &[Gradient],
        index: u32,
        precision: usize,
    ) -> Result<String, AvengerSvgError> {
        let gradient = gradients.get(index as usize).ok_or_else(|| {
            AvengerSvgError::UnsupportedPaint(format!("gradient index {index} is out of range"))
        })?;

        if let Some((_, id)) = self
            .gradient_ids
            .iter()
            .find(|(existing, _)| existing == gradient)
        {
            return Ok(id.clone());
        }

        let id = format!("svg-gradient-{}", self.next_gradient_id);
        self.next_gradient_id += 1;
        self.write_gradient_def(&id, gradient, precision)?;
        self.gradient_ids.push((gradient.clone(), id.clone()));
        Ok(id)
    }

    fn write_gradient_def(
        &mut self,
        id: &str,
        gradient: &Gradient,
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        match gradient {
            Gradient::LinearGradient(gradient) => {
                self.body.push_str(r#"<linearGradient id=""#);
                self.body.push_str(id);
                self.body
                    .push_str(r#"" gradientUnits="objectBoundingBox" x1=""#);
                push_number(&mut self.body, gradient.x0, precision)?;
                self.body.push_str(r#"" y1=""#);
                push_number(&mut self.body, gradient.y0, precision)?;
                self.body.push_str(r#"" x2=""#);
                push_number(&mut self.body, gradient.x1, precision)?;
                self.body.push_str(r#"" y2=""#);
                push_number(&mut self.body, gradient.y1, precision)?;
                self.body.push_str("\">\n");
                self.write_gradient_stops(gradient.stops.as_slice(), precision)?;
                self.body.push_str("</linearGradient>\n");
            }
            Gradient::RadialGradient(gradient) => {
                self.body.push_str(r#"<radialGradient id=""#);
                self.body.push_str(id);
                self.body
                    .push_str(r#"" gradientUnits="objectBoundingBox" fx=""#);
                push_number(&mut self.body, gradient.x0, precision)?;
                self.body.push_str(r#"" fy=""#);
                push_number(&mut self.body, gradient.y0, precision)?;
                self.body.push_str(r#"" cx=""#);
                push_number(&mut self.body, gradient.x1, precision)?;
                self.body.push_str(r#"" cy=""#);
                push_number(&mut self.body, gradient.y1, precision)?;
                self.body.push_str(r#"" fr=""#);
                push_number(&mut self.body, gradient.r0, precision)?;
                self.body.push_str(r#"" r=""#);
                push_number(&mut self.body, gradient.r1, precision)?;
                self.body.push_str("\">\n");
                self.write_gradient_stops(gradient.stops.as_slice(), precision)?;
                self.body.push_str("</radialGradient>\n");
            }
        }

        Ok(())
    }

    fn write_gradient_stops(
        &mut self,
        stops: &[avenger_color::GradientStop],
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        for stop in stops {
            self.body.push_str(r#"<stop offset=""#);
            push_number(&mut self.body, stop.offset, precision)?;
            self.body.push('"');
            push_stop_color_attrs(&mut self.body, stop.color, precision)?;
            self.body.push_str("/>\n");
        }
        Ok(())
    }

    fn clip_id(
        &mut self,
        clip: &Clip,
        precision: usize,
    ) -> Result<Option<String>, AvengerSvgError> {
        if matches!(clip, Clip::None) {
            return Ok(None);
        }

        if let Some((_, id)) = self.clip_ids.iter().find(|(existing, _)| existing == clip) {
            return Ok(Some(id.clone()));
        }

        let id = format!("svg-clip-{}", self.next_clip_id);
        self.next_clip_id += 1;
        self.write_clip_def(&id, clip, precision)?;
        self.clip_ids.push((clip.clone(), id.clone()));
        Ok(Some(id))
    }

    fn write_clip_def(
        &mut self,
        id: &str,
        clip: &Clip,
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        self.body.push_str(r#"<clipPath id=""#);
        self.body.push_str(id);
        self.body.push_str(r#"" clipPathUnits="userSpaceOnUse">"#);
        match clip {
            Clip::None => {}
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                self.body.push_str(r#"<rect x=""#);
                push_number(&mut self.body, *x, precision)?;
                self.body.push_str(r#"" y=""#);
                push_number(&mut self.body, *y, precision)?;
                self.body.push_str(r#"" width=""#);
                push_number(&mut self.body, *width, precision)?;
                self.body.push_str(r#"" height=""#);
                push_number(&mut self.body, *height, precision)?;
                self.body.push_str(r#""/>"#);
            }
            Clip::Path(path) => {
                self.body.push_str(r#"<path d=""#);
                self.body.push_str(&lyon_path_to_svg_d(path, precision)?);
                self.body.push_str(r#""/>"#);
            }
        }
        self.body.push_str("</clipPath>\n");
        Ok(())
    }
}

struct PaintContext<'a, 'b> {
    defs: &'a mut SvgDefs,
    gradients: &'b [Gradient],
}

impl PaintResolver for PaintContext<'_, '_> {
    fn push_paint_attrs(
        &mut self,
        output: &mut String,
        attr: &str,
        paint: &ColorOrGradient,
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        match paint {
            ColorOrGradient::Color(color) => push_color_attrs(output, attr, *color, precision),
            ColorOrGradient::GradientIndex(index) => {
                let id = self.defs.gradient_id(self.gradients, *index, precision)?;
                output.push(' ');
                output.push_str(attr);
                output.push_str(r#"="url(#"#);
                output.push_str(&id);
                output.push_str(r#")""#);
                Ok(())
            }
        }
    }
}

fn push_clip_attr(output: &mut String, clip_id: Option<&str>) {
    if let Some(clip_id) = clip_id {
        output.push_str(r#" clip-path="url(#"#);
        output.push_str(clip_id);
        output.push_str(r#")""#);
    }
}

fn rgba_image_to_png_data_uri(image: &RgbaImage) -> Result<String, AvengerSvgError> {
    let Some(rgba_image) = image.to_image() else {
        return Err(AvengerSvgError::ImageEncoding(
            "invalid RGBA image buffer".to_string(),
        ));
    };

    let mut cursor = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba_image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|err| AvengerSvgError::ImageEncoding(err.to_string()))?;

    Ok(format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(cursor.into_inner())
    ))
}

fn push_color_or_text_paint(
    output: &mut String,
    color: &ColorOrGradient,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    match color {
        ColorOrGradient::Color(color) => push_color_attrs(output, "fill", *color, precision),
        ColorOrGradient::GradientIndex(index) => Err(AvengerSvgError::UnsupportedPaint(format!(
            "text gradient index {index}"
        ))),
    }
}

fn text_anchor(align: &TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "start",
        TextAlign::Center => "middle",
        TextAlign::Right => "end",
    }
}

fn dominant_baseline(baseline: &TextBaseline) -> &'static str {
    match baseline {
        TextBaseline::Top | TextBaseline::LineTop => "text-before-edge",
        TextBaseline::Middle => "central",
        TextBaseline::Bottom | TextBaseline::LineBottom => "text-after-edge",
        TextBaseline::Alphabetic => "alphabetic",
    }
}

fn font_weight_value(
    font_weight: &FontWeight,
    precision: usize,
) -> Result<String, AvengerSvgError> {
    match font_weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => Ok("normal".to_string()),
        FontWeight::Name(FontWeightNameSpec::Bold) => Ok("bold".to_string()),
        FontWeight::Number(weight) => format_number(*weight, precision),
    }
}

fn font_style_value(font_style: &FontStyle) -> &'static str {
    match font_style {
        FontStyle::Normal => "normal",
        FontStyle::Italic => "italic",
    }
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

fn line_path_d(
    mark: &SceneLineMark,
    origin: [f32; 2],
    precision: usize,
) -> Result<String, AvengerSvgError> {
    let mut d = String::new();
    let mut path_len = 0;
    let mut last = None;

    for (x, y, defined) in izip!(mark.x_iter(), mark.y_iter(), mark.defined_iter()) {
        if *defined {
            let point = [*x + origin[0], *y + origin[1]];
            if path_len == 0 {
                if !d.is_empty() {
                    d.push(' ');
                }
                d.push('M');
            } else {
                d.push(' ');
                d.push('L');
            }
            push_point(&mut d, point[0], point[1], precision)?;
            path_len += 1;
            last = Some(point);
        } else {
            close_single_point_subpath(&mut d, path_len, last, precision)?;
            path_len = 0;
            last = None;
        }
    }

    close_single_point_subpath(&mut d, path_len, last, precision)?;
    Ok(d)
}

fn trail_path_d(
    mark: &SceneTrailMark,
    origin: [f32; 2],
    precision: usize,
) -> Result<String, AvengerSvgError> {
    let mut d = String::new();
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
                push_trail_segment(&mut d, prev_point, prev_radius, point, radius, precision)?;
            }
            prev = Some((point, radius));
            run_len += 1;
        } else {
            if run_len == 1 {
                if let Some((point, radius)) = prev {
                    push_trail_circle(&mut d, point, radius, precision)?;
                }
            }
            prev = None;
            run_len = 0;
        }
    }

    if run_len == 1 {
        if let Some((point, radius)) = prev {
            push_trail_circle(&mut d, point, radius, precision)?;
        }
    }

    Ok(d)
}

fn push_trail_segment(
    d: &mut String,
    p0: [f32; 2],
    r0: f32,
    p1: [f32; 2],
    r1: f32,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let len = (dx * dx + dy * dy).sqrt();

    if len <= f32::EPSILON {
        push_trail_circle(d, p0, r0.max(r1), precision)?;
        return Ok(());
    }

    if r0 <= 0.0 && r1 <= 0.0 {
        return Ok(());
    }

    let nx = -dy / len;
    let ny = dx / len;
    let p0_left = [p0[0] + nx * r0, p0[1] + ny * r0];
    let p1_left = [p1[0] + nx * r1, p1[1] + ny * r1];
    let p1_right = [p1[0] - nx * r1, p1[1] - ny * r1];
    let p0_right = [p0[0] - nx * r0, p0[1] - ny * r0];

    if !d.is_empty() {
        d.push(' ');
    }
    d.push('M');
    push_point(d, p0_left[0], p0_left[1], precision)?;
    d.push(' ');
    d.push('L');
    push_point(d, p1_left[0], p1_left[1], precision)?;
    push_arc_to(d, r1, p1_right, precision)?;
    d.push(' ');
    d.push('L');
    push_point(d, p0_right[0], p0_right[1], precision)?;
    push_arc_to(d, r0, p0_left, precision)?;
    d.push(' ');
    d.push('Z');
    Ok(())
}

fn push_trail_circle(
    d: &mut String,
    point: [f32; 2],
    radius: f32,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    if radius <= 0.0 {
        return Ok(());
    }

    if !d.is_empty() {
        d.push(' ');
    }
    d.push('M');
    push_point(d, point[0] + radius, point[1], precision)?;
    push_arc_to(d, radius, [point[0] - radius, point[1]], precision)?;
    push_arc_to(d, radius, [point[0] + radius, point[1]], precision)?;
    d.push(' ');
    d.push('Z');
    Ok(())
}

fn push_arc_to(
    d: &mut String,
    radius: f32,
    point: [f32; 2],
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let radius = radius.max(0.0);
    d.push(' ');
    d.push('A');
    push_number(d, radius, precision)?;
    d.push(' ');
    push_number(d, radius, precision)?;
    d.push_str(" 0 0 1 ");
    push_point(d, point[0], point[1], precision)?;
    Ok(())
}

fn close_single_point_subpath(
    d: &mut String,
    path_len: usize,
    last: Option<[f32; 2]>,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    if path_len == 1 {
        if let Some([x, y]) = last {
            d.push(' ');
            d.push('L');
            push_point(d, x, y, precision)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient};
    use avenger_common::value::ScalarOrArray;
    use avenger_image::RgbaImage;
    use avenger_scenegraph::{
        marks::{
            arc::SceneArcMark,
            group::{Clip, SceneGroup},
            image::SceneImageMark,
            rect::SceneRectMark,
            rule::SceneRuleMark,
            symbol::SceneSymbolMark,
            text::SceneTextMark,
            trail::SceneTrailMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};

    use super::*;

    #[test]
    fn renders_simple_rect_svg_that_usvg_can_parse() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(4.0)),
                height: Some(ScalarOrArray::new_scalar(5.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
        assert!(svg.contains(r##"fill="#ff0000""##));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_rule_with_native_line_and_dasharray() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [1.0, 2.0],
            marks: vec![SceneRuleMark {
                len: 1,
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                x2: ScalarOrArray::new_scalar(4.0),
                y2: ScalarOrArray::new_scalar(5.0),
                stroke_dash: Some(ScalarOrArray::new_scalar(vec![2.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<line x1="3" y1="5" x2="5" y2="7""#));
        assert!(svg.contains(r#"stroke-dasharray="2 1""#));
    }

    #[test]
    fn renders_group_fill_path_from_display_list() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: avenger_scenegraph::marks::group::Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                fill: Some(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r##"fill="#0000ff""##));
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

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<clipPath id="svg-clip-0" clipPathUnits="userSpaceOnUse"><rect x="4" y="6" width="5" height="6"/></clipPath>"#));
        assert!(svg.contains(r#"clip-path="url(#svg-clip-0)""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_native_linear_and_radial_gradients() {
        let stops = vec![
            GradientStop {
                offset: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            GradientStop {
                offset: 1.0,
                color: [0.0, 0.0, 1.0, 0.5],
            },
        ];
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                gradients: vec![
                    Gradient::LinearGradient(LinearGradient {
                        x0: 0.0,
                        y0: 0.0,
                        x1: 1.0,
                        y1: 0.0,
                        stops: stops.clone(),
                    }),
                    Gradient::RadialGradient(RadialGradient {
                        x0: 0.5,
                        y0: 0.5,
                        x1: 0.5,
                        y1: 0.5,
                        r0: 0.0,
                        r1: 0.5,
                        stops,
                    }),
                ],
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(4.0)),
                height: Some(ScalarOrArray::new_scalar(5.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(1)),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<linearGradient id="svg-gradient-0" gradientUnits="objectBoundingBox" x1="0" y1="0" x2="1" y2="0">"#));
        assert!(svg.contains(r#"<radialGradient id="svg-gradient-1" gradientUnits="objectBoundingBox" fx="0.5" fy="0.5" cx="0.5" cy="0.5" fr="0" r="0.5">"#));
        assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
        assert!(svg.contains(r#"stroke="url(#svg-gradient-1)""#));
        assert!(svg.contains(r##"stop-color="#0000ff" stop-opacity="0.5""##));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_symbol_marks_as_paths() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneSymbolMark {
                len: 2,
                x: ScalarOrArray::new_array(vec![5.0, 12.0]),
                y: ScalarOrArray::new_array(vec![5.0, 5.0]),
                size: ScalarOrArray::new_array(vec![9.0, 16.0]),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 1.0, 0.0, 1.0])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: Some(1.5),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert_eq!(svg.matches("<path ").count(), 2);
        assert!(svg.contains(r##"fill="#00ff00""##));
        assert!(svg.contains(r#"stroke-width="1.5""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
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
                image: ScalarOrArray::new_scalar(RgbaImage {
                    width: 1,
                    height: 1,
                    data: vec![255, 0, 0, 255],
                }),
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: ScalarOrArray::new_scalar(4.0),
                height: ScalarOrArray::new_scalar(5.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<image x="3" y="5" width="4" height="5""#));
        assert!(svg.contains(r#"preserveAspectRatio="none""#));
        assert!(svg.contains(r#"href="data:image/png;base64,"#));
        assert!(svg.contains(r#"image-rendering="pixelated""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_basic_arc_marks_as_paths() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneArcMark {
                x: ScalarOrArray::new_scalar(10.0),
                y: ScalarOrArray::new_scalar(10.0),
                start_angle: ScalarOrArray::new_scalar(0.0),
                end_angle: ScalarOrArray::new_scalar(std::f32::consts::PI),
                inner_radius: ScalarOrArray::new_scalar(2.0),
                outer_radius: ScalarOrArray::new_scalar(5.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: ScalarOrArray::new_scalar(1.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<path "));
        assert!(svg.contains(r##"fill="#ff0000""##));
        assert!(svg.contains(r#"stroke-width="1""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
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
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 0.5])),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Bold)),
                font_style: ScalarOrArray::new_scalar(FontStyle::Italic),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r##"<text x="11" y="14" fill="#ff0000" fill-opacity="0.5""##));
        assert!(svg.contains(r#"text-anchor="middle""#));
        assert!(svg.contains(r#"dominant-baseline="central""#));
        assert!(svg.contains(r#"font-family="Atkinson Hyperlegible""#));
        assert!(svg.contains(r#"font-size="12""#));
        assert!(svg.contains(r#"font-weight="bold""#));
        assert!(svg.contains(r#"font-style="italic""#));
        assert!(svg.contains(r#"transform="rotate(45 11 14)""#));
        assert!(svg.contains("&lt;A&amp;B&gt;</text>"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_trail_marks_as_filled_outline_paths() {
        let scene_graph = SceneGraph {
            width: 30.0,
            height: 20.0,
            origin: [1.0, 2.0],
            marks: vec![SceneTrailMark {
                len: 4,
                x: ScalarOrArray::new_array(vec![2.0, 10.0, 0.0, 20.0]),
                y: ScalarOrArray::new_array(vec![3.0, 3.0, 0.0, 8.0]),
                size: ScalarOrArray::new_array(vec![4.0, 8.0, 4.0, 6.0]),
                defined: ScalarOrArray::new_array(vec![true, true, false, true]),
                stroke: ColorOrGradient::Color([0.0, 0.0, 1.0, 0.5]),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert_eq!(svg.matches("<path ").count(), 1);
        assert!(svg.contains(r##"fill="#0000ff" fill-opacity="0.5" stroke="none""##));
        assert!(svg.contains(" A"));
        assert!(svg.contains(" Z M"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }
}
