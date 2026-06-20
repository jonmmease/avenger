use std::{
    sync::{Arc, Mutex as StdMutex},
    time::Instant as StdInstant,
};

use avenger_app::{
    app::{AppUpdate, AvengerApp},
    error::AvengerAppError,
};
use avenger_common::{canvas::CanvasDimensions, time::Instant as AvengerInstant};
use avenger_eventstream::window::WindowEvent;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::{
    error::AvengerWgpuError,
    frame_publisher::{
        BeginFrameError, FrameGeneration, FramePublisher, FramePublisherStatus, FrameRenderMetrics,
        PublishResult, RenderedFrame,
    },
};
use egui_wgpu::wgpu;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    event::{EguiEventTranslator, EguiResponseState, coalesce_render_pending_event},
    gpu::{EguiCanvasGpuState, FrameStatus, TextureRenderStatus},
    metrics::{CanvasMetrics, duration_us, update_metrics},
};

#[derive(Clone)]
pub struct AvengerCanvasHandle<State>
where
    State: Clone + Send + Sync + 'static,
{
    state: State,
    app: Option<Arc<AsyncMutex<AvengerApp<State>>>>,
    event_translator: Arc<StdMutex<EguiEventTranslator>>,
    pending_events: Arc<StdMutex<Vec<WindowEvent>>>,
    coalesced_render_pending_events: Arc<StdMutex<Vec<WindowEvent>>>,
    scene_rebuild_request: Arc<StdMutex<Option<bool>>>,
    scene_publisher: FramePublisher<Arc<SceneGraph>>,
    scene_error: Arc<StdMutex<Option<String>>>,
    metrics: Arc<StdMutex<CanvasMetrics>>,
    gpu: Arc<StdMutex<Option<EguiCanvasGpuState>>>,
}

impl<State> AvengerCanvasHandle<State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn new(state: State) -> Self {
        Self {
            state,
            app: None,
            event_translator: Arc::new(StdMutex::new(EguiEventTranslator::default())),
            pending_events: Arc::new(StdMutex::new(Vec::new())),
            coalesced_render_pending_events: Arc::new(StdMutex::new(Vec::new())),
            scene_rebuild_request: Arc::new(StdMutex::new(None)),
            scene_publisher: FramePublisher::new(),
            scene_error: Arc::new(StdMutex::new(None)),
            metrics: Arc::new(StdMutex::new(CanvasMetrics::default())),
            gpu: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn from_app(mut app: AvengerApp<State>) -> Self {
        let state = app.app_state_mut().clone();
        Self {
            state,
            app: Some(Arc::new(AsyncMutex::new(app))),
            event_translator: Arc::new(StdMutex::new(EguiEventTranslator::default())),
            pending_events: Arc::new(StdMutex::new(Vec::new())),
            coalesced_render_pending_events: Arc::new(StdMutex::new(Vec::new())),
            scene_rebuild_request: Arc::new(StdMutex::new(None)),
            scene_publisher: FramePublisher::new(),
            scene_error: Arc::new(StdMutex::new(None)),
            metrics: Arc::new(StdMutex::new(CanvasMetrics::default())),
            gpu: Arc::new(StdMutex::new(None)),
        }
    }

    pub fn app_state(&self) -> &State {
        &self.state
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
            tracing::debug!(event_count = count, "queued avenger canvas events");
        }
        self.pending_events
            .lock()
            .expect("avenger egui pending-event lock poisoned")
            .extend(events);
    }

    pub(crate) fn queue_frame_events(
        &self,
        events: impl IntoIterator<Item = WindowEvent>,
        render_pending: bool,
    ) -> Vec<WindowEvent> {
        let mut queued = Vec::new();
        let mut coalesced_count = 0;
        let mut replayed_count = 0;

        {
            let mut coalesced_events = self
                .coalesced_render_pending_events
                .lock()
                .expect("avenger egui coalesced-event lock poisoned");

            if !render_pending {
                replayed_count += coalesced_events.len() as u64;
                queued.extend(coalesced_events.drain(..));
            }

            for event in events {
                if render_pending && event.skip_if_render_pending() {
                    coalesce_render_pending_event(&mut coalesced_events, event);
                    coalesced_count += 1;
                } else {
                    if render_pending && !coalesced_events.is_empty() {
                        replayed_count += coalesced_events.len() as u64;
                        queued.extend(coalesced_events.drain(..));
                    }
                    queued.push(event);
                }
            }
        }

        if coalesced_count > 0 || replayed_count > 0 {
            update_metrics(&self.metrics, |metrics| {
                metrics.render_pending_events_coalesced += coalesced_count;
                metrics.render_pending_events_replayed += replayed_count;
            });
            tracing::debug!(
                coalesced_events = coalesced_count,
                replayed_events = replayed_count,
                "coalesced avenger canvas events while render pending"
            );
        }

        self.queue_events(queued.iter().cloned());
        queued
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

    #[cfg(test)]
    pub(crate) fn coalesced_render_pending_event_count(&self) -> usize {
        self.coalesced_render_pending_events
            .lock()
            .expect("avenger egui coalesced-event lock poisoned")
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

    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics
            .lock()
            .expect("avenger egui metrics lock poisoned")
            .clone()
    }

    pub fn reset_metrics(&self) {
        *self
            .metrics
            .lock()
            .expect("avenger egui metrics lock poisoned") = CanvasMetrics::default();
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
            "events coalesced/replayed: {}/{}",
            metrics.render_pending_events_coalesced, metrics.render_pending_events_replayed
        ));
        ui.label(format!(
            "set_scene/encode/submit us: {}/{}/{}",
            metrics.last_set_scene_us, metrics.last_command_encode_us, metrics.last_submit_us
        ));
        ui.label(format!(
            "gpu render total us: {}",
            metrics.last_gpu_render_us()
        ));
        ui.label(format!(
            "scene eval / texture publish us: {} / {}",
            metrics.last_scene_evaluation_us, metrics.last_texture_publish_us
        ));
        let bottleneck = metrics.latency_bottleneck();
        ui.label(format!(
            "latency bottleneck: {} ({} us)",
            bottleneck.as_str(),
            metrics.latency_bottleneck_us()
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
            EguiCanvasGpuState::new(
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
            EguiCanvasGpuState::new(
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
            .and_then(EguiCanvasGpuState::texture_id)
    }

    pub fn frame_status(&self) -> FrameStatus {
        let mut status = self
            .gpu
            .lock()
            .expect("avenger egui gpu lock poisoned")
            .as_ref()
            .map(EguiCanvasGpuState::frame_status)
            .unwrap_or_default();
        let scene_status = self.scene_publisher.status();
        status.latest_scene_generation = scene_status
            .latest_published_generation
            .map(FrameGeneration::get);
        status.requested_generation = (scene_status.requested_generation != FrameGeneration::ZERO)
            .then_some(scene_status.requested_generation.get());
        let scene_work_pending = scene_status.in_progress_generation.is_some()
            || !self
                .pending_events
                .lock()
                .expect("avenger egui pending-event lock poisoned")
                .is_empty()
            || self
                .scene_rebuild_request
                .lock()
                .expect("avenger egui scene-rebuild lock poisoned")
                .is_some();
        status.render_pending = scene_work_pending || status.gpu_render_pending;
        status
    }

    fn spawn_scene_worker(&self, runtime: tokio::runtime::Handle, repaint: Option<egui::Context>) {
        SceneWorkerParts::from_handle(self).spawn(runtime, repaint);
    }
}

#[derive(Clone)]
struct SceneWorkerParts<State>
where
    State: Clone + Send + Sync + 'static,
{
    app: Option<Arc<AsyncMutex<AvengerApp<State>>>>,
    pending_events: Arc<StdMutex<Vec<WindowEvent>>>,
    scene_rebuild_request: Arc<StdMutex<Option<bool>>>,
    scene_publisher: FramePublisher<Arc<SceneGraph>>,
    scene_error: Arc<StdMutex<Option<String>>>,
    metrics: Arc<StdMutex<CanvasMetrics>>,
}

impl<State> SceneWorkerParts<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn from_handle(handle: &AvengerCanvasHandle<State>) -> Self {
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
            tracing::debug!("dispatching canvas event through avenger app");
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

pub struct Canvas<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    handle: &'a AvengerCanvasHandle<State>,
    desired_size: Option<egui::Vec2>,
    sense: egui::Sense,
}

impl<'a, State> Canvas<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn new(handle: &'a AvengerCanvasHandle<State>) -> Self {
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

    pub fn show(self, ui: &mut egui::Ui) -> CanvasOutput {
        let _span = tracing::debug_span!("avenger_egui.canvas_show").entered();
        let desired_size = self.desired_size.unwrap_or_else(|| {
            let available = ui.available_size_before_wrap();
            egui::vec2(available.x.max(320.0), available.y.max(240.0))
        });
        let (rect, response) = ui.allocate_exact_size(desired_size, self.sense);
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
                "painted avenger canvas texture"
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
        let routed_events = self
            .handle
            .queue_frame_events(events, frame_status.render_pending);

        CanvasOutput {
            response,
            frame_status,
            events: routed_events,
        }
    }
}

pub struct CanvasOutput {
    pub response: egui::Response,
    pub frame_status: FrameStatus,
    pub events: Vec<WindowEvent>,
}

impl CanvasOutput {
    pub fn changed(&self) -> bool {
        self.response.changed()
    }
}
