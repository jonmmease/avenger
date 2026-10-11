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

/// A surface frame's attachment, cleared to white. With more than one sample, `multisampled`
/// (the size the surface is configured to) resolves into `view`. Otherwise the frame draws into
/// `view` at the size the surface handed out.
pub(crate) fn surface_frame_target<'a>(
    view: &'a TextureView,
    view_extent: Extent3d,
    multisampled: &'a TextureView,
    configured_size: [u32; 2],
    format: TextureFormat,
    sample_count: u32,
) -> AvengerRenderTarget<'a> {
    if sample_count > 1 {
        let extent = Extent3d {
            width: configured_size[0],
            height: configured_size[1],
            depth_or_array_layers: 1,
        };
        AvengerRenderTarget::multisampled(
            multisampled,
            view,
            extent,
            format,
            sample_count,
            WHITE_CLEAR,
        )
    } else {
        AvengerRenderTarget::new(view, view_extent, format, 1, WHITE_CLEAR)
    }
}

/// The scissor rectangle `[x, y, width, height]`, in physical pixels within `extent`, that a clip
/// rectangle in logical coordinates covers.
pub(crate) fn clip_scissor(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: f32,
    extent: Extent3d,
) -> [u32; 4] {
    let axis = |start: f32, length: f32, limit: u32| {
        let (start, length) = (start * scale, length * scale);
        // A clip that starts before the target keeps only the part inside it.
        let (start, length) = if start < 0.0 {
            (0.0, (start + length).max(0.0))
        } else {
            (start, length)
        };
        let start = (start as u32).min(limit);
        (start, (length as u32).min(limit - start))
    };
    let (x, width) = axis(x, width, extent.width);
    let (y, height) = axis(y, height, extent.height);
    [x, y, width, height]
}
