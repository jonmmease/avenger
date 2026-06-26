use crate::api::TypstEngineConfig;
use crate::error::MathTypesetError;
use crate::paths::{MathPathArtifact, MathPathItem, MathPathKind, MathTransform};
use crate::pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::types::{
    MathFragmentOptions, MathOutputRequest, PositionedTextLineRun, PositionedTextLineRunKind,
    TextLineArtifact, TextLineOptions, TypesetMetrics,
};
use crate::warnings::MathTypesetWarning;

use super::ast::{OwnedLine, OwnedLineNode, OwnedPlainText};
use super::font::{OwnedShapedText, OwnedTextFace};
use super::glyph_path::outline_glyph_path;
use super::math::metrics::try_typeset_simple_row_fragment;
use super::math::syntax::parse_owned_math;

pub(crate) fn try_typeset_owned_text_line(
    source: &str,
    line: &OwnedLine,
    options: &TextLineOptions,
    config: &TypstEngineConfig,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    if line
        .nodes
        .iter()
        .any(|node| !matches!(node, OwnedLineNode::Plain(_) | OwnedLineNode::Math(_)))
    {
        return Ok(None);
    }

    if line
        .nodes
        .iter()
        .any(|node| matches!(node, OwnedLineNode::Math(_)))
    {
        return try_typeset_mixed_metrics_text_line(source, line, options, config);
    }

    try_typeset_plain_text_line(source, line, options)
}

pub(crate) fn try_typeset_plain_text_line(
    source: &str,
    line: &OwnedLine,
    options: &TextLineOptions,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }

    let [OwnedLineNode::Plain(plain)] = &line.nodes[..] else {
        return Ok(None);
    };

    let Some(face) = OwnedTextFace::for_plain_style(&options.text_style)? else {
        return Ok(None);
    };

    typeset_plain_text_line(source, plain, options, face)
}

fn try_typeset_mixed_metrics_text_line(
    source: &str,
    line: &OwnedLine,
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
            OwnedLineNode::Plain(plain) => {
                if plain.text.is_empty() {
                    continue;
                }
                let shaped = text_face.shaped_text(&plain.text, text_font_size);
                if shaped.has_missing_glyph {
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
                            text_font_size,
                            options.text_style.fill,
                        )
                    });
                let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
                    let font_id = MathFontResourceId(0);
                    (
                        Some(plain_pdf_text_from_shaped(
                            &plain.text,
                            &shaped,
                            metrics,
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
                    metrics,
                    paths,
                    pdf_text,
                    font_resources,
                });
            }
            OwnedLineNode::Math(span) => {
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
                    metrics: artifact.metrics,
                    paths: artifact.paths,
                    pdf_text: artifact.pdf_text,
                    font_resources: artifact.font_resources,
                });
            }
            OwnedLineNode::TextSpan(_) | OwnedLineNode::Emoji(_) => return Ok(None),
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
                let paths = matches!(part.kind, PositionedTextLineRunKind::Math)
                    .then(|| {
                        part.paths.clone().map(|paths| {
                            offset_path_artifact(paths, x, dy, part.metrics.width, metrics.height)
                        })
                    })
                    .flatten();
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
                    x,
                    y: metrics.baseline,
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
    metrics: TypesetMetrics,
    paths: Option<MathPathArtifact>,
    pdf_text: Option<MathPdfTextLayer>,
    font_resources: Vec<MathFontResource>,
}

fn plain_path_artifact_from_shaped(
    face: &OwnedTextFace<'_>,
    shaped: &OwnedShapedText,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: crate::style::Color,
) -> MathPathArtifact {
    let items = shaped
        .glyphs
        .iter()
        .enumerate()
        .filter_map(|(glyph_index, glyph)| {
            let path = outline_glyph_path(
                &face.face,
                glyph.glyph_id,
                font_size,
                glyph.x,
                metrics.baseline + glyph.y,
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
        })
        .collect();
    MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items,
    }
}

fn plain_pdf_text_from_shaped(
    semantic_text: &str,
    shaped: &OwnedShapedText,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: crate::style::Color,
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
                            dy: metrics.baseline + glyph.y,
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
    options: &TextLineOptions,
    face: OwnedTextFace<'_>,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    let font_size = options.text_style.font_size.max(1.0);
    let shaped = face.shaped_text(&plain.text, font_size);
    if shaped.has_missing_glyph {
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
    let pdf_text = options.outputs.pdf_text_layer.then(|| MathPdfTextLayer {
        logical_width: metrics.width,
        logical_height: metrics.height,
        semantic_text: source.to_string(),
        glyph_runs: (!shaped.glyphs.is_empty())
            .then(|| MathPdfGlyphRun {
                font: font_id,
                font_size,
                fill: options.text_style.fill,
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
                            dy: metrics.baseline + glyph.y,
                            ..MathTransform::IDENTITY
                        },
                    })
                    .collect(),
            })
            .into_iter()
            .collect(),
    });
    let path_artifact = (options.outputs.paths || options.outputs.raster.is_some()).then(|| {
        let items = shaped
            .glyphs
            .iter()
            .enumerate()
            .filter_map(|(glyph_index, glyph)| {
                let path = outline_glyph_path(
                    &face.face,
                    glyph.glyph_id,
                    font_size,
                    glyph.x,
                    metrics.baseline + glyph.y,
                );
                (!path.commands.is_empty()).then(|| MathPathItem {
                    path,
                    kind: MathPathKind::GlyphOutline {
                        glyph_run: 0,
                        glyph_index,
                    },
                    fill: Some(options.text_style.fill),
                    stroke: None,
                    transform: MathTransform::IDENTITY,
                    clip: None,
                })
            })
            .collect();
        MathPathArtifact {
            logical_width: metrics.width,
            logical_height: metrics.height,
            items,
        }
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
    let positioned_runs = options.outputs.positioned_runs.then(|| {
        vec![PositionedTextLineRun {
            kind: PositionedTextLineRunKind::Plain,
            text: plain.text.clone(),
            byte_range: plain.byte_range.clone(),
            x: 0.0,
            y: metrics.baseline,
            metrics,
            paths: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delimiter::MathDelimiterOptions;
    use crate::owned::syntax::parse_owned_line;
    use crate::types::TextLineOutputRequest;

    #[test]
    fn plain_line_fast_path_returns_positioned_plain_run() {
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
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
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
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
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.raster = Some(crate::raster::RasterRequest::default());

        assert!(try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .is_none());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn plain_line_fast_path_can_emit_raster() {
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
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
    fn plain_line_fast_path_declines_missing_glyphs() {
        let line = parse_owned_line("Revenue 🚀", &MathDelimiterOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        assert!(try_typeset_plain_text_line("Revenue 🚀", &line, &options)
            .unwrap()
            .is_none());
    }

    #[test]
    fn plain_line_fast_path_can_emit_pdf_glyph_metadata() {
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
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
}
