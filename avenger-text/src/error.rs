use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerTextError {
    #[error("Typst text typesetting failed: {0}")]
    Typesetting(#[from] avenger_typst_label::LabelError),

    #[error("Typst text rasterization failed: {0}")]
    Rasterization(#[from] avenger_typst_label::RasterError),
}

impl AvengerTextError {
    /// Whether the label's source is invalid, so that it can show as plain text instead.
    /// Limit and font errors don't fall back.
    pub(crate) fn allows_plain_fallback(&self) -> bool {
        matches!(
            self,
            Self::Typesetting(avenger_typst_label::LabelError::Source { .. })
        )
    }
}
