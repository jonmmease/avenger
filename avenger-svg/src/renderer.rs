use std::io::Cursor;

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_image::RgbaImage;
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
    measurement::{truncate_text_to_limit_with, TextMeasurementConfig},
    path::{
        TextPathBuffer, TextPathDrawItem, TextPathExtractionConfig, TextPathImageFormat,
        TextPathImageItem, TextPathItem, TextPathStrokeCap, TextPathStrokeJoin,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode},
    TextEngine,
};
use base64::{prelude::BASE64_STANDARD, Engine};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;

use crate::{
    error::AvengerSvgError,
    fonts::{resolve_font_family_for_output, SvgFontCollector},
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
        self.write_background(&mut document, scene_graph.width, scene_graph.height)?;

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
        document.defs.font_css = document
            .fonts
            .font_face_css(&self.options.font_resolution)?;

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

    fn write_background(
        &self,
        document: &mut SvgDocument,
        width: f32,
        height: f32,
    ) -> Result<(), AvengerSvgError> {
        let color = match self.options.background {
            SvgBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            SvgBackground::Transparent => None,
            SvgBackground::Color(color) => Some(color),
        };

        let Some(color) = color else {
            return Ok(());
        };

        let output = &mut document.body;
        output.push_str(r#"<rect x="0" y="0" width=""#);
        output.push_str(&format_number(width, self.options.precision)?);
        output.push_str(r#"" height=""#);
        output.push_str(&format_number(height, self.options.precision)?);
        output.push('"');
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
        let text_engine = self.text_engine()?;
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

            let output_font = resolve_font_family_for_output(
                font,
                font_weight,
                font_style,
                &self.options.font_resolution,
            )?;
            let text = truncate_text_to_limit(
                text,
                *limit,
                &output_font,
                *font_size,
                font_weight,
                font_style,
                mark.text_syntax,
                &text_engine,
            );
            let target = [target[0] + origin[0], target[1] + origin[1]];
            let label = [label[0] + origin[0], label[1] + origin[1]];
            let typst_text_path_buffer = self.extract_typst_text_paths(
                &text_engine,
                &text,
                color,
                &output_font,
                *font_size,
                font_weight,
                font_style,
                mark.text_syntax,
            )?;
            let text_bounds = typst_text_path_buffer.bounds.clone();
            if self.options.font_embedding == crate::options::SvgFontEmbedding::EmbedSubsetWoff2 {
                for run in &typst_text_path_buffer.plain_runs {
                    document.fonts.collect_text(
                        &run.font,
                        &run.font_weight,
                        &run.font_style,
                        &run.text,
                        &self.options.font_resolution,
                    )?;
                }
            }
            if *leader {
                if let Some(geometry) = compute_text_leader_geometry(TextLeaderGeometryInput {
                    target,
                    label_anchor: label,
                    angle_degrees: *angle,
                    text_bounds: &text_bounds,
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
                    self.write_text_leader(
                        document,
                        &geometry,
                        leader_stroke,
                        *leader_stroke_width,
                        *leader_stroke_cap,
                        *leader_stroke_join,
                        leader_stroke_dash_values
                            .as_ref()
                            .and_then(|values| values.get(index).map(Vec::as_slice)),
                        clip_id,
                    )?;
                }
            }

            self.write_typst_text_paths(
                document,
                &typst_text_path_buffer,
                label,
                align,
                baseline,
                *angle,
                color,
                &output_font,
                *font_size,
                font_weight,
                font_style,
                clip_id,
            )?;
        }

        Ok(())
    }

    fn text_engine(&self) -> Result<TextEngine, AvengerSvgError> {
        TextEngine::with_default_config().map_err(|err| AvengerSvgError::Text(err.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    fn extract_typst_text_paths(
        &self,
        text_engine: &TextEngine,
        text: &String,
        color: &ColorOrGradient,
        font: &String,
        font_size: f32,
        font_weight: &FontWeight,
        font_style: &FontStyle,
        syntax_mode: TextSyntaxMode,
    ) -> Result<TextPathBuffer, AvengerSvgError> {
        let path_color = match color {
            ColorOrGradient::Color(color) => *color,
            ColorOrGradient::GradientIndex(_) => [0.0, 0.0, 0.0, 1.0],
        };

        let config = TextPathExtractionConfig {
            text,
            color: path_color,
            font,
            font_size,
            font_weight: *font_weight,
            font_style: *font_style,
            limit: f32::INFINITY,
            syntax_mode,
        };
        let buffer = text_engine
            .extract_paths_with_plain_fallback(&config)
            .map_err(|err| AvengerSvgError::Text(err.to_string()))?;
        if matches!(color, ColorOrGradient::GradientIndex(_)) && !buffer.items.is_empty() {
            return Err(AvengerSvgError::UnsupportedPaint(
                "Typst SVG math paths do not support gradient text paint".to_string(),
            ));
        }

        Ok(buffer)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_typst_text_paths(
        &self,
        document: &mut SvgDocument,
        buffer: &TextPathBuffer,
        label: [f32; 2],
        align: &avenger_text::types::TextAlign,
        baseline: &avenger_text::types::TextBaseline,
        angle: f32,
        color: &ColorOrGradient,
        _font: &str,
        _font_size: f32,
        _font_weight: &FontWeight,
        _font_style: &FontStyle,
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let [x, text_top] = buffer.bounds.calculate_origin(label, align, baseline);
        document.body.push_str("<g");
        if angle != 0.0 {
            document.body.push_str(r#" transform="rotate("#);
            push_number(&mut document.body, angle, self.options.precision)?;
            document.body.push(' ');
            push_number(&mut document.body, label[0], self.options.precision)?;
            document.body.push(' ');
            push_number(&mut document.body, label[1], self.options.precision)?;
            document.body.push(')');
            document.body.push('"');
        }
        push_clip_attr(&mut document.body, clip_id);
        document.body.push_str(">\n");

        for draw_item in &buffer.draw_items {
            match *draw_item {
                TextPathDrawItem::PlainRun(index) => {
                    let Some(run) = buffer.plain_runs.get(index) else {
                        return Err(AvengerSvgError::Text(
                            "Typst SVG text buffer referenced a missing plain run".to_string(),
                        ));
                    };
                    if self.options.rasterize_color_emoji && is_color_emoji_family(&run.font) {
                        continue;
                    }
                    self.write_plain_text_run(
                        document,
                        &run.text,
                        x + run.x,
                        text_top + run.y_offset + run.bounds.ascent,
                        color,
                        &run.font,
                        run.font_size,
                        &run.font_weight,
                        &run.font_style,
                    )?;
                }
                TextPathDrawItem::PathItem(index) => {
                    let Some(item) = buffer.items.get(index) else {
                        return Err(AvengerSvgError::Text(
                            "Typst SVG text buffer referenced a missing path item".to_string(),
                        ));
                    };
                    self.write_math_text_path_item(document, item, x, text_top)?;
                }
                TextPathDrawItem::ImageItem(index) => {
                    if !self.options.rasterize_color_emoji {
                        continue;
                    }
                    let Some(item) = buffer.images.get(index) else {
                        return Err(AvengerSvgError::Text(
                            "Typst SVG text buffer referenced a missing image item".to_string(),
                        ));
                    };
                    self.write_text_path_image_item(document, item, x, text_top)?;
                }
            }
        }

        document.body.push_str("</g>\n");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_plain_text_run(
        &self,
        document: &mut SvgDocument,
        text: &str,
        x: f32,
        y: f32,
        color: &ColorOrGradient,
        font: &str,
        font_size: f32,
        font_weight: &FontWeight,
        font_style: &FontStyle,
    ) -> Result<(), AvengerSvgError> {
        document.body.push_str(r#"<text x=""#);
        push_number(&mut document.body, x, self.options.precision)?;
        document.body.push_str(r#"" y=""#);
        push_number(&mut document.body, y, self.options.precision)?;
        document.body.push('"');
        push_color_or_text_paint(&mut document.body, color, self.options.precision)?;
        document
            .body
            .push_str(r#" text-anchor="start" dominant-baseline="alphabetic""#);
        document.body.push_str(r#" font-family=""#);
        document.body.push_str(&crate::style::escape_attr(font));
        document.body.push('"');
        document.body.push_str(r#" font-size=""#);
        push_number(&mut document.body, font_size, self.options.precision)?;
        document.body.push('"');
        document.body.push_str(r#" font-weight=""#);
        document
            .body
            .push_str(&font_weight_value(font_weight, self.options.precision)?);
        document.body.push('"');
        document.body.push_str(r#" font-style=""#);
        document.body.push_str(font_style_value(font_style));
        document.body.push_str(r#"" xml:space="preserve">"#);
        document.body.push_str(&crate::style::escape_text(text));
        document.body.push_str("</text>\n");
        Ok(())
    }

    fn write_math_text_path_item(
        &self,
        document: &mut SvgDocument,
        item: &TextPathItem,
        x: f32,
        y: f32,
    ) -> Result<(), AvengerSvgError> {
        let d = lyon_path_to_svg_d(&item.path, self.options.precision)?;
        if d.is_empty() {
            return Ok(());
        }

        document.body.push_str(r#"<path d=""#);
        document.body.push_str(&d);
        document.body.push('"');
        if let Some(fill) = item.fill {
            push_color_attrs(&mut document.body, "fill", fill, self.options.precision)?;
        } else {
            document.body.push_str(r#" fill="none""#);
        }
        if let Some(stroke) = &item.stroke {
            push_color_attrs(
                &mut document.body,
                "stroke",
                stroke.color,
                self.options.precision,
            )?;
            document.body.push_str(r#" stroke-width=""#);
            push_number(&mut document.body, stroke.width, self.options.precision)?;
            document.body.push('"');
            document.body.push_str(r#" stroke-linecap=""#);
            document.body.push_str(match stroke.line_cap {
                TextPathStrokeCap::Butt => "butt",
                TextPathStrokeCap::Round => "round",
                TextPathStrokeCap::Square => "square",
            });
            document.body.push('"');
            document.body.push_str(r#" stroke-linejoin=""#);
            document.body.push_str(match stroke.line_join {
                TextPathStrokeJoin::Bevel => "bevel",
                TextPathStrokeJoin::Miter => "miter",
                TextPathStrokeJoin::Round => "round",
            });
            document.body.push('"');
            if let Some(dash) = &stroke.dash {
                if !dash.is_empty() {
                    document.body.push_str(r#" stroke-dasharray=""#);
                    for (index, value) in dash.iter().enumerate() {
                        if index > 0 {
                            document.body.push(' ');
                        }
                        push_number(&mut document.body, *value, self.options.precision)?;
                    }
                    document.body.push('"');
                }
            }
        }
        document.body.push_str(r#" transform="translate("#);
        push_number(&mut document.body, x, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, y, self.options.precision)?;
        document.body.push_str(r#")"/>"#);
        document.body.push('\n');
        Ok(())
    }

    fn write_text_path_image_item(
        &self,
        document: &mut SvgDocument,
        item: &TextPathImageItem,
        x: f32,
        y: f32,
    ) -> Result<(), AvengerSvgError> {
        let TextPathImageFormat::Png = item.format;
        let [xx, yx, xy, yy, dx, dy] = item.transform;
        document.body.push_str(r#"<image x="0" y="0" width=""#);
        push_number(&mut document.body, item.width, self.options.precision)?;
        document.body.push_str(r#"" height=""#);
        push_number(&mut document.body, item.height, self.options.precision)?;
        document
            .body
            .push_str(r#"" preserveAspectRatio="none" href="data:image/png;base64,"#);
        document.body.push_str(&BASE64_STANDARD.encode(&item.data));
        document.body.push_str(r#"" transform="matrix("#);
        push_number(&mut document.body, xx, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, yx, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, xy, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, yy, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, x + dx, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, y + dy, self.options.precision)?;
        document.body.push_str(r#")"/>"#);
        document.body.push('\n');
        Ok(())
    }

    fn write_text_leader(
        &self,
        document: &mut SvgDocument,
        geometry: &TextLeaderGeometry,
        stroke: &ColorOrGradient,
        stroke_width: f32,
        stroke_cap: StrokeCap,
        stroke_join: StrokeJoin,
        stroke_dash: Option<&[f32]>,
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        self.write_path_element(
            document,
            &text_leader_path_d(&geometry.spine, self.options.precision)?,
            PathStyle {
                fill: None,
                stroke: Some(stroke),
                stroke_width: Some(stroke_width.max(0.0)),
                stroke_cap: Some(stroke_cap),
                stroke_join: Some(stroke_join),
                stroke_dash,
                gradients: &[],
            },
            clip_id,
        )?;

        if let Some(arrowhead) = &geometry.arrowhead {
            match arrowhead {
                TextLeaderArrowhead::Open { .. } => {
                    self.write_path_element(
                        document,
                        &text_leader_arrowhead_d(arrowhead, self.options.precision)?,
                        PathStyle {
                            fill: None,
                            stroke: Some(stroke),
                            stroke_width: Some(stroke_width.max(0.0)),
                            stroke_cap: Some(stroke_cap),
                            stroke_join: Some(stroke_join),
                            stroke_dash: None,
                            gradients: &[],
                        },
                        clip_id,
                    )?;
                }
                TextLeaderArrowhead::Triangle { .. } => {
                    self.write_path_element(
                        document,
                        &text_leader_arrowhead_d(arrowhead, self.options.precision)?,
                        PathStyle {
                            fill: Some(stroke),
                            stroke: None,
                            stroke_width: None,
                            stroke_cap: None,
                            stroke_join: None,
                            stroke_dash: None,
                            gradients: &[],
                        },
                        clip_id,
                    )?;
                }
            }
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
        for (image_source, path) in
            izip!(mark.image_source_iter(), mark.transformed_path_iter(origin))
        {
            let Some(image) = image_source.inline_image() else {
                return Err(AvengerSvgError::UnsupportedFeature(
                    "resource-backed image marks require resolution before SVG rendering"
                        .to_string(),
                ));
            };
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
        if d.is_empty() {
            return Ok(());
        }

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
    fonts: SvgFontCollector,
    body: String,
}

#[derive(Default)]
struct SvgDefs {
    font_css: String,
    body: String,
    gradient_ids: Vec<(Gradient, String)>,
    clip_ids: Vec<(Clip, String)>,
    next_gradient_id: usize,
    next_clip_id: usize,
}

impl SvgDefs {
    fn write_defs(&self, output: &mut String) {
        if self.body.is_empty() && self.font_css.is_empty() {
            output.push_str("<defs/>\n");
        } else {
            output.push_str("<defs>\n");
            if !self.font_css.is_empty() {
                output.push_str("<style><![CDATA[\n");
                output.push_str(&self.font_css);
                output.push_str("]]></style>\n");
            }
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
                push_gradient_unit(&mut self.body, gradient.x0, precision)?;
                self.body.push_str(r#"" y1=""#);
                push_gradient_unit(&mut self.body, gradient.y0, precision)?;
                self.body.push_str(r#"" x2=""#);
                push_gradient_unit(&mut self.body, gradient.x1, precision)?;
                self.body.push_str(r#"" y2=""#);
                push_gradient_unit(&mut self.body, gradient.y1, precision)?;
                self.body.push_str("\">\n");
                self.write_gradient_stops(gradient.stops.as_slice(), precision)?;
                self.body.push_str("</linearGradient>\n");
            }
            Gradient::RadialGradient(gradient) => {
                let radial_id = format!("{id}-radial");
                self.body.push_str(r#"<pattern id=""#);
                self.body.push_str(id);
                self.body.push_str(
                    r#"" viewBox="0 0 1 1" width="100%" height="100%" preserveAspectRatio="xMidYMid slice">"#,
                );
                self.body
                    .push_str(r#"<rect width="1" height="1" fill="url(#"#);
                self.body.push_str(&radial_id);
                self.body.push_str(r#")"/></pattern>"#);
                self.body.push('\n');

                self.body.push_str(r#"<radialGradient id=""#);
                self.body.push_str(&radial_id);
                self.body
                    .push_str(r#"" gradientUnits="objectBoundingBox" fx=""#);
                push_gradient_unit(&mut self.body, gradient.x0, precision)?;
                self.body.push_str(r#"" fy=""#);
                push_gradient_unit(&mut self.body, gradient.y0, precision)?;
                self.body.push_str(r#"" cx=""#);
                push_gradient_unit(&mut self.body, gradient.x1, precision)?;
                self.body.push_str(r#"" cy=""#);
                push_gradient_unit(&mut self.body, gradient.y1, precision)?;
                self.body.push_str(r#"" fr=""#);
                push_gradient_unit(&mut self.body, gradient.r0, precision)?;
                self.body.push_str(r#"" r=""#);
                push_gradient_unit(&mut self.body, gradient.r1, precision)?;
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

fn push_gradient_unit(
    output: &mut String,
    value: f32,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let value = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    push_number(output, value, precision)
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

fn is_color_emoji_family(family: &str) -> bool {
    matches!(
        family.to_ascii_lowercase().as_str(),
        "apple color emoji" | "noto color emoji" | "twitter color emoji" | "segoe ui emoji"
    )
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

fn truncate_text_to_limit(
    text: &str,
    limit: f32,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
    syntax_mode: TextSyntaxMode,
    text_engine: &TextEngine,
) -> String {
    if !limit.is_finite() {
        return text.to_string();
    }

    truncate_text_to_limit_with(text, limit, |candidate| {
        let config = TextMeasurementConfig {
            text: candidate,
            font,
            font_size,
            font_weight: *font_weight,
            font_style: *font_style,
            syntax_mode,
        };
        Ok::<_, std::convert::Infallible>(
            text_engine
                .measure_bounds_with_plain_fallback_or_approx(&config)
                .width,
        )
    })
    .unwrap_or_else(|_| text.to_string())
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

fn text_leader_path_d(path: &TextLeaderPath, precision: usize) -> Result<String, AvengerSvgError> {
    let mut d = String::new();
    match path {
        TextLeaderPath::Line { start, end } => {
            d.push('M');
            push_point(&mut d, start[0], start[1], precision)?;
            d.push(' ');
            d.push('L');
            push_point(&mut d, end[0], end[1], precision)?;
        }
        TextLeaderPath::Polyline { points } => {
            if let Some(first) = points.first() {
                d.push('M');
                push_point(&mut d, first[0], first[1], precision)?;
                for point in points.iter().skip(1) {
                    d.push(' ');
                    d.push('L');
                    push_point(&mut d, point[0], point[1], precision)?;
                }
            }
        }
        TextLeaderPath::Cubic {
            start,
            ctrl1,
            ctrl2,
            end,
        } => {
            d.push('M');
            push_point(&mut d, start[0], start[1], precision)?;
            d.push_str(" C");
            push_point(&mut d, ctrl1[0], ctrl1[1], precision)?;
            d.push(' ');
            push_point(&mut d, ctrl2[0], ctrl2[1], precision)?;
            d.push(' ');
            push_point(&mut d, end[0], end[1], precision)?;
        }
    }
    Ok(d)
}

fn text_leader_arrowhead_d(
    arrowhead: &TextLeaderArrowhead,
    precision: usize,
) -> Result<String, AvengerSvgError> {
    let mut d = String::new();
    match arrowhead {
        TextLeaderArrowhead::Open { left, right } => {
            d.push('M');
            push_point(&mut d, left[0][0], left[0][1], precision)?;
            d.push(' ');
            d.push('L');
            push_point(&mut d, left[1][0], left[1][1], precision)?;
            d.push(' ');
            d.push('M');
            push_point(&mut d, right[0][0], right[0][1], precision)?;
            d.push(' ');
            d.push('L');
            push_point(&mut d, right[1][0], right[1][1], precision)?;
        }
        TextLeaderArrowhead::Triangle { points } => {
            d.push('M');
            push_point(&mut d, points[0][0], points[0][1], precision)?;
            d.push(' ');
            d.push('L');
            push_point(&mut d, points[1][0], points[1][1], precision)?;
            d.push(' ');
            d.push('L');
            push_point(&mut d, points[2][0], points[2][1], precision)?;
            d.push('Z');
        }
    }
    Ok(d)
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
            image::{SceneImageMark, SceneImageSource},
            rect::SceneRectMark,
            rule::SceneRuleMark,
            symbol::SceneSymbolMark,
            text::SceneTextMark,
            trail::SceneTrailMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
    use base64::{prelude::BASE64_STANDARD, Engine};
    use font_subset::FontReader;

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
    fn renders_document_background_with_explicit_dimensions() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(
            r##"<rect x="0" y="0" width="20" height="10" fill="#ffffff" stroke="none"/>"##
        ));
        assert!(!svg.contains(r#"<rect width="100%" height="100%""#));
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
        assert!(svg.contains(r#"<pattern id="svg-gradient-1" viewBox="0 0 1 1" width="100%" height="100%" preserveAspectRatio="xMidYMid slice">"#));
        assert!(svg.contains(
            r#"<rect width="1" height="1" fill="url(#svg-gradient-1-radial)"/></pattern>"#
        ));
        assert!(svg.contains(r#"<radialGradient id="svg-gradient-1-radial" gradientUnits="objectBoundingBox" fx="0.5" fy="0.5" cx="0.5" cy="0.5" fr="0" r="0.5">"#));
        assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
        assert!(svg.contains(r#"stroke="url(#svg-gradient-1)""#));
        assert!(svg.contains(r##"stop-color="#0000ff" stop-opacity="0.5""##));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn clamps_gradient_control_points_to_object_bounding_box_units() {
        let stops = vec![
            GradientStop {
                offset: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            },
            GradientStop {
                offset: 1.0,
                color: [0.0, 0.0, 1.0, 1.0],
            },
        ];
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 500.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                gradients: vec![Gradient::LinearGradient(LinearGradient {
                    x0: 0.0,
                    y0: 491.0,
                    x1: 0.0,
                    y1: 0.0,
                    stops,
                })],
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(15.0)),
                height: Some(ScalarOrArray::new_scalar(491.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(
            r#"<linearGradient id="svg-gradient-0" gradientUnits="objectBoundingBox" x1="0" y1="1" x2="0" y2="0">"#
        ));
        assert!(!svg.contains(r#"y1="491""#));
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
                image: ScalarOrArray::new_scalar(SceneImageSource::Inline(RgbaImage {
                    width: 1,
                    height: 1,
                    data: vec![255, 0, 0, 255],
                })),
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
    fn resource_image_marks_error_without_svg_resource_resolution() {
        let scene_graph = resource_image_scene_graph();

        let err = SvgRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(err, AvengerSvgError::UnsupportedFeature(_)));
        assert!(err.to_string().contains("resource-backed image marks"));
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
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Bold)),
                font_style: ScalarOrArray::new_scalar(FontStyle::Italic),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r##" fill="#ff0000" fill-opacity="0.5" text-anchor="start" dominant-baseline="alphabetic""##));
        assert!(svg.contains("<style><![CDATA[\n@font-face"));
        assert!(svg.contains(r#"font-family: "Lato";"#));
        assert!(svg.contains("data:font/woff2;base64,"));
        assert!(svg.contains(r#"font-family="Lato""#));
        assert!(svg.contains(r#"font-size="12""#));
        assert!(svg.contains(r#"font-weight="bold""#));
        assert!(svg.contains(r#"font-style="italic""#));
        assert!(svg.contains(r#"transform="rotate(45 11 14)""#));
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
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.25, 1.0, 0.75])),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let math_svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

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
        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<text "));
        assert!(svg.contains("Mood "));
        assert!(svg.contains("😀"));
        assert!(svg.contains(r#"font-family="Apple Color Emoji""#));
        assert!(!svg.contains("#emoji.face"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn rasterizes_typst_named_emoji_when_requested() {
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
                rasterize_color_emoji: true,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(svg.contains("Mood "));
        assert!(svg.contains("<image "));
        assert!(svg.contains("data:image/png;base64,"));
        assert!(!svg.contains("😀"));
        assert!(!svg.contains("#emoji.face"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn renders_typst_static_sub_super_as_smaller_native_svg_text() {
        let scene_graph = SceneGraph {
            width: 120.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("H#sub[2]O #super[\\*]".to_string()),
                x: ScalarOrArray::new_scalar(6.0),
                y: ScalarOrArray::new_scalar(18.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

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

        let svg = SvgRenderer::new()
            .with_options(SvgRenderOptions {
                font_resolution: avenger_text::FontResolutionOptions {
                    missing_font: avenger_text::MissingFontPolicy::Fallback,
                    ..Default::default()
                },
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(svg.contains(r#"font-family="Lato""#));
        assert!(!svg.contains(r#"font-family="Definitely Missing Font""#));
        assert!(svg.contains(r#"font-family: "Lato";"#));
    }

    #[test]
    fn truncates_text_marks_to_limit() {
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
                limit: ScalarOrArray::new_scalar(35.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("\u{2026}</text>"));
        assert!(!svg.contains("Long label text</text>"));
        let rendered_text = first_text_body(&svg);
        assert!(rendered_text.contains('\u{2026}'));
        assert!(!rendered_text.contains('x'));

        let woff2 = first_woff2_payload(&svg);
        let reader = FontReader::new(&woff2).unwrap();
        let font = reader.read().unwrap();
        assert!(font.contains_char('\u{2026}'));
        assert!(!font.contains_char('x'));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn honors_svg_font_embedding_none() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Label".to_string()),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Missing Display Face".to_string()),
                font_size: ScalarOrArray::new_scalar(10.0),
                ..Default::default()
            }
            .into()],
        };

        let svg = SvgRenderer::new()
            .with_options(SvgRenderOptions {
                font_embedding: crate::options::SvgFontEmbedding::None,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(!svg.contains("@font-face"));
        assert!(svg.contains(r#"font-family="Missing Display Face""#));
    }

    #[test]
    fn errors_for_missing_named_svg_fonts_when_embedding_is_required() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Label".to_string()),
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Missing Display Face".to_string()),
                font_size: ScalarOrArray::new_scalar(10.0),
                ..Default::default()
            }
            .into()],
        };

        let err = SvgRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(err, AvengerSvgError::Font(_)));
        assert!(err.to_string().contains("Missing Display Face"));
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

    fn resource_image_scene_graph() -> SceneGraph {
        SceneGraph {
            width: 8.0,
            height: 8.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                image: ScalarOrArray::new_scalar(SceneImageSource::Resource(
                    avenger_scenegraph::marks::image::SceneImageResource {
                        key: "tile/0/0/0".into(),
                        intrinsic_width: 2,
                        intrinsic_height: 2,
                        fallback_key: None,
                    },
                )),
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: ScalarOrArray::new_scalar(8.0),
                height: ScalarOrArray::new_scalar(8.0),
                ..Default::default()
            }
            .into()],
        }
    }
}
