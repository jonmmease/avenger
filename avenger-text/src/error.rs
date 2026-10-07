use thiserror::Error;

#[cfg(target_arch = "wasm32")]
use web_sys::{js_sys::Object, wasm_bindgen::JsValue};

#[derive(Error, Debug)]
pub enum AvengerTextError {
    #[error("Typst text typesetting failed: {0}")]
    Typesetting(#[from] avenger_typst_label::LabelError),

    #[error("Typst text rasterization failed: {0}")]
    Rasterization(#[from] avenger_typst_label::RasterError),

    #[cfg(target_arch = "wasm32")]
    #[error("Failed to convert to JS value")]
    JsError(JsValue),

    #[cfg(target_arch = "wasm32")]
    #[error("Failed to convert to JS object")]
    JsObjectError(Object),
}

#[cfg(target_arch = "wasm32")]
impl From<JsValue> for AvengerTextError {
    fn from(value: JsValue) -> Self {
        AvengerTextError::JsError(value)
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Object> for AvengerTextError {
    fn from(value: Object) -> Self {
        AvengerTextError::JsObjectError(value)
    }
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
