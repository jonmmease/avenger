use std::{
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant as StdInstant},
};

#[cfg(not(target_arch = "wasm32"))]
use std::{
    sync::Condvar,
    thread::{self, JoinHandle},
};

use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::{
    canvas::CanvasConfig,
    error::AvengerWgpuError,
    offscreen::{OffscreenTargetDescriptor, OffscreenTargetPool, RenderedOffscreenFrame},
    renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
};
use egui_wgpu::wgpu;

use crate::metrics::{CanvasMetrics, duration_us, update_metrics};

#[derive(Clone, Copy, Debug)]
pub struct TextureRenderStatus {
    pub texture_id: Option<egui::TextureId>,
    pub texture_generation: Option<u64>,
    pub scene_generation: Option<u64>,
    pub dimensions: Option<CanvasDimensions>,
    pub render_invalidation_epoch: Option<u64>,
    pub render_pending: bool,
}

struct EguiTextureRegistryState {
    texture_id: Option<egui::TextureId>,
    registered_generation: Option<u64>,
    registered_scene_generation: Option<u64>,
    registered_dimensions: Option<CanvasDimensions>,
    registered_render_invalidation_epoch: Option<u64>,
    consumed_render_generation: Option<u64>,
}

impl EguiTextureRegistryState {
    pub(crate) fn new() -> Self {
        Self {
            texture_id: None,
            registered_generation: None,
            registered_scene_generation: None,
            registered_dimensions: None,
            registered_render_invalidation_epoch: None,
            consumed_render_generation: None,
        }
    }

    fn register_texture_view(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        texture_view: &wgpu::TextureView,
        target_generation: u64,
        scene_generation: Option<u64>,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        metrics: &StdMutex<CanvasMetrics>,
    ) -> egui::TextureId {
        let texture_publish_start = StdInstant::now();
        let mut egui_renderer = render_state.renderer.write();
        let reused_texture_id = self.texture_id.is_some();
        let texture_id = if let Some(texture_id) = self.texture_id {
            egui_renderer.update_egui_texture_from_wgpu_texture(
                &render_state.device,
                texture_view,
                wgpu::FilterMode::Linear,
                texture_id,
            );
            texture_id
        } else {
            egui_renderer.register_native_texture(
                &render_state.device,
                texture_view,
                wgpu::FilterMode::Linear,
            )
        };
        let texture_publish_elapsed = texture_publish_start.elapsed();
        self.texture_id = Some(texture_id);
        self.registered_generation = Some(target_generation);
        self.registered_scene_generation = scene_generation;
        self.registered_dimensions = Some(dimensions);
        self.registered_render_invalidation_epoch = Some(render_invalidation_epoch);
        update_metrics(metrics, |metrics| {
            if reused_texture_id {
                metrics.texture_updates += 1;
            } else {
                metrics.texture_registrations += 1;
            }
            metrics.last_texture_publish_us = duration_us(texture_publish_elapsed);
        });
        texture_id
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn register_rendered_texture(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        rendered: &RenderedCanvasTexture,
        metrics: &StdMutex<CanvasMetrics>,
    ) -> Option<egui::TextureId> {
        if self.consumed_render_generation == Some(rendered.render_generation) {
            return self.texture_id;
        }

        let texture_id = self.register_texture_view(
            render_state,
            &rendered.view,
            rendered.target_generation,
            Some(rendered.scene_generation),
            rendered.dimensions,
            rendered.render_invalidation_epoch,
            metrics,
        );
        self.consumed_render_generation = Some(rendered.render_generation);
        update_metrics(metrics, |metrics| {
            metrics.background_render_frames_consumed += 1;
        });
        Some(texture_id)
    }

    pub(crate) fn status(&self) -> TextureRenderStatus {
        TextureRenderStatus {
            texture_id: self.texture_id,
            texture_generation: self.registered_generation,
            scene_generation: self.registered_scene_generation,
            dimensions: self.registered_dimensions,
            render_invalidation_epoch: self.registered_render_invalidation_epoch,
            render_pending: false,
        }
    }
}

pub(crate) struct EguiCanvasGpuState {
    sync: Option<EguiCanvasSyncGpuState>,
    #[cfg(not(target_arch = "wasm32"))]
    background: Option<BackgroundRenderController>,
    registry: EguiTextureRegistryState,
    format: wgpu::TextureFormat,
    canvas_config: CanvasConfig,
}

impl EguiCanvasGpuState {
    pub(crate) fn new(
        _device: &wgpu::Device,
        _dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
        canvas_config: CanvasConfig,
    ) -> Self {
        Self {
            sync: None,
            #[cfg(not(target_arch = "wasm32"))]
            background: None,
            registry: EguiTextureRegistryState::new(),
            format,
            canvas_config,
        }
    }

    pub(crate) fn render_scene(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        metrics: &StdMutex<CanvasMetrics>,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        let sync = self.sync.get_or_insert_with(|| {
            EguiCanvasSyncGpuState::new(
                &render_state.device,
                dimensions,
                self.format,
                self.canvas_config.clone(),
            )
        });
        sync.render_scene(
            render_state,
            scene_graph,
            dimensions,
            render_invalidation_epoch,
            &mut self.registry,
            metrics,
        )
    }

    pub(crate) fn request_background_scene_texture(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        repaint: Option<egui::Context>,
        metrics: Arc<StdMutex<CanvasMetrics>>,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        #[cfg(target_arch = "wasm32")]
        {
            self.render_scene_with_invalidation_epoch(
                render_state,
                &scene_graph,
                dimensions,
                render_invalidation_epoch,
                metrics.as_ref(),
            )?;
            let mut status = self.registry.status();
            status.scene_generation = Some(scene_generation);
            self.registry.registered_scene_generation = Some(scene_generation);
            return Ok(status);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let controller = self.background.get_or_insert_with(|| {
                BackgroundRenderController::new(
                    render_state.device.clone(),
                    render_state.queue.clone(),
                    self.format,
                    self.canvas_config.clone(),
                    metrics.clone(),
                )
            });

            controller.set_front_target_generation(self.registry.registered_generation);
            controller.consume_latest(render_state, &mut self.registry, metrics.as_ref());
            controller.enqueue(
                scene_generation,
                scene_graph,
                dimensions,
                render_invalidation_epoch,
                self.format,
                repaint,
                metrics.as_ref(),
            );
            controller.consume_latest(render_state, &mut self.registry, metrics.as_ref());

            if let Some(error) = controller.take_error() {
                return Err(AvengerWgpuError::ConversionError(error));
            }

            let mut status = self.registry.status();
            status.render_pending = controller.is_pending();
            Ok(status)
        }
    }

    pub(crate) fn texture_id(&self) -> Option<egui::TextureId> {
        self.registry.texture_id
    }

    #[cfg(target_arch = "wasm32")]
    fn render_scene_with_invalidation_epoch(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        metrics: &StdMutex<CanvasMetrics>,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        let sync = self.sync.get_or_insert_with(|| {
            EguiCanvasSyncGpuState::new(
                &render_state.device,
                dimensions,
                self.format,
                self.canvas_config.clone(),
            )
        });
        sync.render_scene(
            render_state,
            scene_graph,
            dimensions,
            render_invalidation_epoch,
            &mut self.registry,
            metrics,
        )
    }

    pub(crate) fn frame_status(&self) -> FrameStatus {
        #[cfg(not(target_arch = "wasm32"))]
        let texture_render_mode = if self.background.is_some() {
            TextureRenderMode::BackgroundGpu
        } else if self.sync.is_some() {
            TextureRenderMode::UiThreadGpu
        } else {
            TextureRenderMode::Uninitialized
        };
        #[cfg(not(target_arch = "wasm32"))]
        let background_status = self
            .background
            .as_ref()
            .map(BackgroundRenderController::status)
            .unwrap_or_default();
        #[cfg(target_arch = "wasm32")]
        let texture_render_mode = if self.sync.is_some() {
            TextureRenderMode::UiThreadGpu
        } else {
            TextureRenderMode::Uninitialized
        };
        #[cfg(target_arch = "wasm32")]
        let background_status = BackgroundRenderStatus::default();
        FrameStatus {
            latest_generation: self.registry.registered_generation,
            latest_scene_generation: None,
            requested_generation: None,
            render_pending: background_status.render_pending,
            latest_texture_scene_generation: self.registry.registered_scene_generation,
            requested_texture_scene_generation: background_status.requested_scene_generation,
            latest_render_invalidation_epoch: self.registry.registered_render_invalidation_epoch,
            requested_render_invalidation_epoch: background_status
                .requested_render_invalidation_epoch,
            gpu_render_pending: background_status.render_pending,
            texture_render_mode,
        }
    }
}

struct EguiCanvasSyncGpuState {
    renderer: AvengerWgpuRenderer,
    targets: OffscreenTargetPool,
    format: wgpu::TextureFormat,
}

impl EguiCanvasSyncGpuState {
    fn new(
        device: &wgpu::Device,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
        canvas_config: CanvasConfig,
    ) -> Self {
        let renderer = AvengerWgpuRenderer::new(
            device,
            AvengerRendererConfig::new(dimensions, format)
                .with_sample_count(1)
                .with_canvas_config(canvas_config),
        );
        let targets = OffscreenTargetPool::triple_buffered(
            device,
            OffscreenTargetDescriptor::new(dimensions, format)
                .with_label("avenger-egui canvas offscreen texture"),
        );
        Self {
            renderer,
            targets,
            format,
        }
    }

    pub(crate) fn render_scene(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        registry: &mut EguiTextureRegistryState,
        metrics: &StdMutex<CanvasMetrics>,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        self.renderer.resize(dimensions);
        let set_scene_start = StdInstant::now();
        self.renderer
            .set_scene(&render_state.device, &render_state.queue, scene_graph)?;
        let set_scene_elapsed = set_scene_start.elapsed();
        self.targets.resize_or_recreate(
            &render_state.device,
            OffscreenTargetDescriptor::new(dimensions, self.format)
                .with_label("avenger-egui canvas offscreen texture"),
        );

        let target = if let Some(target) = self
            .targets
            .acquire_next_excluding_generation(registry.registered_generation)
        {
            target
        } else {
            self.targets.acquire_next()
        };
        let encode_start = StdInstant::now();
        let commands = self.renderer.encode_to_offscreen_commands(
            &render_state.device,
            &render_state.queue,
            target,
        )?;
        let encode_elapsed = encode_start.elapsed();

        let submit_start = StdInstant::now();
        render_state.queue.submit(commands);
        let submit_elapsed = submit_start.elapsed();
        let rendered = RenderedOffscreenFrame::from(&*target);

        update_metrics(metrics, |metrics| {
            metrics.offscreen_texture_renders += 1;
            metrics.last_set_scene_us = duration_us(set_scene_elapsed);
            metrics.last_command_encode_us = duration_us(encode_elapsed);
            metrics.last_submit_us = duration_us(submit_elapsed);
        });
        let texture_id = registry.register_texture_view(
            render_state,
            &target.view,
            rendered.generation,
            None,
            dimensions,
            render_invalidation_epoch,
            metrics,
        );
        tracing::debug!(
            texture_generation = rendered.generation,
            set_scene_us = duration_us(set_scene_elapsed),
            command_encode_us = duration_us(encode_elapsed),
            submit_us = duration_us(submit_elapsed),
            "rendered avenger scene to egui texture"
        );
        Ok(texture_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderRequestKey {
    pub(crate) scene_generation: u64,
    pub(crate) render_invalidation_epoch: u64,
    pub(crate) width_bits: u32,
    pub(crate) height_bits: u32,
    pub(crate) scale_bits: u32,
    pub(crate) format: wgpu::TextureFormat,
}

impl RenderRequestKey {
    pub(crate) fn new(
        scene_generation: u64,
        render_invalidation_epoch: u64,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            scene_generation,
            render_invalidation_epoch,
            width_bits: dimensions.size[0].to_bits(),
            height_bits: dimensions.size[1].to_bits(),
            scale_bits: dimensions.scale.to_bits(),
            format,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub(crate) struct BackgroundRenderRequest {
    pub(crate) render_generation: u64,
    pub(crate) scene_generation: u64,
    pub(crate) render_invalidation_epoch: u64,
    pub(crate) scene_graph: Arc<SceneGraph>,
    pub(crate) dimensions: CanvasDimensions,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) repaint: Option<egui::Context>,
    pub(crate) requested_at: StdInstant,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct RenderedCanvasTexture {
    pub(crate) render_generation: u64,
    pub(crate) scene_generation: u64,
    pub(crate) render_invalidation_epoch: u64,
    pub(crate) target_generation: u64,
    pub(crate) dimensions: CanvasDimensions,
    pub(crate) view: wgpu::TextureView,
    pub(crate) repaint: Option<egui::Context>,
    pub(crate) requested_at: StdInstant,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BackgroundRenderStatus {
    pub(crate) requested_scene_generation: Option<u64>,
    pub(crate) requested_render_invalidation_epoch: Option<u64>,
    pub(crate) render_pending: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct BackgroundRenderState {
    pub(crate) next_render_generation: u64,
    pub(crate) requested_scene_generation: Option<u64>,
    pub(crate) requested_render_invalidation_epoch: Option<u64>,
    pub(crate) last_requested_key: Option<RenderRequestKey>,
    pub(crate) pending_request: Option<BackgroundRenderRequest>,
    pub(crate) latest_rendered: Option<Arc<RenderedCanvasTexture>>,
    pub(crate) consumed_render_generation: Option<u64>,
    pub(crate) front_target_generation: Option<u64>,
    pub(crate) in_progress_generation: Option<u64>,
    pub(crate) last_error: Option<String>,
    pub(crate) stop: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BackgroundEnqueueResult {
    pub(crate) render_generation: Option<u64>,
    pub(crate) coalesced_previous: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderState {
    pub(crate) fn enqueue_request(
        &mut self,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        format: wgpu::TextureFormat,
        repaint: Option<egui::Context>,
        requested_at: StdInstant,
    ) -> BackgroundEnqueueResult {
        let key = RenderRequestKey::new(
            scene_generation,
            render_invalidation_epoch,
            dimensions,
            format,
        );
        if self.last_requested_key == Some(key) {
            return BackgroundEnqueueResult::default();
        }

        self.next_render_generation += 1;
        let render_generation = self.next_render_generation;
        let coalesced_previous = self
            .pending_request
            .replace(BackgroundRenderRequest {
                render_generation,
                scene_generation,
                render_invalidation_epoch,
                scene_graph,
                dimensions,
                format,
                repaint,
                requested_at,
            })
            .is_some();
        self.last_requested_key = Some(key);
        self.requested_scene_generation = Some(scene_generation);
        self.requested_render_invalidation_epoch = Some(render_invalidation_epoch);

        BackgroundEnqueueResult {
            render_generation: Some(render_generation),
            coalesced_previous,
        }
    }

    pub(crate) fn has_unconsumed_rendered_texture(&self) -> bool {
        self.latest_rendered.as_ref().is_some_and(|rendered| {
            Some(rendered.render_generation) != self.consumed_render_generation
        })
    }

    pub(crate) fn take_next_request_if_ready(&mut self) -> Option<BackgroundRenderRequest> {
        if self.stop || self.has_unconsumed_rendered_texture() {
            return None;
        }
        let request = self.pending_request.take()?;
        self.in_progress_generation = Some(request.render_generation);
        Some(request)
    }

    pub(crate) fn is_render_generation_stale(&self, render_generation: u64) -> bool {
        render_generation < self.next_render_generation || self.stop
    }

    pub(crate) fn finish_render_generation(&mut self, render_generation: u64) {
        if self.in_progress_generation == Some(render_generation) {
            self.in_progress_generation = None;
        }
    }

    pub(crate) fn mark_render_consumed(&mut self, render_generation: u64, target_generation: u64) {
        self.consumed_render_generation = Some(render_generation);
        self.front_target_generation = Some(target_generation);
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending_request.is_some()
            || self.in_progress_generation.is_some()
            || self.has_unconsumed_rendered_texture()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct BackgroundRenderShared {
    pub(crate) state: StdMutex<BackgroundRenderState>,
    pub(crate) notify: Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderShared {
    pub(crate) fn new() -> Self {
        Self {
            state: StdMutex::new(BackgroundRenderState::default()),
            notify: Condvar::new(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct BackgroundRenderController {
    pub(crate) shared: Arc<BackgroundRenderShared>,
    pub(crate) worker: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BackgroundRenderWorkerHooks {
    pub(crate) before_set_scene_delay: Duration,
    pub(crate) after_set_scene_delay: Duration,
    pub(crate) after_encode_delay: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
const BEFORE_SET_SCENE_DELAY_ENV: &str = "AVENGER_EGUI_RENDER_DELAY_BEFORE_SET_SCENE_MS";
#[cfg(not(target_arch = "wasm32"))]
const AFTER_SET_SCENE_DELAY_ENV: &str = "AVENGER_EGUI_RENDER_DELAY_AFTER_SET_SCENE_MS";
#[cfg(not(target_arch = "wasm32"))]
const AFTER_ENCODE_DELAY_ENV: &str = "AVENGER_EGUI_RENDER_DELAY_AFTER_ENCODE_MS";

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderWorkerHooks {
    fn from_env() -> Self {
        Self {
            before_set_scene_delay: delay_from_env(BEFORE_SET_SCENE_DELAY_ENV),
            after_set_scene_delay: delay_from_env(AFTER_SET_SCENE_DELAY_ENV),
            after_encode_delay: delay_from_env(AFTER_ENCODE_DELAY_ENV),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_env_values(
        before_set_scene: Option<&str>,
        after_set_scene: Option<&str>,
        after_encode: Option<&str>,
    ) -> Self {
        Self {
            before_set_scene_delay: delay_from_env_value(before_set_scene),
            after_set_scene_delay: delay_from_env_value(after_set_scene),
            after_encode_delay: delay_from_env_value(after_encode),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn delay_from_env(name: &str) -> Duration {
    match std::env::var(name) {
        Ok(value) => delay_from_env_value(Some(value.as_str())),
        Err(_) => Duration::ZERO,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn delay_from_env_value(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::ZERO)
}

#[cfg(not(target_arch = "wasm32"))]
fn sleep_background_render_hook(delay: Duration) {
    if !delay.is_zero() {
        thread::sleep(delay);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderController {
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        canvas_config: CanvasConfig,
        metrics: Arc<StdMutex<CanvasMetrics>>,
    ) -> Self {
        Self::new_with_hooks(
            device,
            queue,
            format,
            canvas_config,
            metrics,
            BackgroundRenderWorkerHooks::from_env(),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_test_hooks(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        canvas_config: CanvasConfig,
        metrics: Arc<StdMutex<CanvasMetrics>>,
        hooks: BackgroundRenderWorkerHooks,
    ) -> Self {
        Self::new_with_hooks(device, queue, format, canvas_config, metrics, hooks)
    }

    fn new_with_hooks(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        canvas_config: CanvasConfig,
        metrics: Arc<StdMutex<CanvasMetrics>>,
        hooks: BackgroundRenderWorkerHooks,
    ) -> Self {
        let shared = Arc::new(BackgroundRenderShared::new());
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("avenger-egui-render-worker".to_string())
            .spawn(move || {
                run_background_render_worker(
                    worker_shared,
                    device,
                    queue,
                    format,
                    canvas_config,
                    metrics.as_ref(),
                    hooks,
                );
            })
            .expect("spawn avenger egui render worker");

        Self {
            shared,
            worker: Some(worker),
        }
    }

    pub(crate) fn enqueue(
        &self,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        render_invalidation_epoch: u64,
        format: wgpu::TextureFormat,
        repaint: Option<egui::Context>,
        metrics: &StdMutex<CanvasMetrics>,
    ) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        let result = state.enqueue_request(
            scene_generation,
            scene_graph,
            dimensions,
            render_invalidation_epoch,
            format,
            repaint,
            StdInstant::now(),
        );
        let Some(render_generation) = result.render_generation else {
            return;
        };
        update_metrics(metrics, |metrics| {
            metrics.background_render_requests += 1;
            if result.coalesced_previous {
                metrics.background_render_requests_coalesced += 1;
            }
        });
        tracing::debug!(
            render_generation,
            scene_generation,
            render_invalidation_epoch,
            "queued background avenger canvas texture render"
        );
        self.shared.notify.notify_one();
    }

    pub(crate) fn set_front_target_generation(&self, generation: Option<u64>) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        state.front_target_generation = generation;
    }

    fn consume_latest(
        &self,
        render_state: &egui_wgpu::RenderState,
        registry: &mut EguiTextureRegistryState,
        metrics: &StdMutex<CanvasMetrics>,
    ) {
        let rendered = {
            let state = self
                .shared
                .state
                .lock()
                .expect("avenger egui render-worker lock poisoned");
            state.latest_rendered.clone()
        };
        let Some(rendered) = rendered else {
            return;
        };
        if registry.consumed_render_generation == Some(rendered.render_generation) {
            return;
        }

        registry.register_rendered_texture(render_state, &rendered, metrics);
        tracing::debug!(
            render_generation = rendered.render_generation,
            scene_generation = rendered.scene_generation,
            texture_generation = rendered.target_generation,
            "consumed background avenger canvas texture on egui thread"
        );

        let mut state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        state.mark_render_consumed(rendered.render_generation, rendered.target_generation);
        self.shared.notify.notify_one();
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned")
            .last_error
            .take()
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.status().render_pending
    }

    pub(crate) fn status(&self) -> BackgroundRenderStatus {
        let state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        BackgroundRenderStatus {
            requested_scene_generation: state.requested_scene_generation,
            requested_render_invalidation_epoch: state.requested_render_invalidation_epoch,
            render_pending: state.is_pending(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for BackgroundRenderController {
    fn drop(&mut self) {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .expect("avenger egui render-worker lock poisoned");
            state.stop = true;
        }
        self.shared.notify.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_background_render_worker(
    shared: Arc<BackgroundRenderShared>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    canvas_config: CanvasConfig,
    metrics: &StdMutex<CanvasMetrics>,
    hooks: BackgroundRenderWorkerHooks,
) {
    let mut renderer: Option<AvengerWgpuRenderer> = None;
    let mut targets: Option<OffscreenTargetPool> = None;

    loop {
        let request = {
            let mut state = shared
                .state
                .lock()
                .expect("avenger egui render-worker lock poisoned");
            loop {
                if state.stop {
                    return;
                }
                if let Some(request) = state.take_next_request_if_ready() {
                    break request;
                }
                state = shared
                    .notify
                    .wait(state)
                    .expect("avenger egui render-worker lock poisoned");
            }
        };

        if background_request_is_stale(&shared, request.render_generation) {
            finish_stale_background_request(&shared, request.render_generation, metrics);
            continue;
        }

        let render_result = render_background_request(
            &shared,
            &device,
            &queue,
            format,
            canvas_config.clone(),
            &mut renderer,
            &mut targets,
            &request,
            hooks,
            metrics,
        );

        match render_result {
            Ok(Some(rendered)) => publish_background_texture(&shared, rendered, metrics),
            Ok(None) => {
                finish_stale_background_request(&shared, request.render_generation, metrics)
            }
            Err(err) => {
                let mut state = shared
                    .state
                    .lock()
                    .expect("avenger egui render-worker lock poisoned");
                state.last_error = Some(err.to_string());
                state.last_requested_key = None;
                state.finish_render_generation(request.render_generation);
                shared.notify.notify_one();
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn render_background_request(
    shared: &Arc<BackgroundRenderShared>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    _default_format: wgpu::TextureFormat,
    canvas_config: CanvasConfig,
    renderer: &mut Option<AvengerWgpuRenderer>,
    targets: &mut Option<OffscreenTargetPool>,
    request: &BackgroundRenderRequest,
    hooks: BackgroundRenderWorkerHooks,
    metrics: &StdMutex<CanvasMetrics>,
) -> Result<Option<RenderedCanvasTexture>, AvengerWgpuError> {
    let format = request.format;
    let renderer = renderer.get_or_insert_with(|| {
        AvengerWgpuRenderer::new(
            device,
            AvengerRendererConfig::new(request.dimensions, format)
                .with_sample_count(1)
                .with_canvas_config(canvas_config),
        )
    });
    renderer.resize(request.dimensions);

    sleep_background_render_hook(hooks.before_set_scene_delay);
    let set_scene_start = StdInstant::now();
    let queue_wait = set_scene_start.duration_since(request.requested_at);
    renderer.set_scene(device, queue, &request.scene_graph)?;
    let set_scene_elapsed = set_scene_start.elapsed();
    sleep_background_render_hook(hooks.after_set_scene_delay);
    if background_request_is_stale(shared, request.render_generation) {
        return Ok(None);
    }

    let descriptor = OffscreenTargetDescriptor::new(request.dimensions, format)
        .with_label("avenger-egui background canvas offscreen texture");
    let targets = targets
        .get_or_insert_with(|| OffscreenTargetPool::triple_buffered(device, descriptor.clone()));
    targets.resize_or_recreate(device, descriptor);

    let front_target_generation = shared
        .state
        .lock()
        .expect("avenger egui render-worker lock poisoned")
        .front_target_generation;
    let target =
        if let Some(target) = targets.acquire_next_excluding_generation(front_target_generation) {
            target
        } else {
            targets.acquire_next()
        };

    let encode_start = StdInstant::now();
    let commands = renderer.encode_to_offscreen_commands(device, queue, target)?;
    let encode_elapsed = encode_start.elapsed();
    sleep_background_render_hook(hooks.after_encode_delay);
    if background_request_is_stale(shared, request.render_generation) {
        return Ok(None);
    }

    let submit_start = StdInstant::now();
    queue.submit(commands);
    let submit_elapsed = submit_start.elapsed();
    let rendered = RenderedOffscreenFrame::from(&*target);
    let view = target.view.clone();
    update_metrics(metrics, |metrics| {
        metrics.offscreen_texture_renders += 1;
        metrics.background_render_frames_submitted += 1;
        metrics.last_background_queue_wait_us = duration_us(queue_wait);
        metrics.last_set_scene_us = duration_us(set_scene_elapsed);
        metrics.last_command_encode_us = duration_us(encode_elapsed);
        metrics.last_submit_us = duration_us(submit_elapsed);
    });
    tracing::debug!(
        render_generation = request.render_generation,
        scene_generation = request.scene_generation,
        texture_generation = rendered.generation,
        queue_wait_us = duration_us(queue_wait),
        set_scene_us = duration_us(set_scene_elapsed),
        command_encode_us = duration_us(encode_elapsed),
        submit_us = duration_us(submit_elapsed),
        "submitted background avenger canvas texture render"
    );
    Ok(Some(RenderedCanvasTexture {
        render_generation: request.render_generation,
        scene_generation: request.scene_generation,
        render_invalidation_epoch: request.render_invalidation_epoch,
        target_generation: rendered.generation,
        dimensions: request.dimensions,
        view,
        repaint: request.repaint.clone(),
        requested_at: request.requested_at,
    }))
}

#[cfg(not(target_arch = "wasm32"))]
fn background_request_is_stale(
    shared: &Arc<BackgroundRenderShared>,
    render_generation: u64,
) -> bool {
    let state = shared
        .state
        .lock()
        .expect("avenger egui render-worker lock poisoned");
    state.is_render_generation_stale(render_generation)
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_background_texture(
    shared: &Arc<BackgroundRenderShared>,
    rendered: RenderedCanvasTexture,
    metrics: &StdMutex<CanvasMetrics>,
) {
    let mut state = shared
        .state
        .lock()
        .expect("avenger egui render-worker lock poisoned");
    if state.is_render_generation_stale(rendered.render_generation) {
        state.finish_render_generation(rendered.render_generation);
        drop(state);
        update_metrics(metrics, |metrics| {
            metrics.stale_background_render_frames_dropped += 1;
        });
        return;
    }

    let scene_to_texture_publish = rendered.requested_at.elapsed();
    let render_generation = rendered.render_generation;
    let scene_generation = rendered.scene_generation;
    let repaint = rendered.repaint.clone();
    state.latest_rendered = Some(Arc::new(rendered));
    state.in_progress_generation = None;
    state.consumed_render_generation = None;
    update_metrics(metrics, |metrics| {
        metrics.background_render_frames_published += 1;
        metrics.last_scene_to_texture_publish_us = duration_us(scene_to_texture_publish);
    });
    tracing::debug!(
        render_generation,
        scene_generation,
        scene_to_texture_publish_us = duration_us(scene_to_texture_publish),
        "published background avenger canvas texture"
    );
    drop(state);
    if let Some(ctx) = repaint {
        ctx.request_repaint();
    }
    shared.notify.notify_one();
}

#[cfg(not(target_arch = "wasm32"))]
fn finish_stale_background_request(
    shared: &Arc<BackgroundRenderShared>,
    render_generation: u64,
    metrics: &StdMutex<CanvasMetrics>,
) {
    let mut state = shared
        .state
        .lock()
        .expect("avenger egui render-worker lock poisoned");
    state.finish_render_generation(render_generation);
    drop(state);
    update_metrics(metrics, |metrics| {
        metrics.stale_background_render_frames_dropped += 1;
    });
    tracing::debug!(
        render_generation,
        "dropped stale background avenger canvas texture render"
    );
    shared.notify.notify_one();
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameStatus {
    pub latest_generation: Option<u64>,
    pub latest_scene_generation: Option<u64>,
    pub requested_generation: Option<u64>,
    pub render_pending: bool,
    pub latest_texture_scene_generation: Option<u64>,
    pub requested_texture_scene_generation: Option<u64>,
    pub latest_render_invalidation_epoch: Option<u64>,
    pub requested_render_invalidation_epoch: Option<u64>,
    pub gpu_render_pending: bool,
    pub texture_render_mode: TextureRenderMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureRenderMode {
    #[default]
    Uninitialized,
    UiThreadGpu,
    BackgroundGpu,
}

impl TextureRenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::UiThreadGpu => "ui-thread-gpu",
            Self::BackgroundGpu => "background-gpu",
        }
    }
}
