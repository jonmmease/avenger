use std::sync::{Arc, Mutex};

use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::paths::MathPathArtifact;
use crate::pdf::MathPdfTextLayer;
use crate::types::{
    MathFragmentOptions, MathRunArtifact, TextLineArtifact, TextLineOptions, TypesetMetrics,
};

use crate::engine::typst::TypstMathEngine;

#[derive(Clone)]
pub(crate) struct OwnedTypstEngine {
    config: TypstEngineConfig,
    delegate: Arc<Mutex<Option<TypstMathEngine>>>,
}

impl std::fmt::Debug for OwnedTypstEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedTypstEngine")
            .field("phase", &"owned-empty-text-line-delegating-rest")
            .field(
                "delegate_initialized",
                &self
                    .delegate
                    .lock()
                    .is_ok_and(|delegate| delegate.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl OwnedTypstEngine {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self {
            config: config.clone(),
            delegate: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        self.with_delegate(|delegate| delegate.typeset_fragment(source, options))
    }

    pub(crate) fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        if source.is_empty() && options.outputs.raster.is_none() {
            return Ok(empty_text_line_artifact(source, options));
        }

        self.with_delegate(|delegate| delegate.typeset_text_line(source, options))
    }

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
    use crate::types::TextLineOutputRequest;

    #[test]
    fn empty_text_line_uses_owned_fast_path_without_initializing_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        assert!(!engine.delegate.lock().unwrap().is_some());

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
        assert!(!engine.delegate.lock().unwrap().is_some());
    }

    #[test]
    fn non_empty_text_line_initializes_delegate() {
        let engine = OwnedTypstEngine::new(&TypstEngineConfig::default()).unwrap();
        let mut options = TextLineOptions::default();
        options.outputs.paths = false;

        let artifact = engine.typeset_text_line("x", &options).unwrap();

        assert!(artifact.metrics.width > 0.0);
        assert!(engine.delegate.lock().unwrap().is_some());
    }
}
