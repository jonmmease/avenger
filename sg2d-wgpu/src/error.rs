use lyon::tessellation::TessellationError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Sg2dWgpuError {
    #[error("SceneGraph error")]
    SceneGraphError(#[from] sg2d::error::SceneGraphError),

    #[error("Device request failed")]
    RequestDeviceError(#[from] wgpu::RequestDeviceError),

    #[error("Failed to create surface")]
    CreateSurfaceError(#[from] wgpu::CreateSurfaceError),

    #[error("WGPU adapter creation failed")]
    MakeWgpuAdapterError,

    #[error("lyon tessellation error")]
    TessellationError(#[from] TessellationError),
}
