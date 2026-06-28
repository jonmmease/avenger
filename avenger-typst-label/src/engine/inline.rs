use crate::error::LabelError;
use crate::label::EngineOptions;
use crate::paths::{PathArtifact, PathData, PathItem, PathKind, Stroke, Transform};
use crate::pdf::{FontResource, FontResourceId, PdfGlyph, PdfGlyphRun, PdfTextLayer};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::style::{Color, PlainTextStyle};
use crate::types::{
    MathFragmentOptions, MathOutputRequest, PositionedTextLineRun, PositionedTextLineRunKind,
    TextLineArtifact, TextLineOptions, TypesetMetrics,
};
use crate::warnings::LabelWarning;

use super::ast::{LineNode, MathSpan, ParsedLine, PlainTextNode, TextMarkupKind};
use super::font::{
    SegmentedText, ShapedText, TextDecorationLineMetrics, TextDecorationMetrics, TextFace,
    TextScript, shape_plain_text_with_fallback,
};
use super::math::metrics::try_typeset_simple_row_fragment;
use super::math::syntax::parse_math;

pub(crate) fn try_typeset_text_line(
    source: &str,
    line: &ParsedLine,
    options: &TextLineOptions,
    config: &EngineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<TextLineArtifact>, LabelError> {
    let Some(line) = line_with_rendered_static_markup(line) else {
        return Ok(None);
    };

    if line.nodes.iter().any(|node| {
        matches!(node, RenderNode::Math(_))
            || matches!(
                node,
                RenderNode::DecoratedText(DecoratedText {
                    kind: TextMarkupKind::Subscript | TextMarkupKind::Superscript,
                    ..
                })
            )
    }) || line.nodes.len() > 1
    {
        return try_typeset_mixed_metrics_text_line(source, &line, options, config, fontdb);
    }

    try_typeset_plain_text_line(source, &line, options, fontdb)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderLine {
    source: String,
    nodes: Vec<RenderNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RenderNode {
    Plain(PlainTextNode),
    DecoratedText(DecoratedText),
    Math(MathSpan),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DecoratedText {
    kind: TextMarkupKind,
    text: String,
    byte_range: std::ops::Range<usize>,
}

fn line_with_rendered_static_markup(line: &ParsedLine) -> Option<RenderLine> {
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
                nodes.push(RenderNode::Plain(PlainTextNode {
                    text: std::mem::take(pending_plain),
                    byte_range: start..pending_end,
                }));
            }
        }
    };

    for node in &line.nodes {
        match node {
            LineNode::Plain(plain) => {
                if pending_start.is_none() {
                    pending_start = Some(plain.byte_range.start);
                }
                pending_end = plain.byte_range.end;
                pending_plain.push_str(&plain.text);
            }
            LineNode::Emoji(alias) => {
                if pending_start.is_none() {
                    pending_start = Some(alias.byte_range.start);
                }
                pending_end = alias.byte_range.end;
                pending_plain.push_str(alias.emoji);
            }
            LineNode::Math(math) => {
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                nodes.push(RenderNode::Math(math.clone()));
            }
            LineNode::TextSpan(span) => {
                let kind = supported_decoration_kind(span.kind)?;
                let text = render_plain_static_body(&span.body)?;
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                if !text.is_empty() {
                    nodes.push(RenderNode::DecoratedText(DecoratedText {
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

fn supported_decoration_kind(kind: TextMarkupKind) -> Option<TextMarkupKind> {
    match kind {
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Highlight => Some(kind),
    }
}

fn render_plain_static_body(nodes: &[LineNode]) -> Option<String> {
    let mut text = String::new();
    for node in nodes {
        match node {
            LineNode::Plain(plain) => text.push_str(&plain.text),
            LineNode::Emoji(alias) => text.push_str(alias.emoji),
            LineNode::Math(_) | LineNode::TextSpan(_) => return None,
        }
    }
    Some(text)
}

fn shape_plain_text_for_style(
    style: &PlainTextStyle,
    text: &str,
    font_size: f32,
    features: &[rustybuzz::Feature],
    fontdb: &fontdb::Database,
) -> Result<Option<SegmentedText>, LabelError> {
    shape_plain_text_with_fallback(fontdb, style, text, font_size, features)
}

fn try_typeset_plain_text_line(
    source: &str,
    line: &RenderLine,
    options: &TextLineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<TextLineArtifact>, LabelError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }

    let [node] = &line.nodes[..] else {
        return Ok(None);
    };

    match node {
        RenderNode::Plain(plain) => {
            let Some(segmented) = shape_plain_text_for_style(
                &options.text_style,
                &plain.text,
                options.text_style.font_size.max(1.0),
                &[],
                fontdb,
            )?
            else {
                return Ok(None);
            };
            typeset_segmented_plain_text_line(source, plain, None, options, segmented)
        }
        RenderNode::DecoratedText(decorated) => {
            let Some(face) =
                TextFace::for_plain_style_and_text(&options.text_style, &decorated.text, fontdb)?
            else {
                return Ok(None);
            };
            typeset_plain_text_line(
                source,
                &PlainTextNode {
                    text: decorated.text.clone(),
                    byte_range: decorated.byte_range.clone(),
                },
                Some(decorated.kind),
                options,
                face,
            )
        }
        RenderNode::Math(_) => Ok(None),
    }
}

fn try_typeset_mixed_metrics_text_line(
    source: &str,
    line: &RenderLine,
    options: &TextLineOptions,
    config: &EngineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<TextLineArtifact>, LabelError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }

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
                let Some(segmented) = shape_plain_text_for_style(
                    &options.text_style,
                    &plain.text,
                    text_font_size,
                    &[],
                    fontdb,
                )?
                else {
                    return Ok(None);
                };
                let metrics = metrics_from_segmented_text(&segmented, 0.0);
                let paths =
                    (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
                        plain_path_artifact_from_segmented(
                            &segmented,
                            metrics,
                            metrics.baseline,
                            text_font_size,
                            options.text_style.fill,
                            None,
                        )
                    });
                let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
                    let (pdf_text, font_resources) = plain_pdf_text_from_segmented(
                        &plain.text,
                        &segmented,
                        metrics,
                        metrics.baseline,
                        text_font_size,
                        options.text_style.fill,
                    );
                    (Some(pdf_text), font_resources)
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
                    positioned_plain_runs: positioned_plain_runs_from_segmented(
                        plain,
                        &segmented,
                        &options.text_style,
                        metrics.baseline,
                        options.outputs.positioned_runs,
                    ),
                });
            }
            RenderNode::DecoratedText(decorated) => {
                if decorated.text.is_empty() {
                    continue;
                }
                let script = text_script_for_kind(decorated.kind);
                let Some(text_face) = TextFace::for_plain_style_and_text(
                    &options.text_style,
                    &decorated.text,
                    fontdb,
                )?
                else {
                    return Ok(None);
                };
                let run_style =
                    text_style_for_decorated_run(&text_face, &options.text_style, decorated.kind);
                let run_font_size = run_style.font_size.max(1.0);
                let baseline_shift = script
                    .map(|script| text_face.script_baseline_shift(text_font_size, script))
                    .unwrap_or(0.0);
                let shaped =
                    shape_text_for_static_run(&text_face, &decorated.text, run_font_size, script);
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
                            Some(&text_face),
                        )
                    })
                    .flatten();
                let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
                    let font_id = FontResourceId(0);
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
                    text_style: Some(run_style.clone()),
                    baseline_shift,
                    metrics,
                    paths,
                    positioned_paths,
                    pdf_text,
                    font_resources,
                    positioned_plain_runs: vec![PositionedTextLineRun {
                        kind: PositionedTextLineRunKind::Plain,
                        text: decorated.text.clone(),
                        byte_range: decorated.byte_range.clone(),
                        text_style: Some(run_style),
                        x: 0.0,
                        y: glyph_baseline_y,
                        metrics,
                        paths: None,
                        pdf_text: None,
                        font_resources: Vec::new(),
                    }],
                });
            }
            RenderNode::Math(span) => {
                let math = parse_math(&span.source, span.source_range.start)?;
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
                    positioned_plain_runs: Vec::new(),
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
            .flat_map(|part| {
                let dy = metrics.baseline - part.metrics.baseline;
                let runs = if matches!(part.kind, PositionedTextLineRunKind::Plain)
                    && !part.positioned_plain_runs.is_empty()
                {
                    part.positioned_plain_runs
                        .iter()
                        .enumerate()
                        .map(|(index, run)| {
                            let mut run = run.clone();
                            run.x += x;
                            run.y += dy;
                            if let Some(paths) = run.paths.take() {
                                run.paths = Some(offset_path_artifact(
                                    paths,
                                    x,
                                    dy,
                                    part.metrics.width,
                                    metrics.height,
                                ));
                            } else if index == 0 {
                                run.paths = part.positioned_paths.clone().map(|paths| {
                                    offset_path_artifact(
                                        paths,
                                        x,
                                        dy,
                                        part.metrics.width,
                                        metrics.height,
                                    )
                                });
                            }
                            run
                        })
                        .collect::<Vec<_>>()
                } else {
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
                    vec![PositionedTextLineRun {
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
                    }]
                };
                x += part.metrics.width;
                runs
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
        warnings: Vec::<LabelWarning>::new(),
    }))
}

struct MixedRunPart {
    kind: PositionedTextLineRunKind,
    text: String,
    byte_range: std::ops::Range<usize>,
    text_style: Option<PlainTextStyle>,
    baseline_shift: f32,
    metrics: TypesetMetrics,
    paths: Option<PathArtifact>,
    positioned_paths: Option<PathArtifact>,
    pdf_text: Option<PdfTextLayer>,
    font_resources: Vec<FontResource>,
    positioned_plain_runs: Vec<PositionedTextLineRun>,
}

fn text_script_for_kind(kind: TextMarkupKind) -> Option<TextScript> {
    match kind {
        TextMarkupKind::Subscript => Some(TextScript::Subscript),
        TextMarkupKind::Superscript => Some(TextScript::Superscript),
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Highlight => None,
    }
}

fn text_style_for_decorated_run(
    face: &TextFace,
    style: &PlainTextStyle,
    kind: TextMarkupKind,
) -> PlainTextStyle {
    text_script_for_kind(kind)
        .map(|script| face.script_style(style, script))
        .unwrap_or_else(|| style.clone())
}

fn shape_text_for_static_run(
    face: &TextFace,
    text: &str,
    font_size: f32,
    script: Option<TextScript>,
) -> ShapedText {
    let Some(script) = script else {
        return face.shaped_text(text, font_size);
    };
    let tag = match script {
        TextScript::Subscript => b"subs",
        TextScript::Superscript => b"sups",
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

fn shaped_feature_changed_glyphs(a: &ShapedText, b: &ShapedText) -> bool {
    a.glyphs.len() == b.glyphs.len()
        && a.glyphs
            .iter()
            .zip(&b.glyphs)
            .any(|(a, b)| a.glyph_id != b.glyph_id)
}

fn shifted_text_metrics(shaped: &ShapedText, baseline_shift: f32) -> TypesetMetrics {
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

fn metrics_from_segmented_text(segmented: &SegmentedText, baseline_shift: f32) -> TypesetMetrics {
    let ascent = (segmented.metrics.ascent - baseline_shift).max(0.0);
    let descent = (segmented.metrics.descent + baseline_shift).max(0.0);
    TypesetMetrics {
        width: segmented.metrics.width,
        height: ascent + descent,
        baseline: ascent,
        ascent,
        descent,
    }
}

fn positioned_plain_runs_from_segmented(
    plain: &PlainTextNode,
    segmented: &SegmentedText,
    text_style: &PlainTextStyle,
    baseline: f32,
    include_color_emoji_images: bool,
) -> Vec<PositionedTextLineRun> {
    let runs = segmented
        .runs
        .iter()
        .map(|run| {
            let run_style = positioned_plain_text_style(text_style, &run.face);
            let metrics = TypesetMetrics {
                width: run.shaped.metrics.width,
                height: run.shaped.metrics.height,
                baseline: run.shaped.metrics.ascent,
                ascent: run.shaped.metrics.ascent,
                descent: run.shaped.metrics.descent,
            };
            let paths =
                if include_color_emoji_images && is_color_emoji_family(&run_style.font_family) {
                    let paths = plain_path_artifact_from_shaped(
                        &run.face,
                        &run.shaped,
                        metrics,
                        baseline,
                        run_style.font_size,
                        text_style.fill,
                        None,
                    );
                    (!paths.images.is_empty()).then(|| {
                        offset_path_artifact(
                            paths,
                            run.x,
                            0.0,
                            run.shaped.metrics.width,
                            metrics.height,
                        )
                    })
                } else {
                    None
                };
            PositionedTextLineRun {
                kind: PositionedTextLineRunKind::Plain,
                text: run.text.clone(),
                byte_range: plain.byte_range.clone(),
                text_style: Some(run_style),
                x: run.x,
                y: baseline,
                metrics,
                paths,
                pdf_text: None,
                font_resources: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    merge_adjacent_positioned_plain_runs(runs)
}

fn merge_adjacent_positioned_plain_runs(
    runs: Vec<PositionedTextLineRun>,
) -> Vec<PositionedTextLineRun> {
    let mut merged: Vec<PositionedTextLineRun> = Vec::new();
    for run in runs {
        if let Some(previous) = merged.last_mut() {
            if previous.text_style == run.text_style
                && previous.paths.is_none()
                && previous.pdf_text.is_none()
                && run.paths.is_none()
                && run.pdf_text.is_none()
                && (previous.y - run.y).abs() <= f32::EPSILON
            {
                previous.text.push_str(&run.text);
                let right = (run.x + run.metrics.width).max(previous.x + previous.metrics.width);
                previous.metrics.width = right - previous.x;
                previous.metrics.ascent = previous.metrics.ascent.max(run.metrics.ascent);
                previous.metrics.descent = previous.metrics.descent.max(run.metrics.descent);
                previous.metrics.height = previous.metrics.ascent + previous.metrics.descent;
                previous.metrics.baseline = previous.metrics.ascent;
                continue;
            }
        }
        merged.push(run);
    }
    merged
}

fn positioned_plain_text_style(text_style: &PlainTextStyle, face: &TextFace) -> PlainTextStyle {
    let mut run_style = text_style.clone();
    let family = face.font_resource(FontResourceId(0)).family;
    if is_color_emoji_family(&family) {
        run_style.font_family = family;
    }
    run_style
}

fn is_color_emoji_family(family: &str) -> bool {
    matches!(
        family.to_ascii_lowercase().as_str(),
        "apple color emoji" | "noto color emoji" | "twitter color emoji" | "segoe ui emoji"
    )
}

fn plain_path_artifact_from_shaped(
    face: &TextFace,
    shaped: &ShapedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    decoration: Option<TextMarkupKind>,
) -> PathArtifact {
    let mut items = Vec::new();
    let mut images = Vec::new();
    if matches!(decoration, Some(TextMarkupKind::Highlight)) {
        if let Some(highlight) =
            decoration_path_item(TextMarkupKind::Highlight, metrics, font_size, fill, None)
        {
            items.push(highlight);
        }
    }

    for (glyph_index, glyph) in shaped.glyphs.iter().enumerate() {
        if let Some(image) = face.raster_glyph_image(
            glyph.glyph_id,
            font_size,
            glyph.x,
            glyph_baseline_y + glyph.y,
        ) {
            images.push(image);
        } else {
            let path = face.outline_glyph_path(
                glyph.glyph_id,
                font_size,
                glyph.x,
                glyph_baseline_y + glyph.y,
            );
            if !path.commands.is_empty() {
                items.push(PathItem {
                    path,
                    kind: PathKind::GlyphOutline {
                        glyph_run: 0,
                        glyph_index,
                    },
                    fill: Some(fill),
                    stroke: None,
                    transform: Transform::IDENTITY,
                    clip: None,
                });
            }
        }
    }

    if let Some(
        kind @ (TextMarkupKind::Underline | TextMarkupKind::Strike | TextMarkupKind::Overline),
    ) = decoration
    {
        if let Some(item) = decoration_path_item(kind, metrics, font_size, fill, Some(face)) {
            items.push(item);
        }
    }

    PathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
        images,
    }
}

fn plain_path_artifact_from_segmented(
    segmented: &SegmentedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    decoration: Option<TextMarkupKind>,
) -> PathArtifact {
    let mut items = Vec::new();
    let mut images = Vec::new();
    if matches!(decoration, Some(TextMarkupKind::Highlight)) {
        if let Some(highlight) =
            decoration_path_item(TextMarkupKind::Highlight, metrics, font_size, fill, None)
        {
            items.push(highlight);
        }
    }

    for (run_index, run) in segmented.runs.iter().enumerate() {
        for (glyph_index, glyph) in run.shaped.glyphs.iter().enumerate() {
            if let Some(image) = run.face.raster_glyph_image(
                glyph.glyph_id,
                font_size,
                run.x + glyph.x,
                glyph_baseline_y + glyph.y,
            ) {
                images.push(image);
            } else {
                let path = run.face.outline_glyph_path(
                    glyph.glyph_id,
                    font_size,
                    run.x + glyph.x,
                    glyph_baseline_y + glyph.y,
                );
                if !path.commands.is_empty() {
                    items.push(PathItem {
                        path,
                        kind: PathKind::GlyphOutline {
                            glyph_run: run_index,
                            glyph_index,
                        },
                        fill: Some(fill),
                        stroke: None,
                        transform: Transform::IDENTITY,
                        clip: None,
                    });
                }
            }
        }
    }

    if let Some(
        kind @ (TextMarkupKind::Underline | TextMarkupKind::Strike | TextMarkupKind::Overline),
    ) = decoration
    {
        let face = segmented.runs.first().map(|run| &run.face);
        if let Some(item) = decoration_path_item(kind, metrics, font_size, fill, face) {
            items.push(item);
        }
    }

    PathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
        images,
    }
}

fn decoration_path_artifact(
    kind: TextMarkupKind,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
    face: Option<&TextFace>,
) -> Option<PathArtifact> {
    decoration_path_item(kind, metrics, font_size, fill, face).map(|item| PathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items: vec![item],
        images: Vec::new(),
    })
}

fn decoration_path_item(
    kind: TextMarkupKind,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
    face: Option<&TextFace>,
) -> Option<PathItem> {
    let item = match kind {
        TextMarkupKind::Highlight => PathItem {
            path: PathData::rect(metrics.width, metrics.height),
            kind: PathKind::MathShape,
            fill: Some(crate::style::Color::rgba(1.0, 0.9, 0.25, 0.35)),
            stroke: None,
            transform: Transform::IDENTITY,
            clip: None,
        },
        TextMarkupKind::Underline => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
        ),
        TextMarkupKind::Strike => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
        ),
        TextMarkupKind::Overline => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
        ),
        TextMarkupKind::Subscript | TextMarkupKind::Superscript => return None,
    };
    Some(item)
}

fn decoration_line(
    face: Option<&TextFace>,
    font_size: f32,
    kind: TextMarkupKind,
) -> TextDecorationLineMetrics {
    let metrics = face
        .map(|face| face.decoration_metrics(font_size))
        .unwrap_or_else(|| TextDecorationMetrics::fallback(font_size));
    match kind {
        TextMarkupKind::Underline => metrics.underline,
        TextMarkupKind::Strike => metrics.strikethrough,
        TextMarkupKind::Overline => metrics.overline,
        TextMarkupKind::Highlight | TextMarkupKind::Subscript | TextMarkupKind::Superscript => {
            TextDecorationMetrics::fallback(font_size).underline
        }
    }
}

fn line_decoration_item(
    width: f32,
    metrics: TypesetMetrics,
    line: TextDecorationLineMetrics,
    fill: Color,
) -> PathItem {
    let y = metrics.baseline - line.position;
    PathItem {
        path: PathData {
            commands: vec![
                crate::paths::PathCommand::MoveTo { x: 0.0, y },
                crate::paths::PathCommand::LineTo { x: width, y },
            ],
        },
        kind: PathKind::MathShape,
        fill: None,
        stroke: Some(Stroke {
            color: fill,
            width: line.thickness,
        }),
        transform: Transform::IDENTITY,
        clip: None,
    }
}

fn plain_pdf_text_from_shaped(
    semantic_text: &str,
    shaped: &ShapedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    font_id: FontResourceId,
) -> PdfTextLayer {
    PdfTextLayer {
        logical_width: metrics.width,
        logical_height: metrics.height,
        semantic_text: semantic_text.to_string(),
        glyph_runs: (!shaped.glyphs.is_empty())
            .then(|| PdfGlyphRun {
                font: font_id,
                font_size,
                fill,
                stroke: None,
                text: semantic_text.to_string(),
                glyphs: shaped
                    .glyphs
                    .iter()
                    .map(|glyph| PdfGlyph {
                        glyph_id: glyph.glyph_id.0,
                        unicode: glyph.unicode.clone(),
                        text_range: glyph.byte_range.clone(),
                        x: 0.0,
                        y: 0.0,
                        x_advance: glyph.x_advance,
                        y_advance: glyph.y_advance,
                        transform: Transform {
                            dx: glyph.x,
                            dy: glyph_baseline_y + glyph.y,
                            ..Transform::IDENTITY
                        },
                    })
                    .collect(),
            })
            .into_iter()
            .collect(),
    }
}

fn plain_pdf_text_from_segmented(
    semantic_text: &str,
    segmented: &SegmentedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
) -> (PdfTextLayer, Vec<FontResource>) {
    let mut font_resources = Vec::new();
    let mut glyph_runs = Vec::new();

    for run in &segmented.runs {
        if run.shaped.glyphs.is_empty() {
            continue;
        }
        let font = intern_font_resource(
            &mut font_resources,
            run.face.font_resource(FontResourceId(0)),
        );
        glyph_runs.push(PdfGlyphRun {
            font,
            font_size,
            fill,
            stroke: None,
            text: run.text.clone(),
            glyphs: run
                .shaped
                .glyphs
                .iter()
                .map(|glyph| PdfGlyph {
                    glyph_id: glyph.glyph_id.0,
                    unicode: glyph.unicode.clone(),
                    text_range: glyph.byte_range.clone(),
                    x: 0.0,
                    y: 0.0,
                    x_advance: glyph.x_advance,
                    y_advance: glyph.y_advance,
                    transform: Transform {
                        dx: run.x + glyph.x,
                        dy: glyph_baseline_y + glyph.y,
                        ..Transform::IDENTITY
                    },
                })
                .collect(),
        });
    }

    (
        PdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: semantic_text.to_string(),
            glyph_runs,
        },
        font_resources,
    )
}

fn full_line_path_artifact(parts: &[MixedRunPart], metrics: TypesetMetrics) -> PathArtifact {
    let mut items = Vec::new();
    let mut images = Vec::new();
    let mut x = 0.0;
    for part in parts {
        let dy = metrics.baseline - part.metrics.baseline;
        if let Some(paths) = &part.paths {
            let paths = offset_path_artifact(paths.clone(), x, dy, metrics.width, metrics.height);
            items.extend(paths.items);
            images.extend(paths.images);
        }
        x += part.metrics.width;
    }

    PathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
        images,
    }
}

fn offset_path_artifact(
    mut paths: PathArtifact,
    dx: f32,
    dy: f32,
    logical_width: f32,
    logical_height: f32,
) -> PathArtifact {
    paths.logical_width = logical_width;
    paths.logical_height = logical_height;
    for item in &mut paths.items {
        item.transform.dx += dx;
        item.transform.dy += dy;
    }
    for image in &mut paths.images {
        image.transform.dx += dx;
        image.transform.dy += dy;
    }
    paths
}

fn full_line_pdf_text(
    source: &str,
    parts: &[MixedRunPart],
    metrics: TypesetMetrics,
) -> (Option<PdfTextLayer>, Vec<FontResource>) {
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
        Some(PdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs,
        }),
        font_resources,
    )
}

fn offset_pdf_text_layer(
    mut pdf_text: PdfTextLayer,
    dx: f32,
    dy: f32,
    logical_width: f32,
    logical_height: f32,
    semantic_text: &str,
) -> PdfTextLayer {
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
    pdf_text: &mut PdfTextLayer,
    source_resources: &[FontResource],
    target_resources: &mut Vec<FontResource>,
) {
    let mut id_map = Vec::new();
    for resource in source_resources {
        let target_id = target_resources
            .iter()
            .find(|existing| same_font_resource(existing, resource))
            .map(|existing| existing.id)
            .unwrap_or_else(|| {
                let mut resource = resource.clone();
                resource.id = FontResourceId(target_resources.len() as u32);
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

fn same_font_resource(a: &FontResource, b: &FontResource) -> bool {
    a.face_index == b.face_index && a.variations == b.variations && a.data == b.data
}

fn intern_font_resource(
    font_resources: &mut Vec<FontResource>,
    mut resource: FontResource,
) -> FontResourceId {
    if let Some(existing) = font_resources
        .iter()
        .find(|existing| same_font_resource(existing, &resource))
    {
        return existing.id;
    }
    resource.id = FontResourceId(font_resources.len() as u32);
    let id = resource.id;
    font_resources.push(resource);
    id
}

fn typeset_plain_text_line(
    source: &str,
    plain: &PlainTextNode,
    decoration: Option<TextMarkupKind>,
    options: &TextLineOptions,
    face: TextFace,
) -> Result<Option<TextLineArtifact>, LabelError> {
    let font_size = options.text_style.font_size.max(1.0);
    let shaped = face.shaped_text(&plain.text, font_size);
    let metrics = TypesetMetrics {
        width: shaped.metrics.width,
        height: shaped.metrics.height,
        baseline: shaped.metrics.ascent,
        ascent: shaped.metrics.ascent,
        descent: shaped.metrics.descent,
    };
    let font_id = FontResourceId(0);
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
        .then(|| {
            decoration_path_artifact(
                decoration?,
                metrics,
                font_size,
                options.text_style.fill,
                Some(&face),
            )
        })
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
        warnings: Vec::<LabelWarning>::new(),
    }))
}

fn typeset_segmented_plain_text_line(
    source: &str,
    plain: &PlainTextNode,
    decoration: Option<TextMarkupKind>,
    options: &TextLineOptions,
    segmented: SegmentedText,
) -> Result<Option<TextLineArtifact>, LabelError> {
    let font_size = options.text_style.font_size.max(1.0);
    let metrics = metrics_from_segmented_text(&segmented, 0.0);
    let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
        let (pdf_text, font_resources) = plain_pdf_text_from_segmented(
            source,
            &segmented,
            metrics,
            metrics.baseline,
            font_size,
            options.text_style.fill,
        );
        (Some(pdf_text), font_resources)
    } else {
        (None, Vec::new())
    };
    let path_artifact = (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
        plain_path_artifact_from_segmented(
            &segmented,
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
            .expect("segmented plain text paths should be built when paths are requested")
    });
    #[cfg(feature = "raster")]
    let raster = match (options.outputs.raster, path_artifact.as_ref()) {
        (Some(request), Some(paths)) => Some(rasterize_path_artifact(paths, request)?),
        _ => None,
    };
    #[cfg(not(feature = "raster"))]
    let raster = None;
    let positioned_runs = options.outputs.positioned_runs.then(|| {
        positioned_plain_runs_from_segmented(
            plain,
            &segmented,
            &options.text_style,
            metrics.baseline,
            true,
        )
    });

    Ok(Some(TextLineArtifact {
        source: source.to_string(),
        metrics,
        paths,
        raster,
        pdf_text,
        positioned_runs: positioned_runs.unwrap_or_default(),
        font_resources,
        warnings: Vec::<LabelWarning>::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::font::build_text_fontdb;
    use crate::engine::syntax::parse_line;
    use crate::label::EngineOptions;
    use crate::paths::PathCommand;
    use crate::types::TextLineOutputRequest;

    fn test_fontdb() -> fontdb::Database {
        build_text_fontdb(&EngineOptions::default())
    }

    fn render_line(source: &str) -> RenderLine {
        let line = parse_line(source).unwrap();
        line_with_rendered_static_markup(&line).expect("test line should be renderable")
    }

    #[test]
    fn plain_line_fast_path_returns_positioned_plain_run() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

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
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

        let paths = artifact.paths.expect("plain paths should exist");
        assert_eq!(paths.items.len(), 5);
    }

    #[cfg(not(feature = "raster"))]
    #[test]
    fn plain_line_fast_path_declines_raster_without_raster_feature() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.raster = Some(crate::raster::RasterRequest::default());

        assert!(
            try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
                .unwrap()
                .is_none()
        );
    }

    #[cfg(feature = "raster")]
    #[test]
    fn plain_line_fast_path_can_emit_raster() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;
        options.outputs.raster = Some(crate::raster::RasterRequest { scale: 2.0 });

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

        assert!(artifact.paths.is_none());
        assert!(
            artifact
                .raster
                .as_ref()
                .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0)
        );
    }

    #[test]
    fn plain_line_fast_path_can_emit_non_rtl_missing_glyphs() {
        let fontdb = test_fontdb();
        let line = render_line("Revenue 🚀");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("Revenue 🚀", &line, &options, &fontdb)
            .unwrap()
            .expect("non-RTL missing glyphs should stay on the Typst path");

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
    }

    #[test]
    fn plain_line_fast_path_handles_rtl_with_fallback_when_available() {
        let fontdb = test_fontdb();
        let line = render_line("שלום");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        if let Some(artifact) =
            try_typeset_plain_text_line("שלום", &line, &options, &fontdb).unwrap()
        {
            assert!(artifact.metrics.width > 0.0);
            assert!(artifact.paths.is_some());
        }
    }

    #[test]
    fn plain_line_fast_path_handles_zwj_with_fallback_when_available() {
        let fontdb = test_fontdb();
        let line = render_line("Family 👨‍👩‍👧‍👦");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        if let Some(artifact) =
            try_typeset_plain_text_line("Family 👨‍👩‍👧‍👦", &line, &options, &fontdb).unwrap()
        {
            assert!(artifact.metrics.width > 0.0);
            assert!(artifact.paths.is_some());
        }
    }

    #[test]
    fn plain_line_fast_path_can_emit_pdf_glyph_metadata() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

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
        let fontdb = test_fontdb();
        let line = render_line("#underline[important]");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact =
            try_typeset_plain_text_line("#underline[important]", &line, &options, &fontdb)
                .unwrap()
                .expect("supported static decoration should use fast path");

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "important");
        assert!(
            artifact.positioned_runs[0]
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 1 && paths.items[0].stroke.is_some())
        );
        assert!(artifact.paths.as_ref().is_some_and(|paths| {
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        }));
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn underline_uses_font_decoration_metrics() {
        let fontdb = test_fontdb();
        let line = render_line("#underline[important]");
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: false,
        };
        let font_size = options.text_style.font_size.max(1.0);
        let face = TextFace::for_plain_style_and_text(&options.text_style, "important", &fontdb)
            .unwrap()
            .expect("default text face should resolve");
        let expected = face.decoration_metrics(font_size).underline;

        let artifact =
            try_typeset_plain_text_line("#underline[important]", &line, &options, &fontdb)
                .unwrap()
                .expect("supported static decoration should use fast path");
        let paths = artifact.paths.expect("decorated paths should exist");
        let underline = paths
            .items
            .iter()
            .find(|item| item.stroke.is_some())
            .expect("underline stroke should be present");
        let y = match underline.path.commands.first() {
            Some(PathCommand::MoveTo { y, .. }) => *y,
            other => panic!("expected underline to start with MoveTo, got {other:?}"),
        };

        assert!((underline.stroke.as_ref().unwrap().width - expected.thickness).abs() < 1e-4);
        assert!((y - (artifact.metrics.baseline - expected.position)).abs() < 1e-4);
        assert!(expected.position < -font_size * 0.2);
        assert!(expected.thickness < font_size * 0.06);
    }

    #[test]
    fn highlighted_plain_line_emits_background_before_glyphs() {
        let fontdb = test_fontdb();
        let line = render_line("#highlight[warning]");
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = try_typeset_plain_text_line("#highlight[warning]", &line, &options, &fontdb)
            .unwrap()
            .expect("supported static highlight should use fast path");
        let paths = artifact.paths.expect("highlight paths should exist");

        assert!(matches!(paths.items[0].kind, PathKind::MathShape));
        assert!(paths.items[0].fill.is_some());
    }
}
