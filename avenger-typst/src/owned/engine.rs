#[cfg(feature = "vendor-typst")]
use std::sync::{Arc, Mutex};

use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::paths::MathPathArtifact;
use crate::pdf::MathPdfTextLayer;
use crate::types::{
    MathFragmentOptions, MathRunArtifact, TextLineArtifact, TextLineOptions, TypesetMetrics,
};

#[cfg(feature = "vendor-typst")]
use crate::engine::typst::TypstMathEngine;
use crate::owned::ast::{OwnedLine, OwnedLineNode};
use crate::owned::inline::try_typeset_owned_text_line;
use crate::owned::math::metrics::try_typeset_simple_row_fragment;
use crate::owned::math::syntax::parse_owned_math;
use crate::owned::syntax::parse_owned_line;

#[derive(Clone)]
pub(crate) struct OwnedTypstEngine {
    config: TypstEngineConfig,
    #[cfg(feature = "vendor-typst")]
    delegate: Arc<Mutex<Option<TypstMathEngine>>>,
}

impl std::fmt::Debug for OwnedTypstEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedTypstEngine")
            .field("phase", &"owned-empty-text-line-delegating-rest")
            .field("delegate_initialized", &self.delegate_initialized())
            .finish_non_exhaustive()
    }
}

impl OwnedTypstEngine {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self {
            config: config.clone(),
            #[cfg(feature = "vendor-typst")]
            delegate: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        let math = parse_owned_math(source, 0)?;
        if let Some(artifact) = try_typeset_simple_row_fragment(&math, options, &self.config)? {
            return Ok(artifact);
        }
        self.with_delegate_or_unsupported_fragment(source, options)
    }

    pub(crate) fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        if source.is_empty() && options.outputs.raster.is_none() {
            return Ok(empty_text_line_artifact(source, options));
        }

        let line = parse_owned_line(source, &options.delimiters)?;
        validate_owned_line_math(&line)?;
        if let Some(artifact) = try_typeset_owned_text_line(source, &line, options, &self.config)? {
            return Ok(artifact);
        }
        if line_contains_static_markup(&line) {
            return Err(MathTypesetError::UnsupportedOutput(
                "owned static text markup is parsed but not rendered yet",
            ));
        }

        self.with_delegate_or_unsupported_text_line(source, options)
    }

    #[cfg(feature = "vendor-typst")]
    fn with_delegate_or_unsupported_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        self.with_delegate(|delegate| delegate.typeset_fragment(source, options))
    }

    #[cfg(not(feature = "vendor-typst"))]
    fn with_delegate_or_unsupported_fragment(
        &self,
        _source: &str,
        _options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        Err(MathTypesetError::UnsupportedOutput(
            "owned Typst backend does not support this math subset yet",
        ))
    }

    #[cfg(feature = "vendor-typst")]
    fn with_delegate_or_unsupported_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        self.with_delegate(|delegate| delegate.typeset_text_line(source, options))
    }

    #[cfg(not(feature = "vendor-typst"))]
    fn with_delegate_or_unsupported_text_line(
        &self,
        _source: &str,
        _options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        Err(MathTypesetError::UnsupportedOutput(
            "owned Typst backend does not support this text-line subset yet",
        ))
    }

    #[cfg(feature = "vendor-typst")]
    fn with_delegate<T>(
        &self,
        run: impl FnOnce(&TypstMathEngine) -> Result<T, MathTypesetError>,
    ) -> Result<T, MathTypesetError> {
        let mut delegate = self.delegate.lock().map_err(|_| MathTypesetError::Engine {
            start: 0,
            end: 0,
            message: "owned Typst delegate lock was poisoned".to_string(),
        })?;

        if delegate.is_none() {
            *delegate = Some(TypstMathEngine::new(&self.config).map_err(|err| {
                MathTypesetError::Engine {
                    start: 0,
                    end: 0,
                    message: err.to_string(),
                }
            })?);
        }

        run(delegate
            .as_ref()
            .expect("owned Typst delegate should be initialized"))
    }

    fn delegate_initialized(&self) -> bool {
        #[cfg(feature = "vendor-typst")]
        {
            self.delegate
                .lock()
                .is_ok_and(|delegate| delegate.is_some())
        }
        #[cfg(not(feature = "vendor-typst"))]
        {
            false
        }
    }
}

fn line_contains_static_markup(line: &OwnedLine) -> bool {
    nodes_contain_static_markup(&line.nodes)
}

fn validate_owned_line_math(line: &OwnedLine) -> Result<(), MathTypesetError> {
    for node in &line.nodes {
        if let OwnedLineNode::Math(math) = node {
            parse_owned_math(&math.source, math.source_range.start)?;
        }
    }
    Ok(())
}

fn nodes_contain_static_markup(nodes: &[OwnedLineNode]) -> bool {
    nodes.iter().any(|node| match node {
        OwnedLineNode::Plain(_) | OwnedLineNode::Math(_) | OwnedLineNode::Emoji(_) => false,
        OwnedLineNode::TextSpan(_) => true,
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
    fn empty_text_line_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        assert!(!engine.delegate_initialized());

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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_paths_use_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_text_line("Hello", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact
            .paths
            .as_ref()
            .is_some_and(|paths| paths.items.len() == 5));
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_with_non_rtl_missing_glyph_paths_uses_owned_fast_path() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;

        let artifact = engine.typeset_text_line("Revenue 🚀", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(artifact.paths.is_some());
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn non_atkinson_plain_text_can_use_owned_fontdb_fallback_without_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn non_atkinson_mixed_script_text_can_segment_fallback_fonts_without_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn default_mixed_script_text_can_segment_fallback_fonts_without_delegate_when_available() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_with_rtl_text_uses_owned_fallback_without_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_with_zwj_emoji_uses_owned_missing_glyph_path_without_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn named_emoji_alias_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert_eq!(artifact.positioned_runs.len(), 1);
        assert_eq!(artifact.positioned_runs[0].text, "Revenue 🚀");
        assert!(artifact.paths.is_some());
        assert!(artifact.pdf_text.is_some());
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn unknown_static_command_errors_before_delegate_initialization() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn static_command_options_error_before_delegate_initialization() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn supported_static_decoration_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn static_subscript_and_superscript_use_owned_fast_path_without_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn matrix_math_fragment_errors_before_delegate_initialization() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();

        let err = engine
            .typeset_fragment("mat(1, 2; 3, 4)", &MathFragmentOptions::default())
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "matrix/table math is not supported in owned Typst subset"
            }
        );
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_row_math_fragment_metrics_only_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_row_math_fragment_paths_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_row_math_fragment_pdf_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_script_math_fragment_pdf_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
            .expect("owned script path should emit PDF glyph metadata");
        assert!(pdf.glyph_runs.iter().any(|run| run.font_size < 12.0));
        assert_eq!(artifact.font_resources.len(), 1);
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_fraction_math_fragment_paths_use_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_frac_call_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_binom_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_cancel_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_sqrt_fraction_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_indexed_root_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_group_math_fragment_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn identifier_subscript_group_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_delimiter_call_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn simple_operator_call_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn operator_identifier_script_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn common_named_symbols_use_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_math_fragment_raster_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn matrix_text_line_span_reports_source_offset_before_delegate_initialization() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let err = engine
            .typeset_text_line("before $mat(1, 2; 3, 4)$ after", &options)
            .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 8,
                message: "matrix/table math is not supported in owned Typst subset"
            }
        );
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_metrics_only_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn plain_text_line_pdf_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn mixed_text_math_metrics_use_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn mixed_text_math_paths_use_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = true;
        options.outputs.positioned_runs = true;

        let artifact = engine
            .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
            .unwrap();

        assert!(artifact.paths.is_some());
        assert_eq!(artifact.positioned_runs.len(), 3);
        assert!(artifact.positioned_runs[1].paths.is_some());
        assert!(!engine.delegate_initialized());
    }

    #[test]
    fn mixed_text_math_pdf_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }

    #[cfg(feature = "raster")]
    #[test]
    fn mixed_text_math_raster_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
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
        assert!(!engine.delegate_initialized());
    }
}
