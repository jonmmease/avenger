//! Render target abstractions for the GUI offscreen refactor.

use wgpu::{Extent3d, LoadOp, TextureFormat, TextureView};

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

    pub fn swapchain(
        view: &'a TextureView,
        extent: Extent3d,
        format: TextureFormat,
        load: LoadOp<wgpu::Color>,
    ) -> Self {
        Self::new(view, extent, format, 1, load)
    }

    pub fn offscreen(
        view: &'a TextureView,
        extent: Extent3d,
        format: TextureFormat,
        load: LoadOp<wgpu::Color>,
    ) -> Self {
        Self::new(view, extent, format, 1, load)
    }

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
