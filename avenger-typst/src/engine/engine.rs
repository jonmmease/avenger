use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::paths::MathPathArtifact;
use crate::pdf::MathPdfTextLayer;
use crate::types::{
    MathFragmentOptions, MathRunArtifact, MathSyntaxMode, TextLineArtifact, TextLineOptions,
    TypesetMetrics,
};

use crate::engine::ast::{LineNode, ParsedLine, PlainTextNode};
use crate::engine::inline::try_typeset_text_line;
use crate::engine::math::metrics::try_typeset_simple_row_fragment;
use crate::engine::math::syntax::parse_math;
use crate::engine::syntax::parse_line;

#[derive(Clone)]
pub(crate) struct TypstEngineCore {
    config: TypstEngineConfig,
}

impl std::fmt::Debug for TypstEngineCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypstEngineCore").finish_non_exhaustive()
    }
}

impl TypstEngineCore {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self {
            config: config.clone(),
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        let math = parse_math(source, 0)?;
        if let Some(artifact) = try_typeset_simple_row_fragment(&math, options, &self.config)? {
            return Ok(artifact);
        }
        unsupported_fragment()
    }

    pub(crate) fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        if source.is_empty() && options.outputs.raster.is_none() {
            return Ok(empty_text_line_artifact(source, options));
        }

        let line = match options.syntax {
            MathSyntaxMode::TypstFragmentStrict => {
                let line = parse_line(source, &options.delimiters)?;
                validate_line_math(&line)?;
                line
            }
            MathSyntaxMode::PlainText => plain_text_line(source),
        };
        if let Some(artifact) = try_typeset_text_line(source, &line, options, &self.config)? {
            return Ok(artifact);
        }
        if line_contains_static_markup(&line) {
            return Err(MathTypesetError::UnsupportedOutput(
                "static text markup is parsed but not rendered yet",
            ));
        }

        unsupported_text_line()
    }
}

fn unsupported_fragment() -> Result<MathRunArtifact, MathTypesetError> {
    Err(MathTypesetError::UnsupportedOutput(
        "this Typst math subset is not supported yet",
    ))
}

fn unsupported_text_line() -> Result<TextLineArtifact, MathTypesetError> {
    Err(MathTypesetError::UnsupportedOutput(
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

fn validate_line_math(line: &ParsedLine) -> Result<(), MathTypesetError> {
    for node in &line.nodes {
        if let LineNode::Math(math) = node {
            parse_math(&math.source, math.source_range.start)?;
        }
    }
    Ok(())
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
    let paths = options.outputs.paths.then(|| MathPathArtifact {
        logical_width: 0.0,
        logical_height: 0.0,
        items: Vec::new(),
        images: Vec::new(),
    });
    let pdf_text = options.outputs.pdf_text_layer.then(|| MathPdfTextLayer {
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
    use crate::types::{MathOutputRequest, TextLineOutputRequest};

    #[test]
    fn empty_text_line_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();

        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };
        let artifact = engine.typeset_text_line("", &options).unwrap();

        assert_eq!(artifact.metrics.width, 0.0);
        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.is_empty()));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf_text| pdf_text.glyph_runs.is_empty()));
        assert!(artifact.positioned_runs.is_empty());
    }

    #[test]
    fn plain_text_line_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_text_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
    }

    #[test]
    fn plain_text_line_with_non_rtl_missing_glyph_paths_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_text_line("Revenue 🚀", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
    }

    #[test]
    fn non_atkinson_plain_text_can_use_fontdb_fallback() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.text_style.font_family = "serif".to_string();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let Ok(artifact) = engine.typeset_text_line("Fallback font", &options) else {
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.text_style.font_family = "serif".to_string();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let Ok(artifact) = engine.typeset_text_line("Hello 温度", &options) else {
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_text_line("Hello 温度", &options).unwrap();
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_text_line("שלום", &options).unwrap();

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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.syntax = MathSyntaxMode::PlainText;

        let artifact = engine
            .typeset_text_line("before $x^$ after", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
    }

    #[test]
    fn plain_text_line_with_zwj_emoji_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_text_line("Family 👨‍👩‍👧‍👦", &options).unwrap();

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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_text_line("Revenue #emoji.rocket", &options)
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: Some(crate::RasterRequest { scale: 2.0 }),
            pdf_text_layer: false,
            positioned_runs: false,
        };

        let artifact = engine
            .typeset_text_line("Revenue #emoji.rocket", &options)
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_text_line("#let x = 1", &options)
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        );
    }

    #[test]
    fn static_command_options_error_before_rendering() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_text_line("#underline(stroke: red)[important]", &options)
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "static text commands do not support Typst-style options"
            }
        );
    }

    #[test]
    fn supported_static_decoration_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_text_line("#underline[important]", &options)
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "important");
        assert!(artifact.positioned_runs[0].paths.is_some());
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn static_subscript_and_superscript_use_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_text_line("H#sub[2]O #super[*]", &options)
            .unwrap();

        assert_eq!(artifact.positioned_runs.len(), 4);
        assert_eq!(artifact.positioned_runs[0].text, "H");
        assert_eq!(artifact.positioned_runs[1].text, "2");
        assert_eq!(artifact.positioned_runs[2].text, "O ");
        assert_eq!(artifact.positioned_runs[3].text, "*");
        assert!(artifact.positioned_runs[1].y > artifact.positioned_runs[0].y);
        assert!(artifact.positioned_runs[3].y < artifact.positioned_runs[0].y);
        assert!(artifact.positioned_runs[1]
            .text_style
            .as_ref()
            .is_some_and(|style| style.font_size < options.text_style.font_size));
        assert!(artifact.positioned_runs[3]
            .text_style
            .as_ref()
            .is_some_and(|style| style.font_size < options.text_style.font_size));
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
    }

    #[test]
    fn matrix_math_fragment_errors_before_rendering() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();

        let err = engine
            .typeset_fragment("mat(1, 2; 3, 4)", &MathFragmentOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn simple_row_math_fragment_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        };

        let artifact = engine
            .typeset_fragment("alpha + beta -> gamma", &options)
            .unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| !paths.items.is_empty()));
    }

    #[test]
    fn simple_row_math_fragment_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("a / (b + c)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 4));
    }

    #[test]
    fn simple_frac_call_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("frac(x + y, z)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 4));
    }

    #[test]
    fn simple_binom_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("binom(n, k)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 4));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 4));
    }

    #[test]
    fn simple_cancel_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("cancel(x)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 2));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 1));
    }

    #[test]
    fn simple_sqrt_fraction_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("sqrt(x) / (1 + x^2)", &options)
            .unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 8));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 6));
    }

    #[test]
    fn simple_indexed_root_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("root(3, x)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 4));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 3));
    }

    #[test]
    fn simple_group_math_fragment_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("x(t)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 4));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 4));
    }

    #[test]
    fn identifier_subscript_group_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("J_n(x)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 5));
    }

    #[test]
    fn simple_delimiter_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("abs(x)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 3));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 3));
    }

    #[test]
    fn simple_lr_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("lr(|x + y|)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 5));
    }

    #[test]
    fn simple_operator_call_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine.typeset_fragment("sin(x)", &options).unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 6));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 4));
    }

    #[test]
    fn operator_identifier_script_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = engine
            .typeset_fragment("lim_(x -> oo) f(x)", &options)
            .unwrap();

        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| !paths.items.is_empty()));
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() > 6));
    }

    #[test]
    fn common_named_symbols_use_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| !paths.items.is_empty()));
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

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_math_fragment_raster_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
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
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_text_line("before $mat(1, 2; 3, 4)$ after", &options)
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 8,
                message: "matrix/table math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn plain_text_line_metrics_only_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = engine.typeset_text_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Hello");
    }

    #[test]
    fn plain_text_line_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine.typeset_text_line("Hello", &options).unwrap();

        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.font_resources.len(), 1);
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| pdf.glyph_runs.len() == 1));
    }

    #[test]
    fn mixed_text_math_metrics_use_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert_eq!(artifact.positioned_runs[0].text, "Price $7, score ");
        assert_eq!(artifact.positioned_runs[1].text, "R^2");
        assert_eq!(artifact.positioned_runs[2].text, " = 0.94");
    }

    #[test]
    fn mixed_text_math_paths_use_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;
        options.outputs.positioned_runs = true;

        let artifact = engine
            .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.paths.is_some());
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[1].paths.is_some());
    }

    #[test]
    fn mixed_text_math_pdf_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
            positioned_runs: true,
        };

        let artifact = engine
            .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert_eq!(artifact.font_resources.len(), 2);
        assert!(artifact
            .pdf_text
            .as_ref()
            .is_some_and(|pdf| !pdf.glyph_runs.is_empty()));
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[0].pdf_text.is_none());
        assert!(artifact.positioned_runs[1].pdf_text.is_some());
        assert!(artifact.positioned_runs[2].pdf_text.is_none());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn mixed_text_math_raster_uses_typst_engine() {
        let engine = TypstEngineCore::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs = TextLineOutputRequest {
            paths: false,
            raster: Some(crate::raster::RasterRequest { scale: 1.5 }),
            pdf_text_layer: false,
            positioned_runs: false,
        };

        let artifact = engine
            .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.paths.is_none());
        assert!(artifact
            .raster
            .as_ref()
            .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0));
    }
}
