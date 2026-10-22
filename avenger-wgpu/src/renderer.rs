use std::{collections::HashMap, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::{
    canvas::CanvasDimensions, types::LinearScaleAdjustment, value::ScalarOrArray,
};
use avenger_image::{ImageLease, ImageResolver, ImageState};
use avenger_scenegraph::{
    marks::{group::Clip, rect::SceneRectMark},
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;
use wgpu::{
    BindGroup, CommandBuffer, CommandEncoder, CommandEncoderDescriptor, Device, LoadOp, Operations,
    Queue, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TextureFormat,
};

use crate::{
    canvas::{Canvas, CanvasResizeFrame, ImageAnswers, MarkContext},
    context::AvengerGpuContext,
    error::AvengerWgpuError,
    marks::{
        instanced_mark::{InstancedMarkPipeline, InstancedMarkRenderer},
        multi::{MultiMarkRenderResources, MultiMarkRenderer},
        text::TextAtlasBuilder,
    },
    offscreen::OffscreenTarget,
    target::{AvengerRenderTarget, PassTarget, WHITE_CLEAR},
};

#[derive(Clone)]
pub enum MarkRenderer {
    Instanced {
        renderer: Arc<InstancedMarkRenderer>,
        x_adjustment: Option<LinearScaleAdjustment>,
        y_adjustment: Option<LinearScaleAdjustment>,
    },
    /// A run of multi-marks between instanced marks, as a half-open range of batch indices
    /// into the renderer's single shared `MultiMarkRenderer`, which is prepared once per frame.
    Multi { batch_range: std::ops::Range<usize> },
}

#[derive(Clone)]
pub struct AvengerRendererConfig {
    pub dimensions: CanvasDimensions,
    pub texture_format: TextureFormat,
    pub sample_count: u32,
    /// The resolver of the scenes' image resources, which the renderer asks for them each
    /// frame. Without one, resources draw their placeholders.
    pub images: Option<Arc<dyn ImageResolver>>,
}

impl AvengerRendererConfig {
    pub fn new(dimensions: CanvasDimensions, texture_format: TextureFormat) -> Self {
        Self {
            dimensions,
            texture_format,
            sample_count: 1,
            images: None,
        }
    }

    pub fn with_sample_count(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count.max(1);
        self
    }

    pub fn with_images(mut self, images: Arc<dyn ImageResolver>) -> Self {
        self.images = Some(images);
        self
    }
}

/// The installed scene's image resources that aren't ready, as of the last frame. They draw
/// their placeholders.
#[derive(Debug, Clone, Default)]
pub struct ImageStatus {
    /// The URLs of the images that the resolver is loading.
    pub pending: Vec<String>,
    /// The URLs of the images that failed, with the reasons.
    pub failed: Vec<(String, Arc<str>)>,
}

impl ImageStatus {
    /// Whether every image is ready.
    pub fn is_complete(&self) -> bool {
        self.pending.is_empty() && self.failed.is_empty()
    }
}

/// What the renderer did for its last frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Whether the renderer installed a scene for the frame: a new scene, the scene again after
    /// a resize, or the scene again because an image resource's answer changed.
    pub installed: bool,
    /// Whether the frame built and uploaded the multi-marks' buffers and atlases, which it does
    /// only after an install or when its target's size changes.
    pub prepared: bool,
    /// The bytes of mesh image textures that the frame uploaded. A mesh image uploads when it
    /// first draws, and again only when its layer held another image in between.
    pub texture_upload_bytes: u64,
    /// The bytes of the mesh image texture arrays on the GPU after the frame. An array frees
    /// once its images are gone, as when the scene stops drawing them and the image cache
    /// evicts them.
    pub texture_bytes: u64,
}

pub struct AvengerWgpuRenderer {
    core: AvengerRendererCore,
}

impl AvengerWgpuRenderer {
    /// A renderer that draws with `context`'s pipelines.
    pub fn new(context: &AvengerGpuContext, config: AvengerRendererConfig) -> Self {
        let core = AvengerRendererCore::new(
            context,
            config.dimensions,
            config.texture_format,
            config.sample_count,
            config.images,
        );
        Self { core }
    }

    /// The installed scene's image resources that are pending or failed, as of the last frame.
    pub fn image_status(&self) -> ImageStatus {
        self.core.image_status()
    }

    /// What the last frame did.
    pub fn frame_stats(&self) -> FrameStats {
        self.core.frame_stats()
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

    /// Rebuild dimension-dependent scene resources on the next frame.
    /// Logical mark coordinates remain unchanged.
    pub fn resize(&mut self, dimensions: CanvasDimensions) {
        self.core.set_dimensions(dimensions);
    }

    /// Installs a scene, drawing its text with the engine that measured it.
    pub fn set_scene(
        &mut self,
        device: &Device,
        queue: &Queue,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        self.core.set_scene(device, queue, scene_graph, text_engine)
    }

    /// Record a frame of the installed scene into `encoder`, for the caller to submit.
    pub fn encode(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        target: AvengerRenderTarget<'_>,
    ) -> Result<(), AvengerWgpuError> {
        self.core.encode_frame(device, queue, encoder, target, None)
    }

    /// A frame of the installed scene, as command buffers for the caller to submit.
    pub fn build_frame_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: AvengerRenderTarget<'_>,
        resize_frame: Option<CanvasResizeFrame>,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        self.core
            .build_frame_commands(device, queue, target, resize_frame)
    }

    pub fn encode_to_offscreen_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &OffscreenTarget,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        self.core
            .encode_to_offscreen_commands(device, queue, target)
    }

    pub fn render_to_offscreen(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &OffscreenTarget,
    ) -> Result<(), AvengerWgpuError> {
        self.core.render_to_offscreen(device, queue, target)
    }
}

struct RendererCanvasAdapter<'a> {
    renderer: &'a mut AvengerRendererCore,
    device: &'a Device,
    queue: &'a Queue,
}

impl Canvas for RendererCanvasAdapter<'_> {
    fn get_multi_renderer(&mut self) -> &mut MultiMarkRenderer {
        self.renderer.shared_multi_mut()
    }

    fn text_atlas_builder(&mut self) -> &mut TextAtlasBuilder {
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

    fn begin_scene(
        &mut self,
        scene: &avenger_scenegraph::scene_graph::SceneGraph,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> Arc<ImageAnswers> {
        self.renderer.begin_scene(scene, text_engine)
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

    fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline> {
        self.renderer.symbol_pipeline()
    }
}

/// Surface-independent renderer state shared by host canvases.
pub(crate) struct AvengerRendererCore {
    dimensions: CanvasDimensions,
    texture_format: TextureFormat,
    sample_count: u32,
    /// The scene's marks in draw order.
    marks: Vec<MarkRenderer>,
    // All multi-marks accumulate into one shared renderer; each run between instanced marks
    // is recorded in `marks` as a batch range. `run_start` is the batch index where the current
    // (uncommitted) run began.
    shared_multi: MultiMarkRenderer,
    run_start: usize,
    /// The instanced renderers that the installed scene draws, by cache key.
    instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    /// The previous scene's instanced renderers while a scene installs, for the new scene to
    /// reuse. Installation drops the ones it doesn't reuse.
    previous_instanced_renderers: HashMap<u64, Arc<InstancedMarkRenderer>>,
    multi_render_resources: MultiMarkRenderResources,
    symbol_pipeline: Arc<InstancedMarkPipeline>,
    /// The installed scene and the engine that draws its text, to install again after a
    /// resize or when an image resource's answer changes.
    scene: Option<(Arc<SceneGraph>, LabelEngine)>,
    scene_dirty: bool,
    // Text atlas shared by all multi-renderers; built and uploaded after each install.
    text_atlas_builder: TextAtlasBuilder,
    text_bind_groups: Option<Arc<Vec<BindGroup>>>,
    images: Option<Arc<dyn ImageResolver>>,
    /// The installed scene's image resources, in the scene's order.
    image_urls: Vec<String>,
    /// The answers that the installed scene draws with.
    answers: Arc<ImageAnswers>,
    /// Keeps the installed scene's images in the resolver's memory.
    _lease: ImageLease,
    /// Whether a scene installed since the last frame's report.
    installed: bool,
    stats: FrameStats,
}

impl AvengerRendererCore {
    pub(crate) fn new(
        context: &AvengerGpuContext,
        dimensions: CanvasDimensions,
        texture_format: TextureFormat,
        sample_count: u32,
        images: Option<Arc<dyn ImageResolver>>,
    ) -> Self {
        let pipelines = context.pipelines(texture_format, sample_count);
        let text_atlas_builder = TextAtlasBuilder::new();

        let shared_multi = MultiMarkRenderer::new(dimensions);
        Self {
            dimensions,
            texture_format,
            sample_count,
            marks: Vec::new(),
            shared_multi,
            run_start: 0,
            instanced_renderers: HashMap::new(),
            previous_instanced_renderers: HashMap::new(),
            multi_render_resources: pipelines.multi,
            symbol_pipeline: pipelines.symbol,
            scene: None,
            scene_dirty: false,
            text_atlas_builder,
            text_bind_groups: None,
            images,
            image_urls: Vec::new(),
            answers: Arc::default(),
            _lease: ImageLease::default(),
            installed: false,
            stats: FrameStats::default(),
        }
    }

    /// Retain a scene to install, lease its image resources, and resolve them, in the scene's
    /// order. Returns the answers that the scene draws with.
    pub(crate) fn begin_scene(
        &mut self,
        scene: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Arc<ImageAnswers> {
        self.reset_marks();
        self.previous_instanced_renderers = std::mem::take(&mut self.instanced_renderers);
        self.scene = Some((Arc::new(scene.clone()), text_engine.clone()));
        self.scene_dirty = true;
        self.installed = true;
        if let Some(images) = &self.images {
            let urls = scene.image_urls();
            // The new lease starts before the old one ends, so images that both scenes draw
            // stay in memory.
            self._lease = images.retain(&urls);
            self.answers = Arc::new(
                urls.iter()
                    .map(|url| (url.to_string(), images.image(url)))
                    .collect(),
            );
            self.image_urls = urls.into_iter().map(str::to_string).collect();
        }
        self.answers.clone()
    }

    /// Bring the installed scene's answers up to date with the resolver. A mesh image that
    /// arrived draws in place, in the instances that waited for it. Returns whether the scene
    /// must install again to draw a change: an image mark's image, or a mesh image that changed
    /// after it drew.
    fn refresh_images(&mut self, device: &Device, queue: &Queue) -> Result<bool, AvengerWgpuError> {
        let Some(images) = self.images.clone() else {
            return Ok(false);
        };
        for url in &self.image_urls {
            let answer = images.image(url);
            let installed = self.answers.get(url);
            if installed.is_some_and(|installed| installed.draws_like(&answer)) {
                continue;
            }
            let drew = installed.is_some_and(|installed| installed.ready().is_some());
            match (&answer, self.shared_multi.waits_for(url)) {
                (ImageState::Ready(image), true) => self.shared_multi.patch_image(
                    url,
                    image,
                    device,
                    queue,
                    &self.multi_render_resources,
                )?,
                // A waiting image that fails, or stays pending, draws as it did.
                (_, true) if !drew => {}
                _ => return Ok(true),
            }
            Arc::make_mut(&mut self.answers).insert(url.clone(), answer);
        }
        Ok(false)
    }

    /// The installed scene's image resources that are pending or failed.
    pub(crate) fn image_status(&self) -> ImageStatus {
        let mut status = ImageStatus::default();
        for url in &self.image_urls {
            match self.answers.get(url) {
                Some(ImageState::Pending) => status.pending.push(url.clone()),
                Some(ImageState::Failed(reason)) => {
                    status.failed.push((url.clone(), reason.clone()))
                }
                Some(ImageState::Ready(_)) | None => {}
            }
        }
        status
    }

    pub(crate) fn frame_stats(&self) -> FrameStats {
        self.stats
    }

    pub(crate) fn finish_scene(&mut self) {
        self.scene_dirty = false;
        self.previous_instanced_renderers.clear();
    }

    /// Installs a scene, drawing its text with the engine that measured it.
    pub(crate) fn set_scene(
        &mut self,
        device: &Device,
        queue: &Queue,
        scene_graph: &SceneGraph,
        text_engine: &LabelEngine,
    ) -> Result<(), AvengerWgpuError> {
        let mut canvas = RendererCanvasAdapter {
            renderer: self,
            device,
            queue,
        };
        Canvas::set_scene(&mut canvas, scene_graph, text_engine)
    }

    /// Whether the renderer has no marks to draw: no installed scene, or a scene without marks.
    pub(crate) fn is_empty(&self) -> bool {
        (self.scene.as_ref()).is_none_or(|(scene, _)| scene.marks.is_empty())
    }

    pub(crate) fn dimensions(&self) -> CanvasDimensions {
        self.dimensions
    }

    pub(crate) fn set_dimensions(&mut self, dimensions: CanvasDimensions) {
        if self.dimensions.size != dimensions.size || self.dimensions.scale != dimensions.scale {
            self.dimensions = dimensions;
            self.scene_dirty = self.scene.is_some();
            self.shared_multi.set_dimensions(dimensions);
        }
    }

    pub(crate) fn texture_format(&self) -> TextureFormat {
        self.texture_format
    }

    pub(crate) fn symbol_pipeline(&self) -> Arc<InstancedMarkPipeline> {
        self.symbol_pipeline.clone()
    }

    pub(crate) fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The shared multi-renderer, to add marks to. Its prepared buffers no longer hold every
    /// mark, so the next frame prepares them again.
    pub(crate) fn shared_multi_mut(&mut self) -> &mut MultiMarkRenderer {
        self.shared_multi.invalidate_prepared();
        &mut self.shared_multi
    }

    /// The text atlas, to register text in. Its bind groups no longer hold every glyph, so the
    /// next frame builds them again.
    pub(crate) fn text_atlas_builder_mut(&mut self) -> &mut TextAtlasBuilder {
        self.text_bind_groups = None;
        &mut self.text_atlas_builder
    }

    /// The text atlas's bind groups, built and uploaded once after each install.
    pub(crate) fn text_bind_groups(
        &mut self,
        device: &Device,
        queue: &Queue,
    ) -> Arc<Vec<BindGroup>> {
        let resources = &self.multi_render_resources;
        let builder = &self.text_atlas_builder;
        self.text_bind_groups
            .get_or_insert_with(|| {
                let (text_atlas_size, text_atlas_images) = builder.build();
                Arc::new(MultiMarkRenderer::make_texture_bind_groups(
                    device,
                    queue,
                    resources.text_layout(),
                    text_atlas_size,
                    &text_atlas_images,
                    &[wgpu::FilterMode::Linear, wgpu::FilterMode::Nearest],
                ))
            })
            .clone()
    }

    /// Apply the target's load operation, such as the clear to the background color.
    fn encode_background(encoder: &mut CommandEncoder, target: &AvengerRenderTarget<'_>) {
        encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Render Background Pass"),
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

    /// The pass target for `target`, or an error when the renderer can't draw into it.
    fn pass_target<'a>(
        &self,
        target: &AvengerRenderTarget<'a>,
    ) -> Result<PassTarget<'a>, AvengerWgpuError> {
        if target.format != self.texture_format || target.sample_count != self.sample_count {
            return Err(AvengerWgpuError::InvalidTarget(format!(
                "the target is {:?} with {} samples, and the renderer draws {:?} with {}",
                target.format, target.sample_count, self.texture_format, self.sample_count
            )));
        }
        let viewport = match target.viewport_origin {
            None => None,
            Some([x, y]) => {
                if matches!(target.load, LoadOp::Clear(_)) {
                    return Err(AvengerWgpuError::InvalidTarget(
                        "a target with a viewport origin loads its contents, because a clear \
                         erases the whole attachment"
                            .to_string(),
                    ));
                }
                let width = self.dimensions.to_physical_width();
                let height = self.dimensions.to_physical_height();
                if x + width > target.extent.width || y + height > target.extent.height {
                    return Err(AvengerWgpuError::InvalidTarget(format!(
                        "a {width}x{height} scene at ({x}, {y}) doesn't fit in a {}x{} attachment",
                        target.extent.width, target.extent.height
                    )));
                }
                Some([x, y, width, height])
            }
        };
        Ok(PassTarget {
            view: target.view,
            resolve_target: target.resolve_target,
            extent: target.extent,
            viewport,
        })
    }

    pub(crate) fn build_frame_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: AvengerRenderTarget<'_>,
        resize_frame: Option<CanvasResizeFrame>,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Avenger Frame Encoder"),
        });
        self.encode_frame(device, queue, &mut encoder, target, resize_frame)?;
        Ok(vec![encoder.finish()])
    }

    pub(crate) fn encode_frame(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        target: AvengerRenderTarget<'_>,
        resize_frame: Option<CanvasResizeFrame>,
    ) -> Result<(), AvengerWgpuError> {
        let pass_target = self.pass_target(&target)?;
        if !self.scene_dirty && self.refresh_images(device, queue)? {
            self.scene_dirty = true;
        }
        if let Some((scene, text_engine)) =
            self.scene.as_ref().filter(|_| self.scene_dirty).cloned()
        {
            let mut canvas = RendererCanvasAdapter {
                renderer: self,
                device,
                queue,
            };
            Canvas::set_scene(&mut canvas, &scene, &text_engine)?;
        }
        self.commit_multi_renderer_if_needed();

        // Loading the target preserves earlier drawing, so skip the background clear.
        if matches!(target.load, LoadOp::Clear(_)) {
            Self::encode_background(encoder, &target);
        }

        let multi_render_resources = self.multi_render_resources.clone();
        let text_bind_groups = self.text_bind_groups(device, queue);
        let (prepared, rebuilt) = self.shared_multi.prepare_cached(
            device,
            queue,
            target.extent,
            &multi_render_resources,
        )?;
        // The frame reports the installs since the last one, and its own.
        self.stats = FrameStats {
            installed: std::mem::take(&mut self.installed),
            prepared: rebuilt,
            texture_upload_bytes: self.shared_multi.take_uploaded_bytes(),
            texture_bytes: self.shared_multi.texture_bytes(),
        };

        let mut pending: Vec<std::ops::Range<usize>> = Vec::new();
        for mark in &self.marks {
            match mark {
                MarkRenderer::Multi { batch_range } => pending.push(batch_range.clone()),
                MarkRenderer::Instanced {
                    renderer,
                    x_adjustment,
                    y_adjustment,
                } => {
                    self.shared_multi.encode_multi_ranges_into(
                        encoder,
                        pass_target,
                        &multi_render_resources,
                        &text_bind_groups,
                        &prepared,
                        &pending,
                    );
                    pending.clear();
                    renderer.encode_into(
                        device,
                        encoder,
                        pass_target,
                        *x_adjustment,
                        *y_adjustment,
                    );
                }
            }
        }
        self.shared_multi.encode_multi_ranges_into(
            encoder,
            pass_target,
            &multi_render_resources,
            &text_bind_groups,
            &prepared,
            &pending,
        );

        self.encode_resize_frame(
            device,
            queue,
            encoder,
            resize_frame,
            pass_target,
            &text_bind_groups,
        )
    }

    pub(crate) fn encode_to_offscreen_commands(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &OffscreenTarget,
    ) -> Result<Vec<CommandBuffer>, AvengerWgpuError> {
        let render_target = target.render_target(WHITE_CLEAR);
        self.build_frame_commands(device, queue, render_target, None)
    }

    pub(crate) fn render_to_offscreen(
        &mut self,
        device: &Device,
        queue: &Queue,
        target: &OffscreenTarget,
    ) -> Result<(), AvengerWgpuError> {
        let commands = self.encode_to_offscreen_commands(device, queue, target)?;
        queue.submit(commands);
        Ok(())
    }

    pub(crate) fn commit_multi_renderer_if_needed(&mut self) {
        let end = self.shared_multi.batch_count();
        if end > self.run_start {
            self.marks.push(MarkRenderer::Multi {
                batch_range: self.run_start..end,
            });
        }
        self.run_start = end;
    }

    pub(crate) fn get_instanced_renderer(
        &self,
        fingerprint: u64,
    ) -> Option<Arc<InstancedMarkRenderer>> {
        self.instanced_renderers
            .get(&fingerprint)
            .or_else(|| self.previous_instanced_renderers.get(&fingerprint))
            .cloned()
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
        self.marks.push(MarkRenderer::Instanced {
            renderer: mark_renderer,
            x_adjustment,
            y_adjustment,
        });
    }

    /// Clear the marks and the installed scene, and release the scene's image resources, so
    /// the resolver can let them go and later frames ask for nothing.
    pub(crate) fn clear_mark_renderer(&mut self) {
        self.reset_marks();
        self.image_urls.clear();
        self.answers = Arc::default();
        self._lease = ImageLease::default();
    }

    /// Clear the marks and the installed scene. A scene install keeps the image state, so its
    /// new lease starts before the old one ends.
    fn reset_marks(&mut self) {
        self.scene = None;
        self.scene_dirty = false;
        // One shared multi-renderer per canvas: reset it in place and clear the recorded
        // runs. The instanced renderers stay for the next scene to reuse.
        self.shared_multi.reset_for_frame(self.dimensions);
        self.run_start = 0;
        self.marks.clear();
        self.text_atlas_builder.reset();
        self.text_bind_groups = None;
    }

    /// Draw a canvas resize frame: its outline and resize handles.
    fn encode_resize_frame(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        resize_frame: Option<CanvasResizeFrame>,
        target: PassTarget<'_>,
        text_bind_groups: &[BindGroup],
    ) -> Result<(), AvengerWgpuError> {
        let Some(frame) = resize_frame else {
            return Ok(());
        };

        let width = frame.size[0].max(0.0);
        let height = frame.size[1].max(0.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        let border = 1.0;
        let handle = frame.handle_thickness.max(border);
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

        if frame.resize_width {
            x.push((width - handle).max(0.0));
            y.push(0.0);
            rect_width.push(handle);
            rect_height.push(height);
            fill.push(handle_fill.clone());
        }

        if frame.resize_height {
            x.push(0.0);
            y.push((height - handle).max(0.0));
            rect_width.push(width);
            rect_height.push(handle);
            fill.push(handle_fill);
        }

        let mark = SceneRectMark {
            name: "canvas_resize_frame".to_string(),
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
            fill_pattern: Default::default(),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::transparent()),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
            interactive: None,
        };

        let mut renderer = MultiMarkRenderer::new(self.dimensions);
        renderer.add_rect_mark(
            &mark,
            &MarkContext {
                origin: [0.0, 0.0],
                clip: &Clip::None,
                pattern_origin: [0.0, 0.0],
            },
        )?;

        // The frame has no text, but encoding binds `text_bind_groups[0]`, so it reuses the
        // frame's shared text bind groups.
        renderer.encode_with_resources(
            device,
            queue,
            encoder,
            target,
            &self.multi_render_resources,
            text_bind_groups,
        )
    }
}

#[cfg(test)]
mod text_raster_tests {
    use avenger_common::canvas::CanvasDimensions;
    use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
    use avenger_scenegraph::marks::text::{text_label, text_style};
    use avenger_typst_label::{bundled_label_engine, LabelOptions};

    use crate::marks::text::{TextAtlasBuilder, TextInstance};

    #[test]
    fn text_atlas_builder_registers_whole_mixed_label() {
        let mut builder = TextAtlasBuilder::new();

        let registrations = builder
            .register_text(
                TextInstance {
                    label: text_label(
                        "speed $v^2$",
                        TextSyntaxMode::TypstMarkup,
                        LabelOptions {
                            text: text_style(
                                "",
                                16.0,
                                FontWeight::default(),
                                FontStyle::default(),
                                [0.1, 0.2, 0.3, 1.0],
                            ),
                            ..Default::default()
                        },
                    ),
                    position: [10.0, 20.0],
                    align: TextAlign::Left,
                    baseline: TextBaseline::Alphabetic,
                    angle: 0.0,
                    use_nearest_filter: false,
                },
                CanvasDimensions {
                    size: [200.0, 80.0],
                    scale: 1.0,
                },
                &bundled_label_engine(),
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
        let mut builder = TextAtlasBuilder::new();

        // Long single-line title at scale 2.0 rasterizes wider than the atlas
        // page (1024px); previously this failed allocation with
        // ImageAllocationError and panicked the winit app.
        let instance = TextInstance {
            label: text_label(
                "Synthetic annual-mean temperature — drag to pan, scroll to zoom",
                TextSyntaxMode::Plain,
                LabelOptions {
                    text: text_style(
                        "",
                        22.0,
                        FontWeight::default(),
                        FontStyle::default(),
                        [0.1, 0.2, 0.3, 1.0],
                    ),
                    ..Default::default()
                },
            ),
            position: [10.0, 20.0],
            align: TextAlign::Left,
            baseline: TextBaseline::Alphabetic,
            angle: 0.0,
            use_nearest_filter: false,
        };
        let dimensions = CanvasDimensions {
            size: [900.0, 560.0],
            scale: 2.0,
        };

        let registrations = builder
            .register_text(instance.clone(), dimensions, &bundled_label_engine())
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
            .register_text(instance, dimensions, &bundled_label_engine())
            .expect("cached oversized entry should register");
        let cached_quads: usize = cached_registrations
            .iter()
            .map(|registration| registration.verts.len() / 4)
            .sum();
        assert_eq!(cached_quads, spans.len());
    }
}

#[cfg(test)]
mod instanced_cache_tests {
    use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
    use avenger_scenegraph::{
        marks::{group::SceneGroup, symbol::SceneSymbolMark},
        scene_graph::SceneGraph,
    };

    use super::{AvengerRendererConfig, AvengerWgpuRenderer};
    use crate::canvas::{make_wgpu_adapter, make_wgpu_instance, request_wgpu_device};

    /// Installing a scene keeps only the instanced renderers that it draws.
    #[test]
    fn scenes_keep_only_the_instanced_renderers_they_draw() {
        let (device, queue) = pollster::block_on(async {
            let instance = make_wgpu_instance();
            let adapter = make_wgpu_adapter(&instance, None).await.unwrap();
            request_wgpu_device(&adapter).await.unwrap()
        });
        let dimensions = CanvasDimensions {
            size: [200.0, 200.0],
            scale: 1.0,
        };
        let mut renderer = AvengerWgpuRenderer::new(
            &crate::context::AvengerGpuContext::new(&device),
            AvengerRendererConfig::new(dimensions, wgpu::TextureFormat::Rgba8Unorm),
        );
        let engine = avenger_typst_label::bundled_label_engine();
        for step in 0..10 {
            let scene = SceneGraph {
                width: 200.0,
                height: 200.0,
                origin: [0.0, 0.0],
                marks: vec![SceneGroup {
                    origin: [step as f32, 0.0],
                    marks: vec![SceneSymbolMark {
                        len: 100,
                        x: ScalarOrArray::new_array((0..100).map(|i| i as f32).collect()),
                        y: ScalarOrArray::new_scalar(50.0),
                        ..Default::default()
                    }
                    .into()],
                    ..Default::default()
                }
                .into()],
            };
            renderer
                .set_scene(&device, &queue, &scene, &engine)
                .unwrap();
        }
        assert_eq!(renderer.core.instanced_renderers.len(), 1);
        assert!(renderer.core.previous_instanced_renderers.is_empty());
    }
}
