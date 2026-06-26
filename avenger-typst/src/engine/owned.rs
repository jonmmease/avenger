use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::types::{MathFragmentOptions, MathRunArtifact, TextLineArtifact, TextLineOptions};

use super::typst::TypstMathEngine;

#[derive(Clone)]
pub(crate) struct OwnedTypstEngine {
    delegate: TypstMathEngine,
}

impl std::fmt::Debug for OwnedTypstEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedTypstEngine")
            .field("phase", &"delegating-to-vendor")
            .finish_non_exhaustive()
    }
}

impl OwnedTypstEngine {
    pub(crate) fn new(config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self {
            delegate: TypstMathEngine::new(config)?,
        })
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        self.delegate.typeset_fragment(source, options)
    }

    pub(crate) fn typeset_text_line(
        &self,
        source: &str,
        options: &TextLineOptions,
    ) -> Result<TextLineArtifact, MathTypesetError> {
        self.delegate.typeset_text_line(source, options)
    }
}
