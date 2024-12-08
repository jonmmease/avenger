use thiserror::Error;

#[cfg(target_arch = "wasm32")]
use web_sys::{js_sys::Object, wasm_bindgen::JsValue};

#[derive(Error, Debug)]
pub enum AvengerTextError {
    #[error("Internal error: `{0}`")]
    TextMeasurementError(String),

    #[error("Failed to allocate image")]
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
