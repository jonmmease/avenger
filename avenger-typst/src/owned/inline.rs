use crate::error::MathTypesetError;
use crate::paths::{MathPathArtifact, MathPathItem, MathPathKind, MathTransform};
use crate::pdf::{MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::types::{
    PositionedTextLineRun, PositionedTextLineRunKind, TextLineArtifact, TextLineOptions,
    TypesetMetrics,
};
use crate::warnings::MathTypesetWarning;

use super::ast::{OwnedLine, OwnedLineNode, OwnedPlainText};
use super::font::OwnedTextFace;
use super::glyph_path::outline_glyph_path;

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
