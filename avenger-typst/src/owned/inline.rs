use crate::error::MathTypesetError;
use crate::types::{
    PositionedTextLineRun, PositionedTextLineRunKind, TextLineArtifact, TextLineOptions,
    TypesetMetrics,
};
use crate::warnings::MathTypesetWarning;

use super::ast::{OwnedLine, OwnedLineNode, OwnedPlainText};
use super::font::OwnedTextFace;

pub(crate) fn try_typeset_plain_text_line(
    source: &str,
    line: &OwnedLine,
    options: &TextLineOptions,
) -> Result<Option<TextLineArtifact>, MathTypesetError> {
    if options.outputs.paths || options.outputs.raster.is_some() || options.outputs.pdf_text_layer {
        return Ok(None);
    }

    let [OwnedLineNode::Plain(plain)] = &line.nodes[..] else {
        return Ok(None);
    };

    let Some(face) = OwnedTextFace::for_plain_style(&options.text_style)? else {
        return Ok(None);
    };

    Ok(Some(typeset_plain_text_line(source, plain, options, face)))
}

fn typeset_plain_text_line(
    source: &str,
    plain: &OwnedPlainText,
    options: &TextLineOptions,
    face: OwnedTextFace<'_>,
) -> TextLineArtifact {
    let font_size = options.text_style.font_size.max(1.0);
    let shaped = face.shaped_metrics(&plain.text, font_size);
    let metrics = TypesetMetrics {
        width: shaped.width,
        height: shaped.height,
        baseline: shaped.ascent,
        ascent: shaped.ascent,
        descent: shaped.descent,
    };
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

    TextLineArtifact {
        source: source.to_string(),
        metrics,
        paths: None,
        raster: None,
        pdf_text: None,
        positioned_runs: positioned_runs.unwrap_or_default(),
        font_resources: Vec::new(),
        warnings: Vec::<MathTypesetWarning>::new(),
    }
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
    fn plain_line_fast_path_declines_heavy_outputs() {
        let line = parse_owned_line("Hello", &MathDelimiterOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        assert!(try_typeset_plain_text_line("Hello", &line, &options)
            .unwrap()
            .is_none());
    }
}
