use lyon::tessellation::TessellationError;
use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyValueError, PyErr};

#[derive(Error, Debug)]
pub enum AvengerWgpuError {
    #[error("Avenger error")]
    AvengerError(#[from] avenger::error::AvengerError),

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

    #[error("Text support is not enabled: {0}")]
    TextNotEnabled(String),
}

// Conversion to PyO3 error
#[cfg(feature = "pyo3")]
impl From<AvengerWgpuError> for PyErr {
    fn from(err: AvengerWgpuError) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}
