use crate::api::TypstEngineConfig;
use crate::error::MathTypesetError;
use crate::paths::{
    MathPathArtifact, MathPathData, MathPathItem, MathPathKind, MathStroke, MathTransform,
};
use crate::pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::style::{Color, PlainTextStyle};
use crate::types::{
    MathFragmentOptions, MathOutputRequest, PositionedTextLineRun, PositionedTextLineRunKind,
    TextLineArtifact, TextLineOptions, TypesetMetrics,
};
use crate::warnings::MathTypesetWarning;

use super::ast::{OwnedLine, OwnedLineNode, OwnedMathSpan, OwnedPlainText, OwnedTextSpanKind};
use super::font::{OwnedShapedText, OwnedTextFace, OwnedTextScript};
use super::glyph_path::outline_glyph_path;
use super::math::metrics::try_typeset_simple_row_fragment;
use super::math::syntax::parse_owned_math;

pub(crate) fn try_typeset_owned_text_line(
    source: &str,
    line: &OwnedLine,
    options: &TextLineOptions,
    config: &TypstEngineConfig,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    let Some(line) = line_with_rendered_static_markup(line) else {
        return Ok(None);
    };

    if line.nodes.iter().any(|node| {
        matches!(node, RenderNode::Math(_))
            || matches!(
                node,
                RenderNode::DecoratedText(OwnedDecoratedText {
                    kind: OwnedTextSpanKind::Subscript | OwnedTextSpanKind::Superscript,
                    ..
                })
            )
    }) || line.nodes.len() > 1
    {
        return try_typeset_mixed_metrics_text_line(source, &line, options, config);
    }

    try_typeset_plain_text_line(source, &line, options)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderLine {
    source: String,
    nodes: Vec<RenderNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RenderNode {
    Plain(OwnedPlainText),
    DecoratedText(OwnedDecoratedText),
    Math(OwnedMathSpan),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedDecoratedText {
    kind: OwnedTextSpanKind,
    text: String,
    byte_range: std::ops::Range<usize>,
}

fn line_with_rendered_static_markup(line: &OwnedLine) -> Option<RenderLine> {
    let mut nodes: Vec<RenderNode> = Vec::new();
    let mut pending_plain = String::new();
    let mut pending_start = None;
    let mut pending_end = 0usize;

    let flush_plain = |nodes: &mut Vec<RenderNode>,
                       pending_plain: &mut String,
                       pending_start: &mut Option<usize>,
                       pending_end: usize| {
        if let Some(start) = pending_start.take() {
            if !pending_plain.is_empty() {
                nodes.push(RenderNode::Plain(OwnedPlainText {
                    text: std::mem::take(pending_plain),
                    byte_range: start..pending_end,
                }));
            }
        }
    };

    for node in &line.nodes {
        match node {
            OwnedLineNode::Plain(plain) => {
                if pending_start.is_none() {
                    pending_start = Some(plain.byte_range.start);
                }
                pending_end = plain.byte_range.end;
                pending_plain.push_str(&plain.text);
            }
            OwnedLineNode::Emoji(alias) => {
                if pending_start.is_none() {
                    pending_start = Some(alias.byte_range.start);
                }
                pending_end = alias.byte_range.end;
                pending_plain.push_str(alias.emoji);
            }
            OwnedLineNode::Math(math) => {
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                nodes.push(RenderNode::Math(math.clone()));
            }
            OwnedLineNode::TextSpan(span) => {
                let kind = supported_decoration_kind(span.kind)?;
                let text = render_plain_static_body(&span.body)?;
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                if !text.is_empty() {
                    nodes.push(RenderNode::DecoratedText(OwnedDecoratedText {
                        kind,
                        text,
                        byte_range: span.body_range.clone(),
                    }));
                }
            }
        }
    }

    flush_plain(
        &mut nodes,
        &mut pending_plain,
        &mut pending_start,
        pending_end,
    );

    Some(RenderLine {
        source: line.source.clone(),
        nodes,
    })
}

fn supported_decoration_kind(kind: OwnedTextSpanKind) -> Option<OwnedTextSpanKind> {
    match kind {
        OwnedTextSpanKind::Underline
        | OwnedTextSpanKind::Strike
        | OwnedTextSpanKind::Overline
        | OwnedTextSpanKind::Subscript
        | OwnedTextSpanKind::Superscript
        | OwnedTextSpanKind::Highlight => Some(kind),
    }
}

fn render_plain_static_body(nodes: &[OwnedLineNode]) -> Option<String> {
    let mut text = String::new();
    for node in nodes {
        match node {
            OwnedLineNode::Plain(plain) => text.push_str(&plain.text),
            OwnedLineNode::Emoji(alias) => text.push_str(alias.emoji),
            OwnedLineNode::Math(_) | OwnedLineNode::TextSpan(_) => return None,
        }
    }
    Some(text)
}

fn try_typeset_plain_text_line(
    source: &str,
    line: &RenderLine,
    options: &TextLineOptions,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }

    let [node] = &line.nodes[..] else {
        return Ok(None);
    };

    let Some(face) = OwnedTextFace::for_plain_style(&options.text_style)? else {
        return Ok(None);
    };

    match node {
        RenderNode::Plain(plain) => typeset_plain_text_line(source, plain, None, options, face),
        RenderNode::DecoratedText(decorated) => typeset_plain_text_line(
            source,
            &OwnedPlainText {
                text: decorated.text.clone(),
                byte_range: decorated.byte_range.clone(),
            },
            Some(decorated.kind),
            options,
            face,
        ),
        RenderNode::Math(_) => Ok(None),
    }
}

fn try_typeset_mixed_metrics_text_line(
    source: &str,
    line: &RenderLine,
    options: &TextLineOptions,
    config: &TypstEngineConfig,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }

    let Some(text_face) = OwnedTextFace::for_plain_style(&options.text_style)? else {
        return Ok(None);
    };
    let text_font_size = options.text_style.font_size.max(1.0);
    let math_options = MathFragmentOptions {
        style: options.math_style.clone(),
        outputs: MathOutputRequest {
            paths: options.outputs.paths
                || options.outputs.raster.is_some()
                || options.outputs.positioned_runs,
            raster: None,
            pdf_text_layer: options.outputs.pdf_text_layer,
        },
        syntax: options.syntax,
        limits: options.limits,
    };
    let mut run_parts = Vec::new();
    let mut width = 0.0f32;
    let mut ascent = 0.0f32;
    let mut descent = 0.0f32;

    for node in &line.nodes {
        match node {
            RenderNode::Plain(plain) => {
                if plain.text.is_empty() {
                    continue;
                }
                let shaped = text_face.shaped_text(&plain.text, text_font_size);
                if shaped.has_missing_glyph && requires_delegate_for_missing_glyph_text(&plain.text)
                {
                    return Ok(None);
                }
                let metrics = TypesetMetrics {
                    width: shaped.metrics.width,
                    height: shaped.metrics.height,
                    baseline: shaped.metrics.ascent,
                    ascent: shaped.metrics.ascent,
                    descent: shaped.metrics.descent,
                };
                let paths =
                    (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
                        plain_path_artifact_from_shaped(
                            &text_face,
                            &shaped,
                            metrics,
                            metrics.baseline,
                            text_font_size,
                            options.text_style.fill,
                            None,
                        )
                    });
                let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
                    let font_id = MathFontResourceId(0);
                    (
                        Some(plain_pdf_text_from_shaped(
                            &plain.text,
                            &shaped,
                            metrics,
                            metrics.baseline,
                            text_font_size,
                            options.text_style.fill,
                            font_id,
                        )),
                        vec![text_face.font_resource(font_id)],
                    )
                } else {
                    (None, Vec::new())
                };
                width += metrics.width;
                ascent = ascent.max(metrics.ascent);
                descent = descent.max(metrics.descent);
                run_parts.push(MixedRunPart {
                    kind: PositionedTextLineRunKind::Plain,
                    text: plain.text.clone(),
                    byte_range: plain.byte_range.clone(),
                    text_style: Some(options.text_style.clone()),
                    baseline_shift: 0.0,
                    metrics,
                    paths,
                    positioned_paths: None,
                    pdf_text,
                    font_resources,
                });
            }
            RenderNode::DecoratedText(decorated) => {
                if decorated.text.is_empty() {
                    continue;
                }
                let script = text_script_for_kind(decorated.kind);
                let run_style =
                    text_style_for_decorated_run(&text_face, &options.text_style, decorated.kind);
                let run_font_size = run_style.font_size.max(1.0);
                let baseline_shift = script
                    .map(|script| text_face.script_baseline_shift(text_font_size, script))
                    .unwrap_or(0.0);
                let shaped =
                    shape_text_for_static_run(&text_face, &decorated.text, run_font_size, script);
                if shaped.has_missing_glyph
                    && requires_delegate_for_missing_glyph_text(&decorated.text)
                {
                    return Ok(None);
                }
                let metrics = shifted_text_metrics(&shaped, baseline_shift);
                let glyph_baseline_y = metrics.baseline + baseline_shift;
                let paths =
                    (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
                        plain_path_artifact_from_shaped(
                            &text_face,
                            &shaped,
                            metrics,
                            glyph_baseline_y,
                            run_font_size,
                            options.text_style.fill,
                            Some(decorated.kind),
                        )
                    });
                let positioned_paths = options
                    .outputs
                    .positioned_runs
                    .then(|| {
                        decoration_path_artifact(
                            decorated.kind,
                            metrics,
                            run_font_size,
                            options.text_style.fill,
                        )
                    })
                    .flatten();
                let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
                    let font_id = MathFontResourceId(0);
                    (
                        Some(plain_pdf_text_from_shaped(
                            &decorated.text,
                            &shaped,
                            metrics,
                            glyph_baseline_y,
                            run_font_size,
                            options.text_style.fill,
                            font_id,
                        )),
                        vec![text_face.font_resource(font_id)],
                    )
                } else {
                    (None, Vec::new())
                };
                width += metrics.width;
                ascent = ascent.max(metrics.ascent);
                descent = descent.max(metrics.descent);
                run_parts.push(MixedRunPart {
                    kind: PositionedTextLineRunKind::Plain,
                    text: decorated.text.clone(),
                    byte_range: decorated.byte_range.clone(),
                    text_style: Some(run_style),
                    baseline_shift,
                    metrics,
                    paths,
                    positioned_paths,
                    pdf_text,
                    font_resources,
                });
            }
            RenderNode::Math(span) => {
                let math = parse_owned_math(&span.source, span.source_range.start)?;
                let Some(artifact) = try_typeset_simple_row_fragment(&math, &math_options, config)?
                else {
                    return Ok(None);
                };
                width += artifact.metrics.width;
                ascent = ascent.max(artifact.metrics.ascent);
                descent = descent.max(artifact.metrics.descent);
                run_parts.push(MixedRunPart {
                    kind: PositionedTextLineRunKind::Math,
                    text: span.source.clone(),
                    byte_range: span.source_range.clone(),
                    text_style: None,
                    baseline_shift: 0.0,
                    metrics: artifact.metrics,
                    positioned_paths: artifact.paths.clone(),
                    paths: artifact.paths,
                    pdf_text: artifact.pdf_text,
                    font_resources: artifact.font_resources,
                });
            }
        }
    }

    let metrics = TypesetMetrics {
        width,
        height: ascent + descent,
        baseline: ascent,
        ascent,
        descent,
    };
    let path_artifact = if options.outputs.paths || options.outputs.raster.is_some() {
        Some(full_line_path_artifact(&run_parts, metrics))
    } else {
        None
    };
    #[cfg(feature = "raster")]
    let raster = match (options.outputs.raster, path_artifact.as_ref()) {
        (Some(request), Some(paths)) => Some(rasterize_path_artifact(paths, request)?),
        _ => None,
    };
    #[cfg(not(feature = "raster"))]
    let raster = None;
    let paths = options.outputs.paths.then(|| {
        path_artifact
            .clone()
            .expect("mixed text paths should be built when paths are requested")
    });
    let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
        full_line_pdf_text(source, &run_parts, metrics)
    } else {
        (None, Vec::new())
    };
    let positioned_runs = if options.outputs.positioned_runs {
        let mut x = 0.0;
        run_parts
            .iter()
            .map(|part| {
                let dy = metrics.baseline - part.metrics.baseline;
                let paths = part.positioned_paths.clone().map(|paths| {
                    offset_path_artifact(paths, x, dy, part.metrics.width, metrics.height)
                });
                let (pdf_text, font_resources) =
                    if matches!(part.kind, PositionedTextLineRunKind::Math) {
                        (
                            part.pdf_text.clone().map(|pdf_text| {
                                offset_pdf_text_layer(
                                    pdf_text,
                                    x,
                                    dy,
                                    metrics.width,
                                    metrics.height,
                                    &part.text,
                                )
                            }),
                            part.font_resources.clone(),
                        )
                    } else {
                        (None, Vec::new())
                    };
                let run = PositionedTextLineRun {
                    kind: part.kind,
                    text: part.text.clone(),
                    byte_range: part.byte_range.clone(),
                    text_style: part.text_style.clone(),
                    x,
                    y: metrics.baseline + part.baseline_shift,
                    metrics: TypesetMetrics {
                        width: part.metrics.width,
                        ..metrics
                    },
                    paths,
                    pdf_text,
                    font_resources,
                };
                x += part.metrics.width;
                run
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Some(TextLineArtifact {
        source: source.to_string(),
        metrics,
        paths,
        raster,
        pdf_text,
        positioned_runs,
        font_resources,
        warnings: Vec::<MathTypesetWarning>::new(),
    }))
}

struct MixedRunPart {
    kind: PositionedTextLineRunKind,
    text: String,
    byte_range: std::ops::Range<usize>,
    text_style: Option<PlainTextStyle>,
    baseline_shift: f32,
    metrics: TypesetMetrics,
    paths: Option<MathPathArtifact>,
    positioned_paths: Option<MathPathArtifact>,
    pdf_text: Option<MathPdfTextLayer>,
    font_resources: Vec<MathFontResource>,
}

fn text_script_for_kind(kind: OwnedTextSpanKind) -> Option<OwnedTextScript> {
    match kind {
        OwnedTextSpanKind::Subscript => Some(OwnedTextScript::Subscript),
        OwnedTextSpanKind::Superscript => Some(OwnedTextScript::Superscript),
        OwnedTextSpanKind::Underline
        | OwnedTextSpanKind::Strike
        | OwnedTextSpanKind::Overline
        | OwnedTextSpanKind::Highlight => None,
    }
}

fn text_style_for_decorated_run(
    face: &OwnedTextFace<'_>,
    style: &PlainTextStyle,
    kind: OwnedTextSpanKind,
) -> PlainTextStyle {
    text_script_for_kind(kind)
        .map(|script| face.script_style(style, script))
        .unwrap_or_else(|| style.clone())
}

fn shape_text_for_static_run(
    face: &OwnedTextFace<'_>,
    text: &str,
    font_size: f32,
    script: Option<OwnedTextScript>,
) -> OwnedShapedText {
    let Some(script) = script else {
        return face.shaped_text(text, font_size);
    };
    let tag = match script {
        OwnedTextScript::Subscript => b"subs",
        OwnedTextScript::Superscript => b"sups",
    };
    let features = [rustybuzz::Feature::new(
        rustybuzz::ttf_parser::Tag::from_bytes(tag),
        1,
        ..,
    )];
    let shaped_with_feature = face.shaped_text_with_features(text, font_size, &features);
    let shaped_without_feature = face.shaped_text(text, font_size);

    if shaped_feature_changed_glyphs(&shaped_with_feature, &shaped_without_feature) {
        shaped_with_feature
    } else {
        shaped_without_feature
    }
}

fn shaped_feature_changed_glyphs(a: &OwnedShapedText, b: &OwnedShapedText) -> bool {
    a.glyphs.len() == b.glyphs.len()
        && a.glyphs
            .iter()
            .zip(&b.glyphs)
            .any(|(a, b)| a.glyph_id != b.glyph_id)
}

fn shifted_text_metrics(shaped: &OwnedShapedText, baseline_shift: f32) -> TypesetMetrics {
    let ascent = (shaped.metrics.ascent - baseline_shift).max(0.0);
    let descent = (shaped.metrics.descent + baseline_shift).max(0.0);
    TypesetMetrics {
        width: shaped.metrics.width,
        height: ascent + descent,
        baseline: ascent,
        ascent,
        descent,
    }
}

fn plain_path_artifact_from_shaped(
    face: &OwnedTextFace<'_>,
    shaped: &OwnedShapedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    decoration: Option<OwnedTextSpanKind>,
) -> MathPathArtifact {
    let mut items = Vec::new();
    if matches!(decoration, Some(OwnedTextSpanKind::Highlight)) {
        if let Some(highlight) =
            decoration_path_item(OwnedTextSpanKind::Highlight, metrics, font_size, fill)
        {
            items.push(highlight);
        }
    }

    items.extend(
        shaped
            .glyphs
            .iter()
            .enumerate()
            .filter_map(|(glyph_index, glyph)| {
                let path = outline_glyph_path(
                    &face.face,
                    glyph.glyph_id,
                    font_size,
                    glyph.x,
                    glyph_baseline_y + glyph.y,
                );
                (!path.commands.is_empty()).then(|| MathPathItem {
                    path,
                    kind: MathPathKind::GlyphOutline {
                        glyph_run: 0,
                        glyph_index,
                    },
                    fill: Some(fill),
                    stroke: None,
                    transform: MathTransform::IDENTITY,
                    clip: None,
                })
            }),
    );

    if let Some(
        kind @ (OwnedTextSpanKind::Underline
        | OwnedTextSpanKind::Strike
        | OwnedTextSpanKind::Overline),
    ) = decoration
    {
        if let Some(item) = decoration_path_item(kind, metrics, font_size, fill) {
            items.push(item);
        }
    }

    MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
    }
}

fn decoration_path_artifact(
    kind: OwnedTextSpanKind,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
) -> Option<MathPathArtifact> {
    decoration_path_item(kind, metrics, font_size, fill).map(|item| MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items: vec![item],
    })
}

fn decoration_path_item(
    kind: OwnedTextSpanKind,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
) -> Option<MathPathItem> {
    let thickness = (font_size * 0.06).max(0.5);
    let item = match kind {
        OwnedTextSpanKind::Highlight => MathPathItem {
            path: MathPathData::rect(metrics.width, metrics.height),
            kind: MathPathKind::MathShape,
            fill: Some(crate::style::Color::rgba(1.0, 0.9, 0.25, 0.35)),
            stroke: None,
            transform: MathTransform::IDENTITY,
            clip: None,
        },
        OwnedTextSpanKind::Underline => line_decoration_item(
            metrics.width,
            (metrics.baseline + thickness).min(metrics.height),
            thickness,
            fill,
        ),
        OwnedTextSpanKind::Strike => line_decoration_item(
            metrics.width,
            (metrics.baseline - font_size * 0.32).max(0.0),
            thickness,
            fill,
        ),
        OwnedTextSpanKind::Overline => {
            line_decoration_item(metrics.width, thickness, thickness, fill)
        }
        OwnedTextSpanKind::Subscript | OwnedTextSpanKind::Superscript => return None,
    };
    Some(item)
}

fn line_decoration_item(width: f32, y: f32, thickness: f32, fill: Color) -> MathPathItem {
    MathPathItem {
        path: MathPathData {
            commands: vec![
                crate::paths::MathPathCommand::MoveTo { x: 0.0, y },
                crate::paths::MathPathCommand::LineTo { x: width, y },
            ],
        },
        kind: MathPathKind::MathShape,
        fill: None,
        stroke: Some(MathStroke {
            color: fill,
            width: thickness,
        }),
        transform: MathTransform::IDENTITY,
        clip: None,
    }
}

fn plain_pdf_text_from_shaped(
    semantic_text: &str,
    shaped: &OwnedShapedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    font_id: MathFontResourceId,
) -> MathPdfTextLayer {
    MathPdfTextLayer {
        logical_width: metrics.width,
        logical_height: metrics.height,
        semantic_text: semantic_text.to_string(),
        glyph_runs: (!shaped.glyphs.is_empty())
            .then(|| MathPdfGlyphRun {
                font: font_id,
                font_size,
                fill,
                stroke: None,
                glyphs: shaped
                    .glyphs
                    .iter()
                    .map(|glyph| MathPdfGlyph {
                        glyph_id: glyph.glyph_id.0,
                        unicode: glyph.unicode.clone(),
                        x: 0.0,
                        y: 0.0,
                        x_advance: glyph.x_advance,
                        y_advance: glyph.y_advance,
                        transform: MathTransform {
                            dx: glyph.x,
                            dy: glyph_baseline_y + glyph.y,
                            ..MathTransform::IDENTITY
                        },
                    })
                    .collect(),
            })
            .into_iter()
            .collect(),
    }
}

fn full_line_path_artifact(parts: &[MixedRunPart], metrics: TypesetMetrics) -> MathPathArtifact {
    let mut items = Vec::new();
    let mut x = 0.0;
    for part in parts {
        let dy = metrics.baseline - part.metrics.baseline;
        if let Some(paths) = &part.paths {
            let paths = offset_path_artifact(paths.clone(), x, dy, metrics.width, metrics.height);
            items.extend(paths.items);
        }
        x += part.metrics.width;
    }

    MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
    }
}

fn offset_path_artifact(
    mut paths: MathPathArtifact,
    dx: f32,
    dy: f32,
    logical_width: f32,
    logical_height: f32,
) -> MathPathArtifact {
    paths.logical_width = logical_width;
    paths.logical_height = logical_height;
    for item in &mut paths.items {
        item.transform.dx += dx;
        item.transform.dy += dy;
    }
    paths
}

fn full_line_pdf_text(
    source: &str,
    parts: &[MixedRunPart],
    metrics: TypesetMetrics,
) -> (Option<MathPdfTextLayer>, Vec<MathFontResource>) {
    let mut glyph_runs = Vec::new();
    let mut font_resources = Vec::new();
    let mut x = 0.0;

    for part in parts {
        let dy = metrics.baseline - part.metrics.baseline;
        if let Some(pdf_text) = &part.pdf_text {
            let mut pdf_text = offset_pdf_text_layer(
                pdf_text.clone(),
                x,
                dy,
                metrics.width,
                metrics.height,
                source,
            );
            remap_pdf_fonts(&mut pdf_text, &part.font_resources, &mut font_resources);
            glyph_runs.extend(pdf_text.glyph_runs);
        }
        x += part.metrics.width;
    }

    (
        Some(MathPdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs,
        }),
        font_resources,
    )
}

fn offset_pdf_text_layer(
    mut pdf_text: MathPdfTextLayer,
    dx: f32,
    dy: f32,
    logical_width: f32,
    logical_height: f32,
    semantic_text: &str,
) -> MathPdfTextLayer {
    pdf_text.logical_width = logical_width;
    pdf_text.logical_height = logical_height;
    pdf_text.semantic_text = semantic_text.to_string();
    for run in &mut pdf_text.glyph_runs {
        for glyph in &mut run.glyphs {
            glyph.transform.dx += dx;
            glyph.transform.dy += dy;
        }
    }
    pdf_text
}

fn remap_pdf_fonts(
    pdf_text: &mut MathPdfTextLayer,
    source_resources: &[MathFontResource],
    target_resources: &mut Vec<MathFontResource>,
) {
    let mut id_map = Vec::new();
    for resource in source_resources {
        let target_id = target_resources
            .iter()
            .find(|existing| same_font_resource(existing, resource))
            .map(|existing| existing.id)
            .unwrap_or_else(|| {
                let mut resource = resource.clone();
                resource.id = MathFontResourceId(target_resources.len() as u32);
                let id = resource.id;
                target_resources.push(resource);
                id
            });
        id_map.push((resource.id, target_id));
    }

    for run in &mut pdf_text.glyph_runs {
        if let Some((_, target_id)) = id_map.iter().find(|(source_id, _)| *source_id == run.font) {
            run.font = *target_id;
        }
    }
}

fn same_font_resource(a: &MathFontResource, b: &MathFontResource) -> bool {
    a.face_index == b.face_index && a.data == b.data
}

fn typeset_plain_text_line(
    source: &str,
    plain: &OwnedPlainText,
    decoration: Option<OwnedTextSpanKind>,
    options: &TextLineOptions,
    face: OwnedTextFace<'_>,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    let font_size = options.text_style.font_size.max(1.0);
    let shaped = face.shaped_text(&plain.text, font_size);
    if shaped.has_missing_glyph && requires_delegate_for_missing_glyph_text(&plain.text) {
        return Ok(None);
    }
    let metrics = TypesetMetrics {
        width: shaped.metrics.width,
        height: shaped.metrics.height,
        baseline: shaped.metrics.ascent,
        ascent: shaped.metrics.ascent,
        descent: shaped.metrics.descent,
    };
    let font_id = MathFontResourceId(0);
    let font_resources = options
        .outputs
        .pdf_text_layer
        .then(|| vec![face.font_resource(font_id)])
        .unwrap_or_default();
    let pdf_text = options.outputs.pdf_text_layer.then(|| {
        plain_pdf_text_from_shaped(
            source,
            &shaped,
            metrics,
            metrics.baseline,
            font_size,
            options.text_style.fill,
            font_id,
        )
    });
    let path_artifact = (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
        plain_path_artifact_from_shaped(
            &face,
            &shaped,
            metrics,
            metrics.baseline,
            font_size,
            options.text_style.fill,
            decoration,
        )
    });
    let paths = options.outputs.paths.then(|| {
        path_artifact
            .clone()
            .expect("plain text paths should be built when paths are requested")
    });
    #[cfg(feature = "raster")]
    let raster = match (options.outputs.raster, path_artifact.as_ref()) {
        (Some(request), Some(paths)) => Some(rasterize_path_artifact(paths, request)?),
        _ => None,
    };
    #[cfg(not(feature = "raster"))]
    let raster = None;
    let positioned_paths = options
        .outputs
        .positioned_runs
        .then(|| decoration_path_artifact(decoration?, metrics, font_size, options.text_style.fill))
        .flatten();
    let positioned_runs = options.outputs.positioned_runs.then(|| {
        vec![PositionedTextLineRun {
            kind: PositionedTextLineRunKind::Plain,
            text: plain.text.clone(),
            byte_range: plain.byte_range.clone(),
            text_style: Some(options.text_style.clone()),
            x: 0.0,
            y: metrics.baseline,
            metrics,
            paths: positioned_paths,
            pdf_text: None,
            font_resources: Vec::new(),
        }]
    });

    Ok(Some(TextLineArtifact {
        source: source.to_string(),
        metrics,
        paths,
        raster,
        pdf_text,
        positioned_runs: positioned_runs.unwrap_or_default(),
        font_resources,
        warnings: Vec::<MathTypesetWarning>::new(),
    }))
}

fn requires_delegate_for_missing_glyph_text(text: &str) -> bool {
    text.chars()
        .any(|ch| is_rtl_char(ch) || is_zero_width_joiner(ch))
}

fn is_rtl_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{0590}'..='\u{08FF}'
            | '\u{FB1D}'..='\u{FDFF}'
            | '\u{FE70}'..='\u{FEFF}'
            | '\u{10800}'..='\u{10FFF}'
            | '\u{1E800}'..='\u{1EFFF}'
    )
}

fn is_zero_width_joiner(ch: char) -> bool {
    ch == '\u{200D}'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delimiter::MathDelimiterOptions;
    use crate::owned::syntax::parse_owned_line;
    use crate::types::TextLineOutputRequest;

    fn render_line(source: &str) -> RenderLine {
        let line = parse_owned_line(source, &MathDelimiterOptions::default()).unwrap();
        line_with_rendered_static_markup(&line).expect("test line should be renderable")
    }

    #[test]
    fn plain_line_fast_path_returns_positioned_plain_run() {
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .expect("plain Atkinson text should use fast path");

        assert_eq!(artifact.source, "Hello");
        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(
            artifact.positioned_runs[0].kind,
            PositionedTextLineRunKind::Plain
        );
        assert_eq!(artifact.positioned_runs[0].text, "Hello");
    }

    #[test]
    fn plain_line_fast_path_can_emit_paths() {
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .expect("plain Atkinson text should use fast path");

        let paths = artifact.paths.expect("plain paths should exist");
        assert_eq!(paths.items.len(), 5);
    }

    #[cfg(not(feature = "raster"))]
    #[test]
    fn plain_line_fast_path_declines_raster_without_raster_feature() {
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.raster = Some(crate::raster::RasterRequest::default());

        assert!(try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .is_none());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn plain_line_fast_path_can_emit_raster() {
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;
        options.outputs.raster = Some(crate::raster::RasterRequest { scale: 2.0 });

        let artifact = try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .expect("plain Atkinson text should use fast path");

        assert!(artifact.paths.is_none());
        assert!(artifact
            .raster
            .as_ref()
            .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0));
    }

    #[test]
    fn plain_line_fast_path_can_emit_non_rtl_missing_glyphs() {
        let line = render_line("Revenue 🚀");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("Revenue 🚀", &line, &options)
            .unwrap()
            .expect("non-RTL missing glyphs should stay on the owned path");

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
    }

    #[test]
    fn plain_line_fast_path_declines_rtl_missing_glyphs() {
        let line = render_line("שלום");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        assert!(try_typeset_plain_text_line("שלום", &line, &options)
            .unwrap()
            .is_none());
    }

    #[test]
    fn plain_line_fast_path_declines_zwj_missing_glyphs() {
        let line = render_line("Family 👨‍👩‍👧‍👦");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        assert!(try_typeset_plain_text_line("Family 👨‍👩‍👧‍👦", &line, &options)
            .unwrap()
            .is_none());
    }

    #[test]
    fn plain_line_fast_path_can_emit_pdf_glyph_metadata() {
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .expect("plain Atkinson text should use fast path");

        assert_eq!(artifact.font_resources.len(), 1);
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.semantic_text, "Hello");
        assert_eq!(pdf.glyph_runs.len(), 1);
        assert_eq!(pdf.glyph_runs[0].glyphs.len(), 5);
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert!(artifact.positioned_runs[0].pdf_text.is_none());
    }

    #[test]
    fn decorated_plain_line_emits_text_and_decoration_paths() {
        let line = render_line("#underline[important]");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = try_typeset_plain_text_line("#underline[important]", &line, &options)
            .unwrap()
            .expect("supported static decoration should use fast path");

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "important");
        assert!(artifact.positioned_runs[0]
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 1 && paths.items[0].stroke.is_some()));
        assert!(artifact.paths.as_ref().is_some_and(|paths| {
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, MathPathKind::MathShape) && item.stroke.is_some())
        }));
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn highlighted_plain_line_emits_background_before_glyphs() {
        let line = render_line("#highlight[warning]");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("#highlight[warning]", &line, &options)
            .unwrap()
            .expect("supported static highlight should use fast path");
        let paths = artifact.paths.expect("highlight paths should exist");

        assert!(matches!(paths.items[0].kind, MathPathKind::MathShape));
        assert!(paths.items[0].fill.is_some());
    }
}
