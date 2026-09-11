use std::{collections::HashMap, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::{
    canvas::CanvasDimensions, types::LinearScaleAdjustment, value::ScalarOrArray,
};
use avenger_scenegraph::{
    marks::{group::Clip, mark::SceneMark, pattern::default_no_fill_pattern, rect::SceneRectMark},
    pattern_geometry::PatternRect,
    render_order::compute_zindex_layers,
    scene_graph::SceneGraph,
};
use wgpu::{
    BindGroup, CommandBuffer, CommandEncoderDescriptor, Device, Extent3d, Operations, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TextureFormat, TextureView,
};

use crate::{
    canvas::{Canvas, CanvasConfig, CanvasFrameOverlay, TextBuildCtor},
    error::AvengerWgpuError,
    image_resources::WgpuImageResourceStatus,
    marks::{
        instanced_mark::InstancedMarkRenderer,
        multi::{MultiMarkRenderResources, MultiMarkRenderer},
        text::TextAtlasBuilderTrait,
    },
    offscreen::{OffscreenTarget, RenderedOffscreenFrame},
    target::{AvengerRenderTarget, WHITE_CLEAR},
};

#[derive(Clone)]
pub enum MarkRenderer {
    Instanced {
        renderer: Arc<InstancedMarkRenderer>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    },
    /// A contiguous run of multi-marks at one z-index, stored as a half-open range
    /// of batch indices into the canvas's single shared `MultiMarkRenderer`. The
    /// renderer is prepared once per frame; consecutive runs are coalesced and
    /// encoded via `encode_multi_ranges`, preserving exact draw order and clipping.
    Multi { batch_range: std::ops::Range<usize> },
}

/// A mark renderer with its associated z-index.
#[derive(Clone)]
pub struct ZIndexedMark {
    pub zindex: i32,
    pub renderer: MarkRenderer,
}

#[derive(Clone)]
pub struct AvengerRendererConfig {
    pub dimensions: CanvasDimensions,
    pub texture_format: TextureFormat,
    pub sample_count: u32,
    pub canvas_config: CanvasConfig,
}

impl AvengerRendererConfig {
    pub fn new(dimensions: CanvasDimensions, texture_format: TextureFormat) -> Self {
        Self {
            dimensions,
            texture_format,
            sample_count: 1,
            canvas_config: CanvasConfig::default(),
        }
    }

    pub fn with_sample_count(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count.max(1);
        self
    }

    pub fn with_canvas_config(mut self, canvas_config: CanvasConfig) -> Self {
        self.canvas_config = canvas_config;
        self
    }
}

pub struct AvengerWgpuRenderer {
    core: AvengerRendererCore,
}

impl AvengerWgpuRenderer {
    pub fn new(device: &Device, config: AvengerRendererConfig) -> Self {
        let core = AvengerRendererCore::new(
            device,
            config.dimensions,
            config.texture_format,
            config.sample_count,
            config.canvas_config,
        );
        Self { core }
    }

    pub fn dimensions(&self) -> CanvasDimensions {
        self.core.dimensions()
    }

    pub fn texture_format(&self) -> TextureFormat {
        self.core.texture_format()
    }

    pub fn sample_count(&self) -> u32 {
        self.core.sample_count()
    }

    pub fn image_resource_status(&self) -> &WgpuImageResourceStatus {
        self.core.image_resource_status()
    }

    /// Rebuild dimension-dependent scene resources on the next frame.
    /// Logical mark coordinates remain unchanged.
    pub fn resize(&mut self, dimensions: CanvasDimensions) {
        self.core.set_dimensions(dimensions);
    }

    pub fn set_scene(
        &mut self,
        device: &Device,
        queue: &Queue,
        scene_graph: &SceneGraph,
    ) -> Result<(), AvengerWgpuError> {
        let mut canvas = RendererCanvasAdapter {
            renderer: &mut self.core,
            device,
            queue,
        };
        Canvas::set_scene(&mut canvas, scene_graph)
    }

    pub fn build_frame_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: AvengerRenderTarget<'_>,
        overlay: Option<CanvasFrameOverlay>,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        self.core
            .build_frame_commands(device, queue, target, overlay)
    }

    pub fn encode_to_offscreen_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        self.core
            .encode_to_offscreen_commands(device, queue, target)
    }

    pub fn render_to_offscreen(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<RenderedOffscreenFrame, AvengerWgpuError> {
        self.core.render_to_offscreen(device, queue, target)
    }
}

struct RendererCanvasAdapter<'a> {
    renderer: &'a mut AvengerRendererCore,
    device: &'a Device,
    queue: &'a Queue,
}

impl Canvas for RendererCanvasAdapter<'_> {
    fn set_current_zindex(&mut self, zindex: i32) {
        self.renderer.set_current_zindex(zindex);
    }

    fn commit_multi_renderer_if_needed(&mut self, _new_zindex: i32) {
        self.renderer.commit_multi_renderer_if_needed();
    }

    fn get_current_zindex(&self) -> i32 {
        self.renderer.current_zindex()
    }

    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        self.renderer.text_atlas_builder_mut()
    }

    fn get_instanced_renderer(&mut self, fingerprint: u64) -> Option<Arc<InstancedMarkRenderer>> {
        self.renderer.get_instanced_renderer(fingerprint)
    }

    fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.renderer.add_instanced_mark_renderer(
            mark_renderer,
            fingerprint,
            x_adjustment,
            y_adjustment,
        );
    }

    fn clear_mark_renderer(&mut self) {
        self.renderer.clear_mark_renderer();
    }

    fn begin_scene(&mut self, scene: &avenger_scenegraph::scene_graph::SceneGraph) {
        self.renderer.begin_scene(scene);
    }

    fn finish_scene(&mut self) {
        self.renderer.finish_scene();
    }

    fn device(&self) -> &Device {
        self.device
    }

    fn queue(&self) -> &Queue {
        self.queue
    }

    fn dimensions(&self) -> CanvasDimensions {
        self.renderer.dimensions()
    }

    fn font_resolution(&self) -> &avenger_text::FontResolutionOptions {
        self.renderer.font_resolution()
    }

    fn text_engine(&self) -> avenger_text::TextEngine {
        self.renderer.text_engine()
    }

    fn texture_format(&self) -> TextureFormat {
        self.renderer.texture_format()
    }

    fn sample_count(&self) -> u32 {
        self.renderer.sample_count()
    }
}

pub(crate) fn mark_renderer_counts(marks: &[ZIndexedMark]) -> (usize, usize) {
    marks
        .iter()
        .fold((0, 0), |(instanced_count, multi_count), mark| {
            match &mark.renderer {
                MarkRenderer::Instanced { .. } => (instanced_count + 1, multi_count),
                MarkRenderer::Multi { .. } => (instanced_count, multi_count + 1),
            }
        })
}

/// Surface-independent renderer state shared by host canvases.
pub(crate) struct AvengerRendererCore {
    dimensions: CanvasDimensions,
    texture_format: TextureFormat,
    sample_count: u32,
    marks: Vec<ZIndexedMark>,
    // All multi-marks accumulate into one shared renderer; each z-run is recorded
    // in `marks` as a batch range. `run_start` is the batch index where the current
    // (uncommitted) z-run began.
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    current_zindex: i32,
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    config: CanvasConfig,
    image_resource_status: WgpuImageResourceStatus,
    scene: Option<Arc<SceneGraph>>,
    scene_dirty: bool,
    image_resource_lease: avenger_image::ImageResourceLease,
    // Text atlas shared by all multi-renderers; built + uploaded once per frame.
    text_atlas_builder: Box<dyn TextAtlasBuilderTrait>,
    // Persistent tile texture arrays: survive clear_mark_renderer (like
    // instanced_renderers) so tiles upload once and pan/zoom frames
    // re-upload nothing.
    tile_arrays: crate::marks::tile_array::TileTextureArrays,
}

impl AvengerRendererCore {
    pub(crate) fn new(
        device: &Device,
        dimensions: CanvasDimensions,
        texture_format: TextureFormat,
        sample_count: u32,
        mut config: CanvasConfig,
    ) -> Self {
        let multi_render_resources =
            MultiMarkRenderResources::new(device, texture_format, sample_count);
        let text_engine = config.resolved_text_engine();
        config.text_engine = Some(text_engine.clone());
        let text_atlas_builder = make_text_atlas_builder(&config.text_builder_ctor, &text_engine);

        let tile_arrays = crate::marks::tile_array::TileTextureArrays::new();
        let mut shared_multi = MultiMarkRenderer::new(dimensions);
        shared_multi.set_tile_slot_allocator(tile_arrays.allocator());
        Self {
            dimensions,
            texture_format,
            sample_count,
            marks: Vec::new(),
            shared_multi,
            run_start: 0,
            current_zindex: 0,
            instanced_renderers: HashMap::new(),
            multi_render_resources,
            config,
            image_resource_status: WgpuImageResourceStatus::default(),
            scene: None,
            scene_dirty: false,
            image_resource_lease: Default::default(),
            text_atlas_builder,
            tile_arrays,
        }
    }

    pub(crate) fn begin_scene(&mut self, scene: &SceneGraph) {
        // Acquire the new working set before releasing the previous one: shared
        // images must remain resident even when both sets exceed the LRU budget.
        let lease = self
            .config
            .image_resource_config
            .resolver
            .as_ref()
            .map_or_else(Default::default, |resolver| {
                resolver.retain_images(&scene_image_keys(scene))
            });
        self.clear_mark_renderer();
        self.scene = Some(Arc::new(scene.clone()));
        self.image_resource_lease = lease;
        self.scene_dirty = true;
    }

    pub(crate) fn finish_scene(&mut self) {
        self.scene_dirty = false;
    }

    pub(crate) fn dimensions(&self) -> CanvasDimensions {
        self.dimensions
    }

    pub(crate) fn set_dimensions(&mut self, dimensions: CanvasDimensions) {
        if self.dimensions.size != dimensions.size || self.dimensions.scale != dimensions.scale {
            self.dimensions = dimensions;
            self.scene_dirty = self.scene.is_some();
            // Cached instanced uniforms include dimensions and cannot be reused
            // at the new size. Release old sizes instead of accumulating them.
            self.instanced_renderers.clear();
            self.shared_multi.set_dimensions(dimensions);
        }
    }

    pub(crate) fn texture_format(&self) -> TextureFormat {
        self.texture_format
    }

    pub(crate) fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub(crate) fn font_resolution(&self) -> &avenger_text::FontResolutionOptions {
        &self.config.font_resolution
    }

    pub(crate) fn text_engine(&self) -> avenger_text::TextEngine {
        self.config
            .text_engine
            .as_ref()
            .expect("renderer text context")
            .clone()
    }

    pub(crate) fn image_resource_status(&self) -> &WgpuImageResourceStatus {
        &self.image_resource_status
    }

    pub(crate) fn set_image_resource_resolver(
        &mut self,
        resolver: Arc<dyn crate::image_resources::ImageResourceResolver>,
    ) {
        self.image_resource_lease = self.scene.as_ref().map_or_else(Default::default, |scene| {
            resolver.retain_images(&scene_image_keys(scene))
        });
        self.config.image_resource_config.resolver = Some(resolver);
        self.image_resource_status = WgpuImageResourceStatus::default();
    }

    /// Tile-array upload accounting for the most recent prepared frame
    /// (and cumulative totals). Steady-state frames upload zero bytes.
    pub(crate) fn tile_upload_stats(
        &self,
    ) -> (
        crate::marks::tile_array::TileUploadStats,
        crate::marks::tile_array::TileUploadStats,
    ) {
        (
            self.tile_arrays.frame_stats(),
            self.tile_arrays.total_stats(),
        )
    }

    pub(crate) fn marks(&self) -> &[ZIndexedMark] {
        &self.marks
    }

    pub(crate) fn shared_multi_mut(&mut self) -> &mut MultiMarkRenderer {
        &mut self.shared_multi
    }

    pub(crate) fn text_atlas_builder_mut(&mut self) -> &mut dyn TextAtlasBuilderTrait {
        &mut *self.text_atlas_builder
    }

    pub(crate) fn build_text_bind_groups(&self, device: &Device, queue: &Queue) -> Vec<BindGroup> {
        let (text_atlas_size, text_atlas_images) = self.text_atlas_builder.build();
        MultiMarkRenderer::make_text_bind_groups_dual_sampler(
            device,
            queue,
            self.multi_render_resources.text_layout(),
            text_atlas_size,
            &text_atlas_images,
        )
    }

    pub(crate) fn make_background_command(
        &self,
        device: &Device,
        target: AvengerRenderTarget<'_>,
    ) -> CommandBuffer {
        debug_assert_eq!(target.format, self.texture_format);
        Self::make_background_command_for_target(device, target)
    }

    pub(crate) fn make_background_command_for_target(
        device: &Device,
        target: AvengerRenderTarget<'_>,
    ) -> CommandBuffer {
        let mut background_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Background Encoder"),
        });

        {
            let _render_pass = background_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target.view,
                    resolve_target: target.resolve_target,
                    depth_slice: None,
                    ops: Operations {
                        load: target.load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        background_encoder.finish()
    }

    #[allow(dead_code)]
    pub(crate) fn build_frame_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: AvengerRenderTarget<'_>,
        overlay: Option<CanvasFrameOverlay>,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        debug_assert_eq!(target.format, self.texture_format);
        debug_assert_eq!(target.sample_count, self.sample_count);

        if let Some(scene) = self.scene.as_ref().filter(|_| self.scene_dirty).cloned() {
            let mut canvas = RendererCanvasAdapter {
                renderer: self,
                device,
                queue,
            };
            Canvas::set_scene(&mut canvas, &scene)?;
        }
        self.commit_all_multi_renderers();
        let marks = self.marks.clone();
        let zindices: Vec<i32> = marks.iter().map(|m| m.zindex).collect();
        let layers = if zindices.is_empty() {
            vec![]
        } else {
            compute_zindex_layers(zindices)
        };

        let background_command = self.make_background_command(device, target);
        let mut commands = vec![background_command];

        let multi_render_resources = self.multi_render_resources.clone();
        let text_bind_groups = self.build_text_bind_groups(device, queue);

        // Upload changed tile layers (usually none) and collect their
        // pending/missing/failed keys alongside the atlas-resolved ones.
        let tile_status = self.tile_arrays.sync(
            device,
            queue,
            multi_render_resources.tile_texture_layout(),
            &self.config.image_resource_config,
        )?;
        let (prepared, mut image_resource_status) = self.shared_multi.prepare(
            device,
            queue,
            target.extent,
            &multi_render_resources,
            &self.config.image_resource_config,
        )?;
        image_resource_status.pending.extend(tile_status.pending);
        image_resource_status.missing.extend(tile_status.missing);
        image_resource_status.failed.extend(tile_status.failed);
        self.image_resource_status = image_resource_status;

        let mut mark_encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Avenger Mark Render Encoder"),
        });
        let mut encoded_marks = false;
        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for (min_z, max_z) in layers {
            for mark in &marks {
                if mark.zindex >= min_z && mark.zindex <= max_z {
                    match &mark.renderer {
                        MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                        MarkRenderer::Instanced {
                            renderer,
                            x_adjustment,
                            y_adjustment,
                        } => {
                            if !pending.is_empty() {
                                self.shared_multi.encode_multi_ranges_into(
                                    &mut mark_encoder,
                                    target.extent,
                                    target.view,
                                    target.resolve_target,
                                    &multi_render_resources,
                                    &text_bind_groups,
                                    &prepared,
                                    Some(&self.tile_arrays),
                                    &pending,
                                );
                                pending.clear();
                            }
                            renderer.encode_into(
                                device,
                                &mut mark_encoder,
                                target.extent,
                                target.view,
                                target.resolve_target,
                                *x_adjustment,
                                *y_adjustment,
                            );
                            encoded_marks = true;
                        }
                    }
                }
            }
        }

        if !pending.is_empty() {
            self.shared_multi.encode_multi_ranges_into(
                &mut mark_encoder,
                target.extent,
                target.view,
                target.resolve_target,
                &multi_render_resources,
                &text_bind_groups,
                &prepared,
                Some(&self.tile_arrays),
                &pending,
            );
            encoded_marks = true;
        }

        if encoded_marks {
            commands.push(mark_encoder.finish());
        }

        if let Some(command) = self.make_frame_overlay_command(
            device,
            queue,
            overlay,
            target.extent,
            target.view,
            target.resolve_target,
            &text_bind_groups,
        )? {
            commands.push(command);
        }

        Ok(commands)
    }

    #[allow(dead_code)]
    pub(crate) fn encode_to_offscreen_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        let render_target = target.render_target(WHITE_CLEAR);
        self.build_frame_commands(device, queue, render_target, None)
    }

    #[allow(dead_code)]
    pub(crate) fn render_to_offscreen(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &mut OffscreenTarget,
    ) -> Result<RenderedOffscreenFrame, AvengerWgpuError> {
        let commands = self.encode_to_offscreen_commands(device, queue, target)?;
        queue.submit(commands);
        Ok(RenderedOffscreenFrame::from(&*target))
    }

    pub(crate) fn set_current_zindex(&mut self, zindex: i32) {
        if zindex != self.current_zindex {
            self.commit_multi_renderer_if_needed();
        }
        self.current_zindex = zindex;
    }

    pub(crate) fn commit_multi_renderer_if_needed(&mut self) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(ZIndexedMark {
                zindex: self.current_zindex,
                renderer: MarkRenderer::Multi {
                    batch_range: self.run_start..end,
                },
            });
        }
        self.run_start = end;
    }

    pub(crate) fn commit_all_multi_renderers(&mut self) {
        self.commit_multi_renderer_if_needed();
    }

    pub(crate) fn current_zindex(&self) -> i32 {
        self.current_zindex
    }

    pub(crate) fn get_instanced_renderer(
        &self,
        fingerprint: u64,
    ) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers.get(&fingerprint).cloned()
    }

    pub(crate) fn add_instanced_mark_renderer(
        &mut self,
        mark_renderer: Arc<InstancedMarkRenderer>,
        fingerprint: u64,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    ) {
        self.commit_multi_renderer_if_needed();
        self.instanced_renderers
            .insert(fingerprint, mark_renderer.clone());
        self.marks.push(ZIndexedMark {
            zindex: self.current_zindex,
            renderer: MarkRenderer::Instanced {
                renderer: mark_renderer,
                x_adjustment,
                y_adjustment,
            },
        });
    }

    pub(crate) fn clear_mark_renderer(&mut self) {
        self.scene = None;
        self.scene_dirty = false;
        self.image_resource_lease = Default::default();
        // One shared multi-renderer per canvas: reset it in place each frame and
        // clear the recorded z-run marks. (Instanced fingerprint cache is retained.)
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();

        // Reset atlas contents while retaining expensive builder-owned services
        // such as the text engine/font database.
        self.text_atlas_builder.reset();

        // New scene epoch: tile layers from the previous scene become
        // evictable but stay resident (returning viewports reuse them).
        self.tile_arrays.begin_scene();
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
    )]
    pub(crate) fn make_frame_overlay_command(
        &self,
        device: &Device,
        queue: &Queue,
        overlay: Option<CanvasFrameOverlay>,
        render_target_extent: Extent3d,
        texture_view: &TextureView,
        resolve_target: Option<&TextureView>,
        text_bind_groups: &[BindGroup],
    ) -> Result<Option<CommandBuffer>, AvengerWgpuError> {
        let Some(overlay) = overlay else {
            return Ok(None);
        };

        let width = overlay.size[0].max(0.0);
        let height = overlay.size[1].max(0.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(None);
        }

        let border = 1.0;
        let handle = overlay.handle_thickness.max(border);
        let outline = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.95]);
        let handle_fill = ColorOrGradient::Color([0.12, 0.15, 0.18, 0.16]);

        let mut x = vec![0.0, 0.0, 0.0, (width - border).max(0.0)];
        let mut y = vec![0.0, (height - border).max(0.0), 0.0, 0.0];
        let mut rect_width = vec![width, width, border, border];
        let mut rect_height = vec![border, border, height, height];
        let mut fill = vec![
            outline.clone(),
            outline.clone(),
            outline.clone(),
            outline.clone(),
        ];

        if overlay.resize_width {
            x.push((width - handle).max(0.0));
            y.push(0.0);
            rect_width.push(handle);
            rect_height.push(height);
            fill.push(handle_fill.clone());
        }

        if overlay.resize_height {
            x.push(0.0);
            y.push((height - handle).max(0.0));
            rect_width.push(width);
            rect_height.push(handle);
            fill.push(handle_fill);
        }

        let mark = SceneRectMark {
            name: "canvas_frame_overlay".to_string(),
            clip: false,
            len: x.len() as u32,
            gradients: Vec::new(),
            x: ScalarOrArray::new_array(x),
            y: ScalarOrArray::new_array(y),
            width: Some(ScalarOrArray::new_array(rect_width)),
            height: Some(ScalarOrArray::new_array(rect_height)),
            x2: None,
            y2: None,
            fill: ScalarOrArray::new_array(fill),
            fill_pattern: default_no_fill_pattern(),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::transparent()),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
            interactive: true,
        };

        let mut renderer = MultiMarkRenderer::new(self.dimensions);
        renderer.add_rect_mark(
            &mark,
            [0.0, 0.0],
            &Clip::None,
            None,
            PatternRect::new(0.0, 0.0, self.dimensions.size[0], self.dimensions.size[1]),
        )?;

        // The overlay has no text, but `render_with_resources` indexes
        // `text_bind_groups[0]`; reuse the frame's shared text bind groups.
        Ok(Some(renderer.render_with_resources(
            device,
            queue,
            render_target_extent,
            texture_view,
            resolve_target,
            &self.multi_render_resources,
            text_bind_groups,
        )?))
    }
}

/// Construct a text atlas builder, shared by a single canvas across all of its
/// multi-renderers. This mirrors the construction logic that previously lived in
/// `MultiMarkRenderer::new`: honor a caller-supplied `text_builder_ctor`,
/// otherwise use the default text rasterizer.
pub(crate) fn make_text_atlas_builder(
    text_builder_ctor: &Option<TextBuildCtor>,
    text_engine: &avenger_text::TextEngine,
) -> Box<dyn TextAtlasBuilderTrait> {
    if let Some(text_builder_ctor) = text_builder_ctor {
        text_builder_ctor()
    } else {
        Box::new(crate::marks::text::TextAtlasBuilder::new(Arc::new(
            text_engine.clone(),
        )))
    }
}

fn scene_image_keys(scene: &SceneGraph) -> Vec<avenger_resource::ResourceKey> {
    fn source_keys(
        source: &avenger_scenegraph::marks::image::SceneImageSource,
    ) -> Vec<avenger_resource::ResourceKey> {
        match source {
            avenger_scenegraph::marks::image::SceneImageSource::Resource(resource) => {
                let mut keys = vec![resource.key.clone()];
                keys.extend(resource.fallback_key.clone());
                keys
            }
            _ => Vec::new(),
        }
    }
    fn collect(marks: &[SceneMark], keys: &mut Vec<avenger_resource::ResourceKey>) {
        for mark in marks {
            match mark {
                SceneMark::Image(mark) => {
                    keys.extend(mark.image_source_iter().flat_map(source_keys))
                }
                SceneMark::WarpedImage(mark) => keys.extend(source_keys(&mark.image)),
                SceneMark::Group(group) => collect(&group.marks, keys),
                _ => {}
            }
        }
    }
    let mut keys = Vec::new();
    collect(&scene.marks, &mut keys);
    keys
}

#[cfg(test)]
mod text_raster_tests {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};

    use crate::{
        canvas::CanvasConfig, marks::text::TextInstance, renderer::make_text_atlas_builder,
    };

    #[test]
    fn text_atlas_builder_registers_whole_mixed_label() {
        let config = CanvasConfig::default();
        let mut builder = make_text_atlas_builder(&None, &config.resolved_text_engine());

        let text = "speed $v^2$".to_string();
        let color = [0.1, 0.2, 0.3, 1.0];
        let align = TextAlign::Left;
        let baseline = TextBaseline::Alphabetic;
        let font = "sans-serif".to_string();
        let font_weight = FontWeight::default();
        let font_style = FontStyle::default();

        let registrations = builder
            .register_text(
                TextInstance {
                    position: [10.0, 20.0],
                    text: &text,
                    color: &color,
                    align: &align,
                    angle: 0.0,
                    baseline: &baseline,
                    font: &font,
                    font_size: 16.0,
                    font_weight: &font_weight,
                    font_style: &font_style,
                    limit: f32::INFINITY,
                    syntax_mode: TextSyntaxMode::TypstMarkup,
                    params: avenger_text::empty_label_params(),
                    number_locale: None,
                    number_locale_specs: &avenger_text::NumberLocaleSpecs::default(),
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: &avenger_text::DateTimeLocaleSpecs::default(),
                    use_nearest_filter: false,
                },
                CanvasDimensions {
                    size: [200.0, 80.0],
                    scale: 1.0,
                },
            )
            .expect("whole-line Typst label should register");

        let vertex_count: usize = registrations
            .iter()
            .map(|registration| registration.verts.len())
            .sum();
        assert_eq!(vertex_count, 4, "expected one atlas quad for one text line");

        let (extent, atlases) = builder.build();
        assert!(extent.width > 1);
        assert!(!atlases.is_empty());
        assert!(atlases.iter().any(|atlas| {
            atlas
                .as_rgba8()
                .is_some_and(|image| image.pixels().any(|pixel| pixel[3] > 0))
        }));
    }

    #[test]
    fn text_atlas_builder_tiles_entry_wider_than_atlas() {
        let config = CanvasConfig::default();
        let mut builder = make_text_atlas_builder(&None, &config.resolved_text_engine());

        // Long single-line title at scale 2.0 rasterizes wider than the atlas
        // page (1024px); previously this failed allocation with
        // ImageAllocationError and panicked the winit app.
        let text = "Synthetic annual-mean temperature — drag to pan, scroll to zoom".to_string();
        let color = [0.1, 0.2, 0.3, 1.0];
        let align = TextAlign::Left;
        let baseline = TextBaseline::Alphabetic;
        let font = "sans-serif".to_string();
        let font_weight = FontWeight::default();
        let font_style = FontStyle::default();

        let instance = TextInstance {
            position: [10.0, 20.0],
            text: &text,
            color: &color,
            align: &align,
            angle: 0.0,
            baseline: &baseline,
            font: &font,
            font_size: 22.0,
            font_weight: &font_weight,
            font_style: &font_style,
            limit: f32::INFINITY,
            syntax_mode: TextSyntaxMode::Plain,
            params: avenger_text::empty_label_params(),
            number_locale: None,
            number_locale_specs: &avenger_text::NumberLocaleSpecs::default(),
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: &avenger_text::DateTimeLocaleSpecs::default(),
            use_nearest_filter: false,
        };
        let dimensions = CanvasDimensions {
            size: [900.0, 560.0],
            scale: 2.0,
        };

        let registrations = builder
            .register_text(instance.clone(), dimensions)
            .expect("oversized text raster entry should tile instead of failing allocation");

        // Collect quads as (x0, x1) spans; the oversized entry must split into
        // multiple tiles that stitch back together seamlessly.
        let mut spans: Vec<(f32, f32)> = registrations
            .iter()
            .flat_map(|registration| registration.verts.chunks(4))
            .map(|quad| (quad[0].position[0], quad[2].position[0]))
            .collect();
        assert!(
            spans.len() >= 2,
            "expected the oversized entry to split into multiple tiles, got {} quad(s)",
            spans.len()
        );
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        for pair in spans.windows(2) {
            assert!(
                (pair[1].0 - pair[0].1).abs() < 1e-3,
                "adjacent tiles should stitch exactly: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }

        // Re-registering the same text must hit the atlas cache and produce the
        // same tiling.
        let cached_registrations = builder
            .register_text(instance, dimensions)
            .expect("cached oversized entry should register");
        let cached_quads: usize = cached_registrations
            .iter()
            .map(|registration| registration.verts.len() / 4)
            .sum();
        assert_eq!(cached_quads, spans.len());
    }
}
