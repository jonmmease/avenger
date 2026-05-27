#![recursion_limit = "512"]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_chart::{
    layout::{ChartResizeAxisPolicy, ChartResizePolicy},
    plot::{CompiledPlot, EvaluationRequest, PlotSession},
    render::{EvaluationMetrics, EvaluationMode},
};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::scene_graph::SceneGraph;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use indexmap::IndexMap;
use tokio::sync::Mutex;

#[cfg(feature = "winit-wgpu")]
pub use avenger_winit_wgpu::{WindowSceneSizing, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};

/// Parameter names that receive accepted window resize dimensions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartResizeBinding {
    pub width_param: Option<String>,
    pub height_param: Option<String>,
}

impl ChartResizeBinding {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn width(param: impl Into<String>) -> Self {
        Self {
            width_param: Some(param.into()),
            height_param: None,
        }
    }

    pub fn height(param: impl Into<String>) -> Self {
        Self {
            width_param: None,
            height_param: Some(param.into()),
        }
    }

    pub fn width_height(width_param: impl Into<String>, height_param: impl Into<String>) -> Self {
        Self {
            width_param: Some(width_param.into()),
            height_param: Some(height_param.into()),
        }
    }

    pub fn has_width(&self) -> bool {
        self.width_param.is_some()
    }

    pub fn has_height(&self) -> bool {
        self.height_param.is_some()
    }
}

/// Options for constructing an `AvengerApp` around a chart session.
#[derive(Clone, Debug)]
pub struct ChartAppOptions {
    pub resize_binding: ChartResizeBinding,
    pub resize_throttle_ms: Option<u64>,
    pub log_metrics: bool,
}

impl Default for ChartAppOptions {
    fn default() -> Self {
        Self {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: Some(8),
            log_metrics: false,
        }
    }
}

/// Cloneable app state wrapper around the stateful chart session runtime.
#[derive(Clone)]
pub struct ChartAppState {
    runtime: Arc<Mutex<ChartAppRuntime>>,
}

struct ChartAppRuntime {
    session: PlotSession,
    resize_policy: ChartResizePolicy,
    resize_binding: ChartResizeBinding,
    next_evaluation_mode: EvaluationMode,
    log_metrics: bool,
    last_metrics: Option<EvaluationMetrics>,
    last_evaluation_elapsed: Option<Duration>,
    last_scene_size: Option<[f32; 2]>,
    accepted_resize_count: usize,
}

impl ChartAppState {
    pub fn new(
        session: PlotSession,
        resize_policy: ChartResizePolicy,
        options: ChartAppOptions,
    ) -> Self {
        warn_about_ignored_bindings(resize_policy, &options.resize_binding);
        Self {
            runtime: Arc::new(Mutex::new(ChartAppRuntime {
                session,
                resize_policy,
                resize_binding: options.resize_binding,
                next_evaluation_mode: EvaluationMode::Exact,
                log_metrics: options.log_metrics,
                last_metrics: None,
                last_evaluation_elapsed: None,
                last_scene_size: None,
                accepted_resize_count: 0,
            })),
        }
    }

    pub async fn resize_policy(&self) -> ChartResizePolicy {
        self.runtime.lock().await.resize_policy
    }

    pub async fn params(&self) -> IndexMap<String, ScalarValue> {
        self.runtime.lock().await.session.params().clone()
    }

    pub async fn last_metrics(&self) -> Option<EvaluationMetrics> {
        self.runtime.lock().await.last_metrics.clone()
    }

    pub async fn last_evaluation_elapsed(&self) -> Option<Duration> {
        self.runtime.lock().await.last_evaluation_elapsed
    }

    pub async fn last_scene_size(&self) -> Option<[f32; 2]> {
        self.runtime.lock().await.last_scene_size
    }

    pub async fn accepted_resize_count(&self) -> usize {
        self.runtime.lock().await.accepted_resize_count
    }
}

/// Scene graph builder that evaluates the chart session stored in state.
pub struct ChartSceneGraphBuilder;

#[async_trait]
impl SceneGraphBuilder<ChartAppState> for ChartSceneGraphBuilder {
    async fn build(&self, state: &mut ChartAppState) -> Result<SceneGraph, AvengerAppError> {
        let mut runtime = state.runtime.lock().await;
        let mode = runtime.next_evaluation_mode;
        runtime.next_evaluation_mode = EvaluationMode::Exact;

        let start = Instant::now();
        let (evaluated, metrics) = runtime
            .session
            .evaluate_with_metrics(EvaluationRequest::new().mode(mode))
            .await
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let elapsed = start.elapsed();
        let scene_size = [evaluated.scene_graph.width, evaluated.scene_graph.height];

        if runtime.log_metrics {
            eprintln!(
                "chart eval mode={:?} elapsed={:?} scene={:.1}x{:.1} preview_reuse={} reflow_reuse={} cell_reuse={} chrome_refresh={} skipped_measures={} guide_measures={}",
                metrics.mode,
                elapsed,
                scene_size[0],
                scene_size[1],
                metrics.pipeline.preview_profile_reuses,
                metrics.pipeline.preview_structure_reflow_reuses,
                metrics.pipeline.facet_cell_measurement_profile_reuses,
                metrics
                    .pipeline
                    .facet_cell_measurement_profile_chrome_refreshes,
                metrics.pipeline.skipped_component_measure_calls,
                metrics.pipeline.guide_overflow_measure_calls,
            );
        }

        runtime.last_metrics = Some(metrics);
        runtime.last_evaluation_elapsed = Some(elapsed);
        runtime.last_scene_size = Some(scene_size);
        Ok(evaluated.scene_graph)
    }
}

/// Resize handler that patches only canvas-constrained, bound chart dimensions.
pub struct ChartResizeHandler;

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartResizeHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::WindowResize(event) = event else {
            return UpdateStatus::default();
        };

        let mut runtime = state.runtime.lock().await;
        let mut patch = IndexMap::new();
        maybe_patch_axis(
            &mut patch,
            runtime.resize_policy.width,
            runtime.resize_binding.width_param.as_deref(),
            event.size[0],
            runtime.session.params(),
        );
        maybe_patch_axis(
            &mut patch,
            runtime.resize_policy.height,
            runtime.resize_binding.height_param.as_deref(),
            event.size[1],
            runtime.session.params(),
        );

        if patch.is_empty() {
            return UpdateStatus::default();
        }

        runtime.session.apply_param_patch(patch);
        runtime.next_evaluation_mode = EvaluationMode::Preview;
        runtime.accepted_resize_count += 1;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
        }
    }
}

pub async fn chart_avenger_app(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
) -> Result<AvengerApp<ChartAppState>, AvengerAppError> {
    let resize_policy = compiled_plot.resize_policy();
    let session = Arc::new(compiled_plot).instantiate(ctx);
    let resize_throttle_ms = options.resize_throttle_ms;
    let state = ChartAppState::new(session, resize_policy, options);
    let streams = vec![(
        EventStreamConfig {
            types: vec![SceneGraphEventType::WindowResize],
            throttle: resize_throttle_ms,
            ..Default::default()
        },
        Arc::new(ChartResizeHandler) as Arc<dyn EventStreamHandler<ChartAppState>>,
    )];

    AvengerApp::try_new(state, Arc::new(ChartSceneGraphBuilder), streams).await
}

#[cfg(feature = "winit-wgpu")]
pub fn window_scene_sizing_for_resize_policy(policy: ChartResizePolicy) -> WindowSceneSizing {
    let match_width = !policy.width.is_canvas_constrained();
    let match_height = !policy.height.is_canvas_constrained();
    if match_width || match_height {
        WindowSceneSizing::MatchSceneGraphAxes {
            width: match_width,
            height: match_height,
        }
    } else {
        WindowSceneSizing::SurfaceFollowsWindow
    }
}

fn maybe_patch_axis(
    patch: &mut IndexMap<String, ScalarValue>,
    policy: ChartResizeAxisPolicy,
    param: Option<&str>,
    size: f32,
    current_params: &IndexMap<String, ScalarValue>,
) {
    if !policy.is_canvas_constrained() {
        return;
    }
    let Some(param) = param else {
        return;
    };

    let value = ScalarValue::Float64(Some(size as f64));
    if current_params.get(param) == Some(&value) {
        return;
    }
    patch.insert(param.to_string(), value);
}

fn warn_about_ignored_bindings(policy: ChartResizePolicy, binding: &ChartResizeBinding) {
    warn_about_ignored_axis("width", policy.width, binding.width_param.as_deref());
    warn_about_ignored_axis("height", policy.height, binding.height_param.as_deref());
}

fn warn_about_ignored_axis(axis: &str, policy: ChartResizeAxisPolicy, param: Option<&str>) {
    if let Some(param) = param {
        if !policy.is_canvas_constrained() {
            log::warn!(
                "chart resize binding for {axis} param '{param}' will be ignored because the {axis} axis policy is {policy:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use avenger_chart::prelude::*;
    use avenger_eventstream::{scene::SceneGraphEvent, window::WindowResizeEvent};
    use avenger_scenegraph::scene_graph::SceneGraph;

    use super::*;

    async fn resize_test_state() -> ChartAppState {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let height = Param::new("height", ScalarValue::Float64(Some(480.0)));
        let compiled = Plot::<Cartesian>::new()
            .add_params([width.clone(), height.clone()])
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(height.expr()))
            .compile(&ctx)
            .await
            .expect("compile resize test plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        ChartAppState::new(
            session,
            policy,
            ChartAppOptions {
                resize_binding: ChartResizeBinding::width_height("width", "height"),
                resize_throttle_ms: None,
                log_metrics: false,
            },
        )
    }

    fn empty_rtree() -> SceneGraphRTree {
        SceneGraphRTree::from_scene_graph(&SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    #[tokio::test]
    async fn resize_handler_patches_only_canvas_constrained_axes() {
        let mut state = resize_test_state().await;
        let handler = ChartResizeHandler;
        let rtree = empty_rtree();
        let status = handler
            .handle(
                &SceneGraphEvent::WindowResize(WindowResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        assert!(status.rerender);
        assert!(status.rebuild_geometry);
        let params = state.params().await;
        assert_eq!(
            params.get("width"),
            Some(&ScalarValue::Float64(Some(800.0)))
        );
        assert_eq!(
            params.get("height"),
            Some(&ScalarValue::Float64(Some(480.0)))
        );
        assert_eq!(state.accepted_resize_count().await, 1);
    }

    #[tokio::test]
    async fn resize_handler_ignores_unbound_canvas_axes() {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let compiled = Plot::<Cartesian>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .compile(&ctx)
            .await
            .expect("compile resize test plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, policy, ChartAppOptions::default());
        let rtree = empty_rtree();

        let status = ChartResizeHandler
            .handle(
                &SceneGraphEvent::WindowResize(WindowResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        assert!(!status.rerender);
        assert!(!status.rebuild_geometry);
        assert_eq!(state.accepted_resize_count().await, 0);
    }
}
