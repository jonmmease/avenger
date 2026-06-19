use std::sync::{Arc, Mutex as StdMutex};

use avenger_app::{
    app::{AppUpdate, AvengerApp},
    error::AvengerAppError,
};
use avenger_chart_app::{
    ChartAppState, IntoChartParamValue, ParamChange, ParamSetResult, ParamSnapshot,
};
use avenger_common::{canvas::CanvasDimensions, time::Instant};
use avenger_eventstream::window::{
    CanvasResizeEvent, ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::{
    error::AvengerWgpuError,
    offscreen::{OffscreenTargetDescriptor, OffscreenTargetPool},
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
        self.state.set_param(name, value)
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
            updates.push(app.update_with_status(&event, Instant::now()).await?);
        }
        Ok(updates)
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
        gpu.render_scene(render_state, scene_graph, dimensions)
    }

    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.gpu
            .lock()
            .expect("avenger egui gpu lock poisoned")
            .as_ref()
            .and_then(|gpu| gpu.texture_id)
    }

    pub fn frame_status(&self) -> FrameStatus {
        self.gpu
            .lock()
            .expect("avenger egui gpu lock poisoned")
            .as_ref()
            .map(EguiPlotGpuState::frame_status)
            .unwrap_or_default()
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
}

#[derive(Clone, Debug, Default)]
struct ObservedState {
    param_revision: u64,
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
        let desired_size = self.desired_size.unwrap_or_else(|| {
            let available = ui.available_size_before_wrap();
            egui::vec2(available.x.max(320.0), available.y.max(240.0))
        });
        let (rect, mut response) = ui.allocate_exact_size(desired_size, self.sense);
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if let Some(texture_id) = self.handle.texture_id() {
            ui.painter().image(
                texture_id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        let events = ui.input(|input| {
            self.handle
                .event_translator
                .lock()
                .expect("avenger egui event-translator lock poisoned")
                .translate_frame(rect, &response, input)
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
            frame_status: self.handle.frame_status(),
            events,
        }
    }
}

struct EguiPlotGpuState {
    renderer: AvengerWgpuRenderer,
    targets: OffscreenTargetPool,
    texture_id: Option<egui::TextureId>,
    registered_generation: Option<u64>,
    format: wgpu::TextureFormat,
}

impl EguiPlotGpuState {
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
            texture_id: None,
            registered_generation: None,
            format,
        }
    }

    fn render_scene(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        self.renderer.resize(dimensions);
        self.renderer
            .set_scene(&render_state.device, &render_state.queue, scene_graph)?;
        self.targets.resize_or_recreate(
            &render_state.device,
            OffscreenTargetDescriptor::new(dimensions, self.format)
                .with_label("avenger-egui plot offscreen texture"),
        );

        let target = if let Some(target) = self
            .targets
            .acquire_next_excluding_generation(self.registered_generation)
        {
            target
        } else {
            self.targets.acquire_next()
        };
        let rendered =
            self.renderer
                .render_to_offscreen(&render_state.device, &render_state.queue, target)?;

        let mut egui_renderer = render_state.renderer.write();
        let texture_id = if let Some(texture_id) = self.texture_id {
            egui_renderer.update_egui_texture_from_wgpu_texture(
                &render_state.device,
                &target.view,
                wgpu::FilterMode::Linear,
                texture_id,
            );
            texture_id
        } else {
            egui_renderer.register_native_texture(
                &render_state.device,
                &target.view,
                wgpu::FilterMode::Linear,
            )
        };
        self.texture_id = Some(texture_id);
        self.registered_generation = Some(rendered.generation);
        Ok(texture_id)
    }

    fn frame_status(&self) -> FrameStatus {
        FrameStatus {
            latest_generation: self.registered_generation,
            requested_generation: None,
            render_pending: false,
        }
    }
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
    pub requested_generation: Option<u64>,
    pub render_pending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionChange {
    pub name: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Default)]
pub struct EguiEventTranslator {
    hovered: bool,
    pointer_captured: bool,
    last_size: Option<[f32; 2]>,
}

impl EguiEventTranslator {
    pub fn translate_frame(
        &mut self,
        rect: egui::Rect,
        response: &egui::Response,
        input: &egui::InputState,
    ) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        let pointer_pos = input.pointer.hover_pos();
        let hovered = response.hovered();

        if hovered && !self.hovered {
            events.push(WindowEvent::CursorEntered);
        } else if !hovered && self.hovered && !self.pointer_captured {
            events.push(WindowEvent::CursorLeft);
        }
        self.hovered = hovered;

        if response.drag_started() {
            self.pointer_captured = true;
        }

        if (hovered || self.pointer_captured)
            && let Some(pos) = pointer_pos
        {
            events.push(WindowEvent::CursorMoved(WindowCursorMoved {
                position: local_position(rect, pos),
            }));
        }

        for button in [
            egui::PointerButton::Primary,
            egui::PointerButton::Secondary,
            egui::PointerButton::Middle,
        ] {
            if response.clicked_by(button) {
                self.pointer_captured = true;
                events.push(WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Pressed,
                    button: egui_button_to_avenger(button),
                }));
                events.push(WindowEvent::MouseInput(WindowMouseInput {
                    state: ElementState::Released,
                    button: egui_button_to_avenger(button),
                }));
            }
        }

        if response.drag_stopped() {
            self.pointer_captured = false;
        }

        let size = [rect.width(), rect.height()];
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            events.push(WindowEvent::CanvasResize(CanvasResizeEvent { size }));
        }

        events
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

pub fn scalar_f64(value: f64) -> ScalarValue {
    ScalarValue::Float64(Some(value))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_chart::prelude as chart;
    use avenger_chart_app::ChartAppOptions;
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

    #[test]
    fn translator_emits_canvas_resize_once_per_size() {
        let mut translator = EguiEventTranslator::default();
        let response = response_for_size(egui::vec2(300.0, 200.0));
        let input = egui::InputState::default();

        let first = translator.translate_frame(response.rect, &response, &input);
        assert_eq!(
            first.last(),
            Some(&WindowEvent::CanvasResize(CanvasResizeEvent {
                size: [300.0, 200.0],
            }))
        );

        let second = translator.translate_frame(response.rect, &response, &input);
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
