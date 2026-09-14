//! Borrowed texture attachments for rendering a scene.

use wgpu::{Extent3d, LoadOp, TextureFormat, TextureView};

/// A render attachment whose format and sample count match the renderer.
#[derive(Clone, Copy)]
pub struct AvengerRenderTarget<'a> {
    pub view: &'a TextureView,
    pub resolve_target: Option<&'a TextureView>,
    pub extent: Extent3d,
    pub format: TextureFormat,
    pub sample_count: u32,
    pub load: LoadOp<wgpu::Color>,
}

impl<'a> AvengerRenderTarget<'a> {
    /// Describe a render attachment without an automatic resolve.
    pub fn new(
        view: &'a TextureView,
        extent: Extent3d,
        format: TextureFormat,
        sample_count: u32,
        load: LoadOp<wgpu::Color>,
    ) -> Self {
        Self {
            view,
            resolve_target: None,
            extent,
            format,
            sample_count,
            load,
        }
    }

    /// Resolve a multisampled attachment into a single-sample destination.
    pub fn multisampled(
        view: &'a TextureView,
        resolve_target: &'a TextureView,
        extent: Extent3d,
        format: TextureFormat,
        sample_count: u32,
        load: LoadOp<wgpu::Color>,
    ) -> Self {
        Self {
            view,
            resolve_target: Some(resolve_target),
            extent,
            format,
            sample_count,
            load,
        }
    }

    pub fn with_load(mut self, load: LoadOp<wgpu::Color>) -> Self {
        self.load = load;
        self
    }
}

pub const WHITE_CLEAR: LoadOp<wgpu::Color> = LoadOp::Clear(wgpu::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
});
