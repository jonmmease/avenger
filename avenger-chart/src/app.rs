use crate::*;
use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    background::{BackgroundTask, BackgroundTasks},
    error::AvengerAppError,
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;

/// Requested inputs and the last successfully displayed frame.
#[derive(Clone)]
pub struct ChartAppState {
    chart: Chart,
    rendered: Arc<RenderedChart>,
    requested: Inputs,
    task: BackgroundTask<RenderedChart>,
    last_error: Option<String>,
}
impl ChartAppState {
    /// The immutable frame currently displayed by the app.
    pub fn rendered(&self) -> &RenderedChart {
        &self.rendered
    }
    /// Whether the latest requested frame is still being evaluated.
    pub fn is_pending(&self) -> bool {
        self.task.is_pending()
    }
    /// The latest background error; the last valid frame remains displayed.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    /// Read a requested root parameter, including updates still in flight.
    pub fn parameter(&self, name: &str) -> Result<&ScalarValue> {
        let input = self.parameter_input(name)?;
        Ok(self.requested.scalar_value(&input)?)
    }
    fn parameter_input(&self, name: &str) -> Result<dataflow::ScalarInput> {
        let interface = &self.chart.0.interface;
        self.chart
            .definition()
            .parameters()
            .iter()
            .find(|p| {
                p.name == name
                    && interface
                        .scalar_input_reference(&p.input)
                        .is_ok_and(|r| r.scope.is_empty())
            })
            .map(|p| p.input.clone())
            .ok_or_else(|| error(format!("unknown root parameter {name}")))
    }
    /// Submit one root parameter change. Equal successful or pending requests are ignored.
    pub fn set_parameter(&mut self, name: impl Into<String>, value: ScalarValue) -> Result<bool> {
        self.set_parameters([(name.into(), value)])
    }
    /// Validate an atomic set of root changes, then replace the pending request.
    pub fn set_parameters(
        &mut self,
        values: impl IntoIterator<Item = (String, ScalarValue)>,
    ) -> Result<bool> {
        let mut builder = self.requested.edit();
        let mut changed = false;
        for (name, value) in values {
            let input = self.parameter_input(&name)?;
            changed |= self.requested.scalar_value(&input)? != &value;
            builder = builder.scalar(&input, value)?;
        }
        let inputs = builder.finish()?;
        if !changed && self.last_error.is_none() {
            return Ok(false);
        }
        self.submit(inputs)?;
        Ok(true)
    }
    /// Replace the complete input snapshot, including table or scoped bindings.
    pub fn set_inputs(&mut self, inputs: Inputs) -> Result<()> {
        let inputs = self
            .chart
            .resolve_inputs(&RenderOptions::default().inputs(inputs))?;
        self.submit(inputs)
    }
    fn submit(&mut self, inputs: Inputs) -> Result<()> {
        let chart = self.chart.clone();
        let request = inputs.clone();
        self.task
            .submit(async move { chart.render(RenderOptions::default().inputs(request)).await })
            .map_err(error)?;
        self.requested = inputs;
        self.last_error = None;
        Ok(())
    }
}
impl Chart {
    /// Evaluate an initial frame and return an app ready for host-specific event handlers.
    pub async fn into_app(self, options: RenderOptions) -> Result<AvengerApp<ChartAppState>> {
        let rendered = Arc::new(self.render(options).await?);
        let tasks = BackgroundTasks::new();
        let text = self.0.text.clone();
        let state = ChartAppState {
            chart: self,
            requested: rendered.inputs.clone(),
            rendered,
            task: tasks.task(),
            last_error: None,
        };
        let app = AvengerApp::try_new_with_text_engine(
            state,
            Arc::new(Builder),
            vec![(
                EventStreamConfig {
                    types: vec![SceneGraphEventType::RuntimeWake],
                    ..Default::default()
                },
                Arc::new(Completion),
            )],
            text,
        )
        .await
        .map_err(error)?;
        Ok(app.with_background_tasks(tasks))
    }
}
struct Builder;
#[async_trait]
impl SceneGraphBuilder<ChartAppState> for Builder {
    async fn build(
        &self,
        state: &mut ChartAppState,
    ) -> std::result::Result<SceneGraph, AvengerAppError> {
        Ok((*state.rendered.scene).clone())
    }
}
struct Completion;
#[async_trait]
impl EventStreamHandler<ChartAppState> for Completion {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _: &SceneGraphRTree,
    ) -> UpdateStatus {
        if let SceneGraphEvent::RuntimeWake(wake) = event {
            if let Some(result) = state.task.handle_wake(wake) {
                match result {
                    Ok(frame) => {
                        tracing::debug!(
                            target: "avenger_chart::frame",
                            evaluation_id = frame.report.evaluation_id,
                            elapsed_ms = frame.elapsed.as_secs_f64() * 1000.0,
                            executed_nodes = ?frame.report.executed_nodes,
                            cache_hits = frame.report.cache_hits,
                            position_builds = frame.geometry.position_builds,
                            position_reuses = frame.geometry.position_reuses,
                            "installed chart frame"
                        );
                        state.rendered = frame;
                        state.last_error = None;
                        return UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                            ..Default::default()
                        };
                    }
                    Err(e) => {
                        tracing::warn!("chart frame failed: {e}");
                        state.last_error = Some(e.to_string());
                    }
                }
            }
        }
        UpdateStatus::default()
    }
}
