use std::sync::{Arc, Mutex as StdMutex};

use avenger_app::{
    app::{AppUpdate, AvengerApp},
    error::AvengerAppError,
};
use avenger_chart_app::{
    ChartAppState, IntoChartParamValue, ParamChange, ParamSetResult, ParamSnapshot,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_egui::{
    AvengerCanvasHandle, Canvas, CanvasMetrics, CanvasOutput, FrameStatus, TextureRenderStatus,
};
use avenger_eventstream::window::WindowEvent;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::{
    error::AvengerWgpuError,
    frame_publisher::{FrameGeneration, FramePublisherStatus, RenderedFrame},
};

pub use avenger_egui::{CanvasLatencyBottleneck as PlotLatencyBottleneck, egui};

pub type PlotMetrics = CanvasMetrics;

#[derive(Clone)]
pub struct AvengerPlotHandle {
    canvas: AvengerCanvasHandle<ChartAppState>,
    observed: Arc<StdMutex<ObservedState>>,
}

impl AvengerPlotHandle {
    pub fn new(state: ChartAppState) -> Self {
        Self {
            canvas: AvengerCanvasHandle::new(state),
            observed: Arc::new(StdMutex::new(ObservedState::default())),
        }
    }

    pub fn from_app(app: AvengerApp<ChartAppState>) -> Self {
        Self {
            canvas: AvengerCanvasHandle::from_app(app),
            observed: Arc::new(StdMutex::new(ObservedState::default())),
        }
    }

    pub fn canvas_handle(&self) -> &AvengerCanvasHandle<ChartAppState> {
        &self.canvas
    }

    pub fn chart_state(&self) -> &ChartAppState {
        self.canvas.app_state()
    }

    pub fn set_param(
        &self,
        name: impl Into<String>,
        value: impl IntoChartParamValue,
    ) -> ParamSetResult {
        let name = name.into();
        let _span = tracing::debug_span!("avenger_chart_egui.set_param", param = %name).entered();
        let result = self.chart_state().set_param(name, value);
        tracing::debug!(
            changed = result.changed,
            revision = result.revision,
            "plot param set"
        );
        result
    }

    pub fn param_snapshot(&self) -> ParamSnapshot {
        self.chart_state().param_snapshot()
    }

    pub fn param_revision(&self) -> u64 {
        self.chart_state().param_revision()
    }

    pub fn param_changes_since(&self, revision: u64) -> Vec<ParamChange> {
        self.chart_state().param_changes_since(revision)
    }

    pub fn param_f64(&self, name: &str) -> Option<f64> {
        self.chart_state().param_f64(name)
    }

    pub fn param_bool(&self, name: &str) -> Option<bool> {
        self.chart_state().param_bool(name)
    }

    pub fn queue_events(&self, events: impl IntoIterator<Item = WindowEvent>) {
        self.canvas.queue_events(events);
    }

    pub fn take_pending_events(&self) -> Vec<WindowEvent> {
        self.canvas.take_pending_events()
    }

    pub fn pending_event_count(&self) -> usize {
        self.canvas.pending_event_count()
    }

    pub async fn dispatch_pending_events(&self) -> Result<Vec<AppUpdate>, AvengerAppError> {
        self.canvas.dispatch_pending_events().await
    }

    pub fn request_scene_rebuild(
        &self,
        runtime: &tokio::runtime::Handle,
        rebuild_geometry: bool,
    ) -> Option<FrameGeneration> {
        self.canvas.request_scene_rebuild(runtime, rebuild_geometry)
    }

    pub fn request_scene_rebuild_with_repaint(
        &self,
        runtime: &tokio::runtime::Handle,
        ctx: &egui::Context,
        rebuild_geometry: bool,
    ) -> Option<FrameGeneration> {
        self.canvas
            .request_scene_rebuild_with_repaint(runtime, ctx, rebuild_geometry)
    }

    pub fn request_event_dispatch(
        &self,
        runtime: &tokio::runtime::Handle,
    ) -> Option<FrameGeneration> {
        self.canvas.request_event_dispatch(runtime)
    }

    pub fn request_event_dispatch_with_repaint(
        &self,
        runtime: &tokio::runtime::Handle,
        ctx: &egui::Context,
    ) -> Option<FrameGeneration> {
        self.canvas
            .request_event_dispatch_with_repaint(runtime, ctx)
    }

    pub fn latest_scene_frame(&self) -> Option<Arc<RenderedFrame<Arc<SceneGraph>>>> {
        self.canvas.latest_scene_frame()
    }

    pub fn latest_scene_graph(&self) -> Option<Arc<SceneGraph>> {
        self.canvas.latest_scene_graph()
    }

    pub fn scene_frame_status(&self) -> FramePublisherStatus {
        self.canvas.scene_frame_status()
    }

    pub fn latest_scene_error(&self) -> Option<String> {
        self.canvas.latest_scene_error()
    }

    pub fn metrics(&self) -> CanvasMetrics {
        self.canvas.metrics()
    }

    pub fn reset_metrics(&self) {
        self.canvas.reset_metrics();
    }

    pub fn show_metrics(&self, ui: &mut egui::Ui) {
        self.canvas.show_metrics(ui);
    }

    pub async fn current_scene_graph(&self) -> Option<Arc<SceneGraph>> {
        self.canvas.current_scene_graph().await
    }

    pub async fn rebuild_scene_graph(
        &self,
        rebuild_geometry: bool,
    ) -> Result<Option<Arc<SceneGraph>>, AvengerAppError> {
        self.canvas.rebuild_scene_graph(rebuild_geometry).await
    }

    pub fn render_scene_to_texture(
        &self,
        render_state: &egui_wgpu::RenderState,
        scene_graph: &SceneGraph,
        dimensions: CanvasDimensions,
    ) -> Result<egui::TextureId, AvengerWgpuError> {
        self.canvas
            .render_scene_to_texture(render_state, scene_graph, dimensions)
    }

    pub fn request_background_scene_texture(
        &self,
        render_state: &egui_wgpu::RenderState,
        scene_generation: u64,
        scene_graph: Arc<SceneGraph>,
        dimensions: CanvasDimensions,
    ) -> Result<TextureRenderStatus, AvengerWgpuError> {
        self.canvas.request_background_scene_texture(
            render_state,
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
        self.canvas.request_background_scene_texture_with_repaint(
            render_state,
            ctx,
            scene_generation,
            scene_graph,
            dimensions,
        )
    }

    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.canvas.texture_id()
    }

    pub fn frame_status(&self) -> FrameStatus {
        self.canvas.frame_status()
    }

    fn param_changes_since_last_show(&self) -> Vec<ParamChange> {
        let current_revision = self.chart_state().param_revision();
        let mut observed = self
            .observed
            .lock()
            .expect("avenger chart egui observed-state lock poisoned");
        let changes = self
            .chart_state()
            .param_changes_since(observed.param_revision);
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
        let mut canvas = Canvas::new(self.handle.canvas_handle()).sense(self.sense);
        if let Some(size) = self.desired_size {
            canvas = canvas.desired_size(size);
        }
        let CanvasOutput {
            mut response,
            frame_status,
            events,
        } = canvas.show(ui);

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

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionChange {
    pub name: String,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_chart::prelude as chart;
    use avenger_chart_app::{ChartAppOptions, ChartResizeBinding, chart_avenger_app};
    use avenger_eventstream::window::{CanvasResizeEvent, WindowEvent};
    use avenger_wgpu::frame_publisher::FrameGeneration;
    use datafusion::{prelude::SessionContext, scalar::ScalarValue};

    use super::*;

    fn scalar_f64(value: f64) -> ScalarValue {
        ScalarValue::Float64(Some(value))
    }

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

    #[tokio::test]
    async fn set_param_updates_chart_state() {
        let handle = test_handle().await;

        let changed = handle.set_param("width", 720.0);
        let unchanged = handle.set_param("width", 720.0);

        assert!(changed.changed);
        assert!(!unchanged.changed);
        assert_eq!(handle.param_f64("width"), Some(720.0));
        assert_eq!(handle.param_changes_since(0).len(), 1);
    }

    #[tokio::test]
    async fn from_app_preserves_param_access() {
        let ctx = Arc::new(SessionContext::new());
        let width = chart::Param::new("width", scalar_f64(640.0));
        let compiled = chart::Plot::<chart::Cartesian>::new()
            .add_param(width.clone())
            .canvas_constraint(chart::CanvasConstraint::width(width.expr()))
            .compile(ctx.as_ref())
            .await
            .expect("compile egui app test plot");
        let app = chart_avenger_app(compiled, ctx, ChartAppOptions::default())
            .await
            .expect("build egui test app");
        let handle = AvengerPlotHandle::from_app(app);

        assert_eq!(handle.param_f64("width"), Some(640.0));
    }

    #[tokio::test]
    async fn plot_show_exposes_canvas_events_and_param_changes() {
        let handle = test_handle().await;
        handle.set_param("width", 720.0);
        let ctx = egui::Context::default();
        let mut output = None;

        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    output = Some(
                        Plot::new(&handle)
                            .desired_size(egui::vec2(320.0, 240.0))
                            .show(ui),
                    );
                })
                .inner;
        });

        let output = output.expect("plot output");
        assert!(output.params_changed());
        assert!(output.param_changed("width"));
        assert!(
            output
                .events
                .iter()
                .any(|event| matches!(event, WindowEvent::CanvasResize(_)))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn chart_wrapper_dispatches_routed_resize_events() {
        let ctx = Arc::new(SessionContext::new());
        let width = chart::Param::new("width", scalar_f64(640.0));
        let compiled = chart::Plot::<chart::Cartesian>::new()
            .add_param(width.clone())
            .canvas_constraint(chart::CanvasConstraint::width(width.expr()))
            .compile(ctx.as_ref())
            .await
            .expect("compile egui app test plot");
        let app = chart_avenger_app(
            compiled,
            ctx,
            ChartAppOptions {
                resize_binding: ChartResizeBinding::width("width"),
                ..ChartAppOptions::default()
            },
        )
        .await
        .expect("build egui test app");
        let handle = AvengerPlotHandle::from_app(app);
        handle.queue_events([WindowEvent::CanvasResize(CanvasResizeEvent {
            size: [720.0, 300.0],
        })]);

        let generation = handle
            .request_event_dispatch(&tokio::runtime::Handle::current())
            .expect("request event dispatch");
        wait_for_scene_generation(&handle, generation).await;

        assert_eq!(handle.pending_event_count(), 0);
        assert_eq!(handle.param_f64("width"), Some(720.0));
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
