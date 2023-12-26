use thiserror::Error;

#[derive(Error, Debug)]
pub enum VegaWgpuError {
    #[error("css color parse error")]
    InvalidColor(#[from] csscolorparser::ParseColorError),

    #[error("Device request failed")]
    RequestDeviceError(#[from] wgpu::RequestDeviceError),

    #[error("Failed to create surface")]
    CreateSurfaceError(#[from] wgpu::CreateSurfaceError),

    #[error("WGPU adapter creation failed")]
    MakeWgpuAdapterError,
}
