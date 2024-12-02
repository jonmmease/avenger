use lyon::tessellation::TessellationError;
use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyValueError, PyErr};

#[cfg(target_arch = "wasm32")]
use web_sys::{js_sys::Object, wasm_bindgen::JsValue};

#[derive(Error, Debug)]
pub enum AvengerWgpuError {
    #[error("Avenger error")]
    AvengerError(#[from] avenger_scenegraph::error::AvengerError),

    #[error("Device request failed")]
    RequestDeviceError(#[from] wgpu::RequestDeviceError),

    #[error("Failed to create surface")]
    CreateSurfaceError(#[from] wgpu::CreateSurfaceError),

    #[error("Failed to create surface")]
    SurfaceError(#[from] wgpu::SurfaceError),

    #[error("WGPU adapter creation failed")]
    MakeWgpuAdapterError,

    #[error("lyon tessellation error")]
    TessellationError(#[from] TessellationError),

    #[error("Image allocation error: {0}")]
    ImageAllocationError(String),

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Text support is not enabled: {0}")]
    TextNotEnabled(String),

    #[error("Text error: {0}")]
    TextError(String),

    #[cfg(target_arch = "wasm32")]
    #[error("JavaScript error")]
    JsError(JsValue),

    #[cfg(target_arch = "wasm32")]
    #[error("JavaScript error")]
    JsObjectError(Object),
}

// Conversion to PyO3 error
#[cfg(feature = "pyo3")]
impl From<AvengerWgpuError> for PyErr {
    fn from(err: AvengerWgpuError) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
impl From<JsValue> for AvengerWgpuError {
    fn from(value: JsValue) -> Self {
        AvengerWgpuError::JsError(value)
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Object> for AvengerWgpuError {
    fn from(value: Object) -> Self {
        AvengerWgpuError::JsObjectError(value)
    }
}
