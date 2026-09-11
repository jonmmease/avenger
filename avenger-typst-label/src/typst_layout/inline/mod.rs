use crate::label::EngineOptions;
use crate::label::LabelError;
use crate::label::{
    FontResource, FontResourceId, LabelWarning, PdfGlyph, PdfGlyphRun, PdfTextLayer,
};
use crate::typst_layout::frame::{
    LineLayoutArtifact, LineLayoutOptions, MathLayoutOptions, PositionedTextLineRun,
    PositionedTextLineRunKind, TypesetMetrics,
};
use crate::typst_library::{Color, FontStyle, FontWeight, TextStyle};
use crate::typst_svg::{
    PathArtifact, PathCommand, PathData, PathItem, PathKind, Stroke, Transform,
};

pub(crate) mod font;

use self::font::{
    SegmentedText, ShapedText, TextDecorationLineMetrics, TextDecorationMetrics, TextFace,
    TextScript, shape_plain_text_with_fallback,
};
use crate::typst_eval::math::parse_math_with_params;
use crate::typst_layout::math::try_typeset_simple_row_fragment_with_fontdb;
use crate::typst_library::text::content::{
    LabelContent, PlainTextNode, TextDecorationOptions, TextMarkupKind, TextMarkupOptions,
};
use crate::typst_realize::{DecoratedText, RenderLine, RenderNode, realize_static_markup_line};

pub(crate) fn try_layout_text_line(
    source: &str,
    line: &LabelContent,
    options: &LineLayoutOptions,
    config: &EngineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<LineLayoutArtifact>, LabelError> {
    let Some(line) = realize_static_markup_line(line, &options.params)? else {
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

#[derive(Debug, Clone, PartialEq)]
struct TextDecoration {
    kind: TextMarkupKind,
    options: TextDecorationOptions,
}

impl TextDecoration {
    fn from_markup(kind: TextMarkupKind, options: TextMarkupOptions) -> Self {
        Self {
            kind,
            options: options.decoration,
        }
    }

    fn is_line_decoration(&self) -> bool {
        self.kind.is_line_decoration()
    }

    fn is_background(&self) -> bool {
        self.is_line_decoration() && self.options.background
    }

    fn is_foreground(&self) -> bool {
        self.is_line_decoration() && !self.options.background
    }
}

fn text_decorations_for_run(decorated: &DecoratedText) -> Vec<TextDecoration> {
    let mut decorations = Vec::new();
    if decorated.kind.is_line_decoration() {
        decorations.push(TextDecoration::from_markup(
            decorated.kind,
            decorated.options.clone(),
        ));
    }
    decorations.extend(
        decorated
            .nested
            .iter()
            .filter(|nested| nested.kind.is_line_decoration())
            .map(|nested| TextDecoration::from_markup(nested.kind, nested.options.clone())),
    );
    decorations
}

fn shape_plain_text_for_style(
    style: &TextStyle,
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
    options: &LineLayoutOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<LineLayoutArtifact>, LabelError> {
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
            typeset_segmented_plain_text_line(source, plain, &[], options, segmented)
        }
        RenderNode::DecoratedText(decorated) => {
            let decorations = text_decorations_for_run(decorated);
            let run_style =
                text_style_for_static_run(&options.text_style, decorated.kind, &decorated.options);
            let Some(face) =
                TextFace::for_plain_style_and_text(&run_style, &decorated.text, fontdb)?
            else {
                return Ok(None);
            };
            let features = text_features_for_static_run(decorated.kind, &decorated.options);
            typeset_plain_text_line(
                source,
                &PlainTextNode {
                    text: decorated.text.clone(),
                    byte_range: decorated.byte_range.clone(),
                },
                &decorations,
                &features,
                &run_style,
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
    options: &LineLayoutOptions,
    config: &EngineOptions,
    fontdb: &fontdb::Database,
) -> Result<Option<LineLayoutArtifact>, LabelError> {
    let text_font_size = options.text_style.font_size.max(1.0);
    let math_options = MathLayoutOptions {
        style: options.math_style.clone(),
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
                let paths = Some(plain_path_artifact_from_segmented(
                    &segmented,
                    metrics,
                    metrics.baseline,
                    text_font_size,
                    options.text_style.fill,
                    &[],
                ));
                let (pdf_text, font_resources) = plain_pdf_text_from_segmented(
                    &plain.text,
                    &segmented,
                    metrics,
                    metrics.baseline,
                    text_font_size,
                    options.text_style.fill,
                );
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
                    pdf_text: Some(pdf_text),
                    font_resources,
                    positioned_plain_runs: positioned_plain_runs_from_segmented(
                        plain,
                        &segmented,
                        &options.text_style,
                        metrics.baseline,
                        true,
                    ),
                });
            }
            RenderNode::DecoratedText(decorated) => {
                if decorated.text.is_empty() {
                    continue;
                }
                let decorations = text_decorations_for_run(decorated);
                let script = text_script_for_kind(decorated.kind);
                let base_run_style = text_style_for_static_run(
                    &options.text_style,
                    decorated.kind,
                    &decorated.options,
                );
                let Some(text_face) =
                    TextFace::for_plain_style_and_text(&base_run_style, &decorated.text, fontdb)?
                else {
                    return Ok(None);
                };
                let run_style = script
                    .map(|script| {
                        text_face.script_style_with_size(
                            &base_run_style,
                            script,
                            decorated.options.script.size,
                        )
                    })
                    .unwrap_or(base_run_style);
                let run_font_size = run_style.font_size.max(1.0);
                let baseline_shift = script
                    .map(|script| {
                        text_face.script_baseline_shift_with_baseline(
                            text_font_size,
                            script,
                            decorated.options.script.baseline,
                        )
                    })
                    .unwrap_or(0.0);
                let features = text_features_for_static_run(decorated.kind, &decorated.options);
                let shaped = shape_text_for_static_run(
                    &text_face,
                    &decorated.text,
                    run_font_size,
                    script,
                    &features,
                );
                let metrics = shifted_text_metrics(&shaped, baseline_shift);
                let glyph_baseline_y = metrics.baseline + baseline_shift;
                let paths = Some(plain_path_artifact_from_shaped(
                    &text_face,
                    &shaped,
                    metrics,
                    glyph_baseline_y,
                    run_font_size,
                    run_style.fill,
                    &decorations,
                ));
                let glyph_paths = glyph_outline_paths_from_shaped(
                    &text_face,
                    &shaped,
                    run_font_size,
                    glyph_baseline_y,
                );
                let positioned_paths = decoration_path_artifact(
                    &decorations,
                    metrics,
                    run_font_size,
                    run_style.fill,
                    Some(&text_face),
                    &glyph_paths,
                );
                let font_id = FontResourceId(0);
                let pdf_text = plain_pdf_text_from_shaped(
                    &decorated.text,
                    &shaped,
                    metrics,
                    glyph_baseline_y,
                    run_font_size,
                    run_style.fill,
                    font_id,
                );
                let font_resources = vec![text_face.font_resource(font_id)];
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
                    pdf_text: Some(pdf_text),
                    font_resources,
                    positioned_plain_runs: vec![PositionedTextLineRun {
                        kind: PositionedTextLineRunKind::Plain,
                        text: decorated.text.clone(),
                        byte_range: decorated.byte_range.clone(),
                        is_rtl: false,
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
                let math =
                    parse_math_with_params(&span.source, span.source_range.start, &options.params)?;
                let Some(artifact) = try_typeset_simple_row_fragment_with_fontdb(
                    &math,
                    &math_options,
                    config,
                    fontdb,
                )?
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
                    positioned_paths: Some(artifact.paths.clone()),
                    paths: Some(artifact.paths),
                    pdf_text: Some(artifact.pdf_text),
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
    let paths = full_line_path_artifact(&run_parts, metrics);
    let (pdf_text, font_resources) = full_line_pdf_text(source, &run_parts, metrics);
    let positioned_runs = {
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
                        is_rtl: false,
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
    };

    Ok(Some(LineLayoutArtifact {
        source: source.to_string(),
        metrics,
        paths,
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
    text_style: Option<TextStyle>,
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
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Smallcaps
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => None,
    }
}

fn text_style_for_static_run(
    style: &TextStyle,
    kind: TextMarkupKind,
    options: &TextMarkupOptions,
) -> TextStyle {
    let mut run_style = style.clone();
    match kind {
        TextMarkupKind::Emph => {
            run_style.font_style = match run_style.font_style {
                FontStyle::Normal => FontStyle::Italic,
                FontStyle::Italic | FontStyle::Oblique => FontStyle::Normal,
            };
        }
        TextMarkupKind::Strong => {
            run_style.font_weight =
                thicken_font_weight(&run_style.font_weight, options.strong.delta);
        }
        TextMarkupKind::Raw => {
            run_style.font_family = "monospace".to_string();
            run_style.font_size = (run_style.font_size * 0.8).max(1.0);
        }
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Smallcaps => {}
    }
    run_style
}

fn thicken_font_weight(weight: &FontWeight, delta: i64) -> FontWeight {
    let number = (font_weight_number(weight) as i64 + delta).clamp(1, 1000) as u16;
    font_weight_from_number(number)
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn font_weight_from_number(number: u16) -> FontWeight {
    match number {
        400 => FontWeight::Normal,
        700 => FontWeight::Bold,
        other => FontWeight::Number(other),
    }
}

fn shape_text_for_static_run(
    face: &TextFace,
    text: &str,
    font_size: f32,
    script: Option<TextScript>,
    features: &[rustybuzz::Feature],
) -> ShapedText {
    if script.is_none() {
        return face.shaped_text_with_features(text, font_size, features);
    }
    let shaped_with_feature = face.shaped_text_with_features(text, font_size, features);
    let shaped_without_feature = face.shaped_text(text, font_size);

    if shaped_feature_changed_glyphs(&shaped_with_feature, &shaped_without_feature) {
        shaped_with_feature
    } else {
        shaped_without_feature
    }
}

fn text_features_for_static_run(
    kind: TextMarkupKind,
    options: &TextMarkupOptions,
) -> Vec<rustybuzz::Feature> {
    let mut features = Vec::new();
    let mut push_feature = |tag: &[u8; 4]| {
        features.push(rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(tag),
            1,
            ..,
        ));
    };

    match kind {
        TextMarkupKind::Subscript if options.script.typographic => push_feature(b"subs"),
        TextMarkupKind::Superscript if options.script.typographic => push_feature(b"sups"),
        TextMarkupKind::Smallcaps => {
            push_feature(b"smcp");
            if options.smallcaps.all {
                push_feature(b"c2sc");
            }
        }
        TextMarkupKind::Subscript | TextMarkupKind::Superscript => {}
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => {}
    }

    features
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
    text_style: &TextStyle,
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
                        &[],
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
                byte_range: (plain.byte_range.start + run.byte_range.start)
                    ..(plain.byte_range.start + run.byte_range.end),
                is_rtl: run.is_rtl,
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
        if let Some(previous) = merged.last_mut()
            && previous.text_style == run.text_style
            && previous.is_rtl == run.is_rtl
            && previous.paths.is_none()
            && previous.pdf_text.is_none()
            && run.paths.is_none()
            && run.pdf_text.is_none()
            && (previous.y - run.y).abs() <= f32::EPSILON
        {
            previous.text.push_str(&run.text);
            previous.byte_range.start = previous.byte_range.start.min(run.byte_range.start);
            previous.byte_range.end = previous.byte_range.end.max(run.byte_range.end);
            let right = (run.x + run.metrics.width).max(previous.x + previous.metrics.width);
            previous.metrics.width = right - previous.x;
            previous.metrics.ascent = previous.metrics.ascent.max(run.metrics.ascent);
            previous.metrics.descent = previous.metrics.descent.max(run.metrics.descent);
            previous.metrics.height = previous.metrics.ascent + previous.metrics.descent;
            previous.metrics.baseline = previous.metrics.ascent;
            continue;
        }
        merged.push(run);
    }
    merged
}

fn positioned_plain_text_style(text_style: &TextStyle, face: &TextFace) -> TextStyle {
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

fn glyph_outline_paths_from_shaped(
    face: &TextFace,
    shaped: &ShapedText,
    font_size: f32,
    glyph_baseline_y: f32,
) -> Vec<PathData> {
    shaped
        .glyphs
        .iter()
        .filter_map(|glyph| {
            let path = face.outline_glyph_path(
                glyph.glyph_id,
                font_size,
                glyph.x,
                glyph_baseline_y + glyph.y,
            );
            (!path.commands.is_empty()).then_some(path)
        })
        .collect()
}

fn glyph_outline_paths_from_segmented(
    segmented: &SegmentedText,
    font_size: f32,
    glyph_baseline_y: f32,
) -> Vec<PathData> {
    segmented
        .runs
        .iter()
        .flat_map(|run| {
            run.shaped.glyphs.iter().filter_map(|glyph| {
                let path = run.face.outline_glyph_path(
                    glyph.glyph_id,
                    font_size,
                    run.x + glyph.x,
                    glyph_baseline_y + glyph.y,
                );
                (!path.commands.is_empty()).then_some(path)
            })
        })
        .collect()
}

fn plain_path_artifact_from_shaped(
    face: &TextFace,
    shaped: &ShapedText,
    metrics: TypesetMetrics,
    glyph_baseline_y: f32,
    font_size: f32,
    fill: Color,
    decorations: &[TextDecoration],
) -> PathArtifact {
    let mut items = Vec::new();
    let mut images = Vec::new();
    let mut glyph_items = Vec::new();
    let mut glyph_paths = Vec::new();

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
                glyph_paths.push(path.clone());
                glyph_items.push(PathItem {
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

    for decoration in decorations
        .iter()
        .filter(|decoration| decoration.is_background())
    {
        if let Some(item) = decoration_path_item(
            decoration.clone(),
            metrics,
            font_size,
            fill,
            None,
            &glyph_paths,
        ) {
            items.push(item);
        }
    }
    items.extend(glyph_items);

    for decoration in decorations
        .iter()
        .filter(|decoration| decoration.is_foreground())
    {
        if let Some(item) = decoration_path_item(
            decoration.clone(),
            metrics,
            font_size,
            fill,
            Some(face),
            &glyph_paths,
        ) {
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
    decorations: &[TextDecoration],
) -> PathArtifact {
    let mut items = Vec::new();
    let mut images = Vec::new();
    let mut glyph_items = Vec::new();
    let mut glyph_paths = Vec::new();

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
                    glyph_paths.push(path.clone());
                    glyph_items.push(PathItem {
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

    for decoration in decorations
        .iter()
        .filter(|decoration| decoration.is_background())
    {
        if let Some(item) = decoration_path_item(
            decoration.clone(),
            metrics,
            font_size,
            fill,
            None,
            &glyph_paths,
        ) {
            items.push(item);
        }
    }
    items.extend(glyph_items);

    for decoration in decorations
        .iter()
        .filter(|decoration| decoration.is_foreground())
    {
        let face = segmented.runs.first().map(|run| &run.face);
        if let Some(item) = decoration_path_item(
            decoration.clone(),
            metrics,
            font_size,
            fill,
            face,
            &glyph_paths,
        ) {
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
    decorations: &[TextDecoration],
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
    face: Option<&TextFace>,
    glyph_paths: &[PathData],
) -> Option<PathArtifact> {
    let items = decorations
        .iter()
        .filter_map(|decoration| {
            decoration_path_item(
                decoration.clone(),
                metrics,
                font_size,
                fill,
                face,
                glyph_paths,
            )
        })
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(PathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
        images: Vec::new(),
    })
}

fn decoration_path_item(
    decoration: TextDecoration,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
    face: Option<&TextFace>,
    glyph_paths: &[PathData],
) -> Option<PathItem> {
    let kind = decoration.kind;
    let item = match kind {
        TextMarkupKind::Underline => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
            decoration.options,
            font_size,
            glyph_paths,
            true,
        ),
        TextMarkupKind::Strike => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
            decoration.options,
            font_size,
            glyph_paths,
            false,
        ),
        TextMarkupKind::Overline => line_decoration_item(
            metrics.width,
            metrics,
            decoration_line(face, font_size, kind),
            fill,
            decoration.options,
            font_size,
            glyph_paths,
            true,
        ),
        TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Smallcaps
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => return None,
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
        TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Smallcaps
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => TextDecorationMetrics::fallback(font_size).underline,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn line_decoration_item(
    width: f32,
    metrics: TypesetMetrics,
    line: TextDecorationLineMetrics,
    fill: Color,
    options: TextDecorationOptions,
    font_size: f32,
    glyph_paths: &[PathData],
    default_evade: bool,
) -> PathItem {
    let position = options
        .offset
        .map(|offset| -offset.resolve(font_size))
        .unwrap_or(line.position);
    let extent = options.extent.resolve(font_size);
    let y = metrics.baseline - position;
    let evade = options.evade.unwrap_or(default_evade);
    let path = line_decoration_path(width, y, extent, evade, font_size, glyph_paths);
    let stroke_width = options
        .stroke
        .thickness
        .map(|thickness| thickness.resolve(font_size))
        .unwrap_or(line.thickness);
    PathItem {
        path,
        kind: PathKind::MathShape,
        fill: None,
        stroke: Some(Stroke {
            color: options.stroke.paint.unwrap_or(fill),
            width: stroke_width,
            line_cap: options.stroke.line_cap.unwrap_or_default(),
            line_join: options.stroke.line_join.unwrap_or_default(),
            dash: options
                .stroke
                .dash
                .as_ref()
                .and_then(|dash| dash.resolve(stroke_width, font_size)),
            miter_limit: options.stroke.miter_limit.unwrap_or(4.0),
        }),
        transform: Transform::IDENTITY,
        clip: None,
    }
}

fn line_decoration_path(
    width: f32,
    y: f32,
    extent: f32,
    evade: bool,
    font_size: f32,
    glyph_paths: &[PathData],
) -> PathData {
    let start = -extent;
    let end = width + extent;
    if !evade {
        return line_decoration_segments([(start, end)], y);
    }

    let gap_padding = 0.08 * font_size;
    let min_width = 0.162 * font_size;
    let mut intersections = Vec::new();
    intersections.push(start - gap_padding);
    intersections.push(end + gap_padding);
    for path in glyph_paths {
        intersections.extend(path_horizontal_intersections(path, y, 0.0, width));
    }
    sort_and_dedup_positions(&mut intersections);

    let mut segments = Vec::new();
    for edge in intersections.windows(2) {
        let l = edge[0];
        let r = edge[1];
        if r - l < gap_padding {
            continue;
        }
        let from = (l + gap_padding).max(start);
        let to = (r - gap_padding).min(end);
        if to - from >= min_width {
            segments.push((from, to));
        }
    }
    line_decoration_segments(segments, y)
}

fn line_decoration_segments<I>(segments: I, y: f32) -> PathData
where
    I: IntoIterator<Item = (f32, f32)>,
{
    let mut commands = Vec::new();
    for (from, to) in segments {
        if to <= from {
            continue;
        }
        commands.push(PathCommand::MoveTo { x: from, y });
        commands.push(PathCommand::LineTo { x: to, y });
    }
    PathData { commands }
}

#[derive(Clone, Copy)]
struct PathPoint {
    x: f32,
    y: f32,
}

fn path_horizontal_intersections(path: &PathData, y: f32, x_min: f32, x_max: f32) -> Vec<f32> {
    let mut intersections = Vec::new();
    let mut current = None;
    let mut contour_start = None;

    for command in &path.commands {
        match *command {
            PathCommand::MoveTo { x, y } => {
                let point = PathPoint { x, y };
                current = Some(point);
                contour_start = Some(point);
            }
            PathCommand::LineTo { x, y: next_y } => {
                let next = PathPoint { x, y: next_y };
                if let Some(from) = current {
                    push_line_intersection(&mut intersections, from, next, y, x_min, x_max);
                }
                current = Some(next);
            }
            PathCommand::QuadTo {
                x1,
                y1,
                x,
                y: next_y,
            } => {
                let control = PathPoint { x: x1, y: y1 };
                let next = PathPoint { x, y: next_y };
                if let Some(from) = current {
                    push_curve_intersections(&mut intersections, y, x_min, x_max, 16, |t| {
                        quad_point(from, control, next, t)
                    });
                }
                current = Some(next);
            }
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y: next_y,
            } => {
                let control1 = PathPoint { x: x1, y: y1 };
                let control2 = PathPoint { x: x2, y: y2 };
                let next = PathPoint { x, y: next_y };
                if let Some(from) = current {
                    push_curve_intersections(&mut intersections, y, x_min, x_max, 24, |t| {
                        cubic_point(from, control1, control2, next, t)
                    });
                }
                current = Some(next);
            }
            PathCommand::Close => {
                if let (Some(from), Some(next)) = (current, contour_start) {
                    push_line_intersection(&mut intersections, from, next, y, x_min, x_max);
                }
                current = contour_start;
            }
        }
    }

    sort_and_dedup_positions(&mut intersections);
    intersections
}

fn push_curve_intersections<F>(
    intersections: &mut Vec<f32>,
    y: f32,
    x_min: f32,
    x_max: f32,
    steps: usize,
    mut point_at: F,
) where
    F: FnMut(f32) -> PathPoint,
{
    let mut previous = point_at(0.0);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let next = point_at(t);
        push_line_intersection(intersections, previous, next, y, x_min, x_max);
        previous = next;
    }
}

fn push_line_intersection(
    intersections: &mut Vec<f32>,
    from: PathPoint,
    to: PathPoint,
    y: f32,
    x_min: f32,
    x_max: f32,
) {
    let dy = to.y - from.y;
    if dy.abs() <= f32::EPSILON {
        return;
    }
    let crosses = (from.y <= y && y < to.y) || (to.y <= y && y < from.y);
    if !crosses {
        return;
    }
    let t = (y - from.y) / dy;
    let x = from.x + (to.x - from.x) * t;
    if x >= x_min && x <= x_max {
        intersections.push(x);
    }
}

fn quad_point(p0: PathPoint, p1: PathPoint, p2: PathPoint, t: f32) -> PathPoint {
    let mt = 1.0 - t;
    PathPoint {
        x: mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
        y: mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
    }
}

fn cubic_point(p0: PathPoint, p1: PathPoint, p2: PathPoint, p3: PathPoint, t: f32) -> PathPoint {
    let mt = 1.0 - t;
    PathPoint {
        x: mt * mt * mt * p0.x
            + 3.0 * mt * mt * t * p1.x
            + 3.0 * mt * t * t * p2.x
            + t * t * t * p3.x,
        y: mt * mt * mt * p0.y
            + 3.0 * mt * mt * t * p1.y
            + 3.0 * mt * t * t * p2.y
            + t * t * t * p3.y,
    }
}

fn sort_and_dedup_positions(values: &mut Vec<f32>) {
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup_by(|a, b| (*a - *b).abs() <= 0.01);
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
                            tx: glyph.x,
                            ty: glyph_baseline_y + glyph.y,
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
                        tx: run.x + glyph.x,
                        ty: glyph_baseline_y + glyph.y,
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
        item.transform.tx += dx;
        item.transform.ty += dy;
    }
    for image in &mut paths.images {
        image.transform.tx += dx;
        image.transform.ty += dy;
    }
    paths
}

fn full_line_pdf_text(
    source: &str,
    parts: &[MixedRunPart],
    metrics: TypesetMetrics,
) -> (PdfTextLayer, Vec<FontResource>) {
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
        PdfTextLayer {
            logical_width: metrics.width,
            logical_height: metrics.height,
            semantic_text: source.to_string(),
            glyph_runs,
        },
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
            glyph.transform.tx += dx;
            glyph.transform.ty += dy;
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
    a.face_index == b.face_index && a.data == b.data
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
    decorations: &[TextDecoration],
    features: &[rustybuzz::Feature],
    text_style: &TextStyle,
    _options: &LineLayoutOptions,
    face: TextFace,
) -> Result<Option<LineLayoutArtifact>, LabelError> {
    let font_size = text_style.font_size.max(1.0);
    let shaped = face.shaped_text_with_features(&plain.text, font_size, features);
    let metrics = TypesetMetrics {
        width: shaped.metrics.width,
        height: shaped.metrics.height,
        baseline: shaped.metrics.ascent,
        ascent: shaped.metrics.ascent,
        descent: shaped.metrics.descent,
    };
    let font_id = FontResourceId(0);
    let font_resources = vec![face.font_resource(font_id)];
    let pdf_text = plain_pdf_text_from_shaped(
        source,
        &shaped,
        metrics,
        metrics.baseline,
        font_size,
        text_style.fill,
        font_id,
    );
    let paths = plain_path_artifact_from_shaped(
        &face,
        &shaped,
        metrics,
        metrics.baseline,
        font_size,
        text_style.fill,
        decorations,
    );
    let glyph_paths = glyph_outline_paths_from_shaped(&face, &shaped, font_size, metrics.baseline);
    let positioned_paths = decoration_path_artifact(
        decorations,
        metrics,
        font_size,
        text_style.fill,
        Some(&face),
        &glyph_paths,
    );
    let positioned_runs = vec![PositionedTextLineRun {
        kind: PositionedTextLineRunKind::Plain,
        text: plain.text.clone(),
        byte_range: plain.byte_range.clone(),
        is_rtl: false,
        text_style: Some(text_style.clone()),
        x: 0.0,
        y: metrics.baseline,
        metrics,
        paths: positioned_paths,
        pdf_text: None,
        font_resources: Vec::new(),
    }];

    Ok(Some(LineLayoutArtifact {
        source: source.to_string(),
        metrics,
        paths,
        pdf_text,
        positioned_runs,
        font_resources,
        warnings: Vec::<LabelWarning>::new(),
    }))
}

fn typeset_segmented_plain_text_line(
    source: &str,
    plain: &PlainTextNode,
    decorations: &[TextDecoration],
    options: &LineLayoutOptions,
    segmented: SegmentedText,
) -> Result<Option<LineLayoutArtifact>, LabelError> {
    let font_size = options.text_style.font_size.max(1.0);
    let metrics = metrics_from_segmented_text(&segmented, 0.0);
    let (pdf_text, font_resources) = plain_pdf_text_from_segmented(
        source,
        &segmented,
        metrics,
        metrics.baseline,
        font_size,
        options.text_style.fill,
    );
    let paths = plain_path_artifact_from_segmented(
        &segmented,
        metrics,
        metrics.baseline,
        font_size,
        options.text_style.fill,
        decorations,
    );
    let mut positioned_runs = positioned_plain_runs_from_segmented(
        plain,
        &segmented,
        &options.text_style,
        metrics.baseline,
        true,
    );
    if let Some(first) = positioned_runs.first_mut()
        && !decorations.is_empty()
    {
        let glyph_paths =
            glyph_outline_paths_from_segmented(&segmented, font_size, metrics.baseline);
        first.paths = decoration_path_artifact(
            decorations,
            metrics,
            font_size,
            options.text_style.fill,
            segmented.runs.first().map(|run| &run.face),
            &glyph_paths,
        );
    }

    Ok(Some(LineLayoutArtifact {
        source: source.to_string(),
        metrics,
        paths,
        pdf_text,
        positioned_runs,
        font_resources,
        warnings: Vec::<LabelWarning>::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::EngineOptions;
    use crate::typst_eval::markup::parse_line;
    use crate::typst_layout::inline::font::build_text_fontdb;

    fn test_fontdb() -> fontdb::Database {
        build_text_fontdb(&EngineOptions::default())
    }

    fn render_line(source: &str) -> RenderLine {
        let line = parse_line(source).unwrap();
        realize_static_markup_line(&line, &Default::default())
            .expect("test line should not error")
            .expect("test line should be renderable")
    }

    fn first_stroke_item(paths: &PathArtifact) -> &PathItem {
        paths
            .items
            .iter()
            .find(|item| item.stroke.is_some())
            .expect("stroke path should be present")
    }

    fn stroke_commands(paths: &PathArtifact) -> &[PathCommand] {
        &first_stroke_item(paths).path.commands
    }

    fn has_path_output(paths: &PathArtifact) -> bool {
        !paths.items.is_empty() || !paths.images.is_empty()
    }

    fn has_pdf_text(pdf_text: &PdfTextLayer) -> bool {
        !pdf_text.glyph_runs.is_empty()
    }

    #[test]
    fn plain_line_fast_path_returns_positioned_plain_run() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let options = LineLayoutOptions::default();

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
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 5);
    }

    #[test]
    fn plain_line_fast_path_can_emit_non_rtl_missing_glyphs() {
        let fontdb = test_fontdb();
        let line = render_line("Revenue 🚀");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line("Revenue 🚀", &line, &options, &fontdb)
            .unwrap()
            .expect("non-RTL missing glyphs should stay on the Typst path");

        assert!(artifact.metrics.width > 0.0);
        assert!(has_path_output(&artifact.paths));
    }

    #[test]
    fn plain_line_fast_path_handles_rtl_with_fallback_when_available() {
        let fontdb = test_fontdb();
        let line = render_line("שלום");
        let options = LineLayoutOptions::default();

        if let Some(artifact) =
            try_typeset_plain_text_line("שלום", &line, &options, &fontdb).unwrap()
        {
            assert!(artifact.metrics.width > 0.0);
            assert!(has_path_output(&artifact.paths));
        }
    }

    #[test]
    fn plain_line_fast_path_handles_zwj_with_fallback_when_available() {
        let fontdb = test_fontdb();
        let line = render_line("Family 👨‍👩‍👧‍👦");
        let options = LineLayoutOptions::default();

        if let Some(artifact) =
            try_typeset_plain_text_line("Family 👨‍👩‍👧‍👦", &line, &options, &fontdb).unwrap()
        {
            assert!(artifact.metrics.width > 0.0);
            assert!(has_path_output(&artifact.paths));
        }
    }

    #[test]
    fn plain_line_fast_path_can_emit_pdf_glyph_metadata() {
        let fontdb = test_fontdb();
        let line = render_line("Hello");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line("Hello", &line, &options, &fontdb)
            .unwrap()
            .expect("plain embedded text should use fast path");

        assert_eq!(artifact.font_resources.len(), 1);
        let pdf = artifact.pdf_text;
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
        let options = LineLayoutOptions::default();

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
        assert!(
            artifact
                .paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        );
        assert!(has_pdf_text(&artifact.pdf_text));
    }

    #[test]
    fn underline_uses_font_decoration_metrics() {
        let fontdb = test_fontdb();
        let line = render_line("#underline[important]");
        let options = LineLayoutOptions::default();
        let font_size = options.text_style.font_size.max(1.0);
        let face = TextFace::for_plain_style_and_text(&options.text_style, "important", &fontdb)
            .unwrap()
            .expect("default text face should resolve");
        let expected = face.decoration_metrics(font_size).underline;

        let artifact =
            try_typeset_plain_text_line("#underline[important]", &line, &options, &fontdb)
                .unwrap()
                .expect("supported static decoration should use fast path");
        let paths = artifact.paths;
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
    fn underline_literal_stroke_offset_extent() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt)[care]");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(
            "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt)[care]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;
        let underline = first_stroke_item(&paths);
        let stroke = underline.stroke.as_ref().unwrap();
        let (x0, y) = match underline.path.commands.first() {
            Some(PathCommand::MoveTo { x, y }) => (*x, *y),
            other => panic!("expected underline to start with MoveTo, got {other:?}"),
        };
        let x1 = match underline.path.commands.get(1) {
            Some(PathCommand::LineTo { x, .. }) => *x,
            other => panic!("expected underline to end with LineTo, got {other:?}"),
        };

        assert_eq!(stroke.color, Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert!((stroke.width - 1.5).abs() < 1e-4);
        assert!((x0 + 3.0).abs() < 1e-4);
        assert!((x1 - (artifact.metrics.width + 3.0)).abs() < 1e-4);
        assert!((y - (artifact.metrics.baseline + 2.0)).abs() < 1e-4);
    }

    #[test]
    fn underline_background_precedes_glyphs() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(background: true, stroke: red)[care]");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(
            "#underline(background: true, stroke: red)[care]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;

        assert!(matches!(paths.items[0].kind, PathKind::MathShape));
        assert!(paths.items[0].stroke.is_some());
    }

    #[test]
    fn underline_evade_splits_descender_segments() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(evade: true, offset: 2pt)[group]");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(
            "#underline(evade: true, offset: 2pt)[group]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;

        assert!(
            stroke_commands(&paths).len() > 2,
            "evading underline should emit multiple line subpaths"
        );
    }

    #[test]
    fn underline_evade_false_draws_continuous_line() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(evade: false, offset: 2pt)[group]");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(
            "#underline(evade: false, offset: 2pt)[group]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;

        assert_eq!(stroke_commands(&paths).len(), 2);
    }

    #[test]
    fn underline_evades_by_default_like_typst() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(offset: 2pt)[group]");
        let options = LineLayoutOptions::default();

        let artifact =
            try_typeset_plain_text_line("#underline(offset: 2pt)[group]", &line, &options, &fontdb)
                .unwrap()
                .expect("supported static decoration should use fast path");
        let paths = artifact.paths;

        assert!(
            stroke_commands(&paths).len() > 2,
            "Typst defaults underline evade to true"
        );
    }

    #[test]
    fn strike_does_not_evade_by_default() {
        let fontdb = test_fontdb();
        let line = render_line("#strike(offset: -4pt)[group]");
        let options = LineLayoutOptions::default();

        let artifact =
            try_typeset_plain_text_line("#strike(offset: -4pt)[group]", &line, &options, &fontdb)
                .unwrap()
                .expect("supported static decoration should use fast path");
        let paths = artifact.paths;

        assert_eq!(stroke_commands(&paths).len(), 2);
    }

    #[test]
    fn decoration_stroke_dictionary_sets_paint_and_thickness() {
        let fontdb = test_fontdb();
        let line =
            render_line("#underline(stroke: (thickness: 0.4em, paint: maroon, cap: \"round\"))[x]");
        let options = LineLayoutOptions::default();
        let font_size = options.text_style.font_size.max(1.0);

        let artifact = try_typeset_plain_text_line(
            "#underline(stroke: (thickness: 0.4em, paint: maroon, cap: \"round\"))[x]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;
        let stroke = first_stroke_item(&paths).stroke.as_ref().unwrap();

        assert_eq!(stroke.color, Color::rgba(0.5, 0.0, 0.0, 1.0));
        assert!((stroke.width - font_size * 0.4).abs() < 1e-4);
        assert_eq!(stroke.line_cap, crate::typst_svg::LineCap::Round);
        assert_eq!(stroke.line_join, crate::typst_svg::LineJoin::Miter);
    }

    #[test]
    fn decoration_stroke_dictionary_sets_join_and_dash() {
        let fontdb = test_fontdb();
        let line = render_line("#underline(stroke: (join: \"bevel\", dash: \"dash-dotted\"))[x]");
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(
            "#underline(stroke: (join: \"bevel\", dash: \"dash-dotted\"))[x]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;
        let stroke = first_stroke_item(&paths).stroke.as_ref().unwrap();
        let dash = stroke.dash.as_ref().expect("dash should be present");

        assert_eq!(stroke.line_join, crate::typst_svg::LineJoin::Bevel);
        assert_eq!(dash.array.len(), 4);
        assert!((dash.array[0] - 3.0).abs() < 1e-4);
        assert!((dash.array[2] - stroke.width).abs() < 1e-4);
        assert_eq!(dash.phase, 0.0);
        assert_eq!(stroke.miter_limit, 4.0);
    }

    #[test]
    fn overline_supports_negative_em_offset() {
        let fontdb = test_fontdb();
        let line = render_line("#overline(offset: -1.2em, extent: 2pt)[top]");
        let options = LineLayoutOptions::default();
        let font_size = options.text_style.font_size.max(1.0);

        let artifact = try_typeset_plain_text_line(
            "#overline(offset: -1.2em, extent: 2pt)[top]",
            &line,
            &options,
            &fontdb,
        )
        .unwrap()
        .expect("supported static decoration should use fast path");
        let paths = artifact.paths;
        let overline = first_stroke_item(&paths);
        let (x0, y) = match overline.path.commands.first() {
            Some(PathCommand::MoveTo { x, y }) => (*x, *y),
            other => panic!("expected overline to start with MoveTo, got {other:?}"),
        };

        assert!((x0 + 2.0).abs() < 1e-4);
        assert!((y - (artifact.metrics.baseline - font_size * 1.2)).abs() < 1e-4);
    }

    #[test]
    fn overline_supports_same_literal_options_as_typst() {
        let fontdb = test_fontdb();
        let source =
            "#overline(background: true, stroke: 1.5pt + red, offset: -1.2em, extent: 2pt)[top]";
        let line = render_line(source);
        let options = LineLayoutOptions::default();
        let font_size = options.text_style.font_size.max(1.0);

        let artifact = try_typeset_plain_text_line(source, &line, &options, &fontdb)
            .unwrap()
            .expect("supported static overline should use fast path");
        let paths = artifact.paths;

        assert!(matches!(paths.items[0].kind, PathKind::MathShape));
        let overline = first_stroke_item(&paths);
        let stroke = overline.stroke.as_ref().unwrap();
        let (x0, y) = match overline.path.commands.first() {
            Some(PathCommand::MoveTo { x, y }) => (*x, *y),
            other => panic!("expected overline to start with MoveTo, got {other:?}"),
        };
        let x1 = match overline.path.commands.get(1) {
            Some(PathCommand::LineTo { x, .. }) => *x,
            other => panic!("expected overline to end with LineTo, got {other:?}"),
        };

        assert_eq!(stroke.color, Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert!((stroke.width - 1.5).abs() < 1e-4);
        assert!((x0 + 2.0).abs() < 1e-4);
        assert!((x1 - (artifact.metrics.width + 2.0)).abs() < 1e-4);
        assert!((y - (artifact.metrics.baseline - font_size * 1.2)).abs() < 1e-4);
    }

    #[test]
    fn strike_supports_literal_options_except_evade() {
        let fontdb = test_fontdb();
        let source =
            "#strike(background: true, stroke: 1.5pt + red, offset: -3.5pt, extent: 2pt)[gone]";
        let line = render_line(source);
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(source, &line, &options, &fontdb)
            .unwrap()
            .expect("supported static strike should use fast path");
        let paths = artifact.paths;

        assert!(matches!(paths.items[0].kind, PathKind::MathShape));
        let strike = first_stroke_item(&paths);
        let stroke = strike.stroke.as_ref().unwrap();
        let (x0, y) = match strike.path.commands.first() {
            Some(PathCommand::MoveTo { x, y }) => (*x, *y),
            other => panic!("expected strike to start with MoveTo, got {other:?}"),
        };
        let x1 = match strike.path.commands.get(1) {
            Some(PathCommand::LineTo { x, .. }) => *x,
            other => panic!("expected strike to end with LineTo, got {other:?}"),
        };

        assert_eq!(stroke.color, Color::rgba(1.0, 0.0, 0.0, 1.0));
        assert!((stroke.width - 1.5).abs() < 1e-4);
        assert!((x0 + 2.0).abs() < 1e-4);
        assert!((x1 - (artifact.metrics.width + 2.0)).abs() < 1e-4);
        assert!((y - (artifact.metrics.baseline - 3.5)).abs() < 1e-4);
        assert_eq!(stroke_commands(&paths).len(), 2);
    }

    #[test]
    fn nested_decorations_preserve_order() {
        let fontdb = test_fontdb();
        let source = "#underline(background: true, stroke: red)[#overline(stroke: blue)[x]]";
        let line = render_line(source);
        let options = LineLayoutOptions::default();

        let artifact = try_typeset_plain_text_line(source, &line, &options, &fontdb)
            .unwrap()
            .expect("nested static decorations should use fast path");
        let stroke_items = artifact
            .paths
            .items
            .iter()
            .filter(|item| item.stroke.is_some())
            .collect::<Vec<_>>();

        assert_eq!(stroke_items.len(), 2);
        assert_eq!(
            stroke_items[0].stroke.as_ref().unwrap().color,
            Color::rgba(1.0, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            stroke_items[1].stroke.as_ref().unwrap().color,
            Color::rgba(0.0, 0.0, 1.0, 1.0)
        );
    }
}
