use crate::engine::font::build_text_fontdb;
use crate::error::{LabelError, LabelInitError};
use crate::label::EngineOptions;
use crate::limits::LabelLimits;
use crate::paths::PathArtifact;
use crate::pdf::PdfTextLayer;
#[cfg(test)]
use crate::types::{MathFragmentOptions, MathRunArtifact};
use crate::types::{TextLineArtifact, TextLineOptions, TypesetMetrics};

use crate::engine::ast::{LineNode, ParsedLine, PlainTextNode};
use crate::engine::inline::try_typeset_text_line;
#[cfg(test)]
use crate::engine::math::metrics::try_typeset_simple_row_fragment;
use crate::engine::math::syntax::parse_math;
use crate::engine::syntax::parse_line;

#[derive(Clone)]
pub(crate) struct TypstEngineCore {
    config: EngineOptions,
    text_fontdb: std::sync::Arc<fontdb::Database>,
}

impl std::fmt::Debug for TypstEngineCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstEngineCore").finish_non_exhaustive()
    }
}

impl TypstEngineCore {
    pub(crate) fn new(config: &EngineOptions) -> Result<Self, LabelInitError> {
        Ok(Self {
            config: config.clone(),
            text_fontdb: std::sync::Arc::new(build_text_fontdb(config)),
        })
    }

    #[cfg(test)]
    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, LabelError> {
        let math = parse_math(source, 0)?;
        if let Some(artifact) = try_typeset_simple_row_fragment(&math, options, &self.config)? {
            return Ok(artifact);
        }
        unsupported_fragment()
    }

    pub(crate) fn typeset_markup_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, LabelError> {
        if source.is_empty() && options.outputs.raster.is_none() {
            return Ok(empty_text_line_artifact(source, options));
        }

        let line = parse_line(source)?;
        validate_line_math(&line, options.limits)?;
        self.typeset_parsed_line(source, &line, options)
    }

    pub(crate) fn typeset_plain_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, LabelError> {
        if source.is_empty() && options.outputs.raster.is_none() {
            return Ok(empty_text_line_artifact(source, options));
        }

        let line = plain_text_line(source);
        self.typeset_parsed_line(source, &line, options)
    }

    fn typeset_parsed_line(
        &self,
        source: &str,
        line: &ParsedLine,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, LabelError> {
        if let Some(artifact) = try_typeset_text_line(
            source,
            line,
            options,
            &self.config,
            self.text_fontdb.as_ref(),
        )? {
            return Ok(artifact);
        }
        if line_contains_static_markup(&line) {
            return Err(LabelError::UnsupportedOutput(
                "static text markup is parsed but not rendered yet",
            ));
        }

        unsupported_text_line()
    }
}

#[cfg(test)]
fn unsupported_fragment() -> Result<MathRunArtifact, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "this Typst math subset is not supported yet",
    ))
}

fn unsupported_text_line() -> Result<TextLineArtifact, LabelError> {
    Err(LabelError::UnsupportedOutput(
        "this Typst text-line subset is not supported yet",
    ))
}

fn plain_text_line(source: &str) -> ParsedLine {
    ParsedLine {
        source: source.to_string(),
        nodes: if source.is_empty() {
            Vec::new()
        } else {
            vec![LineNode::Plain(PlainTextNode {
                text: source.to_string(),
                byte_range: 0..source.len(),
            })]
        },
    }
}

fn line_contains_static_markup(line: &ParsedLine) -> bool {
    nodes_contain_static_markup(&line.nodes)
}

fn validate_line_math(line: &ParsedLine, limits: LabelLimits) -> Result<(), LabelError> {
    let math_span_count = line
        .nodes
        .iter()
        .filter(|node| matches!(node, LineNode::Math(_)))
        .count();
    if math_span_count > limits.max_math_spans {
        return Err(LabelError::TooManyMathSpans {
            actual: math_span_count,
            limit: limits.max_math_spans,
        });
    }

    for math in line.nodes.iter().filter_map(|node| match node {
        LineNode::Math(math) => Some(math),
        _ => None,
    }) {
        if math.source.trim().is_empty() {
            return Err(LabelError::EmptyMathFragment {
                start: math.source_range.start,
                end: math.source_range.end,
            });
        }

        let depth = max_grouping_depth(&math.source);
        if depth > limits.max_math_depth {
            return Err(LabelError::MathDepthExceeded {
                actual: depth,
                limit: limits.max_math_depth,
            });
        }

        strict_hash_precheck(&math.source, math.source_range.start)?;
        parse_math(&math.source, math.source_range.start)?;
    }
    Ok(())
}

fn strict_hash_precheck(source: &str, offset: usize) -> Result<(), LabelError> {
    let mut escaped = false;
    for (idx, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '#' {
            return Err(LabelError::UnsupportedSyntax {
                position: offset + idx,
                message: "embedded Typst code is not allowed in math fragments",
            });
        }
    }
    Ok(())
}

fn max_grouping_depth(source: &str) -> usize {
    let mut escaped = false;
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max_depth
}

fn nodes_contain_static_markup(nodes: &[LineNode]) -> bool {
    nodes.iter().any(|node| match node {
        LineNode::Plain(_) | LineNode::Math(_) | LineNode::Emoji(_) => false,
        LineNode::TextSpan(_) => true,
    })
}

fn empty_text_line_artifact(source: &str, options: &TextLineOptions) -> TextLineArtifact {
    let metrics = TypesetMetrics {
        width: 0.0,
        height: 0.0,
        baseline: 0.0,
        ascent: 0.0,
        descent: 0.0,
    };
    let paths = options.outputs.paths.then(|| PathArtifact {
        logical_width: 0.0,
        logical_height: 0.0,
        items: Vec::new(),
        images: Vec::new(),
    });
    let pdf_text = options.outputs.pdf_text_layer.then(|| PdfTextLayer {
        logical_width: 0.0,
        logical_height: 0.0,
        semantic_text: source.to_string(),
        glyph_runs: Vec::new(),
    });

    TextLineArtifact {
        source: source.to_string(),
        metrics,
        paths,
        raster: None,
        pdf_text,
        positioned_runs: Vec::new(),
        font_resources: Vec::new(),
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::FontWeight;
    use crate::types::{MathOutputRequest, TextLineOutputRequest};

    #[test]
    fn empty_text_line_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();

        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };
        let artifact = engine.typeset_markup_line("", &options).unwrap();

        assert_eq!(artifact.metrics.width, 0.0);
        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.is_empty())
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf_text| pdf_text.glyph_runs.is_empty())
        );
        assert!(artifact.positioned_runs.is_empty());
    }

    #[test]
    fn plain_text_line_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 5)
        );
    }

    #[test]
    fn plain_text_line_with_non_rtl_missing_glyph_paths_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_markup_line("Revenue 🚀", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
    }

    #[test]
    fn non_atkinson_plain_text_can_use_fontdb_fallback() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.text_style.font_family = "serif".to_string();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let Ok(artifact) = engine.typeset_markup_line("Fallback font", &options) else {
            return;
        };

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Fallback font");
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn non_atkinson_mixed_script_text_can_segment_fallback_fonts() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.text_style.font_family = "serif".to_string();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let Ok(artifact) = engine.typeset_markup_line("Hello 温度", &options) else {
            return;
        };

        if artifact
            .pdf_text
            .as_ref()
            .is_none_or(|pdf_text| pdf_text.glyph_runs.len() < 2)
        {
            return;
        }

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Hello 温度");
        assert!(artifact.paths.is_some());
        assert!(artifact.font_resources.len() >= 2);
    }

    #[test]
    fn default_mixed_script_text_can_segment_fallback_fonts_when_available() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_markup_line("Hello 温度", &options).unwrap();
        let Some(pdf_text) = artifact.pdf_text.as_ref() else {
            return;
        };
        if pdf_text.glyph_runs.len() < 2 {
            return;
        }

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Hello 温度");
        assert!(artifact.paths.is_some());
        assert!(artifact.font_resources.len() >= 2);
    }

    #[test]
    fn plain_text_line_with_rtl_text_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_markup_line("שלום", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["שלום"]
        );
    }

    #[test]
    fn plain_text_syntax_treats_invalid_math_as_literal_text() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let options = TextLineOptions::default();

        let artifact = engine
            .typeset_plain_line("before $x^$ after", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
    }

    #[test]
    fn plain_text_line_with_zwj_emoji_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_markup_line("Family 👨‍👩‍👧‍👦", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Family ", "👨‍👩‍👧‍👦"]
        );
    }

    #[test]
    fn named_emoji_alias_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_markup_line("Revenue #emoji.rocket", &options)
            .unwrap();

        assert_eq!(artifact.source, "Revenue #emoji.rocket");
        assert_eq!(
            artifact
                .positioned_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Revenue ", "🚀"]
        );
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
    }

    #[cfg(all(feature = "raster", target_os = "macos"))]
    #[test]
    fn named_emoji_alias_rasterizes_color_pixels_on_macos() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: Some(crate::RasterRequest { scale: 2.0 }),
            pdf_text_layer: false,
            positioned_runs: false,
        };

        let artifact = engine
            .typeset_markup_line("Revenue #emoji.rocket", &options)
            .unwrap();
        let raster = artifact
            .raster
            .expect("raster output should be produced for emoji text");

        assert!(
            colored_pixel_count(&raster.image.data) > 20,
            "emoji raster should contain colored bitmap pixels rather than monochrome tofu"
        );
    }

    #[cfg(all(feature = "raster", target_os = "macos"))]
    fn colored_pixel_count(data: &[u8]) -> usize {
        data.chunks_exact(4)
            .filter(|pixel| {
                let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                a > 0 && r.abs_diff(g).max(r.abs_diff(b)).max(g.abs_diff(b)) > 16
            })
            .count()
    }

    #[test]
    fn unknown_static_command_errors_before_rendering() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_markup_line("#let x = 1", &options)
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        );
    }

    #[test]
    fn static_command_options_render() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine
            .typeset_markup_line("#underline(stroke: red)[important]", &options)
            .unwrap();

        assert!(
            artifact
                .paths
                .is_some_and(|paths| paths.items.iter().any(|item| item.stroke.is_some()))
        );
    }

    #[test]
    fn supported_static_decoration_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_markup_line("#underline[important]", &options)
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "important");
        assert!(artifact.positioned_runs[0].paths.is_some());
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn static_subscript_and_superscript_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_markup_line("H#sub[2]O #super[\\*]", &options)
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 4);
        assert_eq!(artifact.positioned_runs[0].text, "H");
        assert_eq!(artifact.positioned_runs[1].text, "2");
        assert_eq!(artifact.positioned_runs[2].text, "O ");
        assert_eq!(artifact.positioned_runs[3].text, "*");
        assert!(artifact.positioned_runs[1].y > artifact.positioned_runs[0].y);
        assert!(artifact.positioned_runs[3].y < artifact.positioned_runs[0].y);
        assert!(
            artifact.positioned_runs[1]
                .text_style
                .as_ref()
                .is_some_and(|style| style.font_size < options.text_style.font_size)
        );
        assert!(
            artifact.positioned_runs[3]
                .text_style
                .as_ref()
                .is_some_and(|style| style.font_size < options.text_style.font_size)
        );
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn matrix_math_fragment_errors_before_rendering() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();

        let err = engine
            .typeset_fragment("mat(1, 2; 3, 4)", &MathFragmentOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn simple_row_math_fragment_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
        };

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
    }

    #[test]
    fn simple_row_math_fragment_paths_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        };

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| !paths.items.is_empty())
        );
    }

    #[test]
    fn simple_row_math_fragment_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(artifact.pdf_text.is_some());
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn simple_script_math_fragment_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("R^2 = 0.94", &options).unwrap();

        let pdf = artifact
            .pdf_text
            .as_ref()
            .expect("Typst script path should emit PDF glyph metadata");
        assert!(pdf.glyph_runs.iter().any(|run| run.font_size < 12.0));
        assert_eq!(artifact.font_resources.len(), 1);
    }

    #[test]
    fn simple_fraction_math_fragment_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("a / (b + c)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 5)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 4)
        );
    }

    #[test]
    fn simple_frac_call_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("frac(x + y, z)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 5)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 4)
        );
    }

    #[test]
    fn simple_binom_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("binom(n, k)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 4)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 4)
        );
    }

    #[test]
    fn simple_cancel_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("cancel(x)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 2)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 1)
        );
    }

    #[test]
    fn simple_sqrt_fraction_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("sqrt(x) / (1 + x^2)", &options)
            .unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 8)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 6)
        );
    }

    #[test]
    fn simple_indexed_root_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("root(3, x)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 4)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 3)
        );
    }

    #[test]
    fn simple_group_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("x(t)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 4)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 4)
        );
    }

    #[test]
    fn identifier_subscript_group_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("J_n(x)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 5)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 5)
        );
    }

    #[test]
    fn simple_delimiter_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("abs(x)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 3)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 3)
        );
    }

    #[test]
    fn simple_lr_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("lr(|x + y|)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 5)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 5)
        );
    }

    #[test]
    fn simple_operator_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("sin(x)", &options).unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| paths.items.len() == 6)
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 4)
        );
    }

    #[test]
    fn operator_identifier_script_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("lim_(x -> oo) f(x)", &options)
            .unwrap();

        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| !paths.items.is_empty())
        );
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() > 6)
        );
    }

    #[test]
    fn common_named_symbols_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("forall x in RR + A subset.eq B + arrow.r.double", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(
            artifact
                .paths
                .as_ref()
                .is_some_and(|paths| !paths.items.is_empty())
        );
        let glyph_text = artifact
            .pdf_text
            .as_ref()
            .map(|pdf| {
                pdf.glyph_runs
                    .iter()
                    .flat_map(|run| run.glyphs.iter())
                    .map(|glyph| glyph.unicode.as_str())
                    .collect::<String>()
            })
            .unwrap_or_default();
        assert!(glyph_text.contains('∀'));
        assert!(glyph_text.contains('∈'));
        assert!(glyph_text.contains('⊆'));
        assert!(glyph_text.contains('⇒'));
    }

    #[test]
    fn bold_math_fragment_uses_bundled_bold_math_font() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.style.font_weight = FontWeight::Bold;
        options.outputs = MathOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("R^2 + beta", &options).unwrap();

        assert_eq!(
            artifact.font_resources[0].postscript_name.as_deref(),
            Some("LeteSansMath-Bold")
        );
    }

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_math_fragment_raster_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: false,
            raster: Some(crate::raster::RasterRequest { scale: 1.5 }),
            pdf_text_layer: false,
        };

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(artifact.raster.is_some());
    }

    #[test]
    fn matrix_text_line_span_reports_source_offset() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_markup_line("before $mat(1, 2; 3, 4)$ after", &options)
            .unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 8,
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn plain_text_line_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Hello");
    }

    #[test]
    fn plain_text_line_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_markup_line("Hello", &options).unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.font_resources.len(), 1);
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| pdf.glyph_runs.len() == 1)
        );
    }

    #[test]
    fn mixed_text_math_metrics_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert_eq!(artifact.positioned_runs[0].text, "Price $7, score ");
        assert_eq!(artifact.positioned_runs[1].text, "R^2");
        assert_eq!(artifact.positioned_runs[2].text, " = 0.94");
    }

    #[test]
    fn mixed_text_math_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;
        options.outputs.positioned_runs = true;

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.paths.is_some());
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[1].paths.is_some());
    }

    #[test]
    fn mixed_text_math_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert_eq!(artifact.font_resources.len(), 2);
        assert!(
            artifact
                .pdf_text
                .as_ref()
                .is_some_and(|pdf| !pdf.glyph_runs.is_empty())
        );
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[0].pdf_text.is_none());
        assert!(artifact.positioned_runs[1].pdf_text.is_some());
        assert!(artifact.positioned_runs[2].pdf_text.is_none());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn mixed_text_math_raster_uses_typst_engine() {
        let engine = TypstEngineCore::new(&EngineOptions::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: Some(crate::raster::RasterRequest { scale: 1.5 }),
            pdf_text_layer: false,
            positioned_runs: false,
        };

        let artifact = engine
            .typeset_markup_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.paths.is_none());
        assert!(
            artifact
                .raster
                .as_ref()
                .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0)
        );
    }
}
