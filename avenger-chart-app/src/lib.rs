#![recursion_limit = "512"]

mod event_binding;

use std::{
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_chart::{
    layout::{ChartResizeAxisPolicy, ChartResizePolicy},
    plot::{CompiledPlot, EvaluationRequest, PlotSession, ScopedParamAssignment},
    render::{
        EvaluatedEventDatumState, EvaluatedInteractionScope, EvaluatedInteractionState,
        EvaluationMetrics, EvaluationMode, EvaluationOptions,
    },
};
use avenger_chart_core::ScalarValueHelpers;
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

use crate::event_binding::{event_streams_for_bindings, event_streams_for_plot_bindings};

#[cfg(feature = "winit-wgpu")]
pub use avenger_winit_wgpu::{
    CanvasFrameOptions, WindowSceneSizing, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
};

/// Parameter names that receive accepted virtual canvas resize dimensions.
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
    pub exact_on_resize_settle: bool,
    pub log_metrics: bool,
}

impl Default for ChartAppOptions {
    fn default() -> Self {
        Self {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: false,
        }
    }
}

/// Cloneable app state wrapper around the stateful chart session runtime.
#[derive(Clone)]
pub struct ChartAppState {
    runtime: Arc<Mutex<ChartAppRuntime>>,
    params: Arc<StdMutex<ChartParamState>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartEventMetrics {
    pub event_batches_evaluated: usize,
    pub physical_expression_evaluations: usize,
    pub filter_passes: usize,
    pub filter_failures: usize,
    pub param_patch_events: usize,
    pub params_patched: usize,
    pub store_patch_events: usize,
    pub unchanged_patch_skips: usize,
    pub evaluation_errors: usize,
    pub total_eval_us: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamChange {
    pub name: String,
    pub value: ScalarValue,
    pub previous: Option<ScalarValue>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamSnapshot {
    pub params: IndexMap<String, ScalarValue>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamSetResult {
    pub changed: bool,
    pub revision: u64,
    pub change: Option<ParamChange>,
}

pub trait IntoChartParamValue {
    fn into_chart_param_value(self) -> ScalarValue;
}

impl IntoChartParamValue for ScalarValue {
    fn into_chart_param_value(self) -> ScalarValue {
        self
    }
}

impl IntoChartParamValue for f64 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Float64(Some(self))
    }
}

impl IntoChartParamValue for f32 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Float64(Some(self as f64))
    }
}

impl IntoChartParamValue for bool {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Boolean(Some(self))
    }
}

impl IntoChartParamValue for i64 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Int64(Some(self))
    }
}

impl IntoChartParamValue for i32 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Int64(Some(self as i64))
    }
}

impl IntoChartParamValue for u64 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::UInt64(Some(self))
    }
}

impl IntoChartParamValue for u32 {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::UInt64(Some(self as u64))
    }
}

impl IntoChartParamValue for String {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Utf8(Some(self))
    }
}

impl IntoChartParamValue for &str {
    fn into_chart_param_value(self) -> ScalarValue {
        ScalarValue::Utf8(Some(self.to_string()))
    }
}

#[derive(Clone, Debug)]
struct ChartParamState {
    params: IndexMap<String, ScalarValue>,
    pending_patch: IndexMap<String, ScalarValue>,
    changes: Vec<ParamChange>,
    revision: u64,
}

impl ChartParamState {
    fn new(params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            params,
            pending_patch: IndexMap::new(),
            changes: Vec::new(),
            revision: 0,
        }
    }

    fn snapshot(&self) -> ParamSnapshot {
        ParamSnapshot {
            params: self.params.clone(),
            revision: self.revision,
        }
    }

    fn set_param(&mut self, name: String, value: ScalarValue) -> ParamSetResult {
        let previous = self.params.get(&name).cloned();
        if previous.as_ref() == Some(&value) {
            return ParamSetResult {
                changed: false,
                revision: self.revision,
                change: None,
            };
        }

        self.revision += 1;
        let change = ParamChange {
            name: name.clone(),
            value: value.clone(),
            previous,
            revision: self.revision,
        };
        self.params.insert(name.clone(), value.clone());
        self.pending_patch.insert(name, value);
        self.changes.push(change.clone());

        ParamSetResult {
            changed: true,
            revision: self.revision,
            change: Some(change),
        }
    }

    fn sync_from_params(&mut self, params: IndexMap<String, ScalarValue>) -> Vec<ParamChange> {
        let mut changes = Vec::new();
        for (name, value) in &params {
            let previous = self.params.get(name).cloned();
            if previous.as_ref() == Some(value) {
                continue;
            }

            self.revision += 1;
            let change = ParamChange {
                name: name.clone(),
                value: value.clone(),
                previous,
                revision: self.revision,
            };
            self.changes.push(change.clone());
            changes.push(change);
        }
        self.params = params;
        changes
    }

    fn drain_pending_patch(&mut self) -> Option<IndexMap<String, ScalarValue>> {
        if self.pending_patch.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.pending_patch))
        }
    }

    fn changes_since(&self, revision: u64) -> Vec<ParamChange> {
        self.changes
            .iter()
            .filter(|change| change.revision > revision)
            .cloned()
            .collect()
    }
}

struct ChartAppRuntime {
    session: PlotSession,
    resize_policy: ChartResizePolicy,
    resize_binding: ChartResizeBinding,
    exact_on_resize_settle: bool,
    next_evaluation_mode: EvaluationMode,
    log_metrics: bool,
    trace_resize: bool,
    last_metrics: Option<EvaluationMetrics>,
    last_evaluation_elapsed: Option<Duration>,
    last_scene_size: Option<[f32; 2]>,
    accepted_resize_count: usize,
    event_metrics: ChartEventMetrics,
    last_evaluated_param_revision: u64,
    /// Interaction scopes from the most recent evaluation, used to route pointer
    /// events to coordinate scopes for inversion.
    last_interaction_state: EvaluatedInteractionState,
    /// Event datum rows from the most recent evaluation, used by `ev::datum`.
    last_event_datum_state: EvaluatedEventDatumState,
}

impl ChartAppState {
    pub fn new(
        session: PlotSession,
        resize_policy: ChartResizePolicy,
        options: ChartAppOptions,
    ) -> Self {
        warn_about_ignored_bindings(resize_policy, &options.resize_binding);
        let params = session.params().clone();
        Self {
            params: Arc::new(StdMutex::new(ChartParamState::new(params))),
            runtime: Arc::new(Mutex::new(ChartAppRuntime {
                session,
                resize_policy,
                resize_binding: options.resize_binding,
                exact_on_resize_settle: options.exact_on_resize_settle,
                next_evaluation_mode: EvaluationMode::Exact,
                log_metrics: options.log_metrics,
                trace_resize: std::env::var_os("AVENGER_TRACE_RESIZE").is_some(),
                last_metrics: None,
                last_evaluation_elapsed: None,
                last_scene_size: None,
                accepted_resize_count: 0,
                event_metrics: ChartEventMetrics::default(),
                last_evaluated_param_revision: 0,
                last_interaction_state: EvaluatedInteractionState::default(),
                last_event_datum_state: EvaluatedEventDatumState::default(),
            })),
        }
    }

    pub async fn resize_policy(&self) -> ChartResizePolicy {
        self.runtime.lock().await.resize_policy
    }

    pub async fn params(&self) -> IndexMap<String, ScalarValue> {
        if let Ok(mut runtime) = self.runtime.try_lock() {
            self.drain_pending_params_into_runtime(&mut runtime);
            self.sync_param_state_from_runtime(&runtime);
        }
        self.param_snapshot().params
    }

    pub fn param_snapshot(&self) -> ParamSnapshot {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .snapshot()
    }

    pub fn param_revision(&self) -> u64 {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .revision
    }

    pub fn param_changes_since(&self, revision: u64) -> Vec<ParamChange> {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .changes_since(revision)
    }

    pub fn param_f64(&self, name: &str) -> Option<f64> {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .params
            .get(name)
            .and_then(|value| value.as_f64().ok())
    }

    pub fn param_bool(&self, name: &str) -> Option<bool> {
        match self
            .params
            .lock()
            .expect("chart param lock poisoned")
            .params
            .get(name)
        {
            Some(ScalarValue::Boolean(Some(value))) => Some(*value),
            _ => None,
        }
    }

    pub fn set_param(
        &self,
        name: impl Into<String>,
        value: impl IntoChartParamValue,
    ) -> ParamSetResult {
        let result = self.set_param_inner(name.into(), value.into_chart_param_value());
        if result.changed
            && let Ok(mut runtime) = self.runtime.try_lock()
        {
            if self.drain_pending_params_into_runtime(&mut runtime) {
                runtime.next_evaluation_mode = EvaluationMode::Exact;
            }
        }
        result
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

    pub async fn event_metrics(&self) -> ChartEventMetrics {
        self.runtime.lock().await.event_metrics.clone()
    }

    /// Interaction scopes from the most recent evaluation (for tests/diagnostics).
    pub async fn interaction_scopes(&self) -> Vec<EvaluatedInteractionScope> {
        self.runtime
            .lock()
            .await
            .last_interaction_state
            .scopes
            .clone()
    }

    fn set_param_inner(&self, name: String, value: ScalarValue) -> ParamSetResult {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .set_param(name, value)
    }

    pub(crate) fn drain_pending_params_into_runtime(&self, runtime: &mut ChartAppRuntime) -> bool {
        let patch = self
            .params
            .lock()
            .expect("chart param lock poisoned")
            .drain_pending_patch();
        let Some(patch) = patch else {
            return false;
        };
        runtime.session.apply_param_patch(patch);
        true
    }

    pub(crate) fn apply_root_param_patch_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        patch: IndexMap<String, ScalarValue>,
    ) -> Vec<ParamChange> {
        if patch.is_empty() {
            return Vec::new();
        }
        runtime.session.apply_param_patch(patch);
        self.sync_param_state_from_runtime(runtime)
    }

    pub(crate) fn apply_scoped_param_patch_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        patch: Vec<ScopedParamAssignment>,
    ) -> Vec<ParamChange> {
        if patch.is_empty() {
            return Vec::new();
        }
        runtime.session.apply_scoped_param_patch(patch);
        self.sync_param_state_from_runtime(runtime)
    }

    fn sync_param_state_from_runtime(&self, runtime: &ChartAppRuntime) -> Vec<ParamChange> {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .sync_from_params(runtime.session.params().clone())
    }
}

/// Scene graph builder that evaluates the chart session stored in state.
pub struct ChartSceneGraphBuilder;

#[async_trait]
impl SceneGraphBuilder<ChartAppState> for ChartSceneGraphBuilder {
    async fn build(&self, state: &mut ChartAppState) -> Result<SceneGraph, AvengerAppError> {
        let mut runtime = state.runtime.lock().await;
        let pending_params = state.drain_pending_params_into_runtime(&mut runtime);
        let mode = if pending_params {
            EvaluationMode::Exact
        } else {
            runtime.next_evaluation_mode
        };
        runtime.next_evaluation_mode = EvaluationMode::Exact;
        let evaluation_param_revision = state.param_revision();

        let start = Instant::now();
        tracing::debug!(
            target: "avenger_chart_app::resize",
            mode = ?mode,
            resize_seq = runtime.accepted_resize_count,
            "chart_app.scene_build start"
        );
        let (evaluated, metrics) =
            runtime
                .session
                .evaluate_with_metrics(EvaluationRequest::new().mode(mode).options(
                    EvaluationOptions {
                        build_scene_rtree: false,
                        ..EvaluationOptions::default()
                    },
                ))
                .await
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let elapsed = start.elapsed();
        let scene_size = [evaluated.scene_graph.width, evaluated.scene_graph.height];

        if runtime.log_metrics {
            eprintln!(
                "chart eval mode={:?} elapsed={:?} scene={:.1}x{:.1} preview_reuse={} reflow_reuse={} facet_tree_builds={} facet_tree_reuse={} cell_reuse={} data_reuse={} data_miss={} chrome_refresh={} skipped_measures={} guide_measures={}",
                metrics.mode,
                elapsed,
                scene_size[0],
                scene_size[1],
                metrics.pipeline.preview_profile_reuses,
                metrics.pipeline.preview_structure_reflow_reuses,
                metrics.pipeline.facet_tree_builds,
                metrics.pipeline.facet_tree_profile_reuses,
                metrics.pipeline.facet_cell_measurement_profile_reuses,
                metrics.pipeline.preview_data_mark_reuses,
                metrics.pipeline.preview_data_mark_reuse_misses,
                metrics
                    .pipeline
                    .facet_cell_measurement_profile_chrome_refreshes,
                metrics.pipeline.skipped_component_measure_calls,
                metrics.pipeline.guide_overflow_measure_calls,
            );
        }
        if runtime.trace_resize && runtime.accepted_resize_count > 0 {
            let timing = &metrics.timings;
            eprintln!(
                "resize seq={} width={:.1} height={:.1} mode={:?} total={:.2}ms eval={:.2}ms reflow_reuse={} preview_reuse={} cell_reuse={} data_reuse={} data_miss={} chrome={} chrome_ms={:.2} guides={} guide_ms={:.2} reflow_ms={:.2} probe_ms={:.2} build_ms={:.2} scene_ms={:.2}",
                runtime.accepted_resize_count,
                scene_size[0],
                scene_size[1],
                metrics.mode,
                elapsed.as_secs_f64() * 1000.0,
                elapsed.as_secs_f64() * 1000.0,
                metrics.pipeline.preview_structure_reflow_reuses,
                metrics.pipeline.preview_profile_reuses,
                metrics.pipeline.facet_cell_measurement_profile_reuses,
                metrics.pipeline.preview_data_mark_reuses,
                metrics.pipeline.preview_data_mark_reuse_misses,
                metrics
                    .pipeline
                    .facet_cell_measurement_profile_chrome_refreshes,
                us_to_ms(timing.refresh_reused_profile_layout_us),
                metrics.pipeline.guide_overflow_measure_calls,
                us_to_ms(timing.guide_overflow_measure_us),
                us_to_ms(timing.preview_structure_reflow_us),
                us_to_ms(timing.measure_cells_overflow_probe_us),
                us_to_ms(timing.build_plot_components_us),
                us_to_ms(timing.components_to_evaluated_plot_us),
            );
            tracing::info!(
                target: "avenger_chart_app::resize",
                seq = runtime.accepted_resize_count,
                event_kind = "CanvasResize",
                mode = ?metrics.mode,
                scene_width = scene_size[0],
                scene_height = scene_size[1],
                chart_eval_ms = elapsed.as_secs_f64() * 1000.0,
                preview_reuse = metrics.pipeline.preview_profile_reuses,
                reflow_reuse = metrics.pipeline.preview_structure_reflow_reuses,
                cell_reuse = metrics.pipeline.facet_cell_measurement_profile_reuses,
                data_reuse = metrics.pipeline.preview_data_mark_reuses,
                data_miss = metrics.pipeline.preview_data_mark_reuse_misses,
                chrome_refresh = metrics.pipeline.facet_cell_measurement_profile_chrome_refreshes,
                chrome_ms = us_to_ms(timing.refresh_reused_profile_layout_us),
                guide_measures = metrics.pipeline.guide_overflow_measure_calls,
                guide_ms = us_to_ms(timing.guide_overflow_measure_us),
                reflow_ms = us_to_ms(timing.preview_structure_reflow_us),
                probe_ms = us_to_ms(timing.measure_cells_overflow_probe_us),
                build_plot_components_ms = us_to_ms(timing.build_plot_components_us),
                components_to_evaluated_plot_ms = us_to_ms(timing.components_to_evaluated_plot_us),
                "resize"
            );
        }

        runtime.last_metrics = Some(metrics);
        runtime.last_evaluation_elapsed = Some(elapsed);
        runtime.last_scene_size = Some(scene_size);
        runtime.last_evaluated_param_revision = evaluation_param_revision;
        if state.param_revision() != evaluation_param_revision {
            runtime.next_evaluation_mode = EvaluationMode::Exact;
        }
        runtime.last_interaction_state = evaluated.interaction;
        runtime.last_event_datum_state = evaluated.event_datums;
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
        let SceneGraphEvent::CanvasResize(event) = event else {
            return UpdateStatus::default();
        };

        let mut runtime = state.runtime.lock().await;
        state.drain_pending_params_into_runtime(&mut runtime);
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

        state.apply_root_param_patch_to_runtime(&mut runtime, patch);
        runtime.next_evaluation_mode = EvaluationMode::Preview;
        runtime.accepted_resize_count += 1;
        tracing::debug!(
            target: "avenger_chart_app::resize",
            seq = runtime.accepted_resize_count,
            width = event.size[0],
            height = event.size[1],
            mode = ?runtime.next_evaluation_mode,
            "canvas resize accepted"
        );
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
            ..Default::default()
        }
    }
}

fn us_to_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

/// Resize-settle handler that requests an exact evaluation after preview resize.
pub struct ChartResizeSettleHandler;

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartResizeSettleHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::CanvasResizeSettled(event) = event else {
            return UpdateStatus::default();
        };

        let mut runtime = state.runtime.lock().await;
        if !runtime.exact_on_resize_settle
            || !settled_size_matches_current_params(
                &runtime.resize_policy,
                &runtime.resize_binding,
                event.size,
                runtime.session.params(),
            )
        {
            return UpdateStatus::default();
        }

        runtime.next_evaluation_mode = EvaluationMode::Exact;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
            ..Default::default()
        }
    }
}

pub async fn chart_avenger_app(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
) -> Result<AvengerApp<ChartAppState>, AvengerAppError> {
    let resize_policy = compiled_plot.resize_policy();
    let mut event_streams = event_streams_for_plot_bindings(&compiled_plot, ctx.as_ref())?;
    let resize_bindings = resize_event_bindings(
        resize_policy,
        &options.resize_binding,
        options.resize_throttle_ms,
    );
    event_streams.extend(event_streams_for_bindings(
        &resize_bindings,
        ctx.as_ref(),
        compiled_plot.param_specs(),
        compiled_plot.selection_specs(),
        compiled_plot.store_specs(),
        &[],
        &IndexMap::new(),
    )?);
    let session = Arc::new(compiled_plot).instantiate(ctx);
    let exact_on_resize_settle = options.exact_on_resize_settle;
    let state = ChartAppState::new(session, resize_policy, options);
    let mut streams = event_streams;
    if exact_on_resize_settle {
        streams.push((
            EventStreamConfig {
                types: vec![SceneGraphEventType::CanvasResizeSettled],
                ..Default::default()
            },
            Arc::new(ChartResizeSettleHandler) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));
    }

    AvengerApp::try_new(state, Arc::new(ChartSceneGraphBuilder), streams).await
}

fn resize_event_bindings(
    policy: ChartResizePolicy,
    binding: &ChartResizeBinding,
    throttle_ms: Option<u64>,
) -> Vec<avenger_chart::event::ChartEventBinding> {
    use avenger_chart::event::{self as ev, ChartEventBinding, ChartEventType};

    let mut resize = ChartEventBinding::on(ChartEventType::CanvasResize).preview();
    if let Some(ms) = throttle_ms {
        resize = resize.throttle_ms(ms);
    }
    let mut has_assignment = false;
    if policy.width.is_canvas_constrained()
        && let Some(param) = binding.width_param.as_deref()
    {
        resize = resize.set_param(param, ev::canvas_width());
        has_assignment = true;
    }
    if policy.height.is_canvas_constrained()
        && let Some(param) = binding.height_param.as_deref()
    {
        resize = resize.set_param(param, ev::canvas_height());
        has_assignment = true;
    }
    if has_assignment {
        vec![resize]
    } else {
        Vec::new()
    }
}

#[cfg(feature = "winit-wgpu")]
pub fn window_scene_sizing_for_resize_policy(policy: ChartResizePolicy) -> WindowSceneSizing {
    if policy.has_canvas_constrained_axis() {
        return WindowSceneSizing::SurfaceFollowsWindow;
    }

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

#[cfg(feature = "winit-wgpu")]
pub fn canvas_frame_options_for_resize_policy(
    policy: ChartResizePolicy,
) -> Option<CanvasFrameOptions> {
    if !policy.has_canvas_constrained_axis() {
        return None;
    }

    Some(CanvasFrameOptions {
        resize_width: policy.width.is_canvas_constrained(),
        resize_height: policy.height.is_canvas_constrained(),
        ..Default::default()
    })
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

fn settled_size_matches_current_params(
    policy: &ChartResizePolicy,
    binding: &ChartResizeBinding,
    size: [f32; 2],
    current_params: &IndexMap<String, ScalarValue>,
) -> bool {
    axis_matches_current_param(
        policy.width,
        binding.width_param.as_deref(),
        size[0],
        current_params,
    ) && axis_matches_current_param(
        policy.height,
        binding.height_param.as_deref(),
        size[1],
        current_params,
    )
}

fn axis_matches_current_param(
    policy: ChartResizeAxisPolicy,
    param: Option<&str>,
    size: f32,
    current_params: &IndexMap<String, ScalarValue>,
) -> bool {
    if !policy.is_canvas_constrained() {
        return true;
    }
    let Some(param) = param else {
        return true;
    };
    current_params.get(param) == Some(&ScalarValue::Float64(Some(size as f64)))
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
    use avenger_eventstream::{
        scene::SceneGraphEvent,
        window::{CanvasResizeEvent, WindowResizeEvent},
    };
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
                exact_on_resize_settle: true,
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
    async fn set_param_updates_snapshot_and_typed_getter_before_scene_build() {
        use avenger_app::app::SceneGraphBuilder;

        let mut state = resize_test_state().await;

        let result = state.set_param("width", 720.0);

        assert!(result.changed);
        assert_eq!(result.revision, 1);
        assert_eq!(state.param_f64("width"), Some(720.0));
        assert_eq!(state.param_revision(), 1);
        assert!(state.last_metrics().await.is_none());

        let snapshot = state.param_snapshot();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot.params.get("width"),
            Some(&ScalarValue::Float64(Some(720.0)))
        );

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("exact build after set_param");
        assert_eq!(
            state.last_metrics().await.expect("exact metrics").mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn set_param_unchanged_value_does_not_emit_change() {
        let state = resize_test_state().await;

        let result = state.set_param("width", 640.0);

        assert!(!result.changed);
        assert_eq!(result.revision, 0);
        assert!(result.change.is_none());
        assert!(state.param_changes_since(0).is_empty());
        assert_eq!(state.param_revision(), 0);
    }

    #[tokio::test]
    async fn rapid_set_param_calls_keep_latest_value_and_revision_order() {
        let state = resize_test_state().await;

        state.set_param("width", 700.0);
        state.set_param("width", 710.0);
        state.set_param("width", 720.0);

        assert_eq!(state.param_f64("width"), Some(720.0));
        assert_eq!(state.param_revision(), 3);

        let changes = state.param_changes_since(0);
        assert_eq!(changes.len(), 3);
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.revision, change.value.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, ScalarValue::Float64(Some(700.0))),
                (2, ScalarValue::Float64(Some(710.0))),
                (3, ScalarValue::Float64(Some(720.0))),
            ]
        );
    }

    #[tokio::test]
    async fn param_changes_since_filters_by_revision() {
        let state = resize_test_state().await;

        state.set_param("width", 700.0);
        state.set_param("height", 500.0);

        let changes = state.param_changes_since(1);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "height");
        assert_eq!(changes[0].revision, 2);
        assert_eq!(changes[0].value, ScalarValue::Float64(Some(500.0)));
    }

    #[tokio::test]
    async fn param_bool_reads_bool_param() {
        let ctx = SessionContext::new();
        let enabled = Param::new("enabled", ScalarValue::Boolean(Some(true)));
        let compiled = Plot::<Cartesian>::new()
            .add_param(enabled)
            .compile(&ctx)
            .await
            .expect("compile bool param test plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, ChartAppOptions::default());

        assert_eq!(state.param_bool("enabled"), Some(true));
        let result = state.set_param("enabled", false);

        assert!(result.changed);
        assert_eq!(state.param_bool("enabled"), Some(false));
        assert_eq!(state.param_f64("enabled"), None);
    }

    #[tokio::test]
    async fn set_param_queues_when_runtime_is_busy() {
        use avenger_app::app::SceneGraphBuilder;

        let mut state = resize_test_state().await;
        let runtime = state.runtime.lock().await;

        let result = state.set_param("width", 700.0);

        assert!(result.changed);
        assert_eq!(state.param_f64("width"), Some(700.0));
        assert_eq!(
            runtime.session.params().get("width"),
            Some(&ScalarValue::Float64(Some(640.0)))
        );

        drop(runtime);
        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after queued set_param");
        let runtime = state.runtime.lock().await;
        assert_eq!(
            runtime.session.params().get("width"),
            Some(&ScalarValue::Float64(Some(700.0)))
        );
        assert_eq!(
            runtime
                .last_metrics
                .as_ref()
                .expect("metrics after queued set_param")
                .mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn resize_handler_records_param_change_revision() {
        let mut state = resize_test_state().await;
        let handler = ChartResizeHandler;
        let rtree = empty_rtree();
        let status = handler
            .handle(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        assert!(status.rerender);
        let changes = state.param_changes_since(0);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "width");
        assert_eq!(changes[0].revision, 1);
        assert_eq!(changes[0].value, ScalarValue::Float64(Some(800.0)));
        assert_eq!(state.param_f64("width"), Some(800.0));
    }

    #[tokio::test]
    async fn resize_handler_patches_only_canvas_constrained_axes() {
        let mut state = resize_test_state().await;
        let handler = ChartResizeHandler;
        let rtree = empty_rtree();
        let status = handler
            .handle(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
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
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
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

    #[tokio::test]
    async fn resize_tracing_follows_env_flag() {
        let state = resize_test_state().await;
        assert_eq!(
            state.runtime.lock().await.trace_resize,
            std::env::var_os("AVENGER_TRACE_RESIZE").is_some()
        );
    }

    #[tokio::test]
    async fn native_window_resize_does_not_patch_chart_params() {
        let mut state = resize_test_state().await;
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
        let params = state.params().await;
        assert_eq!(
            params.get("width"),
            Some(&ScalarValue::Float64(Some(640.0)))
        );
        assert_eq!(state.accepted_resize_count().await, 0);
    }

    #[tokio::test]
    async fn resize_settle_handler_requests_exact_for_current_size() {
        use avenger_app::app::SceneGraphBuilder;

        let mut state = resize_test_state().await;
        let rtree = empty_rtree();
        ChartResizeHandler
            .handle(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("preview build");
        assert_eq!(
            state.last_metrics().await.expect("preview metrics").mode,
            EvaluationMode::Preview
        );

        let status = ChartResizeSettleHandler
            .handle(
                &SceneGraphEvent::CanvasResizeSettled(CanvasResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;
        assert!(status.rerender);
        assert!(status.rebuild_geometry);

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("exact build");
        assert_eq!(
            state.last_metrics().await.expect("exact metrics").mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn resize_settle_handler_ignores_stale_size() {
        let mut state = resize_test_state().await;
        let rtree = empty_rtree();
        ChartResizeHandler
            .handle(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        let status = ChartResizeSettleHandler
            .handle(
                &SceneGraphEvent::CanvasResizeSettled(CanvasResizeEvent {
                    size: [720.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;

        assert!(!status.rerender);
        assert!(!status.rebuild_geometry);
    }

    #[cfg(feature = "winit-wgpu")]
    #[test]
    fn winit_sizing_follows_window_when_any_axis_is_canvas_constrained() {
        use avenger_winit_wgpu::WindowSceneSizing;

        let width_canvas = ChartResizePolicy {
            width: ChartResizeAxisPolicy::CanvasConstrained,
            height: ChartResizeAxisPolicy::PlotConstrained,
        };
        assert_eq!(
            window_scene_sizing_for_resize_policy(width_canvas),
            WindowSceneSizing::SurfaceFollowsWindow
        );

        let height_canvas = ChartResizePolicy {
            width: ChartResizeAxisPolicy::PlotConstrained,
            height: ChartResizeAxisPolicy::CanvasConstrained,
        };
        assert_eq!(
            window_scene_sizing_for_resize_policy(height_canvas),
            WindowSceneSizing::SurfaceFollowsWindow
        );
    }

    #[cfg(feature = "winit-wgpu")]
    #[test]
    fn winit_sizing_matches_scene_when_no_axis_is_canvas_constrained() {
        use avenger_winit_wgpu::WindowSceneSizing;

        let fixed_size = ChartResizePolicy {
            width: ChartResizeAxisPolicy::PlotConstrained,
            height: ChartResizeAxisPolicy::PlotConstrained,
        };
        assert_eq!(
            window_scene_sizing_for_resize_policy(fixed_size),
            WindowSceneSizing::MatchSceneGraphAxes {
                width: true,
                height: true,
            }
        );
    }

    #[cfg(feature = "winit-wgpu")]
    #[test]
    fn canvas_frame_options_enable_only_canvas_constrained_axes() {
        let width_canvas = ChartResizePolicy {
            width: ChartResizeAxisPolicy::CanvasConstrained,
            height: ChartResizeAxisPolicy::PlotConstrained,
        };
        let options = canvas_frame_options_for_resize_policy(width_canvas).expect("frame options");
        assert!(options.resize_width);
        assert!(!options.resize_height);

        let fixed_size = ChartResizePolicy {
            width: ChartResizeAxisPolicy::PlotConstrained,
            height: ChartResizeAxisPolicy::PlotConstrained,
        };
        assert!(canvas_frame_options_for_resize_policy(fixed_size).is_none());
    }
}
