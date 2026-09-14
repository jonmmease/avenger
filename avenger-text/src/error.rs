use thiserror::Error;

#[cfg(target_arch = "wasm32")]
use web_sys::{js_sys::Object, wasm_bindgen::JsValue};

#[derive(Error, Debug)]
pub enum AvengerTextError {
    #[error("Typst text typesetting failed: {0}")]
    Typesetting(#[from] avenger_typst_label::LabelError),

    #[error("Failed to allocate image: {0}")]
    ImageAllocationError(String),

    #[error("Internal error: `{0}`")]
    InternalError(String),

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
    pub(crate) fn allows_plain_fallback(&self) -> bool {
        use avenger_typst_label::LabelError;
        matches!(
            self,
            Self::Typesetting(
                LabelError::Syntax { .. }
                    | LabelError::UnsupportedSyntax { .. }
                    | LabelError::UnsupportedFeature { .. }
                    | LabelError::UnsupportedOutput(_)
                    | LabelError::EmptyMathFragment { .. }
                    | LabelError::Engine { .. }
            )
        )
    }
}
