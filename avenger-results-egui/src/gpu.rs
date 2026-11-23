//! Avenger scenes drawn into egui textures on eframe's wgpu device, as in `egui-embed`.

use std::sync::Arc;

use avenger_common::canvas::CanvasDimensions;
use avenger_image::ImageResolver;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_wgpu::{
    context::AvengerGpuContext,
    offscreen::{OffscreenTarget, OffscreenTargetDescriptor},
    renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
    wgpu,
};
use eframe::{egui, egui_wgpu::RenderState};

/// egui shows textures in `Rgba8Unorm`, which needs no sRGB conversion.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A texture that egui shows. Scenes draw into its top-left corner at `dimensions`, a size in
/// logical pixels and a scale, the physical pixels of a logical pixel. The allocation can be
/// larger, so that a size that changes every frame, as while the user resizes, draws again
/// without a new allocation each time. Neither is larger than the device's largest texture: a
/// size that would be draws at a lower scale.
pub struct Texture {
    target: OffscreenTarget,
    pub id: egui::TextureId,
    /// The dimensions that the texture's owner asked for.
    pub requested: CanvasDimensions,
    /// The dimensions that the scenes draw at: the requested size, at a scale that fits the
    /// device's largest texture.
    pub dimensions: CanvasDimensions,
}

impl Texture {
    pub fn new(state: &RenderState, requested: CanvasDimensions) -> Self {
        let dimensions = fit(requested, limit(state));
        let target = target(&state.device, physical(dimensions), dimensions.scale);
        let id = state.renderer.write().register_native_texture(
            &state.device,
            &target.view,
            wgpu::FilterMode::Linear,
        );
        Self {
            target,
            id,
            requested,
            dimensions,
        }
    }

    /// Gives the texture `requested`, which the next draw fills. The allocation grows by a
    /// quarter more than it needs when it is too small, up to the device's largest texture, and
    /// shrinks when it is more than twice as large as it needs in both directions. Returns
    /// whether the dimensions changed.
    pub fn resize(&mut self, state: &RenderState, requested: CanvasDimensions) -> bool {
        if same(requested, self.requested) {
            return false;
        }
        let limit = limit(state);
        let dimensions = fit(requested, limit);
        self.requested = requested;
        self.dimensions = dimensions;
        let needed = physical(dimensions);
        let allocated = [self.target.extent.width, self.target.extent.height];
        let fits = needed[0] <= allocated[0] && needed[1] <= allocated[1];
        let wasteful = 2 * needed[0] < allocated[0] && 2 * needed[1] < allocated[1];
        if fits && !wasteful && dimensions.scale == self.target.dimensions.scale {
            return true;
        }
        let grown = needed.map(|length| (length + length / 4).min(limit));
        self.target = target(&state.device, grown, dimensions.scale);
        state
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &state.device,
                &self.target.view,
                wgpu::FilterMode::Linear,
                self.id,
            );
        true
    }

    /// The part of the texture that the scenes fill, in texture coordinates.
    pub fn uv(&self) -> egui::Rect {
        let [width, height] = physical(self.dimensions);
        let extent = self.target.extent;
        egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(
                width as f32 / extent.width as f32,
                height as f32 / extent.height as f32,
            ),
        )
    }

    /// Releases the texture from egui.
    pub fn free(self, state: &RenderState) {
        state.renderer.write().free_texture(&self.id);
    }
}

/// The physical pixels of `dimensions`, as the renderer counts them.
fn physical(dimensions: CanvasDimensions) -> [u32; 2] {
    [
        dimensions.to_physical_width().max(1),
        dimensions.to_physical_height().max(1),
    ]
}

/// A target of `pixels` physical pixels at `scale`.
fn target(device: &wgpu::Device, pixels: [u32; 2], scale: f32) -> OffscreenTarget {
    // The size in logical pixels lands half a pixel inside each physical length, so that the
    // target's truncation to physical pixels gives `pixels`.
    let size = pixels.map(|length| (length as f32 + 0.5) / scale);
    let dimensions = CanvasDimensions { size, scale };
    OffscreenTarget::new(device, &OffscreenTargetDescriptor::new(dimensions, FORMAT))
}

/// Renderers that draw scenes into a texture, one renderer for each scene, in order, so that each
/// scene draws over the ones before it.
pub struct Painter {
    context: AvengerGpuContext,
    images: Option<Arc<dyn ImageResolver>>,
    renderers: Vec<AvengerWgpuRenderer>,
}

impl Painter {
    pub fn new(state: &RenderState, images: Option<Arc<dyn ImageResolver>>) -> Self {
        Self {
            context: AvengerGpuContext::new(&state.device),
            images,
            renderers: Vec::new(),
        }
    }

    /// Clears `texture` to transparent, so that egui shows through where the scenes draw
    /// nothing, and draws `scenes` into it. Returns whether an image of the scenes is still
    /// loading, so that it draws its placeholder.
    pub fn draw<'a>(
        &mut self,
        state: &RenderState,
        texture: &Texture,
        scenes: impl IntoIterator<Item = &'a SceneGraph>,
        engine: &LabelEngine,
    ) -> bool {
        let (device, queue) = (&state.device, &state.queue);
        let mut loading = false;
        let dimensions = texture.dimensions;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Avenger scenes"),
        });
        clear(&mut encoder, &texture.target.view);
        for (index, scene) in scenes.into_iter().enumerate() {
            if index == self.renderers.len() {
                let mut config = AvengerRendererConfig::new(dimensions, FORMAT);
                config.images = self.images.clone();
                self.renderers
                    .push(AvengerWgpuRenderer::new(&self.context, config));
            }
            let renderer = &mut self.renderers[index];
            if !same(renderer.dimensions(), dimensions) {
                renderer.resize(dimensions);
            }
            if let Err(error) = renderer.set_scene(device, queue, scene, engine) {
                eprintln!("avenger-results-egui: a scene failed to install: {error:?}");
                continue;
            }
            // The scenes fill the top-left corner of the allocation, at the renderer's size.
            let target = (texture.target)
                .render_target(wgpu::LoadOp::Load)
                .with_viewport_origin([0, 0]);
            if let Err(error) = renderer.encode(device, queue, &mut encoder, target) {
                eprintln!("avenger-results-egui: a scene failed to draw: {error:?}");
            }
            loading |= !renderer.image_status().pending.is_empty();
        }
        queue.submit([encoder.finish()]);
        loading
    }
}

fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
    });
}

/// The longest side of a texture that the device allocates.
fn limit(state: &RenderState) -> u32 {
    state.device.limits().max_texture_dimension_2d
}

/// `dimensions` at a scale that fits `limit` physical pixels in both directions: its own scale
/// when it fits, and a lower one otherwise, so that a larger size draws less sharply.
fn fit(dimensions: CanvasDimensions, limit: u32) -> CanvasDimensions {
    let [width, height] = physical(dimensions);
    if width.max(height) <= limit {
        return dimensions;
    }
    // One pixel less than the limit, so that rounding to whole pixels stays within it.
    let longest = dimensions.size[0].max(dimensions.size[1]);
    CanvasDimensions {
        scale: (limit - 1) as f32 / longest,
        ..dimensions
    }
}

/// Whether `a` and `b` have the same size and scale.
pub fn same(a: CanvasDimensions, b: CanvasDimensions) -> bool {
    a.size == b.size && a.scale == b.scale
}

/// The dimensions of a texture that covers `size` logical pixels at `scale`, rounded to whole
/// physical pixels.
pub fn dimensions(size: egui::Vec2, scale: f32) -> CanvasDimensions {
    let round = |length: f32| ((length * scale).round().max(1.0)) / scale;
    CanvasDimensions {
        size: [round(size.x), round(size.y)],
        scale,
    }
}
