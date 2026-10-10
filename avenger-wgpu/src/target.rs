//! Borrowed texture attachments for rendering a scene.

use wgpu::{Extent3d, LoadOp, RenderPass, RenderPassColorAttachment, TextureFormat, TextureView};

/// A render attachment whose format and sample count match the renderer.
#[derive(Clone, Copy)]
pub struct AvengerRenderTarget<'a> {
    pub view: &'a TextureView,
    pub resolve_target: Option<&'a TextureView>,
    pub extent: Extent3d,
    pub format: TextureFormat,
    pub sample_count: u32,
    pub load: LoadOp<wgpu::Color>,
    /// The attachment pixel where the scene's top-left corner draws, when the scene fills a
    /// rectangle of the attachment the renderer's size instead of the whole attachment. A clear
    /// would erase the whole attachment, so a target with an origin loads its contents.
    pub viewport_origin: Option<[u32; 2]>,
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
            viewport_origin: None,
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
            viewport_origin: None,
        }
    }

    pub fn with_load(mut self, load: LoadOp<wgpu::Color>) -> Self {
        self.load = load;
        self
    }

    /// Draw the scene into the rectangle of the attachment at `origin`, the renderer's size.
    pub fn with_viewport_origin(mut self, origin: [u32; 2]) -> Self {
        self.viewport_origin = Some(origin);
        self
    }
}

/// Clear to opaque white, the canvases' background.
pub const WHITE_CLEAR: LoadOp<wgpu::Color> = LoadOp::Clear(wgpu::Color::WHITE);

/// Clear to transparent. Marks drawn onto a transparent clear leave each pixel's color multiplied
/// by its alpha (premultiplied alpha), so a host composites the result with `One,
/// OneMinusSrcAlpha` blending.
pub const TRANSPARENT_CLEAR: LoadOp<wgpu::Color> = LoadOp::Clear(wgpu::Color::TRANSPARENT);

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

/// What a render pass draws into: the attachment, and the rectangle the scene fills.
#[derive(Clone, Copy)]
pub(crate) struct PassTarget<'a> {
    pub view: &'a TextureView,
    pub resolve_target: Option<&'a TextureView>,
    pub extent: Extent3d,
    /// The scene's rectangle `[x, y, width, height]` in physical pixels, when it fills less than
    /// the attachment.
    pub viewport: Option<[u32; 4]>,
}

impl PassTarget<'_> {
    /// The color attachment, keeping what the attachment already holds.
    pub fn load_attachment(&self) -> RenderPassColorAttachment<'_> {
        RenderPassColorAttachment {
            view: self.view,
            resolve_target: self.resolve_target,
            depth_slice: None,
            ops: wgpu::Operations {
                load: LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        }
    }

    /// Restrict a pass to the scene's rectangle.
    pub fn set_viewport(&self, pass: &mut RenderPass<'_>) {
        if let Some([x, y, width, height]) = self.viewport {
            pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
        }
    }

    /// The scissor rectangle `[x, y, width, height]`, in attachment pixels, that a clip rectangle
    /// in the scene's logical coordinates covers.
    pub fn clip_scissor(&self, x: f32, y: f32, width: f32, height: f32, scale: f32) -> [u32; 4] {
        let [origin_x, origin_y, _, _] = self.viewport.unwrap_or_default();
        let axis = |origin: u32, start: f32, length: f32, limit: u32| {
            let (start, length) = (origin as f32 + start * scale, length * scale);
            // A clip that starts before the attachment keeps only the part inside it.
            let (start, length) = if start < 0.0 {
                (0.0, (start + length).max(0.0))
            } else {
                (start, length)
            };
            let start = (start as u32).min(limit);
            (start, (length as u32).min(limit - start))
        };
        let (x, width) = axis(origin_x, x, width, self.extent.width);
        let (y, height) = axis(origin_y, y, height, self.extent.height);
        [x, y, width, height]
    }

    /// The scissor rectangle that covers the whole attachment.
    pub fn full_scissor(&self) -> [u32; 4] {
        [0, 0, self.extent.width, self.extent.height]
    }
}
