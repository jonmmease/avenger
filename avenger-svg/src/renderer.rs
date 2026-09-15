use avenger_scenegraph::path_geometry::{needs_explicit_caps, stroke_outline, GradientBounds};
use std::io::Cursor;

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::types::{FillRule, StrokeCap, StrokeJoin, SCENE_MITER_LIMIT};
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
        pattern::{PatternFill, PatternLayerOperation, PatternReferenceFrame},
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        text_leader::{
            compute_text_leader_geometry, TextLeaderArrowhead, TextLeaderGeometry,
            TextLeaderGeometryInput,
        },
        trail::SceneTrailMark,
    },
    pattern_geometry::{
        build_layered_pattern_geometry, LayeredPatternGeometry, PatternCoverageLayer,
        PatternCoveragePrimitive, PatternGeometryError, PatternRect, PatternRenderContext,
    },
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_text::{
    path::{
        TextPathBuffer, TextPathDrawItem, TextPathExtractionConfig, TextPathImageFormat,
        TextPathImageItem, TextPathItem, TextPathLineCap, TextPathLineJoin,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec},
    TextEngine,
};
use base64::{prelude::BASE64_STANDARD, Engine};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;

use crate::{
    error::AvengerSvgError,
    fonts::SvgFontCollector,
    options::{SvgBackground, SvgRenderOptions},
    path::{format_number, lyon_path_to_svg_d, push_number, push_point},
    style::{
        push_color_attrs, push_fill_attrs, push_stop_color_attrs, push_stroke_attrs, PaintResolver,
    },
};

/// Export scene graphs as self-contained SVG documents.
#[derive(Debug, Clone, Default)]
pub struct SvgRenderer {
    options: SvgRenderOptions,
    text_engine: Option<TextEngine>,
}

impl SvgRenderer {
    /// Create a renderer with bundled fonts and a white background.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set output options. A supplied text engine takes precedence over font options.
    pub fn with_options(mut self, options: SvgRenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Use the same engine and resolved fonts as scene layout.
    pub fn with_text_engine(mut self, text_engine: TextEngine) -> Self {
        self.text_engine = Some(text_engine);
        self
    }

    /// Export one scene. Image resources must be resolved before export.
    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<String, AvengerSvgError> {
        if !scene_graph.width.is_finite()
            || !scene_graph.height.is_finite()
            || scene_graph.width <= 0.0
            || scene_graph.height <= 0.0
        {
            return Err(AvengerSvgError::InvalidGeometry(
                "SVG dimensions must be positive and finite".into(),
            ));
        }
        let mut document = SvgDocument {
            text_engine: match &self.text_engine {
                Some(engine) => engine.clone(),
                None => TextEngine::with_font_resolution(&self.options.font_resolution)
                    .map_err(|err| AvengerSvgError::Text(err.to_string()))?,
            },
            viewport: [scene_graph.width, scene_graph.height],
            defs: SvgDefs::default(),
            fonts: SvgFontCollector::default(),
            body: String::new(),
        };
        let precision = self.options.precision;
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);

        document.body.push_str(&format!(
            "<g fill=\"none\" stroke-miterlimit=\"{SCENE_MITER_LIMIT}\">\n"
        ));
        self.write_background(&mut document, scene_graph.width, scene_graph.height)?;
        let chart_bounds = PatternRect::new(0.0, 0.0, scene_graph.width, scene_graph.height);

        for item in display_list.ordered_items() {
            let clip_id = document.defs.clip_id(&item.clip, precision)?;

            match &item.mark {
                SceneDisplayMark::OwnedGroupPath(mark) => {
                    self.write_path_mark(
                        &mut document,
                        mark,
                        item.origin,
                        clip_id.as_deref(),
                        item.pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
                SceneDisplayMark::Borrowed(mark) => {
                    self.write_scene_mark(
                        &mut document,
                        mark,
                        item.origin,
                        clip_id.as_deref(),
                        item.pattern_reference_frame.as_ref(),
                        chart_bounds,
                    )?;
                }
            }
        }

        document.body.push_str("</g>\n");
        document.defs.font_css = document.fonts.font_face_css()?;

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
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        match mark {
            SceneMark::Rect(mark) => self.write_rect_mark(
                document,
                mark,
                origin,
                clip_id,
                pattern_reference_frame,
                chart_bounds,
            ),
            SceneMark::Path(mark) => self.write_path_mark(
                document,
                mark,
                origin,
                clip_id,
                pattern_reference_frame,
                chart_bounds,
            ),
            SceneMark::Rule(mark) => self.write_rule_mark(document, mark, origin, clip_id),
            SceneMark::Line(mark) => self.write_line_mark(document, mark, origin, clip_id),
            SceneMark::Area(mark) => self.write_area_mark(
                document,
                mark,
                origin,
                clip_id,
                pattern_reference_frame,
                chart_bounds,
            ),
            SceneMark::Symbol(mark) => self.write_symbol_mark(
                document,
                mark,
                origin,
                clip_id,
                pattern_reference_frame,
                chart_bounds,
            ),
            SceneMark::Arc(mark) => self.write_arc_mark(
                document,
                mark,
                origin,
                clip_id,
                pattern_reference_frame,
                chart_bounds,
            ),
            SceneMark::Trail(mark) => self.write_trail_mark(document, mark, origin, clip_id),
            SceneMark::Text(mark) => self.write_text_mark(document, mark, origin, clip_id),
            SceneMark::Image(mark) => self.write_image_mark(document, mark, origin, clip_id),
            SceneMark::WarpedImage(mark) => {
                self.write_warped_image_mark(document, mark, origin, clip_id)
            }
            SceneMark::Group(_) => Ok(()),
        }
    }

    fn write_rect_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneRectMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, fill_pattern, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.write_filled_path_with_optional_pattern(
                document,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    gradient_bounds: None,
                    fill_rule: FillRule::NonZero,
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
                pattern_reference_frame,
                chart_bounds,
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
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, fill_pattern, stroke) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter()
        ) {
            self.write_filled_path_with_optional_pattern(
                document,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    gradient_bounds: None,
                    fill_rule: mark.fill_rule,
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: Some(mark.stroke_cap),
                    stroke_join: Some(mark.stroke_join),
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
                pattern_reference_frame,
                chart_bounds,
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
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, fill_pattern, stroke, x, y, size) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.size_iter()
        ) {
            self.write_filled_path_with_optional_pattern(
                document,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    gradient_bounds: Some(GradientBounds::symbol(
                        [x + origin[0], y + origin[1]],
                        *size,
                    )),
                    fill_rule: mark.fill_rule,
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: mark.stroke_width,
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
                pattern_reference_frame,
                chart_bounds,
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
        let centerline = mark.transformed_path(origin);
        let outline = avenger_scenegraph::path_geometry::trail_outline(&centerline, 0.05, 0)
            .map_err(|error| AvengerSvgError::InvalidGeometry(error.to_string()))?;
        let d = lyon_path_to_svg_d(&outline, self.options.precision)?;
        self.write_path_element(
            document,
            &d,
            PathStyle {
                gradient_bounds: Some(GradientBounds::from_path(&centerline)),
                fill_rule: FillRule::NonZero,
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
        let text_engine = document.text_engine.clone();
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

            let target = [target[0] + origin[0], target[1] + origin[1]];
            let label = [label[0] + origin[0], label[1] + origin[1]];
            let path_color = match color {
                ColorOrGradient::Color(color) => *color,
                ColorOrGradient::GradientIndex(_) => {
                    return Err(AvengerSvgError::UnsupportedPaint(
                        "SVG text does not support gradient paint".into(),
                    ))
                }
            };
            let typst_text_path_buffer = text_engine
                .extract_paths_with_plain_fallback(&TextPathExtractionConfig {
                    text,
                    color: path_color,
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
                })
                .map_err(|err| AvengerSvgError::Text(err.to_string()))?;
            let text_bounds = typst_text_path_buffer.bounds.clone();
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
                clip_id,
            )?;
        }

        Ok(())
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
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let [x, text_top] = buffer.bounds.calculate_origin(label, align, baseline);
        // The scene clip stays in scene coordinates outside the label rotation.
        document.body.push_str("<g");
        push_clip_attr(&mut document.body, clip_id);
        document.body.push_str(">\n<g transform=\"rotate(");
        push_number(&mut document.body, angle, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, label[0], self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, label[1], self.options.precision)?;
        document.body.push_str(") translate(");
        push_number(&mut document.body, x, self.options.precision)?;
        document.body.push(' ');
        push_number(&mut document.body, text_top, self.options.precision)?;
        document.body.push_str(")\"");
        if let Some(width) = buffer.clip_width {
            // These other edges lie outside the viewport in label coordinates.
            let margin = document.viewport[0]
                + document.viewport[1]
                + 2.0 * (label[0].abs() + label[1].abs())
                + x.abs()
                + text_top.abs()
                + width;
            let limit_clip = document.defs.clip_id(
                &Clip::Rect {
                    x: -margin,
                    y: -margin,
                    width: margin + width,
                    height: 2.0 * margin,
                },
                self.options.precision,
            )?;
            push_clip_attr(&mut document.body, limit_clip.as_deref());
        }
        document.body.push_str(">\n");

        for draw_item in &buffer.draw_items {
            match *draw_item {
                TextPathDrawItem::PlainRun(index) => {
                    let Some(run) = buffer.plain_runs.get(index) else {
                        return Err(AvengerSvgError::Text(
                            "Typst SVG text buffer referenced a missing plain run".to_string(),
                        ));
                    };
                    if self.options.rasterize_color_emoji
                        && buffer
                            .images
                            .iter()
                            .any(|image| image.byte_range == run.byte_range)
                    {
                        continue;
                    }
                    let font = if self.options.font_embedding
                        == crate::options::SvgFontEmbedding::EmbedSubsetWoff2
                    {
                        document.fonts.collect_run(run)?
                    } else {
                        run.font.clone()
                    };
                    self.write_plain_text_run(document, run, &font)?;
                }
                TextPathDrawItem::PathItem(index) => {
                    let Some(item) = buffer.items.get(index) else {
                        return Err(AvengerSvgError::Text(
                            "Typst SVG text buffer referenced a missing path item".to_string(),
                        ));
                    };
                    self.write_text_path_item(document, item, 0.0, 0.0)?;
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
                    self.write_text_path_image_item(document, item, 0.0, 0.0)?;
                }
            }
        }

        document.body.push_str("</g>\n</g>\n");
        Ok(())
    }

    fn write_plain_text_run(
        &self,
        document: &mut SvgDocument,
        run: &avenger_text::path::PlainTextPathRun,
        font: &str,
    ) -> Result<(), AvengerSvgError> {
        let x = run.x + if run.is_rtl { run.bounds.width } else { 0.0 };
        let y = run.y_offset + run.bounds.ascent;
        let font_size = run.font_size;
        let font_weight = &run.font_weight;
        let font_style = &run.font_style;
        let text = &run.text;
        document.body.push_str(r#"<text x=""#);
        push_number(&mut document.body, x, self.options.precision)?;
        document.body.push_str(r#"" y=""#);
        push_number(&mut document.body, y, self.options.precision)?;
        document.body.push('"');
        push_color_attrs(
            &mut document.body,
            "fill",
            run.color,
            self.options.precision,
        )?;
        if run.is_rtl {
            document
                .body
                .push_str(r#" direction="rtl" unicode-bidi="isolate""#);
        }
        document.body.push_str(r#" style="font-synthesis:none""#);
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

    fn write_text_path_item(
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
                TextPathLineCap::Butt => "butt",
                TextPathLineCap::Round => "round",
                TextPathLineCap::Square => "square",
            });
            document.body.push('"');
            document.body.push_str(r#" stroke-linejoin=""#);
            document.body.push_str(match stroke.line_join {
                TextPathLineJoin::Bevel => "bevel",
                TextPathLineJoin::Miter => "miter",
                TextPathLineJoin::Round => "round",
            });
            document.body.push('"');
            document.body.push_str(r#" stroke-miterlimit=""#);
            push_number(
                &mut document.body,
                stroke.miter_limit,
                self.options.precision,
            )?;
            document.body.push('"');
            if let Some(dash) = &stroke.dash {
                if !dash.array.is_empty() {
                    document.body.push_str(r#" stroke-dashoffset=""#);
                    push_number(&mut document.body, dash.phase, self.options.precision)?;
                    document.body.push('"');
                    document.body.push_str(r#" stroke-dasharray=""#);
                    for (index, value) in dash.array.iter().enumerate() {
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

    // Keep the complete geometry and paint inputs together.
    #[allow(clippy::too_many_arguments)]
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
        self.write_scene_path(
            document,
            &geometry.spine.to_lyon(),
            PathStyle {
                gradient_bounds: None,
                fill_rule: FillRule::NonZero,
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
                            gradient_bounds: None,
                            fill_rule: FillRule::NonZero,
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
                            gradient_bounds: None,
                            fill_rule: FillRule::NonZero,
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
                document
                    .body
                    .push_str(r#" style="image-rendering:pixelated""#);
            }
            push_clip_attr(&mut document.body, clip_id);
            document.body.push_str("/>\n");
        }

        Ok(())
    }

    fn write_warped_image_mark(
        &self,
        document: &mut SvgDocument,
        mark: &avenger_scenegraph::marks::warped_image::SceneWarpedImageMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        if mark.image.inline_image().is_none() {
            return Err(AvengerSvgError::UnsupportedFeature(
                "resource-backed warped image marks require resolution before SVG rendering"
                    .to_string(),
            ));
        }
        // SVG has no textured-mesh primitive; rasterize the mesh at 2x
        // supersampling and embed the result at its bounding box.
        let Some((raster, [min_x, min_y, max_x, max_y])) = mark.rasterize(origin, 2.0) else {
            return Ok(());
        };
        let width = max_x - min_x;
        let height = max_y - min_y;
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }
        let data_uri = rgba_image_to_png_data_uri(&raster)?;
        document.body.push_str(r##"<image x=""##);
        push_number(&mut document.body, min_x, self.options.precision)?;
        document.body.push_str(r##"" y=""##);
        push_number(&mut document.body, min_y, self.options.precision)?;
        document.body.push_str(r##"" width=""##);
        push_number(&mut document.body, width, self.options.precision)?;
        document.body.push_str(r##"" height=""##);
        push_number(&mut document.body, height, self.options.precision)?;
        document
            .body
            .push_str(r##"" preserveAspectRatio="none" href=""##);
        document.body.push_str(&data_uri);
        document.body.push('"');
        push_clip_attr(&mut document.body, clip_id);
        document.body.push_str("/>\n");
        Ok(())
    }

    fn write_arc_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneArcMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        for (path, fill, fill_pattern, stroke, stroke_width) in izip!(
            mark.transformed_path_iter(origin),
            mark.fill_iter(),
            mark.fill_pattern_iter(),
            mark.stroke_iter(),
            mark.stroke_width_iter()
        ) {
            self.write_filled_path_with_optional_pattern(
                document,
                &path,
                fill,
                fill_pattern.as_ref(),
                PathStyle {
                    gradient_bounds: None,
                    fill_rule: FillRule::NonZero,
                    fill: None,
                    stroke: Some(stroke),
                    stroke_width: Some(*stroke_width),
                    stroke_cap: None,
                    stroke_join: None,
                    stroke_dash: None,
                    gradients: &mark.gradients,
                },
                clip_id,
                pattern_reference_frame,
                chart_bounds,
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
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        let path = mark.transformed_path(origin);
        self.write_filled_path_with_optional_pattern(
            document,
            &path,
            &mark.fill,
            mark.fill_pattern.as_ref(),
            PathStyle {
                gradient_bounds: None,
                fill_rule: FillRule::NonZero,
                fill: None,
                stroke: Some(&mark.stroke),
                stroke_width: Some(mark.stroke_width),
                stroke_cap: Some(mark.stroke_cap),
                stroke_join: Some(mark.stroke_join),
                stroke_dash: mark.stroke_dash.as_deref(),
                gradients: &mark.gradients,
            },
            clip_id,
            pattern_reference_frame,
            chart_bounds,
        )
    }

    fn write_line_mark(
        &self,
        document: &mut SvgDocument,
        mark: &SceneLineMark,
        origin: [f32; 2],
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let path = mark.undashed_path(origin);
        self.write_scene_path(
            document,
            &path,
            PathStyle {
                gradient_bounds: Some(GradientBounds::from_path(&mark.transformed_path(origin))),
                fill_rule: FillRule::NonZero,
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

        let mut paths = mark.transformed_path_iter(origin);
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
            let bounds = paths.next().map(|path| GradientBounds::from_path(&path));
            let mut builder = lyon_path::Path::builder();
            builder.begin(lyon_path::math::point(*x1 + origin[0], *y1 + origin[1]));
            builder.line_to(lyon_path::math::point(*x2 + origin[0], *y2 + origin[1]));
            builder.end(false);
            let path = builder.build();
            let dash = stroke_dashes.get(index).map(|dash| dash.as_slice());
            if needs_explicit_caps(&path, dash) {
                self.write_scene_path(
                    document,
                    &path,
                    PathStyle {
                        gradient_bounds: bounds,
                        fill_rule: FillRule::NonZero,
                        fill: None,
                        stroke: Some(stroke),
                        stroke_width: Some(*stroke_width),
                        stroke_cap: Some(*stroke_cap),
                        stroke_join: None,
                        stroke_dash: dash,
                        gradients: &mark.gradients,
                    },
                    clip_id,
                )?;
                continue;
            }
            let defs = &mut document.defs;
            let body = &mut document.body;
            let mut resolver = PaintContext {
                bounds,
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

    // Keep the complete geometry and paint inputs together.
    #[allow(clippy::too_many_arguments)]
    fn write_filled_path_with_optional_pattern(
        &self,
        document: &mut SvgDocument,
        path: &lyon_path::Path,
        fill: &ColorOrGradient,
        fill_pattern: Option<&PatternFill>,
        stroke_style: PathStyle<'_>,
        clip_id: Option<&str>,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
    ) -> Result<(), AvengerSvgError> {
        let stroke_style = PathStyle {
            gradient_bounds: Some(
                stroke_style
                    .gradient_bounds
                    .unwrap_or_else(|| GradientBounds::from_path(path)),
            ),
            ..stroke_style
        };
        let Some(fill_pattern) = fill_pattern else {
            return self.write_scene_path(
                document,
                path,
                PathStyle {
                    fill: Some(fill),
                    ..stroke_style
                },
                clip_id,
            );
        };

        self.write_scene_path(
            document,
            path,
            PathStyle {
                gradient_bounds: stroke_style.gradient_bounds,
                fill_rule: stroke_style.fill_rule,
                fill: Some(fill),
                stroke: None,
                stroke_width: None,
                stroke_cap: None,
                stroke_join: None,
                stroke_dash: None,
                gradients: stroke_style.gradients,
            },
            clip_id,
        )?;

        self.write_pattern_overlay(
            document,
            path,
            fill,
            fill_pattern,
            stroke_style.gradients,
            clip_id,
            pattern_reference_frame,
            chart_bounds,
            stroke_style.fill_rule,
        )?;

        self.write_scene_path(
            document,
            path,
            PathStyle {
                fill: None,
                ..stroke_style
            },
            clip_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_pattern_overlay(
        &self,
        document: &mut SvgDocument,
        host_path: &lyon_path::Path,
        host_fill: &ColorOrGradient,
        fill_pattern: &PatternFill,
        gradients: &[Gradient],
        parent_clip_id: Option<&str>,
        pattern_reference_frame: Option<&PatternReferenceFrame>,
        chart_bounds: PatternRect,
        fill_rule: FillRule,
    ) -> Result<(), AvengerSvgError> {
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
            .map_err(pattern_geometry_error_to_svg_error)?
        else {
            return Ok(());
        };

        let host_clip_id = document.defs.clip_id(
            &Clip::Path {
                path: host_path.clone(),
                fill_rule,
            },
            self.options.precision,
        )?;
        if geometry.ink[3] <= 0.0 {
            return Ok(());
        }

        let mut opaque_ink = geometry.ink;
        let opacity = opaque_ink[3].clamp(0.0, 1.0);
        opaque_ink[3] = 1.0;

        if let Some(parent_clip_id) = parent_clip_id {
            document.body.push_str("<g");
            push_clip_attr(&mut document.body, Some(parent_clip_id));
            document.body.push_str(">\n");
        }

        if geometry
            .layers
            .iter()
            .all(|layer| matches!(layer.operation, PatternLayerOperation::Add))
        {
            document.body.push_str("<g");
            push_clip_attr(&mut document.body, host_clip_id.as_deref());
            if opacity < 1.0 {
                document.body.push_str(r#" opacity=""#);
                push_number(&mut document.body, opacity, self.options.precision)?;
                document.body.push('"');
            }
            document.body.push_str(">\n");

            for layer in &geometry.layers {
                push_pattern_primitives(
                    &mut document.body,
                    layer,
                    opaque_ink,
                    self.options.precision,
                )?;
            }

            document.body.push_str("</g>\n");
        } else {
            let Some(mask_id) =
                self.write_pattern_operation_mask(&mut document.defs, &geometry, host_bounds)?
            else {
                if parent_clip_id.is_some() {
                    document.body.push_str("</g>\n");
                }
                return Ok(());
            };
            let host_d = lyon_path_to_svg_d(host_path, self.options.precision)?;
            if !host_d.is_empty() {
                let mut resolver = PaintContext {
                    bounds: None,
                    defs: &mut document.defs,
                    gradients,
                };
                let pattern_fill = ColorOrGradient::Color(geometry.ink);
                document.body.push_str(r#"<path d=""#);
                document.body.push_str(&host_d);
                document.body.push('"');
                push_fill_attrs(
                    &mut document.body,
                    Some(&pattern_fill),
                    &mut resolver,
                    self.options.precision,
                )?;
                push_fill_rule(&mut document.body, "fill-rule", fill_rule);
                document.body.push_str(r#" stroke="none""#);
                push_mask_attr(&mut document.body, &mask_id);
                document.body.push_str("/>\n");
            }
        }

        if parent_clip_id.is_some() {
            document.body.push_str("</g>\n");
        }

        Ok(())
    }

    fn write_pattern_operation_mask(
        &self,
        defs: &mut SvgDefs,
        geometry: &LayeredPatternGeometry,
        bounds: PatternRect,
    ) -> Result<Option<String>, AvengerSvgError> {
        let mut previous_mask_id: Option<String> = None;
        let precision = self.options.precision;
        for layer in &geometry.layers {
            let layer_mask = defs.next_mask_id();
            push_pattern_mask_start(&mut defs.body, &layer_mask);
            push_pattern_mask_rect(&mut defs.body, bounds, "black", precision)?;
            push_pattern_primitives(&mut defs.body, layer, [1.0; 4], precision)?;
            defs.body.push_str("</mask>\n");

            let mask_id = defs.next_mask_id();
            push_pattern_mask_start(&mut defs.body, &mask_id);
            push_pattern_mask_rect(&mut defs.body, bounds, "black", precision)?;
            if let Some(previous) = previous_mask_id.as_deref() {
                push_masked_pattern_rect(&mut defs.body, bounds, "white", previous, precision)?;
            }
            let layer_color = if layer.operation == PatternLayerOperation::Subtract {
                "black"
            } else {
                "white"
            };
            push_masked_pattern_rect(&mut defs.body, bounds, layer_color, &layer_mask, precision)?;
            if layer.operation == PatternLayerOperation::Xor {
                if let Some(previous) = previous_mask_id.as_deref() {
                    defs.body.push_str("<g");
                    push_mask_attr(&mut defs.body, previous);
                    defs.body.push_str(">\n");
                    push_masked_pattern_rect(
                        &mut defs.body,
                        bounds,
                        "black",
                        &layer_mask,
                        precision,
                    )?;
                    defs.body.push_str("</g>\n");
                }
            }
            defs.body.push_str("</mask>\n");
            previous_mask_id = Some(mask_id);
        }
        Ok(previous_mask_id)
    }

    fn write_scene_path(
        &self,
        document: &mut SvgDocument,
        path: &lyon_path::Path,
        style: PathStyle<'_>,
        clip_id: Option<&str>,
    ) -> Result<(), AvengerSvgError> {
        let d = lyon_path_to_svg_d(path, self.options.precision)?;
        if style.stroke.is_some()
            && style.stroke_width.is_some_and(|width| width > 0.0)
            && needs_explicit_caps(path, style.stroke_dash)
        {
            let bounds = style
                .gradient_bounds
                .unwrap_or_else(|| GradientBounds::from_path(path));
            self.write_path_element(
                document,
                &d,
                PathStyle {
                    stroke: None,
                    ..style
                },
                clip_id,
            )?;
            let outline = stroke_outline(
                path,
                style.stroke_dash,
                style.stroke_width.unwrap(),
                style.stroke_cap.unwrap_or_default(),
                style.stroke_join.unwrap_or_default(),
            )
            .map_err(|error| AvengerSvgError::InvalidGeometry(error.to_string()))?;
            self.write_path_element(
                document,
                &lyon_path_to_svg_d(&outline, self.options.precision)?,
                PathStyle {
                    gradient_bounds: Some(bounds),
                    fill_rule: FillRule::NonZero,
                    fill: style.stroke,
                    stroke: None,
                    stroke_dash: None,
                    ..style
                },
                clip_id,
            )
        } else {
            self.write_path_element(document, &d, style, clip_id)
        }
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
            bounds: style.gradient_bounds,
            defs,
            gradients: style.gradients,
        };

        body.push_str(r#"<path d=""#);
        body.push_str(d);
        body.push('"');
        push_fill_rule(body, "fill-rule", style.fill_rule);
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

struct SvgDocument {
    text_engine: TextEngine,
    viewport: [f32; 2],
    defs: SvgDefs,
    fonts: SvgFontCollector,
    body: String,
}

#[derive(Default)]
struct SvgDefs {
    font_css: String,
    body: String,
    gradient_ids: Vec<(Gradient, GradientBounds, String)>,
    clip_ids: Vec<(Clip, String)>,
    next_gradient_id: usize,
    next_clip_id: usize,
    next_mask_id: usize,
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
        bounds: GradientBounds,
        precision: usize,
    ) -> Result<Option<String>, AvengerSvgError> {
        let gradient = gradients.get(index as usize).ok_or_else(|| {
            AvengerSvgError::UnsupportedPaint(format!("gradient index {index} is out of range"))
        })?;
        let empty = match gradient {
            Gradient::LinearGradient(_) => {
                bounds.min[0] == bounds.max[0] || bounds.min[1] == bounds.max[1]
            }
            Gradient::RadialGradient(g) => {
                (g.x0 == g.x1 && g.y0 == g.y1 && g.r0 == g.r1) || bounds.min == bounds.max
            }
        };
        if empty {
            return Ok(None);
        }
        if let Some((_, _, id)) = self
            .gradient_ids
            .iter()
            .find(|(g, b, _)| g == gradient && *b == bounds)
        {
            return Ok(Some(id.clone()));
        }
        let id = format!("svg-gradient-{}", self.next_gradient_id);
        self.next_gradient_id += 1;
        self.write_gradient_def(&id, gradient, bounds, precision)?;
        self.gradient_ids
            .push((gradient.clone(), bounds, id.clone()));
        Ok(Some(id))
    }

    fn write_gradient_def(
        &mut self,
        id: &str,
        gradient: &Gradient,
        bounds: GradientBounds,
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        let (tag, bounds, controls): (_, _, Vec<(&str, f32)>) = match gradient {
            Gradient::LinearGradient(g) => (
                "linearGradient",
                bounds,
                vec![("x1", g.x0), ("y1", g.y0), ("x2", g.x1), ("y2", g.y1)],
            ),
            Gradient::RadialGradient(g) => (
                "radialGradient",
                bounds.radial(),
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
        self.body.push_str(&format!(
            "<{tag} id=\"{id}\" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"matrix("
        ));
        let matrix = [
            bounds.max[0] - bounds.min[0],
            0.0,
            0.0,
            bounds.max[1] - bounds.min[1],
            bounds.min[0],
            bounds.min[1],
        ];
        for (index, value) in matrix.into_iter().enumerate() {
            if index > 0 {
                self.body.push(' ');
            }
            push_number(&mut self.body, value, precision)?;
        }
        self.body.push_str(")\"");
        for (name, value) in controls {
            self.body.push_str(&format!(" {name}=\""));
            if !value.is_finite() {
                return Err(AvengerSvgError::InvalidGeometry(
                    "gradient controls must be finite".into(),
                ));
            }
            // Relative circle controls retain precision independently of scene-coordinate rounding.
            self.body.push_str(&value.to_string());
            self.body.push('"');
        }
        self.body.push_str(">\n");
        self.write_gradient_stops(gradient.stops(), precision)?;
        self.body.push_str(&format!("</{tag}>\n"));
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
            Clip::Path { path, fill_rule } => {
                self.body.push_str(r#"<path d=""#);
                self.body.push_str(&lyon_path_to_svg_d(path, precision)?);
                self.body.push('"');
                push_fill_rule(&mut self.body, "clip-rule", *fill_rule);
                self.body.push_str("/>");
            }
        }
        self.body.push_str("</clipPath>\n");
        Ok(())
    }

    fn next_mask_id(&mut self) -> String {
        let id = format!("svg-mask-{}", self.next_mask_id);
        self.next_mask_id += 1;
        id
    }
}

struct PaintContext<'a, 'b> {
    bounds: Option<GradientBounds>,
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
                let id = self.defs.gradient_id(
                    self.gradients,
                    *index,
                    self.bounds.ok_or_else(|| {
                        AvengerSvgError::InvalidGeometry(
                            "gradient paint requires reference bounds".into(),
                        )
                    })?,
                    precision,
                )?;
                output.push(' ');
                output.push_str(attr);
                if let Some(id) = id {
                    output.push_str(r#"="url(#"#);
                    output.push_str(&id);
                    output.push_str(r#")""#);
                } else {
                    output.push_str(r#"="none""#);
                }
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

fn push_mask_attr(output: &mut String, mask_id: &str) {
    output.push_str(r#" mask="url(#"#);
    output.push_str(mask_id);
    output.push_str(r#")""#);
}

fn push_pattern_mask_rect(
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
    output.push_str(r#"" fill=""#);
    output.push_str(fill);
    output.push_str(r#""/>"#);
    output.push('\n');
    Ok(())
}

fn push_masked_pattern_rect(
    output: &mut String,
    bounds: PatternRect,
    fill: &str,
    mask_id: &str,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    output.push_str(r#"<g mask="url(#"#);
    output.push_str(mask_id);
    output.push_str(r#")">"#);
    output.push('\n');
    push_pattern_mask_rect(output, bounds, fill, precision)?;
    output.push_str("</g>\n");
    Ok(())
}

fn push_pattern_mask_start(output: &mut String, id: &str) {
    output.push_str(r#"<mask id=""#);
    output.push_str(id);
    output.push_str(
        r#"" maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse" mask-type="luminance">"#,
    );
    output.push('\n');
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
            PatternCoveragePrimitive::Filled { fill_rule, .. } => {
                push_color_attrs(output, "fill", ink, precision)?;
                push_fill_rule(output, "fill-rule", *fill_rule);
                output.push_str(r#" stroke="none""#);
            }
            PatternCoveragePrimitive::Stroked { stroke_width, .. } => {
                push_color_attrs(output, "stroke", ink, precision)?;
                output.push_str(&format!(r#" fill="none" stroke-linecap="butt" stroke-linejoin="miter" stroke-miterlimit="{SCENE_MITER_LIMIT}" stroke-width=""#));
                push_number(output, *stroke_width, precision)?;
                output.push('"');
            }
        }
        output.push_str("/>\n");
    }
    Ok(())
}

fn pattern_geometry_error_to_svg_error(error: PatternGeometryError) -> AvengerSvgError {
    match error {
        PatternGeometryError::InvalidPattern => {
            AvengerSvgError::InvalidGeometry("invalid pattern fill".to_string())
        }
        PatternGeometryError::MissingPlotReferenceFrame => AvengerSvgError::InvalidGeometry(
            "pattern plot anchor requires an active pattern reference frame".to_string(),
        ),
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

#[derive(Clone, Copy)]
struct PathStyle<'a> {
    gradient_bounds: Option<GradientBounds>,
    fill_rule: FillRule,
    fill: Option<&'a ColorOrGradient>,
    stroke: Option<&'a ColorOrGradient>,
    stroke_width: Option<f32>,
    stroke_cap: Option<StrokeCap>,
    stroke_join: Option<StrokeJoin>,
    stroke_dash: Option<&'a [f32]>,
    gradients: &'a [Gradient],
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

fn push_fill_rule(output: &mut String, attribute: &str, rule: FillRule) {
    let value = match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    };
    output.push_str(&format!(" {attribute}=\"{value}\""));
}

#[cfg(test)]
mod tests {
    use avenger_color::ColorOrGradient;
    use avenger_common::value::ScalarOrArray;
    use avenger_image::RgbaImage;
    use avenger_scenegraph::{
        marks::{
            arc::SceneArcMark,
            group::{Clip, SceneGroup},
            image::{SceneImageMark, SceneImageSource},
            pattern::{
                PatternAnchor, PatternFill, PatternLayer, PatternReferenceFrame, PatternSymbol,
                StripePatternLayer, SymbolLattice2d, SymbolPaint, SymbolPatternLayer,
            },
            rect::SceneRectMark,
            rule::SceneRuleMark,
            symbol::SceneSymbolMark,
            text::SceneTextMark,
        },
        scene_graph::SceneGraph,
    };
    use avenger_text::types::{
        FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextSyntaxMode,
    };
    use base64::{prelude::BASE64_STANDARD, Engine};
    use font_subset::FontReader;

    use super::*;

    fn test_renderer() -> SvgRenderer {
        SvgRenderer::new().with_options(SvgRenderOptions {
            font_resolution: test_font_resolution(),
            ..Default::default()
        })
    }

    fn test_font_resolution() -> avenger_text::FontResolutionOptions {
        avenger_text::FontResolutionOptions {
            load_system_fonts: false,
            registered_fonts: test_registered_fonts(),
            default_sans_serif_family: Some("Lato".to_string()),
            default_monospace_family: Some("DejaVu Sans Mono".to_string()),
            default_math_family: Some("Lete Sans Math".to_string()),
            ..Default::default()
        }
    }

    fn test_registered_fonts() -> Vec<avenger_text::RegisteredFont> {
        avenger_text::fonts::registered_default_fonts()
    }

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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#));
        assert!(svg.contains(r##"fill="#ff0000""##));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
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

        let err = test_renderer()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(err, AvengerSvgError::InvalidGeometry(_)));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<clipPath"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
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
                    fill_rule: avenger_common::types::FillRule::EvenOdd,
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<clipPath"));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<line x1="3" y1="5" x2="5" y2="7""#));
        assert!(svg.contains(r#"stroke-dasharray="2 1""#));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"stroke-dasharray="2 1""#));
        assert!(svg.contains(r#"stroke-dashoffset="0.5""#));
        assert!(svg.contains(r#"stroke-miterlimit="2""#));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<clipPath id="svg-clip-0" clipPathUnits="userSpaceOnUse"><rect x="4" y="6" width="5" height="6"/></clipPath>"#));
        assert!(svg.contains(r#"clip-path="url(#svg-clip-0)""#));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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
                image: ScalarOrArray::new_scalar(SceneImageSource::inline(RgbaImage {
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r#"<image x="3" y="5" width="4" height="5""#));
        assert!(svg.contains(r#"preserveAspectRatio="none""#));
        assert!(svg.contains(r#"href="data:image/png;base64,"#));
        assert!(svg.contains(r#"style="image-rendering:pixelated""#));
        assert!(!svg.contains(r#"image-rendering="pixelated""#));
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let image = first_usvg_image(tree.root().children()).expect("SVG should contain an image");
        assert_eq!(image.rendering_mode(), usvg::ImageRendering::Pixelated);
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains(r##" fill="#ff0000" fill-opacity="0.5""##));
        assert!(svg.contains("<style><![CDATA[\n@font-face"));
        assert!(svg.contains(r#"font-family: "avenger-font-0";"#));
        assert!(svg.contains("data:font/woff2;base64,"));
        assert!(svg.contains(r#"font-family="avenger-font-0""#));
        assert!(svg.contains(r#"font-size="12""#));
        assert!(svg.contains(r#"font-weight="bold""#));
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
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.25, 1.0, 0.75])),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                text_syntax: TextSyntaxMode::TypstMarkup,
                ..Default::default()
            }
            .into()],
        };
        let math_svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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
        let svg = test_renderer()
            .with_options(SvgRenderOptions {
                font_resolution: test_font_resolution(),
                rasterize_color_emoji: false,
                font_embedding: crate::options::SvgFontEmbedding::None,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(svg.contains("<text "));
        assert!(svg.contains("Mood "));
        assert!(svg.contains("😀"));
        assert!(!svg.contains("#emoji.face"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
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
        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

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
                    ..test_font_resolution()
                },
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(svg.contains(r#"font-family="avenger-font-0""#));
        assert!(!svg.contains(r#"font-family="Definitely Missing Font""#));
        assert!(svg.contains(r#"font-family: "avenger-font-0";"#));
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

        let svg = test_renderer().render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("\u{2026}</text>"));
        assert!(!svg.contains("Long label text</text>"));
        let rendered_text = first_text_body(&svg);
        assert!(rendered_text.contains('\u{2026}'));
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
    fn honors_svg_font_embedding_none() {
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
                font_embedding: crate::options::SvgFontEmbedding::None,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(!svg.contains("@font-face"));
        assert!(svg.contains(r#"font-family="Lato""#));
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

        assert!(matches!(err, AvengerSvgError::Text(_)));
        assert!(err.to_string().contains("Missing Display Face"));
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
