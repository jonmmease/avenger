use std::{
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant as StdInstant},
};

#[cfg(not(target_arch = "wasm32"))]
use std::{
    sync::Condvar,
    thread::{self, JoinHandle},
};

use avenger_app::{
    app::{AppUpdate, AvengerApp},
    error::AvengerAppError,
};
use avenger_chart_app::{
    ChartAppState, IntoChartParamValue, ParamChange, ParamSetResult, ParamSnapshot,
};
use avenger_common::{canvas::CanvasDimensions, time::Instant as AvengerInstant};
use avenger_eventstream::window::{
    CanvasResizeEvent, ElementState, Key, MouseButton, MouseScrollDelta, NamedKey,
    WindowCursorMoved, WindowEvent, WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::{
    error::AvengerWgpuError,
    frame_publisher::{
        BeginFrameError, FrameGeneration, FramePublisher, FramePublisherStatus, FrameRenderMetrics,
        PublishResult, RenderedFrame,
    },
    offscreen::{OffscreenTargetDescriptor, OffscreenTargetPool, RenderedOffscreenFrame},
    renderer::{AvengerRendererConfig, AvengerWgpuRenderer},
};
use datafusion::scalar::ScalarValue;
use egui_wgpu::wgpu;
use tokio::sync::Mutex as AsyncMutex;

pub use egui;

#[derive(Clone)]
pub struct AvengerPlotHandle {
    state: ChartAppState,
    app: Option<Arc<AsyncMutex<AvengerApp<ChartAppState>>>>,
    observed: Arc<StdMutex<ObservedState>>,
    event_translator: Arc<StdMutex<EguiEventTranslator>>,
    pending_events: Arc<StdMutex<Vec<WindowEvent>>>,
    scene_rebuild_request: Arc<StdMutex<Option<bool>>>,
    scene_publisher: FramePublisher<Arc<SceneGraph>>,
    scene_error: Arc<StdMutex<Option<String>>>,
    metrics: Arc<StdMutex<PlotMetrics>>,
    gpu: Arc<StdMutex<Option<EguiPlotGpuState>>>,
}

impl AvengerPlotHandle {
    pub fn new(state: ChartAppState) -> Self {
        Self {
            state,
            app: None,
            observed: Arc::new(StdMutex::new(ObservedState::default())),
            event_translator: Arc::new(StdMutex::new(EguiEventTranslator::default())),
            pending_events: Arc::new(StdMutex::new(Vec::new())),
            scene_rebuild_request: Arc::new(StdMutex::new(None)),
            scene_publisher: FramePublisher::new(),
            scene_error: Arc::new(StdMutex::new(None)),
            metrics: Arc::new(StdMutex::new(PlotMetrics::default())),
            gpu: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn from_app(mut app: AvengerApp<ChartAppState>) -> Self {
        let state = app.app_state_mut().clone();
        Self {
            state,
            app: Some(Arc::new(AsyncMutex::new(app))),
            observed: Arc::new(StdMutex::new(ObservedState::default())),
            event_translator: Arc::new(StdMutex::new(EguiEventTranslator::default())),
            pending_events: Arc::new(StdMutex::new(Vec::new())),
            scene_rebuild_request: Arc::new(StdMutex::new(None)),
            scene_publisher: FramePublisher::new(),
            scene_error: Arc::new(StdMutex::new(None)),
            metrics: Arc::new(StdMutex::new(PlotMetrics::default())),
            gpu: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn chart_state(&self) -> &ChartAppState {
        &self.state
    }

    pub fn set_param(
        &self,
        name: impl Into<String>,
        value: impl IntoChartParamValue,
    ) -> ParamSetResult {
        let name = name.into();
        let _span = tracing::debug_span!("avenger_egui.set_param", param = %name).entered();
        let result = self.state.set_param(name, value);
        update_metrics(&self.metrics, |metrics| {
            metrics.param_set_calls += 1;
            if result.changed {
                metrics.param_changes_enqueued += 1;
            }
        });
        tracing::debug!(
            changed = result.changed,
            revision = result.revision,
            "plot param set"
        );
        result
    }

    pub fn param_snapshot(&self) -> ParamSnapshot {
        self.state.param_snapshot()
    }

    pub fn param_revision(&self) -> u64 {
        self.state.param_revision()
    }

    pub fn param_changes_since(&self, revision: u64) -> Vec<ParamChange> {
        self.state.param_changes_since(revision)
    }

    pub fn param_f64(&self, name: &str) -> Option<f64> {
        self.state.param_f64(name)
    }

    pub fn param_bool(&self, name: &str) -> Option<bool> {
        self.state.param_bool(name)
    }

    pub fn queue_events(&self, events: impl IntoIterator<Item = WindowEvent>) {
        let events: Vec<_> = events.into_iter().collect();
        let count = events.len() as u64;
        if count > 0 {
            update_metrics(&self.metrics, |metrics| {
                metrics.routed_event_batches += 1;
                metrics.routed_events += count;
                metrics.last_routed_event_count = count;
            });
            tracing::debug!(event_count = count, "queued avenger plot events");
        }
        self.pending_events
            .lock()
            .expect("avenger egui pending-event lock poisoned")
            .extend(events);
    }

    pub fn take_pending_events(&self) -> Vec<WindowEvent> {
        std::mem::take(
            &mut *self
                .pending_events
                .lock()
                .expect("avenger egui pending-event lock poisoned"),
        )
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending_events
            .lock()
            .expect("avenger egui pending-event lock poisoned")
            .len()
    }

    pub async fn dispatch_pending_events(&self) -> Result<Vec<AppUpdate>, AvengerAppError> {
        let events = self.take_pending_events();
        let Some(app) = &self.app else {
            return Ok(Vec::new());
        };

        let mut updates = Vec::with_capacity(events.len());
        let mut app = app.lock().await;
        for event in events {
            updates.push(
                app.update_with_status(&event, AvengerInstant::now())
                    .await?,
            );
        }
        Ok(updates)
    }

    pub fn request_scene_rebuild(
        &self,
        runtime: &tokio::runtime::Handle,
        rebuild_geometry: bool,
    ) -> Option<FrameGeneration> {
        self.request_scene_rebuild_inner(runtime, rebuild_geometry, None)
    }

    pub fn request_scene_rebuild_with_repaint(
        &self,
        runtime: &tokio::runtime::Handle,
        ctx: &egui::Context,
        rebuild_geometry: bool,
    ) -> Option<FrameGeneration> {
        self.request_scene_rebuild_inner(runtime, rebuild_geometry, Some(ctx.clone()))
    }

    fn request_scene_rebuild_inner(
        &self,
        runtime: &tokio::runtime::Handle,
        rebuild_geometry: bool,
        repaint: Option<egui::Context>,
    ) -> Option<FrameGeneration> {
        self.app.as_ref()?;
        let generation = self.scene_publisher.request_frame();
        update_metrics(&self.metrics, |metrics| {
            metrics.scene_rebuild_requests += 1;
            metrics.last_requested_generation = Some(generation.get());
        });
        tracing::debug!(
            generation = generation.get(),
            rebuild_geometry,
            "requested avenger scene rebuild"
        );
        merge_scene_rebuild_request(&self.scene_rebuild_request, rebuild_geometry);
        self.spawn_scene_worker(runtime.clone(), repaint);
        Some(generation)
    }

    pub fn request_event_dispatch(
        &self,
        runtime: &tokio::runtime::Handle,
    ) -> Option<FrameGeneration> {
        self.request_event_dispatch_inner(runtime, None)
    }

    pub fn request_event_dispatch_with_repaint(
        &self,
        runtime: &tokio::runtime::Handle,
        ctx: &egui::Context,
    ) -> Option<FrameGeneration> {
        self.request_event_dispatch_inner(runtime, Some(ctx.clone()))
    }

    fn request_event_dispatch_inner(
        &self,
        runtime: &tokio::runtime::Handle,
        repaint: Option<egui::Context>,
    ) -> Option<FrameGeneration> {
        self.app.as_ref()?;
        if self.pending_event_count() == 0 {
            return None;
        }
        let generation = self.scene_publisher.request_frame();
        update_metrics(&self.metrics, |metrics| {
            metrics.event_dispatch_requests += 1;
            metrics.last_requested_generation = Some(generation.get());
        });
        tracing::debug!(
            generation = generation.get(),
            "requested avenger event dispatch"
        );
        self.spawn_scene_worker(runtime.clone(), repaint);
        Some(generation)
    }

    pub fn latest_scene_frame(&self) -> Option<Arc<RenderedFrame<Arc<SceneGraph>>>> {
        self.scene_publisher.latest_snapshot()
    }

    pub fn latest_scene_graph(&self) -> Option<Arc<SceneGraph>> {
        self.latest_scene_frame().map(|frame| frame.payload.clone())
    }

    pub fn scene_frame_status(&self) -> FramePublisherStatus {
        self.scene_publisher.status()
    }

    pub fn latest_scene_error(&self) -> Option<String> {
        self.scene_error
            .lock()
            .expect("avenger egui scene-error lock poisoned")
            .clone()
    }

    pub fn metrics(&self) -> PlotMetrics {
        self.metrics
            .lock()
            .expect("avenger egui metrics lock poisoned")
            .clone()
    }

    pub fn reset_metrics(&self) {
        *self
            .metrics
            .lock()
            .expect("avenger egui metrics lock poisoned") = PlotMetrics::default();
    }

    pub fn show_metrics(&self, ui: &mut egui::Ui) {
        let metrics = self.metrics();
        let status = self.frame_status();
        ui.label(format!("frames painted: {}", metrics.frames_painted));
        ui.label(format!(
            "reused latest frame paints: {}",
            metrics.reused_latest_frame_paints
        ));
        ui.label(format!(
            "pending-frame paints: {}",
            metrics.frames_painted_while_pending
        ));
        ui.label(format!(
            "scene published/dropped: {}/{}",
            metrics.scene_frames_published, metrics.stale_scene_frames_dropped
        ));
        ui.label(format!(
            "gpu submitted/published/consumed: {}/{}/{}",
            metrics.background_render_frames_submitted,
            metrics.background_render_frames_published,
            metrics.background_render_frames_consumed
        ));
        ui.label(format!(
            "gpu requests/coalesced/dropped: {}/{}/{}",
            metrics.background_render_requests,
            metrics.background_render_requests_coalesced,
            metrics.stale_background_render_frames_dropped
        ));
        ui.label(format!(
            "events routed: {} in {} batches",
            metrics.routed_events, metrics.routed_event_batches
        ));
        ui.label(format!(
            "set_scene/encode/submit us: {}/{}/{}",
            metrics.last_set_scene_us, metrics.last_command_encode_us, metrics.last_submit_us
        ));
        ui.label(format!(
            "scene eval / texture publish us: {} / {}",
            metrics.last_scene_evaluation_us, metrics.last_texture_publish_us
        ));
        ui.label(format!(
            "scene-to-texture publish us: {}",
            metrics.last_scene_to_texture_publish_us
        ));
        ui.label(format!(
            "gpu queue wait us: {}",
            metrics.last_background_queue_wait_us
        ));
        ui.label(format!(
            "texture render mode: {}",
            status.texture_render_mode.as_str()
        ));
        ui.label(format!(
            "latest texture/scene: {:?}/{:?}",
            status.latest_generation, status.latest_scene_generation
        ));
    }

    pub async fn current_scene_graph(&self) -> Option<Arc<SceneGraph>> {
        let app = self.app.as_ref()?;
        Some(app.lock().await.scene_graph_arc())
    }

    pub async fn rebuild_scene_graph(
        &self,
        rebuild_geometry: bool,
    ) -> Result<Option<Arc<SceneGraph>>, AvengerAppError> {
        let Some(app) = &self.app else {
            return Ok(None);
        };
        let mut app = app.lock().await;
        app.rebuild_scene_graph(rebuild_geometry).await.map(Some)
    }

    pub fn render_scene_to_texture(
        &self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        let mut gpu = self.gpu.lock().expect("avenger egui gpu lock poisoned");
        let gpu = gpu.get_or_insert_with(|| {
            EguiPlotGpuState::new(
                &render_state.device,
                dimensions,
                wgpu::TextureFormat::Rgba8Unorm,
            )
        });
        gpu.render_scene(render_state, scene_graph, dimensions, &self.metrics)
    }

    pub fn request_background_scene_texture(
        &self,
        render_state: &egui_wgpu::RenderState,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        self.request_background_scene_texture_inner(
            render_state,
            None,
            scene_generation,
            scene_graph,
            dimensions,
        )
    }

    pub fn request_background_scene_texture_with_repaint(
        &self,
        render_state: &egui_wgpu::RenderState,
        ctx: &egui::Context,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        self.request_background_scene_texture_inner(
            render_state,
            Some(ctx.clone()),
            scene_generation,
            scene_graph,
            dimensions,
        )
    }

    fn request_background_scene_texture_inner(
        &self,
        render_state: &egui_wgpu::RenderState,
        repaint: Option<egui::Context>,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        let mut gpu = self.gpu.lock().expect("avenger egui gpu lock poisoned");
        let gpu = gpu.get_or_insert_with(|| {
            EguiPlotGpuState::new(
                &render_state.device,
                dimensions,
                wgpu::TextureFormat::Rgba8Unorm,
            )
        });
        gpu.request_background_scene_texture(
            render_state,
            scene_generation,
            scene_graph,
            dimensions,
            repaint,
            self.metrics.clone(),
        )
    }

    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.gpu
            .lock()
            .expect("avenger egui gpu lock poisoned")
            .as_ref()
            .and_then(EguiPlotGpuState::texture_id)
    }

    pub fn frame_status(&self) -> FrameStatus {
        let mut status = self
            .gpu
            .lock()
            .expect("avenger egui gpu lock poisoned")
            .as_ref()
            .map(EguiPlotGpuState::frame_status)
            .unwrap_or_default();
        let scene_status = self.scene_publisher.status();
        status.latest_scene_generation = scene_status
            .latest_published_generation
            .map(FrameGeneration::get);
        status.requested_generation = (scene_status.requested_generation != FrameGeneration::ZERO)
            .then_some(scene_status.requested_generation.get());
        status.render_pending =
            scene_status.in_progress_generation.is_some() || status.gpu_render_pending;
        status
    }

    fn param_changes_since_last_show(&self) -> Vec<ParamChange> {
        let current_revision = self.state.param_revision();
        let mut observed = self
            .observed
            .lock()
            .expect("avenger egui observed-state lock poisoned");
        let changes = self.state.param_changes_since(observed.param_revision);
        observed.param_revision = current_revision;
        changes
    }

    fn spawn_scene_worker(&self, runtime: tokio::runtime::Handle, repaint: Option<egui::Context>) {
        SceneWorkerParts::from_handle(self).spawn(runtime, repaint);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlotMetrics {
    pub param_set_calls: u64,
    pub param_changes_enqueued: u64,
    pub routed_event_batches: u64,
    pub routed_events: u64,
    pub last_routed_event_count: u64,
    pub scene_rebuild_requests: u64,
    pub event_dispatch_requests: u64,
    pub scene_frames_published: u64,
    pub stale_scene_frames_dropped: u64,
    pub background_render_requests: u64,
    pub background_render_requests_coalesced: u64,
    pub background_render_frames_submitted: u64,
    pub background_render_frames_published: u64,
    pub background_render_frames_consumed: u64,
    pub stale_background_render_frames_dropped: u64,
    pub offscreen_texture_renders: u64,
    pub texture_registrations: u64,
    pub texture_updates: u64,
    pub frames_painted: u64,
    pub reused_latest_frame_paints: u64,
    pub frames_painted_while_pending: u64,
    pub last_requested_generation: Option<u64>,
    pub last_published_scene_generation: Option<u64>,
    pub last_painted_texture_generation: Option<u64>,
    pub last_scene_evaluation_us: u64,
    pub last_set_scene_us: u64,
    pub last_command_encode_us: u64,
    pub last_submit_us: u64,
    pub last_texture_publish_us: u64,
    pub last_scene_to_texture_publish_us: u64,
    pub last_background_queue_wait_us: u64,
}

fn update_metrics(metrics: &StdMutex<PlotMetrics>, update: impl FnOnce(&mut PlotMetrics)) {
    update(&mut metrics.lock().expect("avenger egui metrics lock poisoned"));
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Debug, Default)]
struct ObservedState {
    param_revision: u64,
}

#[derive(Clone)]
struct SceneWorkerParts {
    app: Option<Arc<AsyncMutex<AvengerApp<ChartAppState>>>>,
    pending_events: Arc<StdMutex<Vec<WindowEvent>>>,
    scene_rebuild_request: Arc<StdMutex<Option<bool>>>,
    scene_publisher: FramePublisher<Arc<SceneGraph>>,
    scene_error: Arc<StdMutex<Option<String>>>,
    metrics: Arc<StdMutex<PlotMetrics>>,
}

impl SceneWorkerParts {
    fn from_handle(handle: &AvengerPlotHandle) -> Self {
        Self {
            app: handle.app.clone(),
            pending_events: handle.pending_events.clone(),
            scene_rebuild_request: handle.scene_rebuild_request.clone(),
            scene_publisher: handle.scene_publisher.clone(),
            scene_error: handle.scene_error.clone(),
            metrics: handle.metrics.clone(),
        }
    }

    fn spawn(self, runtime: tokio::runtime::Handle, repaint: Option<egui::Context>) {
        let _span = tracing::debug_span!("avenger_egui.spawn_scene_worker").entered();
        let ticket = match self.scene_publisher.try_begin_latest_render() {
            Ok(ticket) => ticket,
            Err(BeginFrameError::RenderInProgress { .. }) => return,
            Err(BeginFrameError::Stale { .. }) => return,
        };
        let generation = ticket.generation();
        let next = self.clone();
        let task_runtime = runtime.clone();

        runtime.spawn(async move {
            tracing::debug!(generation = generation.get(), "scene worker start");
            let start = StdInstant::now();
            let result = self.run_once().await;
            match result {
                Ok(Some(scene_graph)) => {
                    *self
                        .scene_error
                        .lock()
                        .expect("avenger egui scene-error lock poisoned") = None;
                    let scene_evaluation = start.elapsed();
                    let publish_result = ticket.publish(
                        scene_graph,
                        FrameRenderMetrics {
                            scene_evaluation,
                            ..FrameRenderMetrics::default()
                        },
                    );
                    match publish_result {
                        PublishResult::Published { generation } => {
                            update_metrics(&self.metrics, |metrics| {
                                metrics.scene_frames_published += 1;
                                metrics.last_published_scene_generation = Some(generation.get());
                                metrics.last_scene_evaluation_us = duration_us(scene_evaluation);
                            });
                            tracing::debug!(
                                generation = generation.get(),
                                scene_evaluation_us = duration_us(scene_evaluation),
                                "published avenger scene frame"
                            );
                        }
                        PublishResult::DroppedStale {
                            generation,
                            requested_generation,
                        } => {
                            update_metrics(&self.metrics, |metrics| {
                                metrics.stale_scene_frames_dropped += 1;
                            });
                            tracing::debug!(
                                generation = generation.get(),
                                requested_generation = requested_generation.get(),
                                "dropped stale avenger scene frame"
                            );
                        }
                        PublishResult::Canceled { .. } => {}
                    }
                }
                Ok(None) => {
                    let _ = ticket.cancel();
                }
                Err(err) => {
                    *self
                        .scene_error
                        .lock()
                        .expect("avenger egui scene-error lock poisoned") = Some(err.to_string());
                    let _ = ticket.cancel();
                }
            }

            if let Some(ctx) = &repaint {
                ctx.request_repaint();
            }

            if next.scene_publisher.requested_generation() > generation {
                merge_scene_rebuild_request(&next.scene_rebuild_request, true);
            }
            if next.has_more_work_after(generation) {
                next.spawn(task_runtime, repaint);
            }
        });
    }

    async fn run_once(&self) -> Result<Option<Arc<SceneGraph>>, AvengerAppError> {
        let events = std::mem::take(
            &mut *self
                .pending_events
                .lock()
                .expect("avenger egui pending-event lock poisoned"),
        );
        let rebuild_request = self
            .scene_rebuild_request
            .lock()
            .expect("avenger egui scene-rebuild lock poisoned")
            .take();

        if events.is_empty() && rebuild_request.is_none() {
            return Ok(None);
        }

        let Some(app) = &self.app else {
            return Ok(None);
        };

        let mut app = app.lock().await;
        let mut scene_graph = None;
        for event in events {
            tracing::debug!("dispatching plot event through avenger app");
            if let Some(update_scene) = app
                .update_with_status(&event, AvengerInstant::now())
                .await?
                .scene_graph
            {
                scene_graph = Some(update_scene);
            }
        }

        match rebuild_request {
            Some(true) => {
                scene_graph = Some(app.rebuild_scene_graph(true).await?);
            }
            Some(false) if scene_graph.is_none() => {
                scene_graph = Some(app.rebuild_scene_graph(false).await?);
            }
            _ => {}
        }

        Ok(scene_graph)
    }

    fn has_more_work_after(&self, generation: FrameGeneration) -> bool {
        if self.scene_publisher.requested_generation() > generation {
            return true;
        }
        if !self
            .pending_events
            .lock()
            .expect("avenger egui pending-event lock poisoned")
            .is_empty()
        {
            return true;
        }
        self.scene_rebuild_request
            .lock()
            .expect("avenger egui scene-rebuild lock poisoned")
            .is_some()
    }
}

fn merge_scene_rebuild_request(request: &StdMutex<Option<bool>>, rebuild_geometry: bool) {
    let mut request = request
        .lock()
        .expect("avenger egui scene-rebuild lock poisoned");
    *request = Some(request.unwrap_or(false) || rebuild_geometry);
}

pub struct Plot<'a> {
    handle: &'a AvengerPlotHandle,
    desired_size: Option<egui::Vec2>,
    sense: egui::Sense,
}

impl<'a> Plot<'a> {
    pub fn new(handle: &'a AvengerPlotHandle) -> Self {
        Self {
            handle,
            desired_size: None,
            sense: egui::Sense::click_and_drag(),
        }
    }

    pub fn desired_size(mut self, size: egui::Vec2) -> Self {
        self.desired_size = Some(size);
        self
    }

    pub fn sense(mut self, sense: egui::Sense) -> Self {
        self.sense = sense;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> PlotOutput {
        let _span = tracing::debug_span!("avenger_egui.plot_show").entered();
        let desired_size = self.desired_size.unwrap_or_else(|| {
            let available = ui.available_size_before_wrap();
            egui::vec2(available.x.max(320.0), available.y.max(240.0))
        });
        let (rect, mut response) = ui.allocate_exact_size(desired_size, self.sense);
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let frame_status = self.handle.frame_status();
        if let Some(texture_id) = self.handle.texture_id() {
            ui.painter().image(
                texture_id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            update_metrics(&self.handle.metrics, |metrics| {
                metrics.frames_painted += 1;
                if frame_status.render_pending {
                    metrics.frames_painted_while_pending += 1;
                }
                if let Some(generation) = frame_status.latest_generation {
                    if metrics.last_painted_texture_generation == Some(generation) {
                        metrics.reused_latest_frame_paints += 1;
                    }
                    metrics.last_painted_texture_generation = Some(generation);
                }
            });
            tracing::debug!(
                texture_generation = frame_status.latest_generation,
                scene_generation = frame_status.latest_scene_generation,
                render_pending = frame_status.render_pending,
                "painted avenger plot texture"
            );
        }

        let response_state = EguiResponseState::from_response(&response);
        let events = ui.input(|input| {
            self.handle
                .event_translator
                .lock()
                .expect("avenger egui event-translator lock poisoned")
                .translate_frame(rect, response_state, input)
        });
        self.handle.queue_events(events.iter().cloned());
        let param_changes = self.handle.param_changes_since_last_show();
        let selection_changes = Vec::new();
        if !param_changes.is_empty() || !selection_changes.is_empty() {
            response.mark_changed();
        }

        PlotOutput {
            response,
            param_changes,
            selection_changes,
            frame_status,
            events,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TextureRenderStatus {
    pub texture_id: Option<egui::TextureId>,
    pub texture_generation: Option<u64>,
    pub scene_generation: Option<u64>,
    pub dimensions: Option<CanvasDimensions>,
    pub render_pending: bool,
}

struct EguiTextureRegistryState {
    texture_id: Option<egui::TextureId>,
    registered_generation: Option<u64>,
    registered_scene_generation: Option<u64>,
    registered_dimensions: Option<CanvasDimensions>,
    consumed_render_generation: Option<u64>,
}

impl EguiTextureRegistryState {
    fn new() -> Self {
        Self {
            texture_id: None,
            registered_generation: None,
            registered_scene_generation: None,
            registered_dimensions: None,
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
        metrics: &StdMutex<PlotMetrics>,
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
        rendered: &RenderedPlotTexture,
        metrics: &StdMutex<PlotMetrics>,
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
            metrics,
        );
        self.consumed_render_generation = Some(rendered.render_generation);
        update_metrics(metrics, |metrics| {
            metrics.background_render_frames_consumed += 1;
        });
        Some(texture_id)
    }

    fn status(&self) -> TextureRenderStatus {
        TextureRenderStatus {
            texture_id: self.texture_id,
            texture_generation: self.registered_generation,
            scene_generation: self.registered_scene_generation,
            dimensions: self.registered_dimensions,
            render_pending: false,
        }
    }
}

struct EguiPlotGpuState {
    sync: Option<EguiPlotSyncGpuState>,
    #[cfg(not(target_arch = "wasm32"))]
    background: Option<BackgroundRenderController>,
    registry: EguiTextureRegistryState,
    format: wgpu::TextureFormat,
}

impl EguiPlotGpuState {
    fn new(
        _device: &wgpu::Device,
        _dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            sync: None,
            #[cfg(not(target_arch = "wasm32"))]
            background: None,
            registry: EguiTextureRegistryState::new(),
            format,
        }
    }

    fn render_scene(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
        metrics: &StdMutex<PlotMetrics>,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        let sync = self.sync.get_or_insert_with(|| {
            EguiPlotSyncGpuState::new(&render_state.device, dimensions, self.format)
        });
        sync.render_scene(
            render_state,
            scene_graph,
            dimensions,
            &mut self.registry,
            metrics,
        )
    }

    fn request_background_scene_texture(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        repaint: Option<egui::Context>,
        metrics: Arc<StdMutex<PlotMetrics>>,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        #[cfg(target_arch = "wasm32")]
        {
            self.render_scene(render_state, &scene_graph, dimensions, metrics.as_ref())?;
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
                    metrics.clone(),
                )
            });

            controller.set_front_target_generation(self.registry.registered_generation);
            controller.consume_latest(render_state, &mut self.registry, metrics.as_ref());
            controller.enqueue(
                scene_generation,
                scene_graph,
                dimensions,
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

    fn texture_id(&self) -> Option<egui::TextureId> {
        self.registry.texture_id
    }

    fn frame_status(&self) -> FrameStatus {
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
            gpu_render_pending: background_status.render_pending,
            texture_render_mode,
        }
    }
}

struct EguiPlotSyncGpuState {
    renderer: AvengerWgpuRenderer,
    targets: OffscreenTargetPool,
    format: wgpu::TextureFormat,
}

impl EguiPlotSyncGpuState {
    fn new(
        device: &wgpu::Device,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
    ) -> Self {
        let renderer = AvengerWgpuRenderer::new(
            device,
            AvengerRendererConfig::new(dimensions, format).with_sample_count(1),
        );
        let targets = OffscreenTargetPool::triple_buffered(
            device,
            OffscreenTargetDescriptor::new(dimensions, format)
                .with_label("avenger-egui plot offscreen texture"),
        );
        Self {
            renderer,
            targets,
            format,
        }
    }

    fn render_scene(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
        registry: &mut EguiTextureRegistryState,
        metrics: &StdMutex<PlotMetrics>,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        self.renderer.resize(dimensions);
        let set_scene_start = StdInstant::now();
        self.renderer
            .set_scene(&render_state.device, &render_state.queue, scene_graph)?;
        let set_scene_elapsed = set_scene_start.elapsed();
        self.targets.resize_or_recreate(
            &render_state.device,
            OffscreenTargetDescriptor::new(dimensions, self.format)
                .with_label("avenger-egui plot offscreen texture"),
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
struct RenderRequestKey {
    scene_generation: u64,
    width_bits: u32,
    height_bits: u32,
    scale_bits: u32,
    format: wgpu::TextureFormat,
}

impl RenderRequestKey {
    fn new(
        scene_generation: u64,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            scene_generation,
            width_bits: dimensions.size[0].to_bits(),
            height_bits: dimensions.size[1].to_bits(),
            scale_bits: dimensions.scale.to_bits(),
            format,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct BackgroundRenderRequest {
    render_generation: u64,
    scene_generation: u64,
    scene_graph: Arc<SceneGraph>,
    dimensions: CanvasDimensions,
    format: wgpu::TextureFormat,
    repaint: Option<egui::Context>,
    requested_at: StdInstant,
}

#[cfg(not(target_arch = "wasm32"))]
struct RenderedPlotTexture {
    render_generation: u64,
    scene_generation: u64,
    target_generation: u64,
    dimensions: CanvasDimensions,
    view: wgpu::TextureView,
    repaint: Option<egui::Context>,
    requested_at: StdInstant,
}

#[derive(Clone, Copy, Debug, Default)]
struct BackgroundRenderStatus {
    requested_scene_generation: Option<u64>,
    render_pending: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct BackgroundRenderState {
    next_render_generation: u64,
    requested_scene_generation: Option<u64>,
    last_requested_key: Option<RenderRequestKey>,
    pending_request: Option<BackgroundRenderRequest>,
    latest_rendered: Option<Arc<RenderedPlotTexture>>,
    consumed_render_generation: Option<u64>,
    front_target_generation: Option<u64>,
    in_progress_generation: Option<u64>,
    last_error: Option<String>,
    stop: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BackgroundEnqueueResult {
    render_generation: Option<u64>,
    coalesced_previous: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderState {
    fn enqueue_request(
        &mut self,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
        repaint: Option<egui::Context>,
        requested_at: StdInstant,
    ) -> BackgroundEnqueueResult {
        let key = RenderRequestKey::new(scene_generation, dimensions, format);
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
                scene_graph,
                dimensions,
                format,
                repaint,
                requested_at,
            })
            .is_some();
        self.last_requested_key = Some(key);
        self.requested_scene_generation = Some(scene_generation);

        BackgroundEnqueueResult {
            render_generation: Some(render_generation),
            coalesced_previous,
        }
    }

    fn has_unconsumed_rendered_texture(&self) -> bool {
        self.latest_rendered.as_ref().is_some_and(|rendered| {
            Some(rendered.render_generation) != self.consumed_render_generation
        })
    }

    fn take_next_request_if_ready(&mut self) -> Option<BackgroundRenderRequest> {
        if self.stop || self.has_unconsumed_rendered_texture() {
            return None;
        }
        let request = self.pending_request.take()?;
        self.in_progress_generation = Some(request.render_generation);
        Some(request)
    }

    fn is_render_generation_stale(&self, render_generation: u64) -> bool {
        render_generation < self.next_render_generation || self.stop
    }

    fn finish_render_generation(&mut self, render_generation: u64) {
        if self.in_progress_generation == Some(render_generation) {
            self.in_progress_generation = None;
        }
    }

    fn mark_render_consumed(&mut self, render_generation: u64, target_generation: u64) {
        self.consumed_render_generation = Some(render_generation);
        self.front_target_generation = Some(target_generation);
    }

    fn is_pending(&self) -> bool {
        self.pending_request.is_some()
            || self.in_progress_generation.is_some()
            || self.has_unconsumed_rendered_texture()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct BackgroundRenderShared {
    state: StdMutex<BackgroundRenderState>,
    notify: Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderShared {
    fn new() -> Self {
        Self {
            state: StdMutex::new(BackgroundRenderState::default()),
            notify: Condvar::new(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct BackgroundRenderController {
    shared: Arc<BackgroundRenderShared>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundRenderController {
    fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        metrics: Arc<StdMutex<PlotMetrics>>,
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
                    metrics.as_ref(),
                );
            })
            .expect("spawn avenger egui render worker");

        Self {
            shared,
            worker: Some(worker),
        }
    }

    fn enqueue(
        &self,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
        format: wgpu::TextureFormat,
        repaint: Option<egui::Context>,
        metrics: &StdMutex<PlotMetrics>,
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
            "queued background avenger plot texture render"
        );
        self.shared.notify.notify_one();
    }

    fn set_front_target_generation(&self, generation: Option<u64>) {
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
        metrics: &StdMutex<PlotMetrics>,
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
            "consumed background avenger plot texture on egui thread"
        );

        let mut state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        state.mark_render_consumed(rendered.render_generation, rendered.target_generation);
        self.shared.notify.notify_one();
    }

    fn take_error(&self) -> Option<String> {
        self.shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned")
            .last_error
            .take()
    }

    fn is_pending(&self) -> bool {
        self.status().render_pending
    }

    fn status(&self) -> BackgroundRenderStatus {
        let state = self
            .shared
            .state
            .lock()
            .expect("avenger egui render-worker lock poisoned");
        BackgroundRenderStatus {
            requested_scene_generation: state.requested_scene_generation,
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
    metrics: &StdMutex<PlotMetrics>,
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
            &mut renderer,
            &mut targets,
            &request,
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
    renderer: &mut Option<AvengerWgpuRenderer>,
    targets: &mut Option<OffscreenTargetPool>,
    request: &BackgroundRenderRequest,
    metrics: &StdMutex<PlotMetrics>,
) -> Result<Option<RenderedPlotTexture>, AvengerWgpuError> {
    let format = request.format;
    let renderer = renderer.get_or_insert_with(|| {
        AvengerWgpuRenderer::new(
            device,
            AvengerRendererConfig::new(request.dimensions, format).with_sample_count(1),
        )
    });
    renderer.resize(request.dimensions);

    let set_scene_start = StdInstant::now();
    let queue_wait = set_scene_start.duration_since(request.requested_at);
    renderer.set_scene(device, queue, &request.scene_graph)?;
    let set_scene_elapsed = set_scene_start.elapsed();
    if background_request_is_stale(shared, request.render_generation) {
        return Ok(None);
    }

    let descriptor = OffscreenTargetDescriptor::new(request.dimensions, format)
        .with_label("avenger-egui background plot offscreen texture");
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
        "submitted background avenger plot texture render"
    );
    Ok(Some(RenderedPlotTexture {
        render_generation: request.render_generation,
        scene_generation: request.scene_generation,
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
    rendered: RenderedPlotTexture,
    metrics: &StdMutex<PlotMetrics>,
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
        "published background avenger plot texture"
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
    metrics: &StdMutex<PlotMetrics>,
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
        "dropped stale background avenger plot texture render"
    );
    shared.notify.notify_one();
}

pub struct PlotOutput {
    pub response: egui::Response,
    pub param_changes: Vec<ParamChange>,
    pub selection_changes: Vec<SelectionChange>,
    pub frame_status: FrameStatus,
    pub events: Vec<WindowEvent>,
}

impl PlotOutput {
    pub fn changed(&self) -> bool {
        self.response.changed() || self.params_changed() || self.selections_changed()
    }

    pub fn params_changed(&self) -> bool {
        !self.param_changes.is_empty()
    }

    pub fn param_changed(&self, name: &str) -> bool {
        self.param_changes.iter().any(|change| change.name == name)
    }

    pub fn param_changes(&self) -> &[ParamChange] {
        &self.param_changes
    }

    pub fn selections_changed(&self) -> bool {
        !self.selection_changes.is_empty()
    }

    pub fn selection_changes(&self) -> &[SelectionChange] {
        &self.selection_changes
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameStatus {
    pub latest_generation: Option<u64>,
    pub latest_scene_generation: Option<u64>,
    pub requested_generation: Option<u64>,
    pub render_pending: bool,
    pub latest_texture_scene_generation: Option<u64>,
    pub requested_texture_scene_generation: Option<u64>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionChange {
    pub name: String,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EguiResponseState {
    pub hovered: bool,
    pub drag_started: bool,
    pub drag_stopped: bool,
    pub has_focus: bool,
}

impl EguiResponseState {
    pub fn from_response(response: &egui::Response) -> Self {
        Self {
            hovered: response.hovered(),
            drag_started: response.drag_started(),
            drag_stopped: response.drag_stopped(),
            has_focus: response.has_focus(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EguiEventTranslator {
    hovered: bool,
    pointer_captured: bool,
    last_pointer_position: Option<[f32; 2]>,
    last_size: Option<[f32; 2]>,
}

impl EguiEventTranslator {
    pub fn translate_frame(
        &mut self,
        rect: egui::Rect,
        response: EguiResponseState,
        input: &egui::InputState,
    ) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        let hovered = response.hovered;

        if hovered && !self.hovered {
            events.push(WindowEvent::CursorEntered);
        } else if !hovered && self.hovered && !self.pointer_captured {
            events.push(WindowEvent::CursorLeft);
            self.last_pointer_position = None;
        }
        self.hovered = hovered;

        if response.drag_started {
            self.pointer_captured = true;
        }

        for event in &input.raw.events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    if rect.contains(*pos) || self.pointer_captured {
                        self.push_cursor_moved(&mut events, rect, *pos);
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let inside = rect.contains(*pos);
                    if *pressed && inside {
                        self.pointer_captured = true;
                    }
                    if inside || self.pointer_captured {
                        events.push(WindowEvent::MouseInput(WindowMouseInput {
                            state: if *pressed {
                                ElementState::Pressed
                            } else {
                                ElementState::Released
                            },
                            button: egui_button_to_avenger(*button),
                        }));
                    }
                    if !*pressed {
                        self.pointer_captured = false;
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    if hovered {
                        events.push(WindowEvent::MouseWheel(WindowMouseWheel {
                            delta: egui_wheel_to_avenger(*unit, *delta),
                        }));
                    }
                }
                egui::Event::Key { key, pressed, .. } => {
                    if response.has_focus
                        && let Some(key) = egui_key_to_avenger(*key)
                    {
                        events.push(WindowEvent::KeyboardInput(WindowKeyboardInput {
                            key,
                            state: if *pressed {
                                ElementState::Pressed
                            } else {
                                ElementState::Released
                            },
                        }));
                    }
                }
                egui::Event::PointerGone => {
                    if self.hovered || self.pointer_captured {
                        events.push(WindowEvent::CursorLeft);
                    }
                    self.hovered = false;
                    self.pointer_captured = false;
                    self.last_pointer_position = None;
                }
                _ => {}
            }
        }

        if response.drag_stopped {
            self.pointer_captured = false;
            if !hovered {
                self.last_pointer_position = None;
            }
        }

        let size = [rect.width(), rect.height()];
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            events.push(WindowEvent::CanvasResize(CanvasResizeEvent { size }));
        }

        events
    }

    fn push_cursor_moved(
        &mut self,
        events: &mut Vec<WindowEvent>,
        rect: egui::Rect,
        pos: egui::Pos2,
    ) {
        let position = local_position(rect, pos);
        if self.last_pointer_position != Some(position) {
            self.last_pointer_position = Some(position);
            events.push(WindowEvent::CursorMoved(WindowCursorMoved { position }));
        }
    }
}

pub fn local_position(rect: egui::Rect, pos: egui::Pos2) -> [f32; 2] {
    [pos.x - rect.min.x, pos.y - rect.min.y]
}

fn egui_button_to_avenger(button: egui::PointerButton) -> MouseButton {
    match button {
        egui::PointerButton::Primary => MouseButton::Left,
        egui::PointerButton::Secondary => MouseButton::Right,
        egui::PointerButton::Middle => MouseButton::Middle,
        egui::PointerButton::Extra1 => MouseButton::Back,
        egui::PointerButton::Extra2 => MouseButton::Forward,
    }
}

fn egui_wheel_to_avenger(unit: egui::MouseWheelUnit, delta: egui::Vec2) -> MouseScrollDelta {
    match unit {
        egui::MouseWheelUnit::Point => MouseScrollDelta::PixelDelta(delta.x as f64, delta.y as f64),
        egui::MouseWheelUnit::Line => MouseScrollDelta::LineDelta(delta.x, delta.y),
        egui::MouseWheelUnit::Page => MouseScrollDelta::LineDelta(delta.x * 24.0, delta.y * 24.0),
    }
}

fn egui_key_to_avenger(key: egui::Key) -> Option<Key> {
    let named = match key {
        egui::Key::ArrowDown => NamedKey::ArrowDown,
        egui::Key::ArrowLeft => NamedKey::ArrowLeft,
        egui::Key::ArrowRight => NamedKey::ArrowRight,
        egui::Key::ArrowUp => NamedKey::ArrowUp,
        egui::Key::Escape => NamedKey::Escape,
        egui::Key::Tab => NamedKey::Tab,
        egui::Key::Backspace => NamedKey::Backspace,
        egui::Key::Enter => NamedKey::Enter,
        egui::Key::Space => NamedKey::Space,
        egui::Key::Delete => NamedKey::Delete,
        egui::Key::Home => NamedKey::Home,
        egui::Key::End => NamedKey::End,
        egui::Key::PageUp => NamedKey::PageUp,
        egui::Key::PageDown => NamedKey::PageDown,
        egui::Key::F1 => NamedKey::F1,
        egui::Key::F2 => NamedKey::F2,
        egui::Key::F3 => NamedKey::F3,
        egui::Key::F4 => NamedKey::F4,
        egui::Key::F5 => NamedKey::F5,
        egui::Key::F6 => NamedKey::F6,
        egui::Key::F7 => NamedKey::F7,
        egui::Key::F8 => NamedKey::F8,
        egui::Key::F9 => NamedKey::F9,
        egui::Key::F10 => NamedKey::F10,
        egui::Key::F11 => NamedKey::F11,
        egui::Key::F12 => NamedKey::F12,
        egui::Key::A => return Some(Key::Character('a')),
        egui::Key::B => return Some(Key::Character('b')),
        egui::Key::C => return Some(Key::Character('c')),
        egui::Key::D => return Some(Key::Character('d')),
        egui::Key::E => return Some(Key::Character('e')),
        egui::Key::F => return Some(Key::Character('f')),
        egui::Key::G => return Some(Key::Character('g')),
        egui::Key::H => return Some(Key::Character('h')),
        egui::Key::I => return Some(Key::Character('i')),
        egui::Key::J => return Some(Key::Character('j')),
        egui::Key::K => return Some(Key::Character('k')),
        egui::Key::L => return Some(Key::Character('l')),
        egui::Key::M => return Some(Key::Character('m')),
        egui::Key::N => return Some(Key::Character('n')),
        egui::Key::O => return Some(Key::Character('o')),
        egui::Key::P => return Some(Key::Character('p')),
        egui::Key::Q => return Some(Key::Character('q')),
        egui::Key::R => return Some(Key::Character('r')),
        egui::Key::S => return Some(Key::Character('s')),
        egui::Key::T => return Some(Key::Character('t')),
        egui::Key::U => return Some(Key::Character('u')),
        egui::Key::V => return Some(Key::Character('v')),
        egui::Key::W => return Some(Key::Character('w')),
        egui::Key::X => return Some(Key::Character('x')),
        egui::Key::Y => return Some(Key::Character('y')),
        egui::Key::Z => return Some(Key::Character('z')),
        egui::Key::Num0 => return Some(Key::Character('0')),
        egui::Key::Num1 => return Some(Key::Character('1')),
        egui::Key::Num2 => return Some(Key::Character('2')),
        egui::Key::Num3 => return Some(Key::Character('3')),
        egui::Key::Num4 => return Some(Key::Character('4')),
        egui::Key::Num5 => return Some(Key::Character('5')),
        egui::Key::Num6 => return Some(Key::Character('6')),
        egui::Key::Num7 => return Some(Key::Character('7')),
        egui::Key::Num8 => return Some(Key::Character('8')),
        egui::Key::Num9 => return Some(Key::Character('9')),
        _ => return None,
    };
    Some(Key::Named(named))
}

pub fn scalar_f64(value: f64) -> ScalarValue {
    ScalarValue::Float64(Some(value))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_chart::prelude as chart;
    use avenger_chart_app::{ChartAppOptions, ChartResizeBinding, chart_avenger_app};
    use datafusion::prelude::SessionContext;

    use super::*;

    async fn test_handle() -> AvengerPlotHandle {
        let ctx = SessionContext::new();
        let width = chart::Param::new("width", scalar_f64(640.0));
        let compiled = chart::Plot::<chart::Cartesian>::new()
            .add_param(width)
            .compile(&ctx)
            .await
            .expect("compile egui test plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        AvengerPlotHandle::new(ChartAppState::new(
            session,
            policy,
            ChartAppOptions::default(),
        ))
    }

    async fn test_app_handle(options: ChartAppOptions) -> AvengerPlotHandle {
        let ctx = Arc::new(SessionContext::new());
        let width = chart::Param::new("width", scalar_f64(640.0));
        let compiled = chart::Plot::<chart::Cartesian>::new()
            .add_param(width.clone())
            .canvas_constraint(chart::CanvasConstraint::width(width.expr()))
            .compile(ctx.as_ref())
            .await
            .expect("compile egui app test plot");
        let app = chart_avenger_app(compiled, ctx, options)
            .await
            .expect("build egui test app");
        AvengerPlotHandle::from_app(app)
    }

    async fn wait_for_scene_generation(handle: &AvengerPlotHandle, generation: FrameGeneration) {
        for _ in 0..1_000 {
            if handle
                .latest_scene_frame()
                .is_some_and(|frame| frame.generation == generation)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!(
            "scene generation {} was not published; status={:?}, error={:?}",
            generation.get(),
            handle.frame_status(),
            handle.latest_scene_error()
        );
    }

    fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn plot_events_on_ctx(
        ctx: &egui::Context,
        handle: &AvengerPlotHandle,
        raw_input: egui::RawInput,
    ) -> Vec<WindowEvent> {
        let mut output_events = Vec::new();
        let _ = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    output_events = Plot::new(handle)
                        .desired_size(egui::vec2(320.0, 240.0))
                        .show(ui)
                        .events;
                })
                .inner;
        });
        output_events
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn contains_mouse_wheel(events: &[WindowEvent]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, WindowEvent::MouseWheel(_)))
    }

    fn contains_keyboard_input(events: &[WindowEvent]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, WindowEvent::KeyboardInput(_)))
    }

    fn cursor_moved_count(events: &[WindowEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, WindowEvent::CursorMoved(_)))
            .count()
    }

    fn response_for_size(size: egui::Vec2) -> egui::Response {
        let ctx = egui::Context::default();
        let mut response = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    response = Some(ui.allocate_response(size, egui::Sense::click_and_drag()));
                })
                .inner;
        });
        response.expect("allocated response")
    }

    #[test]
    fn local_position_subtracts_widget_origin() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 200.0));

        assert_eq!(local_position(rect, egui::pos2(35.0, 70.0)), [25.0, 50.0]);
    }

    #[test]
    fn wheel_mapping_preserves_point_and_line_units() {
        assert_eq!(
            egui_wheel_to_avenger(egui::MouseWheelUnit::Point, egui::vec2(4.0, -8.0)),
            MouseScrollDelta::PixelDelta(4.0, -8.0)
        );
        assert_eq!(
            egui_wheel_to_avenger(egui::MouseWheelUnit::Line, egui::vec2(1.0, -2.0)),
            MouseScrollDelta::LineDelta(1.0, -2.0)
        );
    }

    #[test]
    fn key_mapping_covers_named_and_character_keys() {
        assert_eq!(
            egui_key_to_avenger(egui::Key::ArrowLeft),
            Some(Key::Named(NamedKey::ArrowLeft))
        );
        assert_eq!(egui_key_to_avenger(egui::Key::A), Some(Key::Character('a')));
        assert_eq!(
            egui_key_to_avenger(egui::Key::Num7),
            Some(Key::Character('7'))
        );
    }

    #[test]
    fn render_request_key_tracks_scene_dimensions_and_format() {
        let dimensions = CanvasDimensions {
            size: [640.0, 480.0],
            scale: 2.0,
        };
        let key = RenderRequestKey::new(7, dimensions, wgpu::TextureFormat::Rgba8Unorm);

        assert_eq!(
            key,
            RenderRequestKey::new(7, dimensions, wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_ne!(
            key,
            RenderRequestKey::new(8, dimensions, wgpu::TextureFormat::Rgba8Unorm)
        );
        assert_ne!(
            key,
            RenderRequestKey::new(
                7,
                CanvasDimensions {
                    size: [641.0, 480.0],
                    scale: 2.0,
                },
                wgpu::TextureFormat::Rgba8Unorm,
            )
        );
        assert_ne!(
            key,
            RenderRequestKey::new(7, dimensions, wgpu::TextureFormat::Bgra8Unorm)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn empty_scene_graph() -> Arc<SceneGraph> {
        Arc::new(SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn test_dimensions(width: f32, height: f32) -> CanvasDimensions {
        CanvasDimensions {
            size: [width, height],
            scale: 2.0,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assert_dimensions_eq(left: CanvasDimensions, right: CanvasDimensions) {
        assert_eq!(left.size, right.size);
        assert_eq!(left.scale, right.scale);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_coalesces_latest_request() {
        let mut state = BackgroundRenderState::default();
        let dimensions = test_dimensions(640.0, 480.0);

        let first = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(first.render_generation, Some(1));
        assert!(!first.coalesced_previous);
        assert_eq!(state.requested_scene_generation, Some(1));

        let duplicate = state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(duplicate, BackgroundEnqueueResult::default());
        assert_eq!(state.next_render_generation, 1);

        let second = state.enqueue_request(
            2,
            empty_scene_graph(),
            dimensions,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(second.render_generation, Some(2));
        assert!(second.coalesced_previous);
        let pending = state.pending_request.as_ref().expect("pending request");
        assert_eq!(pending.render_generation, 2);
        assert_eq!(pending.scene_generation, 2);
        assert_eq!(state.requested_scene_generation, Some(2));
        assert!(state.is_render_generation_stale(1));
        assert!(!state.is_render_generation_stale(2));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_marks_in_progress_generation_stale_after_newer_request() {
        let mut state = BackgroundRenderState::default();
        let dimensions = test_dimensions(640.0, 480.0);
        state.enqueue_request(
            1,
            empty_scene_graph(),
            dimensions,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let first = state
            .take_next_request_if_ready()
            .expect("first request should start");

        assert_eq!(first.render_generation, 1);
        assert_eq!(state.in_progress_generation, Some(1));
        assert!(state.is_pending());

        state.enqueue_request(
            2,
            empty_scene_graph(),
            dimensions,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert!(state.is_render_generation_stale(first.render_generation));
        state.finish_render_generation(first.render_generation);
        assert_eq!(state.in_progress_generation, None);

        let second = state
            .take_next_request_if_ready()
            .expect("newest request should start after stale finish");
        assert_eq!(second.render_generation, 2);
        assert_eq!(second.scene_generation, 2);
        assert!(!state.is_render_generation_stale(second.render_generation));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_replaces_pending_request_on_resize_key_change() {
        let mut state = BackgroundRenderState::default();
        let initial = test_dimensions(640.0, 480.0);
        let resized = test_dimensions(800.0, 480.0);

        state.enqueue_request(
            1,
            empty_scene_graph(),
            initial,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        let resized_result = state.enqueue_request(
            1,
            empty_scene_graph(),
            resized,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );

        assert_eq!(resized_result.render_generation, Some(2));
        assert!(resized_result.coalesced_previous);
        let pending = state.pending_request.as_ref().expect("pending request");
        assert_eq!(pending.render_generation, 2);
        assert_dimensions_eq(pending.dimensions, resized);

        let duplicate_resize = state.enqueue_request(
            1,
            empty_scene_graph(),
            resized,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            StdInstant::now(),
        );
        assert_eq!(duplicate_resize, BackgroundEnqueueResult::default());
        assert_eq!(state.next_render_generation, 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_render_state_tracks_front_target_after_consumption() {
        let mut state = BackgroundRenderState::default();

        state.mark_render_consumed(4, 12);

        assert_eq!(state.consumed_render_generation, Some(4));
        assert_eq!(state.front_target_generation, Some(12));
        assert!(!state.is_pending());
    }

    #[tokio::test]
    async fn handle_queues_and_takes_pending_events() {
        let handle = test_handle().await;
        let event = WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        });

        handle.queue_events([event.clone()]);

        assert_eq!(handle.pending_event_count(), 1);
        assert_eq!(handle.take_pending_events(), vec![event]);
        assert_eq!(handle.pending_event_count(), 0);
        let metrics = handle.metrics();
        assert_eq!(metrics.routed_event_batches, 1);
        assert_eq!(metrics.routed_events, 1);
        assert_eq!(metrics.last_routed_event_count, 1);
    }

    #[tokio::test]
    async fn set_param_updates_metrics() {
        let handle = test_handle().await;

        let changed = handle.set_param("width", 720.0);
        let unchanged = handle.set_param("width", 720.0);

        assert!(changed.changed);
        assert!(!unchanged.changed);
        let metrics = handle.metrics();
        assert_eq!(metrics.param_set_calls, 2);
        assert_eq!(metrics.param_changes_enqueued, 1);
        handle.reset_metrics();
        assert_eq!(handle.metrics(), PlotMetrics::default());
    }

    #[tokio::test]
    async fn dispatch_without_owned_app_drains_events_without_updates() {
        let handle = test_handle().await;
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [300.0, 200.0],
        })]);

        let updates = handle
            .dispatch_pending_events()
            .await
            .expect("dispatch without owned app");

        assert!(updates.is_empty());
        assert_eq!(handle.pending_event_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scene_rebuild_request_publishes_latest_scene() {
        let handle = test_app_handle(ChartAppOptions::default()).await;

        let generation = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request scene rebuild");

        assert_eq!(
            handle.frame_status().requested_generation,
            Some(generation.get())
        );
        wait_for_scene_generation(&handle, generation).await;

        let status = handle.frame_status();
        assert_eq!(status.latest_scene_generation, Some(generation.get()));
        assert!(!status.render_pending);
        assert!(handle.latest_scene_error().is_none());
        let metrics = handle.metrics();
        assert_eq!(metrics.scene_rebuild_requests, 1);
        assert_eq!(metrics.scene_frames_published, 1);
        assert_eq!(
            metrics.last_published_scene_generation,
            Some(generation.get())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rapid_scene_rebuild_requests_publish_newest_generation() {
        let handle = test_app_handle(ChartAppOptions::default()).await;
        handle.set_param("width", 700.0);
        let first = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request first scene rebuild");
        handle.set_param("width", 720.0);
        let second = handle
            .request_scene_rebuild(&tokio::runtime::Handle::current(), true)
            .expect("request second scene rebuild");

        assert!(second > first);
        wait_for_scene_generation(&handle, second).await;

        let latest = handle
            .latest_scene_frame()
            .expect("latest scene after rapid rebuild requests");
        assert_eq!(latest.generation, second);
        assert_eq!(handle.param_f64("width"), Some(720.0));
        let metrics = handle.metrics();
        assert_eq!(metrics.scene_rebuild_requests, 2);
        assert_eq!(metrics.last_requested_generation, Some(second.get()));
        assert_eq!(metrics.last_published_scene_generation, Some(second.get()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn event_dispatch_request_publishes_scene_for_routed_resize() {
        let handle = test_app_handle(ChartAppOptions {
            resize_binding: ChartResizeBinding::width("width"),
            ..ChartAppOptions::default()
        })
        .await;
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [720.0, 300.0],
        })]);

        let generation = handle
            .request_event_dispatch(&tokio::runtime::Handle::current())
            .expect("request event dispatch");
        wait_for_scene_generation(&handle, generation).await;

        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.param_f64("width"), Some(720.0));
        let metrics = handle.metrics();
        assert_eq!(metrics.event_dispatch_requests, 1);
        assert_eq!(metrics.scene_frames_published, 1);
    }

    #[tokio::test]
    async fn plot_show_queues_translated_resize_events() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let mut output_events = Vec::new();

        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    output_events = Plot::new(&handle)
                        .desired_size(egui::vec2(320.0, 240.0))
                        .show(ui)
                        .events;
                })
                .inner;
        });

        assert!(
            output_events
                .iter()
                .any(|event| matches!(event, WindowEvent::CanvasResize(_)))
        );
        assert_eq!(handle.pending_event_count(), output_events.len());
    }

    #[tokio::test]
    async fn wheel_events_route_only_while_plot_is_hovered() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 12.0),
            modifiers: egui::Modifiers::default(),
        };

        let outside = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(700.0, 500.0)),
                wheel.clone(),
            ]),
        );
        assert!(!contains_mouse_wheel(&outside));

        let inside = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                wheel,
            ]),
        );
        assert!(contains_mouse_wheel(&inside));
    }

    #[tokio::test]
    async fn pointer_capture_routes_release_after_pointer_leaves_plot() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();

        let events = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                pointer_button(egui::pos2(40.0, 40.0), true),
                egui::Event::PointerMoved(egui::pos2(700.0, 500.0)),
                pointer_button(egui::pos2(700.0, 500.0), false),
            ]),
        );

        assert!(events.iter().any(|event| {
            matches!(
                event,
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                })
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                })
            )
        }));
    }

    #[tokio::test]
    async fn stationary_hover_does_not_emit_repeated_cursor_moved_events() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();
        let pos = egui::pos2(40.0, 40.0);

        let first = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![egui::Event::PointerMoved(pos)]),
        );
        assert_eq!(cursor_moved_count(&first), 1);

        let second = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![egui::Event::PointerMoved(pos)]),
        );
        assert_eq!(cursor_moved_count(&second), 0);
    }

    #[tokio::test]
    async fn keyboard_events_route_only_after_plot_focus() {
        let handle = test_handle().await;
        let ctx = egui::Context::default();

        let unfocused = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![key_event(egui::Key::A, true)]),
        );
        assert!(!contains_keyboard_input(&unfocused));

        let _ = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![
                egui::Event::PointerMoved(egui::pos2(40.0, 40.0)),
                pointer_button(egui::pos2(40.0, 40.0), true),
                pointer_button(egui::pos2(40.0, 40.0), false),
            ]),
        );
        let focused = plot_events_on_ctx(
            &ctx,
            &handle,
            raw_input(vec![key_event(egui::Key::A, true)]),
        );

        assert!(contains_keyboard_input(&focused));
    }

    #[test]
    fn translator_emits_canvas_resize_once_per_size() {
        let mut translator = EguiEventTranslator::default();
        let response = response_for_size(egui::vec2(300.0, 200.0));
        let input = egui::InputState::default();

        let first = translator.translate_frame(
            response.rect,
            EguiResponseState::from_response(&response),
            &input,
        );
        assert_eq!(
            first.last(),
            Some(&WindowEvent::CanvasResize(CanvasResizeEvent {
                size: [300.0, 200.0],
            }))
        );

        let second = translator.translate_frame(
            response.rect,
            EguiResponseState::from_response(&response),
            &input,
        );
        assert!(
            !second
                .iter()
                .any(|event| matches!(event, WindowEvent::CanvasResize(_)))
        );
    }

    #[test]
    fn plot_output_param_helpers_follow_change_names() {
        let output = PlotOutput {
            response: response_for_size(egui::vec2(10.0, 10.0)),
            param_changes: vec![ParamChange {
                name: "point_size".to_string(),
                value: scalar_f64(12.0),
                previous: Some(scalar_f64(10.0)),
                revision: 7,
            }],
            selection_changes: Vec::new(),
            frame_status: FrameStatus::default(),
            events: Vec::new(),
        };

        assert!(output.changed());
        assert!(output.params_changed());
        assert!(output.param_changed("point_size"));
        assert!(!output.param_changed("opacity"));
    }
}
