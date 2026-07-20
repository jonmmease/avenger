#![recursion_limit = "512"]

mod event_binding;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};

use async_trait::async_trait;
use avenger_app::{
    app::{AvengerApp, SceneGraphBuilder},
    error::AvengerAppError,
};
use avenger_chart::{
    layout::{ChartResizeAxisPolicy, ChartResizePolicy},
    plot::{
        ChartSessionSnapshot, CompiledPlot, EvaluationRequest, NativeWidgetDispatchOutcome,
        NativeWidgetEvaluationIntent, NativeWidgetEvent, NativeWidgetEventRoute,
        NativeWidgetHostServices, NativeWidgetHostTransform, NativeWidgetPlotId,
        NativeWidgetRuntimeResources, PlotSession, PlotSessionOptions,
        ResolvedScopedParamAssignment, ResolvedScopedStoreAssignment, ResolvedSelectionAssignment,
        ScopedParamAssignment, StateMigrationReport,
    },
    render::{
        EvaluatedEventDatumState, EvaluatedInteractionScope, EvaluatedInteractionState,
        EvaluatedNativeWidgetState, EvaluatedWidgetFrame, EvaluatedWidgetFrameState,
        EvaluationMetrics, EvaluationMode, EvaluationOptions,
    },
};
use avenger_chart_core::ScalarValueHelpers;
use avenger_chart_core::{
    ChartEventEvaluationMode, EvaluationInvalidation, EvaluationInvalidationReason,
    EvaluationInvalidationSchedule, EvaluationInvalidationSubscription,
};
use avenger_chart_widgets::register_native_widgets;
use avenger_common::time::{Duration, Instant};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, EventStreamContext, UpdateStatus},
    window::MouseScrollDelta,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_image::{IMAGE_RESOURCE_KIND, ImageResourceCache, ImageResourceResolver};
use avenger_resource::{
    RenderInvalidationHub, RenderInvalidationReason, RenderInvalidationRequest,
    RenderInvalidationSchedule, RenderInvalidationSink, ResourceRequest, ResourceRequestPurpose,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use indexmap::IndexMap;
use tokio::sync::Mutex;

use crate::event_binding::{
    CompiledChartParamChangeGraph, committed_stream_param_provider, event_streams_for_bindings,
    event_streams_for_plot_bindings_with_param_provider, param_change_graph_for_plot,
};

#[cfg(feature = "winit-wgpu")]
pub use avenger_winit_wgpu::{
    CanvasConfig, CanvasFrameOptions, WgpuImagePlaceholder, WgpuImageResourceConfig,
    WgpuMissingImagePolicy, WindowSceneSizing, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    WinitWgpuEvent,
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

/// Shared resource/runtime handles used by chart app evaluation and render hosts.
#[derive(Clone)]
pub struct ChartRuntimeResources {
    pub image_resource_resolver: Arc<dyn ImageResourceResolver>,
    pub render_invalidation_hub: RenderInvalidationHub,
    pub native_widget_runtime: NativeWidgetRuntimeResources,
    pub native_widget_host_services: NativeWidgetHostServices,
}

impl ChartRuntimeResources {
    pub fn new(
        image_resource_resolver: Arc<dyn ImageResourceResolver>,
        render_invalidation_hub: RenderInvalidationHub,
    ) -> Self {
        let mut native_widget_registry = avenger_chart::plot::NativeWidgetRegistry::new();
        register_native_widgets(&mut native_widget_registry)
            .expect("built-in native widget registration must remain valid");
        let native_widget_runtime = NativeWidgetRuntimeResources::new(
            Arc::new(native_widget_registry),
            Arc::new(avenger_chart::plot::InMemoryNativeWidgetInstanceStore::new()),
            avenger_chart::plot::NativeWidgetDocumentId::new(),
        );
        let native_widget_host_services =
            native_widget_runtime.host_services(NativeWidgetPlotId::chart_root());
        Self {
            image_resource_resolver,
            render_invalidation_hub,
            native_widget_runtime,
            native_widget_host_services,
        }
    }

    pub fn with_native_widget_runtime(
        mut self,
        native_widget_runtime: NativeWidgetRuntimeResources,
    ) -> Self {
        self.native_widget_host_services =
            native_widget_runtime.host_services(NativeWidgetPlotId::chart_root());
        self.native_widget_runtime = native_widget_runtime;
        self
    }

    pub fn with_native_widget_host_services(
        mut self,
        native_widget_host_services: NativeWidgetHostServices,
    ) -> Self {
        self.native_widget_runtime.set_host_services(
            NativeWidgetPlotId::chart_root(),
            native_widget_host_services.clone(),
        );
        self.native_widget_host_services = native_widget_host_services;
        self
    }

    #[cfg(feature = "winit-wgpu")]
    pub fn configure_winit_options(
        &self,
        options: WinitWgpuAvengerAppOptions,
    ) -> WinitWgpuAvengerAppOptions {
        let services = self.native_widget_host_services.clone();
        options
            .render_invalidation_hub(self.render_invalidation_hub.clone())
            .clipboard_payload_provider(Arc::new(move || services.focused_clipboard_payload()))
    }
}

pub struct ChartAppBundle {
    pub app: AvengerApp<ChartAppState>,
    pub runtime_resources: ChartRuntimeResources,
}

impl ChartAppBundle {
    #[cfg(feature = "winit-wgpu")]
    pub fn configure_winit_options(
        &self,
        options: WinitWgpuAvengerAppOptions,
    ) -> WinitWgpuAvengerAppOptions {
        self.runtime_resources.configure_winit_options(options)
    }
}

/// Cloneable app state wrapper around the stateful chart session runtime.
#[derive(Clone)]
pub struct ChartAppState {
    runtime: Arc<Mutex<ChartAppRuntime>>,
    params: Arc<StdMutex<ChartParamState>>,
    has_param_reactions: bool,
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
    pub transaction_id: u64,
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
    pub transaction_id: u64,
    pub changed: bool,
    pub revision: u64,
    pub change: Option<ParamChange>,
    pub changes: Vec<ParamChange>,
}

pub(crate) struct ParamTransactionOutcome {
    transaction_id: u64,
    param_changed: bool,
    changes: Vec<ParamChange>,
    store_changed: bool,
    selection_changed: bool,
    evaluation_mode: EvaluationMode,
    settle_exact: bool,
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
    pending_transactions: VecDeque<PendingParamTransaction>,
    changes: Vec<ParamChange>,
    revision: u64,
    next_transaction_id: u64,
}

#[derive(Clone, Debug)]
struct PendingParamTransaction {
    id: u64,
    origin: ParamTransactionOrigin,
    patch: IndexMap<String, ScalarValue>,
    evaluation_mode: EvaluationMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParamTransactionOrigin {
    ExternalHost,
    ChartEvent,
    Resize,
    NativeWidget,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParamTransactionContext {
    id: u64,
    origin: ParamTransactionOrigin,
    evaluation_mode: EvaluationMode,
}

impl ParamTransactionContext {
    fn new(id: u64, origin: ParamTransactionOrigin, evaluation_mode: EvaluationMode) -> Self {
        Self {
            id,
            origin,
            evaluation_mode,
        }
    }
}

impl ChartParamState {
    fn new(params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            params,
            pending_transactions: VecDeque::new(),
            changes: Vec::new(),
            revision: 0,
            next_transaction_id: 1,
        }
    }

    fn snapshot(&self) -> ParamSnapshot {
        ParamSnapshot {
            params: self.params.clone(),
            revision: self.revision,
        }
    }

    fn set_param(&mut self, name: String, value: ScalarValue) -> ParamSetResult {
        let transaction_id = self.allocate_transaction_id();
        let previous = self.params.get(&name).cloned();
        if previous.as_ref() == Some(&value) {
            return ParamSetResult {
                transaction_id,
                changed: false,
                revision: self.revision,
                change: None,
                changes: Vec::new(),
            };
        }

        self.revision += 1;
        let change = ParamChange {
            transaction_id,
            name: name.clone(),
            value: value.clone(),
            previous,
            revision: self.revision,
        };
        self.params.insert(name.clone(), value.clone());
        self.pending_transactions
            .push_back(PendingParamTransaction {
                id: transaction_id,
                origin: ParamTransactionOrigin::ExternalHost,
                patch: IndexMap::from([(name, value)]),
                evaluation_mode: EvaluationMode::Exact,
            });
        self.changes.push(change.clone());

        ParamSetResult {
            transaction_id,
            changed: true,
            revision: self.revision,
            change: Some(change.clone()),
            changes: vec![change],
        }
    }

    fn allocate_transaction_id(&mut self) -> u64 {
        let id = self.next_transaction_id;
        self.next_transaction_id = self.next_transaction_id.saturating_add(1);
        id
    }

    fn queue_param_transaction(&mut self, id: u64, name: String, value: ScalarValue) -> bool {
        let projected = self
            .pending_transactions
            .iter()
            .rev()
            .find_map(|transaction| transaction.patch.get(&name))
            .or_else(|| self.params.get(&name));
        if projected == Some(&value) {
            return false;
        }
        self.pending_transactions
            .push_back(PendingParamTransaction {
                id,
                origin: ParamTransactionOrigin::ExternalHost,
                patch: IndexMap::from([(name, value)]),
                evaluation_mode: EvaluationMode::Exact,
            });
        true
    }

    fn queue_param_patch_transaction(
        &mut self,
        id: u64,
        patch: IndexMap<String, ScalarValue>,
    ) -> bool {
        let mut filtered = IndexMap::new();
        for (name, value) in patch {
            let projected = self
                .pending_transactions
                .iter()
                .rev()
                .find_map(|transaction| transaction.patch.get(&name))
                .or_else(|| self.params.get(&name));
            if projected != Some(&value) {
                filtered.insert(name, value);
            }
        }
        if filtered.is_empty() {
            return false;
        }
        self.pending_transactions
            .push_back(PendingParamTransaction {
                id,
                origin: ParamTransactionOrigin::ExternalHost,
                patch: filtered,
                evaluation_mode: EvaluationMode::Exact,
            });
        true
    }

    fn sync_from_params(
        &mut self,
        transaction_id: u64,
        params: IndexMap<String, ScalarValue>,
    ) -> Vec<ParamChange> {
        let mut changes = Vec::new();
        for (name, value) in &params {
            let previous = self.params.get(name).cloned();
            if previous.as_ref() == Some(value) {
                continue;
            }

            self.revision += 1;
            let change = ParamChange {
                transaction_id,
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

    fn drain_pending_transactions(&mut self) -> VecDeque<PendingParamTransaction> {
        std::mem::take(&mut self.pending_transactions)
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
    // Compiled at app construction in W6.3; consumed by the W6.4 transaction
    // coordinator without re-planning expressions per parameter write.
    param_change_graph: Option<Arc<CompiledChartParamChangeGraph>>,
    resize_policy: ChartResizePolicy,
    resize_binding: ChartResizeBinding,
    exact_on_resize_settle: bool,
    next_evaluation_mode: EvaluationMode,
    evaluation_now: Option<Instant>,
    interaction_settle_exact_pending: bool,
    log_metrics: bool,
    trace_resize: bool,
    last_metrics: Option<EvaluationMetrics>,
    last_evaluation_elapsed: Option<Duration>,
    last_scene_size: Option<[f32; 2]>,
    runtime_resources: Option<ChartRuntimeResources>,
    _evaluation_invalidation_subscription: Option<EvaluationInvalidationSubscription>,
    warned_missing_materialization_wakeup: bool,
    last_resource_requests: Vec<ResourceRequest>,
    accepted_resize_count: usize,
    event_metrics: ChartEventMetrics,
    last_evaluated_param_revision: u64,
    /// Interaction scopes from the most recent evaluation, used to route pointer
    /// events to coordinate scopes for inversion.
    last_interaction_state: EvaluatedInteractionState,
    /// Event datum rows from the most recent evaluation, used by `ev::datum`.
    last_event_datum_state: EvaluatedEventDatumState,
    /// Widget frames from the most recent evaluation, keyed by final mark path.
    last_widget_frame_state: EvaluatedWidgetFrameState,
    /// Native attachment epochs and outcomes from the most recent evaluation.
    last_native_widget_state: EvaluatedNativeWidgetState,
    /// Pointer capture for a native widget, retained through outside drags.
    active_native_widget_gesture: Option<NativeWidgetGestureCapture>,
    /// Widget frame captured before authored streams see a gesture start.
    active_widget_gesture_frame: Option<WidgetGestureFrameCapture>,
}

#[derive(Clone)]
struct WidgetGestureFrameCapture {
    start_instant: Instant,
    frame: EvaluatedWidgetFrame,
}

#[derive(Clone)]
struct NativeWidgetGestureCapture {
    route: NativeWidgetEventRoute,
    frame: EvaluatedWidgetFrame,
    start: [f32; 2],
    previous: [f32; 2],
}

impl ChartAppState {
    pub fn new(
        session: PlotSession,
        resize_policy: ChartResizePolicy,
        options: ChartAppOptions,
    ) -> Self {
        Self::new_with_runtime_resources(session, resize_policy, options, None)
    }

    pub fn new_with_runtime_resources(
        session: PlotSession,
        resize_policy: ChartResizePolicy,
        options: ChartAppOptions,
        runtime_resources: Option<ChartRuntimeResources>,
    ) -> Self {
        warn_about_ignored_bindings(resize_policy, &options.resize_binding);
        let params = session.params().clone();
        let evaluation_invalidation_subscription =
            subscribe_to_session_evaluation_invalidations(&session, runtime_resources.as_ref());
        Self {
            params: Arc::new(StdMutex::new(ChartParamState::new(params))),
            has_param_reactions: false,
            runtime: Arc::new(Mutex::new(ChartAppRuntime {
                session,
                param_change_graph: None,
                resize_policy,
                resize_binding: options.resize_binding,
                exact_on_resize_settle: options.exact_on_resize_settle,
                next_evaluation_mode: EvaluationMode::Exact,
                evaluation_now: None,
                interaction_settle_exact_pending: false,
                log_metrics: options.log_metrics,
                trace_resize: std::env::var_os("AVENGER_TRACE_RESIZE").is_some(),
                last_metrics: None,
                last_evaluation_elapsed: None,
                last_scene_size: None,
                runtime_resources,
                _evaluation_invalidation_subscription: evaluation_invalidation_subscription,
                warned_missing_materialization_wakeup: false,
                last_resource_requests: Vec::new(),
                accepted_resize_count: 0,
                event_metrics: ChartEventMetrics::default(),
                last_evaluated_param_revision: 0,
                last_interaction_state: EvaluatedInteractionState::default(),
                last_event_datum_state: EvaluatedEventDatumState::default(),
                last_widget_frame_state: EvaluatedWidgetFrameState::default(),
                last_native_widget_state: EvaluatedNativeWidgetState::default(),
                active_native_widget_gesture: None,
                active_widget_gesture_frame: None,
            })),
        }
    }

    pub async fn resize_policy(&self) -> ChartResizePolicy {
        self.runtime.lock().await.resize_policy
    }

    pub async fn params(&self) -> IndexMap<String, ScalarValue> {
        if let Ok(mut runtime) = self.runtime.try_lock() {
            self.drain_pending_params_into_runtime(&mut runtime);
            self.sync_param_state_from_runtime(&runtime, 0);
        }
        self.param_snapshot().params
    }

    /// Capture complete typed document state for a replacement generation.
    pub async fn snapshot_state(&self) -> ChartSessionSnapshot {
        let mut runtime = self.runtime.lock().await;
        self.drain_pending_params_into_runtime(&mut runtime);
        self.sync_param_state_from_runtime(&runtime, 0);
        runtime.session.snapshot_state()
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

    fn next_param_transaction_id(&self) -> u64 {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .allocate_transaction_id()
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
    ) -> Result<ParamSetResult, AvengerAppError> {
        let name = name.into();
        let value = value.into_chart_param_value();
        if self.has_param_reactions {
            let transaction_id = self.next_param_transaction_id();
            if let Ok(mut runtime) = self.runtime.try_lock() {
                let outcome = self.apply_root_param_transaction_to_runtime(
                    &mut runtime,
                    ParamTransactionContext::new(
                        transaction_id,
                        ParamTransactionOrigin::ExternalHost,
                        EvaluationMode::Exact,
                    ),
                    IndexMap::from([(name.clone(), value)]),
                    Vec::new(),
                    Vec::new(),
                );
                return match outcome {
                    Ok(outcome) => {
                        if outcome.param_changed {
                            runtime.next_evaluation_mode = outcome.evaluation_mode;
                        }
                        let state = self.params.lock().expect("chart param lock poisoned");
                        let change = outcome.changes.iter().find(|change| change.name == name);
                        Ok(ParamSetResult {
                            transaction_id: outcome.transaction_id,
                            changed: outcome.param_changed,
                            revision: state.revision,
                            change: change.cloned(),
                            changes: outcome.changes,
                        })
                    }
                    Err(error) => Err(error),
                };
            }
            let changed = self
                .params
                .lock()
                .expect("chart param lock poisoned")
                .queue_param_transaction(transaction_id, name, value);
            return Ok(ParamSetResult {
                transaction_id,
                changed,
                revision: self.param_revision(),
                change: None,
                changes: Vec::new(),
            });
        }

        let result = self.set_param_inner(name, value);
        if result.changed
            && let Ok(mut runtime) = self.runtime.try_lock()
            && self.drain_pending_params_into_runtime(&mut runtime)
        {
            runtime.next_evaluation_mode = EvaluationMode::Exact;
        }
        Ok(result)
    }

    /// Apply several root parameter writes as one atomic transaction.
    ///
    /// Reactions observe the whole initiating patch as wave zero. If the app
    /// runtime is busy, the batch retains its boundary in the FIFO queue.
    pub fn apply_param_patch(
        &self,
        patch: IndexMap<String, ScalarValue>,
    ) -> Result<ParamSetResult, AvengerAppError> {
        if patch.is_empty() {
            let transaction_id = self.next_param_transaction_id();
            return Ok(ParamSetResult {
                transaction_id,
                changed: false,
                revision: self.param_revision(),
                change: None,
                changes: Vec::new(),
            });
        }
        let transaction_id = self.next_param_transaction_id();
        if let Ok(mut runtime) = self.runtime.try_lock() {
            return match self.apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    transaction_id,
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Exact,
                ),
                patch,
                Vec::new(),
                Vec::new(),
            ) {
                Ok(outcome) => {
                    if outcome.param_changed {
                        runtime.next_evaluation_mode = outcome.evaluation_mode;
                    }
                    let revision = self.param_revision();
                    Ok(ParamSetResult {
                        transaction_id: outcome.transaction_id,
                        changed: outcome.param_changed
                            || outcome.store_changed
                            || outcome.selection_changed,
                        revision,
                        change: None,
                        changes: outcome.changes,
                    })
                }
                Err(error) => Err(error),
            };
        }
        let changed = self
            .params
            .lock()
            .expect("chart param lock poisoned")
            .queue_param_patch_transaction(transaction_id, patch);
        Ok(ParamSetResult {
            transaction_id,
            changed,
            revision: self.param_revision(),
            change: None,
            changes: Vec::new(),
        })
    }

    /// Alias for [`ChartAppState::apply_param_patch`].
    pub fn set_params(
        &self,
        patch: IndexMap<String, ScalarValue>,
    ) -> Result<ParamSetResult, AvengerAppError> {
        self.apply_param_patch(patch)
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

    pub async fn last_resource_requests(&self) -> Vec<ResourceRequest> {
        self.runtime.lock().await.last_resource_requests.clone()
    }

    pub async fn runtime_resources(&self) -> Option<ChartRuntimeResources> {
        self.runtime.lock().await.runtime_resources.clone()
    }

    #[doc(hidden)]
    pub async fn has_evaluation_invalidation_subscription_for_testing(&self) -> bool {
        self.runtime
            .lock()
            .await
            ._evaluation_invalidation_subscription
            .is_some()
    }

    #[doc(hidden)]
    pub async fn missing_materialization_wakeup_warning_emitted_for_testing(&self) -> bool {
        self.runtime
            .lock()
            .await
            .warned_missing_materialization_wakeup
    }

    pub async fn has_pending_materializations(&self) -> bool {
        self.runtime
            .lock()
            .await
            .session
            .has_pending_materializations()
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
        let transactions = self
            .params
            .lock()
            .expect("chart param lock poisoned")
            .drain_pending_transactions();
        if transactions.is_empty() {
            return false;
        }
        let mut changed = false;
        for transaction in transactions {
            match self.apply_root_param_transaction_to_runtime(
                runtime,
                ParamTransactionContext::new(
                    transaction.id,
                    transaction.origin,
                    transaction.evaluation_mode,
                ),
                transaction.patch,
                Vec::new(),
                Vec::new(),
            ) {
                Ok(outcome) => {
                    changed |= outcome.param_changed;
                    if outcome.evaluation_mode == EvaluationMode::Exact {
                        runtime.next_evaluation_mode = EvaluationMode::Exact;
                    }
                    if outcome.settle_exact && outcome.evaluation_mode == EvaluationMode::Preview {
                        runtime.interaction_settle_exact_pending = true;
                    }
                }
                Err(error) => {
                    log::error!("queued parameter transaction failed: {error}");
                }
            }
        }
        changed
    }

    fn apply_root_param_transaction_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        context: ParamTransactionContext,
        initiating_patch: IndexMap<String, ScalarValue>,
        initiating_store_patch: Vec<avenger_chart::plot::ScopedStoreAssignment>,
        initiating_selection_patch: Vec<avenger_chart::plot::SelectionAssignment>,
    ) -> Result<ParamTransactionOutcome, AvengerAppError> {
        tracing::debug!(
            target: "avenger_chart_app::param_transaction",
            transaction_id = context.id,
            origin = ?context.origin,
            inputs = initiating_patch.len(),
            "processing parameter transaction"
        );
        let param_patch = initiating_patch
            .into_iter()
            .map(|(source_name, value)| {
                let runtime_id = runtime
                    .session
                    .resolve_param_name(&source_name)
                    .ok_or_else(|| {
                        AvengerAppError::InternalError(format!(
                            "Parameter transaction assigns unknown param '{source_name}'"
                        ))
                    })?;
                Ok(ResolvedScopedParamAssignment {
                    runtime_id,
                    source_name,
                    owner_path: Vec::new(),
                    value,
                    replace_scoped_values: false,
                })
            })
            .collect::<Result<Vec<_>, AvengerAppError>>()?;
        let store_patch = initiating_store_patch
            .into_iter()
            .map(|assignment| {
                let runtime_id = runtime
                    .session
                    .resolve_store_name(&assignment.store_name)
                    .ok_or_else(|| {
                        AvengerAppError::InternalError(format!(
                            "Parameter transaction updates unknown store '{}'",
                            assignment.store_name
                        ))
                    })?;
                Ok(ResolvedScopedStoreAssignment {
                    runtime_id,
                    source_name: assignment.store_name,
                    owner_path: assignment.owner_path,
                    replace_scoped_values: assignment.replace_scoped_values,
                    update: assignment.update,
                })
            })
            .collect::<Result<Vec<_>, AvengerAppError>>()?;
        let selection_patch = initiating_selection_patch
            .into_iter()
            .map(|assignment| {
                let runtime_id = runtime
                    .session
                    .resolve_selection_name(&assignment.selection_id)
                    .ok_or_else(|| {
                        AvengerAppError::InternalError(format!(
                            "Parameter transaction updates unknown selection '{}'",
                            assignment.selection_id
                        ))
                    })?;
                Ok(ResolvedSelectionAssignment {
                    runtime_id,
                    source_name: assignment.selection_id,
                    update: assignment.update,
                })
            })
            .collect::<Result<Vec<_>, AvengerAppError>>()?;
        self.apply_resolved_reactive_transaction_to_runtime(
            runtime,
            context,
            param_patch,
            store_patch,
            selection_patch,
        )
    }

    fn apply_resolved_reactive_transaction_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        context: ParamTransactionContext,
        initiating_param_patch: Vec<ResolvedScopedParamAssignment>,
        initiating_store_patch: Vec<ResolvedScopedStoreAssignment>,
        initiating_selection_patch: Vec<ResolvedSelectionAssignment>,
    ) -> Result<ParamTransactionOutcome, AvengerAppError> {
        if initiating_param_patch.is_empty()
            && initiating_store_patch.is_empty()
            && initiating_selection_patch.is_empty()
        {
            return Ok(ParamTransactionOutcome {
                transaction_id: context.id,
                param_changed: false,
                changes: Vec::new(),
                store_changed: false,
                selection_changed: false,
                evaluation_mode: context.evaluation_mode,
                settle_exact: false,
            });
        }

        let entry = runtime.session.params().clone();
        let mut working = runtime.session.begin_resolved_state_transaction();
        for assignment in &initiating_param_patch {
            working
                .apply_param(assignment.clone())
                .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
        }
        for assignment in &initiating_store_patch {
            working
                .apply_store(assignment.clone())
                .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
        }
        for assignment in &initiating_selection_patch {
            working
                .apply_selection(assignment.clone())
                .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
        }

        let graph = runtime.param_change_graph.clone();
        let working_root = working.effective_params_for_owner_paths(&HashMap::new());
        let mut wave = Vec::new();
        let mut wave_ids = HashSet::new();
        if let Some(graph) = graph.as_ref() {
            for assignment in &initiating_param_patch {
                if assignment.owner_path.is_empty()
                    && graph.has_source(&assignment.runtime_id)
                    && entry.get(&assignment.source_name)
                        != working_root.get(&assignment.source_name)
                    && wave_ids.insert(assignment.runtime_id.clone())
                {
                    let previous =
                        entry.get(&assignment.source_name).cloned().ok_or_else(|| {
                            AvengerAppError::InternalError(format!(
                                "Parameter-change source '{}' has no transaction-entry value",
                                assignment.source_name
                            ))
                        })?;
                    wave.push((assignment.runtime_id.clone(), previous));
                }
            }
        }

        let mut param_patch = initiating_param_patch;
        let mut store_patch = initiating_store_patch;
        let mut selection_patch = initiating_selection_patch;
        let mut store_sinks = store_patch
            .iter()
            .map(|assignment| (assignment.runtime_id.clone(), usize::MAX))
            .collect::<HashMap<_, _>>();
        let mut selection_sinks = selection_patch
            .iter()
            .map(|assignment| (assignment.runtime_id.clone(), usize::MAX))
            .collect::<HashMap<_, _>>();
        let mut fired_bindings = HashSet::new();
        let mut evaluation_mode = context.evaluation_mode;
        let mut settle_exact = false;

        while !wave.is_empty() {
            let Some(graph) = graph.as_ref() else {
                break;
            };
            let actions = graph.evaluate_sources(&wave, &mut working, &mut fired_bindings)?;
            let mut next_wave = Vec::new();
            let mut next_wave_ids = HashSet::new();
            for action in actions {
                for assignment in action.store_patch {
                    if let Some(first_binding) =
                        store_sinks.insert(assignment.runtime_id.clone(), action.binding_index)
                        && first_binding != action.binding_index
                    {
                        return Err(AvengerAppError::InternalError(format!(
                            "Parameter-change transaction has multiple active reactions targeting store '{}'",
                            assignment.source_name
                        )));
                    }
                    store_patch.push(assignment);
                }
                for assignment in action.selection_patch {
                    if let Some(first_binding) =
                        selection_sinks.insert(assignment.runtime_id.clone(), action.binding_index)
                        && first_binding != action.binding_index
                    {
                        return Err(AvengerAppError::InternalError(format!(
                            "Parameter-change transaction has multiple active reactions targeting selection '{}'",
                            assignment.source_name
                        )));
                    }
                    selection_patch.push(assignment);
                }
                for assignment in action.param_patch {
                    if graph.has_source(&assignment.runtime_id)
                        && next_wave_ids.insert(assignment.runtime_id.clone())
                    {
                        let previous =
                            entry.get(&assignment.source_name).cloned().ok_or_else(|| {
                                AvengerAppError::InternalError(format!(
                                    "Parameter-change source '{}' has no transaction-entry value",
                                    assignment.source_name
                                ))
                            })?;
                        next_wave.push((assignment.runtime_id.clone(), previous));
                    }
                    param_patch.push(assignment);
                }
                if action.evaluation_mode == ChartEventEvaluationMode::Exact {
                    evaluation_mode = EvaluationMode::Exact;
                }
                settle_exact |= action.settle_exact;
            }
            wave = next_wave;
        }

        let (param_changed, store_changed, selection_changed) = runtime
            .session
            .apply_resolved_state_transaction(param_patch, store_patch, selection_patch)
            .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
        let changes = self.sync_param_state_from_runtime(runtime, context.id);
        Ok(ParamTransactionOutcome {
            transaction_id: context.id,
            param_changed,
            changes,
            store_changed,
            selection_changed,
            evaluation_mode,
            settle_exact,
        })
    }

    pub(crate) fn apply_scoped_param_transaction_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        context: ParamTransactionContext,
        patch: Vec<ScopedParamAssignment>,
        store_patch: Vec<avenger_chart::plot::ScopedStoreAssignment>,
        selection_patch: Vec<avenger_chart::plot::SelectionAssignment>,
    ) -> Result<ParamTransactionOutcome, AvengerAppError> {
        let mut root_patch = IndexMap::new();
        let mut scoped_patch = Vec::new();
        for assignment in patch {
            if assignment.owner_path.is_empty() && !assignment.replace_scoped_values {
                root_patch.insert(assignment.name, assignment.value);
            } else {
                scoped_patch.push(assignment);
            }
        }
        let mut outcome = self.apply_root_param_transaction_to_runtime(
            runtime,
            context,
            root_patch,
            store_patch,
            selection_patch,
        )?;
        if !scoped_patch.is_empty() {
            runtime
                .session
                .apply_scoped_param_patch(scoped_patch)
                .map_err(|error| AvengerAppError::InternalError(error.to_string()))?;
            outcome
                .changes
                .extend(self.sync_param_state_from_runtime(runtime, context.id));
        }
        Ok(outcome)
    }

    pub(crate) fn apply_resolved_event_transaction_to_runtime(
        &self,
        runtime: &mut ChartAppRuntime,
        context: ParamTransactionContext,
        param_patch: Vec<ResolvedScopedParamAssignment>,
        store_patch: Vec<ResolvedScopedStoreAssignment>,
        selection_patch: Vec<ResolvedSelectionAssignment>,
    ) -> Result<ParamTransactionOutcome, AvengerAppError> {
        self.apply_resolved_reactive_transaction_to_runtime(
            runtime,
            context,
            param_patch,
            store_patch,
            selection_patch,
        )
    }

    fn sync_param_state_from_runtime(
        &self,
        runtime: &ChartAppRuntime,
        transaction_id: u64,
    ) -> Vec<ParamChange> {
        self.params
            .lock()
            .expect("chart param lock poisoned")
            .sync_from_params(transaction_id, runtime.session.params().clone())
    }
}

/// Scene graph builder that evaluates the chart session stored in state.
pub struct ChartSceneGraphBuilder;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl SceneGraphBuilder<ChartAppState> for ChartSceneGraphBuilder {
    async fn build(&self, state: &mut ChartAppState) -> Result<SceneGraph, AvengerAppError> {
        let mut runtime = state.runtime.lock().await;
        let pending_params = state.drain_pending_params_into_runtime(&mut runtime);
        let mode = if pending_params {
            runtime.next_evaluation_mode = EvaluationMode::Exact;
            EvaluationMode::Exact
        } else {
            runtime.next_evaluation_mode
        };
        if mode == EvaluationMode::Exact {
            runtime.interaction_settle_exact_pending = false;
        }
        let evaluation_param_revision = state.param_revision();

        let start = Instant::now();
        tracing::debug!(
            target: "avenger_chart_app::resize",
            mode = ?mode,
            resize_seq = runtime.accepted_resize_count,
            "chart_app.scene_build start"
        );
        let mut request = EvaluationRequest::new()
            .mode(mode)
            .options(EvaluationOptions {
                build_scene_rtree: false,
                ..EvaluationOptions::default()
            });
        if let Some(now) = runtime.evaluation_now.take() {
            request = request.at(now);
        }
        let (evaluated, metrics) = runtime
            .session
            .evaluate_with_metrics(request)
            .await
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let elapsed = start.elapsed();
        let scene_size = [evaluated.scene_graph.width, evaluated.scene_graph.height];

        if runtime.log_metrics {
            eprintln!(
                "chart eval mode={:?} elapsed={:?} scene={:.1}x{:.1} preview_reuse={} reflow_reuse={} facet_tree_builds={} facet_tree_reuse={} cell_reuse={} data_reuse={} data_miss={} chrome_refresh={} skipped_measures={} guide_measures={} materialize_emit={} materialize_ready={} materialize_fallback={} materialize_queued={} materialize_running={} materialize_errors={}",
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
                metrics.pipeline.materialization_requests_emitted,
                metrics.pipeline.materialization_ready_used,
                metrics.pipeline.materialization_stale_fallback_used,
                metrics.pipeline.materialization_queued,
                metrics.pipeline.materialization_running,
                metrics.pipeline.materialization_errors,
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

        if runtime._evaluation_invalidation_subscription.is_none()
            && !runtime.warned_missing_materialization_wakeup
            && (metrics.pipeline.materialization_requests_emitted > 0
                || runtime.session.has_pending_materializations())
        {
            log::warn!(
                "async view materializations were queued without ChartRuntimeResources; completed results may not trigger a rerender"
            );
            runtime.warned_missing_materialization_wakeup = true;
        }

        runtime.last_metrics = Some(metrics);
        runtime.last_evaluation_elapsed = Some(elapsed);
        runtime.last_scene_size = Some(scene_size);
        runtime.last_resource_requests = evaluated.resource_requests.clone();
        if let Some(resources) = &runtime.runtime_resources {
            request_image_resources(resources, &evaluated.resource_requests);
            // Refresh the hover-retarget planners: each evaluation's plan
            // is the new baseline for cursor-driven prefetch retargeting.
            resources
                .image_resource_resolver
                .install_retarget_planners(evaluated.prefetch_planners.clone());
        }
        runtime.last_evaluated_param_revision = evaluation_param_revision;
        if state.param_revision() != evaluation_param_revision {
            runtime.next_evaluation_mode = EvaluationMode::Exact;
        }
        runtime.last_interaction_state = evaluated.interaction;
        runtime.last_event_datum_state = evaluated.event_datums;
        runtime.last_widget_frame_state = evaluated.widget_frames;
        runtime.last_native_widget_state = evaluated.native_widgets;
        Ok(evaluated.scene_graph)
    }
}

/// Routes native-widget input ahead of authored streams. Pointer ownership is
/// resolved from the same final mark paths used by the scene R-tree; focused
/// keyboard/IME/clipboard input and wakes resolve through attachment epochs.
struct NativeWidgetEventHandler;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for NativeWidgetEventHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        UpdateStatus::default()
    }

    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut runtime = state.runtime.lock().await;
        state.drain_pending_params_into_runtime(&mut runtime);
        let now = context
            .current_event
            .as_ref()
            .map_or_else(Instant::now, |snapshot| snapshot.instant);
        runtime.evaluation_now = Some(now);

        let pointer_event = matches!(
            event,
            SceneGraphEvent::MouseDown(_)
                | SceneGraphEvent::MouseUp(_)
                | SceneGraphEvent::Click(_)
                | SceneGraphEvent::DoubleClick(_)
                | SceneGraphEvent::MouseWheel(_)
                | SceneGraphEvent::CursorMoved(_)
                | SceneGraphEvent::MouseEnter(_)
                | SceneGraphEvent::MouseLeave(_)
        );
        let current_mark = event.mark_instance().or(context.mark_instance.as_ref());

        let captured = pointer_event
            .then(|| runtime.active_native_widget_gesture.clone())
            .flatten();
        let direct = pointer_event.then(|| {
            let frame = runtime
                .last_widget_frame_state
                .frame_for_mark_instance(current_mark)?
                .clone();
            let attachment = runtime
                .last_native_widget_state
                .by_widget_id
                .get(&frame.widget_id)?;
            Some((
                NativeWidgetEventRoute {
                    key: attachment.key.clone(),
                    epoch: attachment.epoch,
                },
                frame,
            ))
        });
        let direct = direct.flatten();

        // A focused native editor must see pointer focus leave even when the
        // new target is an ordinary mark or another native widget. Dispatch
        // that loss first; its non-consuming outcome is then merged with the
        // new target's outcome below.
        let mut prior_status = UpdateStatus::default();
        if matches!(event, SceneGraphEvent::MouseDown(_))
            && let Some(focused_route) = runtime.session.focused_native_widget_event_route()
            && direct
                .as_ref()
                .is_none_or(|(direct_route, _)| direct_route != &focused_route)
            && let Some(focused_frame) = runtime
                .last_widget_frame_state
                .by_widget_id
                .get(focused_route.key.widget_id())
                .cloned()
        {
            let current = event
                .position()
                .map(|point| focused_frame.local_point(point));
            let blur_event = NativeWidgetEvent {
                event: event.clone(),
                hit_part: None,
                current,
                start: current,
                previous: None,
                wheel_delta: None,
                frame_size: [focused_frame.bounds.width, focused_frame.bounds.height],
            };
            let transform = NativeWidgetHostTransform::from_offsets([[
                focused_frame.bounds.x,
                focused_frame.bounds.y,
            ]])
            .expect("evaluated widget frame has finite origin");
            let base_font_size = runtime.session.base_font_size();
            match runtime.session.dispatch_native_widget_event(
                &focused_route,
                &blur_event,
                transform,
                now,
                base_font_size,
            ) {
                Ok(outcome) => {
                    prior_status = native_widget_outcome_status(state, &mut runtime, outcome);
                }
                Err(error) => {
                    log::error!("focused native widget blur dispatch failed: {error}");
                }
            }
        }

        let routed = if let Some(capture) = captured.as_ref() {
            Some((capture.route.clone(), capture.frame.clone()))
        } else if let Some(direct) = direct.clone() {
            Some(direct)
        } else if !pointer_event {
            runtime
                .session
                .route_native_widget_event(event)
                .and_then(|route| {
                    runtime
                        .last_widget_frame_state
                        .by_widget_id
                        .get(route.key.widget_id())
                        .cloned()
                        .map(|frame| (route, frame))
                })
        } else {
            None
        };
        let Some((route, frame)) = routed else {
            return prior_status;
        };

        let current = event.position().map(|point| frame.local_point(point));
        let start = captured
            .as_ref()
            .map(|capture| capture.start)
            .or_else(|| {
                context
                    .start_event
                    .as_ref()
                    .and_then(|snapshot| snapshot.event.position())
                    .map(|point| frame.local_point(point))
            })
            .or(current);
        let previous = captured
            .as_ref()
            .map(|capture| capture.previous)
            .or_else(|| {
                context
                    .previous_event
                    .as_ref()
                    .and_then(|snapshot| snapshot.event.position())
                    .map(|point| frame.local_point(point))
            });
        let wheel_delta = match event {
            SceneGraphEvent::MouseWheel(wheel) => Some(match wheel.delta {
                MouseScrollDelta::LineDelta(x, y) => [x, y],
                MouseScrollDelta::PixelDelta(x, y) => [x as f32, y as f32],
            }),
            _ => None,
        };
        let hit_part = pointer_event
            .then(|| {
                direct
                    .as_ref()
                    .filter(|(direct_route, _)| direct_route == &route)?;
                current_mark
                    .map(|mark| mark.name.as_str())
                    .filter(|name| !name.is_empty() && *name != frame.widget_id)
                    .map(str::to_string)
            })
            .flatten();
        let native_event = NativeWidgetEvent {
            event: event.clone(),
            hit_part,
            current,
            start,
            previous,
            wheel_delta,
            frame_size: [frame.bounds.width, frame.bounds.height],
        };
        let transform = NativeWidgetHostTransform::from_offsets([[frame.bounds.x, frame.bounds.y]])
            .expect("evaluated widget frame has finite origin");
        let base_font_size = runtime.session.base_font_size();
        let outcome = match runtime.session.dispatch_native_widget_event(
            &route,
            &native_event,
            transform,
            now,
            base_font_size,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                log::error!("native widget event dispatch failed: {error}");
                if matches!(event, SceneGraphEvent::MouseUp(_)) {
                    runtime.active_native_widget_gesture = None;
                }
                return UpdateStatus::default();
            }
        };

        if matches!(event, SceneGraphEvent::MouseDown(_)) && outcome.consume {
            if let Some(current) = current {
                runtime.active_native_widget_gesture = Some(NativeWidgetGestureCapture {
                    route: route.clone(),
                    frame: frame.clone(),
                    start: current,
                    previous: current,
                });
            }
        } else if let Some(current) = current
            && let Some(capture) = runtime.active_native_widget_gesture.as_mut()
            && capture.route == route
        {
            capture.previous = current;
        }
        if matches!(event, SceneGraphEvent::MouseUp(_)) {
            runtime.active_native_widget_gesture = None;
        }

        prior_status.merge(&native_widget_outcome_status(state, &mut runtime, outcome))
    }
}

fn native_widget_outcome_status(
    state: &ChartAppState,
    runtime: &mut ChartAppRuntime,
    outcome: NativeWidgetDispatchOutcome,
) -> UpdateStatus {
    let initiating_mode = match outcome.evaluation_intent {
        NativeWidgetEvaluationIntent::Preview => EvaluationMode::Preview,
        NativeWidgetEvaluationIntent::None | NativeWidgetEvaluationIntent::Exact => {
            EvaluationMode::Exact
        }
    };
    let transaction_id = state.next_param_transaction_id();
    let transaction = match state.apply_scoped_param_transaction_to_runtime(
        runtime,
        ParamTransactionContext::new(
            transaction_id,
            ParamTransactionOrigin::NativeWidget,
            initiating_mode,
        ),
        outcome.param_assignments,
        Vec::new(),
        Vec::new(),
    ) {
        Ok(transaction) => transaction,
        Err(error) => {
            log::error!("native widget parameter transaction failed: {error}");
            return UpdateStatus {
                cursor: outcome.cursor,
                commands: outcome.commands,
                consume: outcome.consume,
                ..Default::default()
            };
        }
    };
    let param_changed = transaction.param_changed;
    match outcome.evaluation_intent {
        NativeWidgetEvaluationIntent::None => {
            if param_changed {
                runtime.next_evaluation_mode = EvaluationMode::Exact;
            }
        }
        NativeWidgetEvaluationIntent::Preview => {
            runtime.next_evaluation_mode = transaction.evaluation_mode;
        }
        NativeWidgetEvaluationIntent::Exact => {
            runtime.next_evaluation_mode = EvaluationMode::Exact;
        }
    }
    if transaction.settle_exact && transaction.evaluation_mode == EvaluationMode::Preview {
        runtime.interaction_settle_exact_pending = true;
    }
    UpdateStatus {
        rerender: param_changed
            || transaction.store_changed
            || transaction.selection_changed
            || outcome.scene_dirty
            || outcome.evaluation_intent != NativeWidgetEvaluationIntent::None,
        rebuild_geometry: outcome.index_dirty,
        cursor: outcome.cursor,
        commands: outcome.commands,
        consume: outcome.consume,
        admission: None,
    }
}

/// Captures widget ownership before authored streams can consume a mouse-down.
struct WidgetGestureFrameCaptureHandler;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for WidgetGestureFrameCaptureHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        UpdateStatus::default()
    }

    async fn handle_with_context(
        &self,
        event: &SceneGraphEvent,
        context: &EventStreamContext,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut runtime = state.runtime.lock().await;
        let mark_instance = context
            .mark_instance
            .as_ref()
            .or_else(|| event.mark_instance());
        runtime.active_widget_gesture_frame = context.current_event.as_ref().and_then(|current| {
            runtime
                .last_widget_frame_state
                .frame_for_mark_instance(mark_instance)
                .cloned()
                .map(|frame| WidgetGestureFrameCapture {
                    start_instant: current.instant,
                    frame,
                })
        });
        UpdateStatus::default()
    }
}

/// Resize handler that patches only canvas-constrained, bound chart dimensions.
pub struct ChartResizeHandler;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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

        let transaction_id = state.next_param_transaction_id();
        let transaction = match state.apply_root_param_transaction_to_runtime(
            &mut runtime,
            ParamTransactionContext::new(
                transaction_id,
                ParamTransactionOrigin::Resize,
                EvaluationMode::Preview,
            ),
            patch,
            Vec::new(),
            Vec::new(),
        ) {
            Ok(transaction) => transaction,
            Err(error) => {
                log::error!("resize parameter transaction failed: {error}");
                return UpdateStatus::default();
            }
        };
        runtime.next_evaluation_mode = transaction.evaluation_mode;
        if transaction.settle_exact && transaction.evaluation_mode == EvaluationMode::Preview {
            runtime.interaction_settle_exact_pending = true;
        }
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

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
    chart_avenger_app_inner(compiled_plot, ctx, options, None, None)
        .await
        .map(|(app, _)| app)
}

pub async fn chart_avenger_app_with_runtime_resources(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
    runtime_resources: ChartRuntimeResources,
) -> Result<AvengerApp<ChartAppState>, AvengerAppError> {
    chart_avenger_app_inner(compiled_plot, ctx, options, Some(runtime_resources), None)
        .await
        .map(|(app, _)| app)
}

pub async fn chart_avenger_app_with_default_runtime_resources(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
) -> Result<ChartAppBundle, AvengerAppError> {
    let render_invalidation_hub = RenderInvalidationHub::default();
    let image_resource_resolver = Arc::new(
        ImageResourceCache::new()
            .with_render_invalidation_sink(Arc::new(render_invalidation_hub.clone())),
    );
    let runtime_resources =
        ChartRuntimeResources::new(image_resource_resolver, render_invalidation_hub);
    let (app, _) = chart_avenger_app_inner(
        compiled_plot,
        ctx,
        options,
        Some(runtime_resources.clone()),
        None,
    )
    .await?;
    Ok(ChartAppBundle {
        app,
        runtime_resources,
    })
}

/// Build a replacement app with compatible state restored before its first
/// evaluation. Generation-local runtime resources remain isolated from the
/// displayed app.
pub async fn chart_avenger_app_with_default_runtime_resources_and_snapshot(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
    snapshot: &ChartSessionSnapshot,
) -> Result<(ChartAppBundle, StateMigrationReport), AvengerAppError> {
    let render_invalidation_hub = RenderInvalidationHub::default();
    let image_resource_resolver = Arc::new(
        ImageResourceCache::new()
            .with_render_invalidation_sink(Arc::new(render_invalidation_hub.clone())),
    );
    let runtime_resources =
        ChartRuntimeResources::new(image_resource_resolver, render_invalidation_hub);
    let (app, report) = chart_avenger_app_inner(
        compiled_plot,
        ctx,
        options,
        Some(runtime_resources.clone()),
        Some(snapshot),
    )
    .await?;
    Ok((
        ChartAppBundle {
            app,
            runtime_resources,
        },
        report,
    ))
}

async fn chart_avenger_app_inner(
    compiled_plot: CompiledPlot,
    ctx: Arc<SessionContext>,
    options: ChartAppOptions,
    runtime_resources: Option<ChartRuntimeResources>,
    snapshot: Option<&ChartSessionSnapshot>,
) -> Result<(AvengerApp<ChartAppState>, StateMigrationReport), AvengerAppError> {
    let compiled_plot = Arc::new(compiled_plot);
    let resize_policy = compiled_plot.resize_policy();
    let param_change_graph = Arc::new(param_change_graph_for_plot(&compiled_plot, ctx.as_ref())?);
    tracing::debug!(
        target: "avenger_chart_app::param_change_binding",
        bindings = param_change_graph.binding_count(),
        sources = param_change_graph.source_count(),
        edges = param_change_graph.edge_count(),
        "compiled parameter-change reaction graph"
    );
    let resize_bindings = resize_event_bindings(
        resize_policy,
        &options.resize_binding,
        options.resize_throttle_ms,
    );
    let mut session = compiled_plot.clone().instantiate(ctx.clone());
    if let Some(resources) = runtime_resources.as_ref() {
        session.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources.native_widget_runtime,
            NativeWidgetPlotId::chart_root(),
        ));
    }
    let migration_report = snapshot
        .map(|snapshot| session.restore_state(snapshot))
        .unwrap_or_default();
    let exact_on_resize_settle = options.exact_on_resize_settle;
    let hover_resolver = runtime_resources
        .as_ref()
        .map(|resources| resources.image_resource_resolver.clone());
    let mut state = ChartAppState::new_with_runtime_resources(
        session,
        resize_policy,
        options,
        runtime_resources,
    );
    if !param_change_graph.is_empty() {
        state.has_param_reactions = true;
        state.runtime.lock().await.param_change_graph = Some(param_change_graph);
    }
    let stream_param_provider = committed_stream_param_provider(&state);
    let mut event_streams = event_streams_for_plot_bindings_with_param_provider(
        compiled_plot.as_ref(),
        ctx.as_ref(),
        Some(stream_param_provider),
    )?;
    event_streams.extend(event_streams_for_bindings(
        &resize_bindings,
        ctx.as_ref(),
        compiled_plot.param_specs(),
        compiled_plot.selection_specs(),
        compiled_plot.store_specs(),
        &IndexMap::new(),
    )?);
    // Native ownership and gesture capture must precede authored streams.
    // Native consumption is dynamic and can stop propagation after the live
    // instance accepts a particular event.
    let mut streams = vec![
        (
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::MouseDown,
                    SceneGraphEventType::MouseUp,
                    SceneGraphEventType::Click,
                    SceneGraphEventType::DoubleClick,
                    SceneGraphEventType::MouseWheel,
                    SceneGraphEventType::CursorMoved,
                    SceneGraphEventType::MarkMouseEnter,
                    SceneGraphEventType::MarkMouseLeave,
                    SceneGraphEventType::KeyPress,
                    SceneGraphEventType::KeyRelease,
                    SceneGraphEventType::Ime,
                    SceneGraphEventType::Clipboard,
                    SceneGraphEventType::RuntimeWake,
                    SceneGraphEventType::WindowFocused,
                ],
                ..Default::default()
            },
            Arc::new(NativeWidgetEventHandler) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ),
        (
            EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                ..Default::default()
            },
            Arc::new(WidgetGestureFrameCaptureHandler)
                as Arc<dyn EventStreamHandler<ChartAppState>>,
        ),
    ];
    streams.extend(event_streams);
    if exact_on_resize_settle {
        streams.push((
            EventStreamConfig {
                types: vec![SceneGraphEventType::CanvasResizeSettled],
                ..Default::default()
            },
            Arc::new(ChartResizeSettleHandler) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));
    }
    if let Some(resolver) = hover_resolver {
        // Cursor-prefetch wiring: hover positions feed the fetch
        // scheduler's focus hint + debounced
        // retargeting; gestures suppress retargeting until settle. All
        // three handlers are param-free no-work UpdateStatus::default()
        // paths — free at mousemove rate.
        streams.push((
            EventStreamConfig {
                types: vec![SceneGraphEventType::CursorMoved],
                ..Default::default()
            },
            Arc::new(HoverFocusHandler {
                resolver: resolver.clone(),
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));
        streams.push((
            EventStreamConfig {
                types: vec![
                    SceneGraphEventType::MouseDown,
                    SceneGraphEventType::MouseWheel,
                ],
                ..Default::default()
            },
            Arc::new(GestureActiveHandler {
                resolver: resolver.clone(),
                active: true,
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));
        streams.push((
            EventStreamConfig {
                types: vec![SceneGraphEventType::InteractionSettled],
                ..Default::default()
            },
            Arc::new(GestureActiveHandler {
                resolver,
                active: false,
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));
    }

    let app = AvengerApp::try_new(state, Arc::new(ChartSceneGraphBuilder), streams).await?;
    Ok((app, migration_report))
}

/// Streams hover cursor positions (canvas px) to the image fetch
/// scheduler as its focus hint. No params, no rerender.
struct HoverFocusHandler {
    resolver: Arc<dyn ImageResourceResolver>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for HoverFocusHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        if let SceneGraphEvent::CursorMoved(event) = event {
            self.resolver.update_focus(event.position);
        }
        UpdateStatus::default()
    }
}

/// Marks a pan/zoom gesture as active on its first event (MouseDown /
/// MouseWheel) and inactive on `InteractionSettled`, suppressing hover
/// retargeting while evaluations own the prefetch set.
struct GestureActiveHandler {
    resolver: Arc<dyn ImageResourceResolver>,
    active: bool,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl EventStreamHandler<ChartAppState> for GestureActiveHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        _state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        self.resolver.set_gesture_active(self.active);
        UpdateStatus::default()
    }
}

fn request_image_resources(resources: &ChartRuntimeResources, requests: &[ResourceRequest]) {
    for purpose in [
        ResourceRequestPurpose::Required,
        ResourceRequestPurpose::Prefetch,
    ] {
        for request in requests
            .iter()
            .filter(|request| request.kind.0 == IMAGE_RESOURCE_KIND && request.purpose == purpose)
        {
            resources.image_resource_resolver.request_image(request);
        }
    }
}

fn subscribe_to_session_evaluation_invalidations(
    session: &PlotSession,
    resources: Option<&ChartRuntimeResources>,
) -> Option<EvaluationInvalidationSubscription> {
    let render_invalidation_hub = resources?.render_invalidation_hub.clone();
    Some(
        session.subscribe_to_evaluation_invalidations(Arc::new(move |invalidation| {
            render_invalidation_hub
                .request_render(render_invalidation_request_for_evaluation(invalidation));
        })),
    )
}

fn render_invalidation_request_for_evaluation(
    invalidation: EvaluationInvalidation,
) -> RenderInvalidationRequest {
    RenderInvalidationRequest {
        reason: render_invalidation_reason_for_evaluation(invalidation.reason),
        schedule: render_invalidation_schedule_for_evaluation(invalidation.schedule),
    }
}

fn render_invalidation_reason_for_evaluation(
    reason: EvaluationInvalidationReason,
) -> RenderInvalidationReason {
    match reason {
        EvaluationInvalidationReason::MaterializationCompleted { kind } => {
            RenderInvalidationReason::EvaluationChanged {
                kind: format!("materialization:{kind}"),
            }
        }
        EvaluationInvalidationReason::MaterializationDeferred { kind } => {
            RenderInvalidationReason::EvaluationChanged {
                kind: format!("materialization-deferred:{kind}"),
            }
        }
        _ => RenderInvalidationReason::EvaluationChanged {
            kind: "evaluation".to_string(),
        },
    }
}

fn render_invalidation_schedule_for_evaluation(
    schedule: EvaluationInvalidationSchedule,
) -> RenderInvalidationSchedule {
    match schedule {
        EvaluationInvalidationSchedule::Now => RenderInvalidationSchedule::Now,
        EvaluationInvalidationSchedule::After(duration) => {
            RenderInvalidationSchedule::After(duration)
        }
    }
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
    if let Some(param) = param
        && !policy.is_canvas_constrained()
    {
        log::warn!(
            "chart resize binding for {axis} param '{param}' will be ignored because the {axis} axis policy is {policy:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use avenger_chart::plot::{SelectionAssignment, SelectionStateUpdate};
    use avenger_chart::prelude::*;
    use avenger_chart_core::{
        DefaultLogicalExprNodeExt, ResolvedSelectionClauseScope, SelectionClause,
        SelectionEqualityDimensionValue, SelectionPredicateSpec,
    };
    #[cfg(feature = "winit-wgpu")]
    use avenger_eventstream::runtime::RuntimeHostCommand;
    use avenger_eventstream::{
        scene::{ModifiersState, SceneGraphEvent, SceneMouseDownEvent, SceneMouseUpEvent},
        stream::EventStreamEventSnapshot,
        window::{CanvasResizeEvent, MouseButton, WindowResizeEvent},
    };
    use avenger_image::{ImageResourceResolver, ImageResourceState};
    use avenger_resource::{
        RenderInvalidationHub, RenderInvalidationReason, RenderInvalidationSchedule,
        ResourceCachePolicy, ResourceKey, ResourceKind, ResourceRequestPurpose, ResourceSource,
    };
    use avenger_scenegraph::marks::mark::SceneMark;
    use avenger_scenegraph::scene_graph::SceneGraph;
    use datafusion::arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use datafusion_proto::protobuf::LogicalExprNode;

    use super::*;

    const TEST_IMAGE_URL: &str = "https://example.com/test-image.png";

    #[derive(Default)]
    struct RecordingImageResolver {
        requests: StdMutex<Vec<ResourceRequest>>,
    }

    struct RecordingNativeWidget;

    struct RecordingNativeWidgetFactory {
        events: Arc<StdMutex<Vec<NativeWidgetEvent>>>,
    }

    struct RecordingNativeWidgetInstance {
        events: Arc<StdMutex<Vec<NativeWidgetEvent>>>,
    }

    impl NativeWidget for RecordingNativeWidget {
        fn id(&self) -> &str {
            "recording-native"
        }

        fn kind(&self) -> &'static str {
            "recording-native"
        }

        fn schema_version(&self) -> u32 {
            1
        }

        fn payload(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        fn measure(&self) -> NativeWidgetMeasureSpec {
            NativeWidgetMeasureSpec::Declarative(WidgetMeasureSpec::fixed(40.0, 24.0))
        }

        fn state(&self) -> NativeWidgetStateSpec {
            NativeWidgetStateSpec::try_new(Vec::new()).unwrap()
        }
    }

    impl NativeWidgetFactory for RecordingNativeWidgetFactory {
        fn kind(&self) -> &'static str {
            "recording-native"
        }

        fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
            1..=1
        }

        fn create(
            &self,
            _spec: &CompiledNativeWidgetSpec,
            _payload: &serde_json::Value,
        ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
            Ok(Box::new(RecordingNativeWidgetInstance {
                events: self.events.clone(),
            }))
        }
    }

    impl NativeWidgetInstance for RecordingNativeWidgetInstance {
        fn on_event(
            &mut self,
            event: &NativeWidgetEvent,
            ctx: &mut NativeWidgetCtx,
        ) -> Result<(), AvengerChartError> {
            self.events
                .lock()
                .expect("recorded native events lock poisoned")
                .push(event.clone());
            match &event.event {
                SceneGraphEvent::MouseDown(_) if event.hit_part.is_some() => {
                    ctx.focus(None, "recording selection");
                    ctx.consume();
                }
                SceneGraphEvent::MouseDown(_) => ctx.blur(),
                _ => ctx.consume(),
            }
            Ok(())
        }

        fn scene(
            &mut self,
            environment: &NativeWidgetEnvironment,
            _ctx: &mut NativeWidgetCtx,
        ) -> Result<NativeWidgetScene, AvengerChartError> {
            NativeWidgetScene::try_from_iter([(
                "body".to_string(),
                SceneMark::Rect(avenger_scenegraph::marks::rect::SceneRectMark {
                    x: 0.0.into(),
                    y: 0.0.into(),
                    width: Some(environment.frame_size[0].into()),
                    height: Some(environment.frame_size[1].into()),
                    ..Default::default()
                }),
            )])
        }
    }

    impl RecordingImageResolver {
        fn requests(&self) -> Vec<ResourceRequest> {
            self.requests
                .lock()
                .expect("recording resolver lock poisoned")
                .clone()
        }
    }

    impl ImageResourceResolver for RecordingImageResolver {
        fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
            ImageResourceState::Missing
        }

        fn request_image(&self, request: &ResourceRequest) {
            self.requests
                .lock()
                .expect("recording resolver lock poisoned")
                .push(request.clone());
        }
    }

    async fn resize_test_state() -> ChartAppState {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let height = Param::new("height", ScalarValue::Float64(Some(480.0)));
        let compiled = Chart::<Cartesian>::new()
            .params([width.clone(), height.clone()])
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

    #[test]
    fn request_image_resources_submits_only_image_requests() {
        let resolver = Arc::new(RecordingImageResolver::default());
        let resources =
            ChartRuntimeResources::new(resolver.clone(), RenderInvalidationHub::default());
        let image_request = ResourceRequest {
            key: ResourceKey::new("image/test"),
            kind: ResourceKind::new(IMAGE_RESOURCE_KIND),
            source: ResourceSource::Url {
                url: TEST_IMAGE_URL.to_string(),
            },
            priority: 1.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        };
        let non_image_request = ResourceRequest {
            key: ResourceKey::new("metadata/test"),
            kind: ResourceKind::new("metadata"),
            source: ResourceSource::Opaque {
                provider: "test".to_string(),
                id: "metadata".to_string(),
            },
            priority: 0.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        };
        let prefetch_image_request = ResourceRequest {
            key: ResourceKey::new("image/prefetch"),
            kind: ResourceKind::new(IMAGE_RESOURCE_KIND),
            source: ResourceSource::Url {
                url: TEST_IMAGE_URL.to_string(),
            },
            priority: -1.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Prefetch,
            screen_center: None,
            prefetch_scope: None,
        };

        request_image_resources(
            &resources,
            &[
                prefetch_image_request.clone(),
                non_image_request.clone(),
                image_request.clone(),
            ],
        );

        let recorded_requests = resolver.requests();
        assert_eq!(recorded_requests.len(), 2);
        assert_eq!(recorded_requests[0], image_request);
        assert_eq!(recorded_requests[1], prefetch_image_request);
    }

    #[tokio::test]
    async fn native_handler_localizes_part_hits_and_keeps_outside_drag_capture() {
        use avenger_app::app::SceneGraphBuilder;

        let ctx = Arc::new(SessionContext::new());
        let compiled = Chart::<ZeroDCoord>::new()
            .native_widget(RecordingNativeWidget.position(LegendPosition::Bottom))
            .compile(ctx.as_ref())
            .await
            .expect("compile native routing plot");
        let resize_policy = compiled.resize_policy();
        let events = Arc::new(StdMutex::new(Vec::new()));
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(RecordingNativeWidgetFactory {
                    events: events.clone(),
                })
                .expect("register recording native widget"),
        );
        let native_runtime = NativeWidgetRuntimeResources::new(
            registry,
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        );
        let resources = ChartRuntimeResources::new(
            Arc::new(RecordingImageResolver::default()),
            RenderInvalidationHub::default(),
        )
        .with_native_widget_runtime(native_runtime);
        let mut session = Arc::new(compiled).instantiate(ctx);
        session.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources.native_widget_runtime,
            NativeWidgetPlotId::chart_root(),
        ));
        let mut state = ChartAppState::new_with_runtime_resources(
            session,
            resize_policy,
            ChartAppOptions::default(),
            Some(resources),
        );
        let scene = ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build native routing scene");
        let frame = state
            .runtime
            .lock()
            .await
            .last_widget_frame_state
            .by_widget_id["recording-native"]
            .clone();
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        let down_point = [frame.bounds.x + 5.0, frame.bounds.y + 6.0];
        let hit = rtree
            .pick_top_mark_at_point(&down_point)
            .expect("native body hit")
            .clone();
        assert_eq!(hit.name, "body");
        let now = Instant::now();
        let down = SceneGraphEvent::MouseDown(SceneMouseDownEvent {
            position: down_point,
            button: MouseButton::Left,
            mark_instance: Some(hit.clone()),
            modifiers: ModifiersState::default(),
        });
        let down_context = EventStreamContext {
            mark_instance: Some(hit.clone()),
            current_event: Some(EventStreamEventSnapshot {
                event: down.clone(),
                mark_instance: Some(hit),
                instant: now,
            }),
            ..Default::default()
        };
        let status = NativeWidgetEventHandler
            .handle_with_context(&down, &down_context, &mut state, &rtree)
            .await;
        assert!(status.consume);

        let up_point = [
            frame.bounds.x + frame.bounds.width + 30.0,
            frame.bounds.y - 20.0,
        ];
        let up = SceneGraphEvent::MouseUp(SceneMouseUpEvent {
            position: up_point,
            button: MouseButton::Left,
            mark_instance: None,
            modifiers: ModifiersState::default(),
        });
        let up_context = EventStreamContext {
            current_event: Some(EventStreamEventSnapshot {
                event: up.clone(),
                mark_instance: None,
                instant: now + avenger_common::time::Duration::from_millis(10),
            }),
            ..Default::default()
        };
        let status = NativeWidgetEventHandler
            .handle_with_context(&up, &up_context, &mut state, &rtree)
            .await;
        assert!(status.consume);

        let outside_down = SceneGraphEvent::MouseDown(SceneMouseDownEvent {
            position: up_point,
            button: MouseButton::Left,
            mark_instance: None,
            modifiers: ModifiersState::default(),
        });
        let outside_context = EventStreamContext {
            current_event: Some(EventStreamEventSnapshot {
                event: outside_down.clone(),
                mark_instance: None,
                instant: now + avenger_common::time::Duration::from_millis(20),
            }),
            ..Default::default()
        };
        let status = NativeWidgetEventHandler
            .handle_with_context(&outside_down, &outside_context, &mut state, &rtree)
            .await;
        assert!(!status.consume);

        let events = events.lock().expect("recorded native events lock poisoned");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].hit_part.as_deref(), Some("body"));
        assert_eq!(events[0].current, Some([5.0, 6.0]));
        assert_eq!(events[0].start, Some([5.0, 6.0]));
        assert_eq!(events[1].hit_part, None);
        assert_eq!(events[1].start, Some([5.0, 6.0]));
        assert_eq!(events[1].current, Some([frame.bounds.width + 30.0, -20.0]));
        assert_eq!(events[1].previous, Some([5.0, 6.0]));
        assert_eq!(events[2].hit_part, None);
        assert_eq!(events[2].current, Some([frame.bounds.width + 30.0, -20.0]));
    }

    #[cfg(feature = "winit-wgpu")]
    #[test]
    fn runtime_resources_configure_winit_with_synchronous_widget_clipboard_provider() {
        let resources = ChartRuntimeResources::new(
            Arc::new(RecordingImageResolver::default()),
            RenderInvalidationHub::default(),
        );
        let namespace = resources
            .native_widget_runtime
            .namespace(NativeWidgetPlotId::chart_root());
        let slot = resources
            .native_widget_runtime
            .instance_store
            .slot(NativeWidgetInstanceKey::new(namespace, "editor"));
        let sink = Arc::new(StdMutex::new(Vec::<RuntimeHostCommand>::new()));
        let ctx = NativeWidgetCtx::attach(
            slot,
            resources.native_widget_host_services.clone(),
            sink,
            NativeWidgetHostTransform::default(),
        );
        ctx.focus(None, "focused selection");
        let options = resources.configure_winit_options(WinitWgpuAvengerAppOptions::new(2.0));
        assert!(options.render_invalidation_hub.is_some());
        let provider = options
            .clipboard_payload_provider
            .expect("widget clipboard provider");
        assert_eq!(provider().as_deref(), Some("focused selection"));
    }

    async fn compile_tiny_async_raster_plot(ctx: &SessionContext) -> CompiledPlot {
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (0.25, 0.25), (0.75, 0.75), (1.0, 1.0)) AS t(x, y)")
            .await
            .expect("build tiny raster data");

        Chart::<Cartesian>::new()
            .canvas_size(240.0, 180.0)
            .data(df)
            .mark(
                UniformRaster2D::new()
                    .view(
                        View::cartesian()
                            .id("density")
                            .x_domain(col("x"))
                            .y_domain(col("y"))
                            .preview_cached(true),
                        |mark, view| {
                            mark.transform(
                                Rasterize2D::new(col("x"), col("y"))
                                    .x(|x| {
                                        x.extent(view.x().domain_start(), view.x().domain_end())
                                            .bins(4_usize)
                                    })
                                    .y(|y| {
                                        y.extent(view.y().domain_start(), view.y().domain_end())
                                            .bins(4_usize)
                                    })
                                    .agg("count"),
                                |mark, hist| {
                                    mark.raster_with(hist.raster(), |r| {
                                        r.x_with(hist.x_dim(), |x| {
                                            x.scale_with::<Linear>(|scale| {
                                                scale.nice(false).zero(false)
                                            })
                                            .axis(|axis| axis.visible(false))
                                        })
                                        .y_with(hist.y_dim(), |y| {
                                            y.scale_with::<Linear>(|scale| {
                                                scale.nice(false).zero(false)
                                            })
                                            .axis(|axis| axis.visible(false))
                                        })
                                        .fill(|fill| {
                                            fill.scale_with::<Sqrt>(|scale| {
                                                scale.domain((0.0, 4.0)).nice(false).zero(false)
                                            })
                                            .no_legend()
                                        })
                                    })
                                },
                            )
                        },
                    )
                    .smooth(false),
            )
            .compile(ctx)
            .await
            .expect("compile tiny async raster plot")
    }

    #[tokio::test]
    async fn materialization_completion_requests_render_invalidation() {
        use avenger_app::app::SceneGraphBuilder;

        let ctx = Arc::new(SessionContext::new());
        let compiled = compile_tiny_async_raster_plot(&ctx).await;
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(ctx);
        let render_invalidation_hub = RenderInvalidationHub::default();
        let resources = ChartRuntimeResources::new(
            Arc::new(RecordingImageResolver::default()),
            render_invalidation_hub.clone(),
        );
        let mut state = ChartAppState::new_with_runtime_resources(
            session,
            policy,
            ChartAppOptions::default(),
            Some(resources),
        );
        assert!(
            state
                .has_evaluation_invalidation_subscription_for_testing()
                .await
        );

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build initial async raster scene");
        let metrics = state.last_metrics().await.expect("initial metrics");
        assert!(
            metrics.pipeline.materialization_queued > 0,
            "initial evaluation should queue view-local raster materialization"
        );
        assert_eq!(render_invalidation_hub.epoch(), 0);

        for _ in 0..200 {
            if render_invalidation_hub.epoch() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(
            render_invalidation_hub.epoch(),
            1,
            "completed raster materialization should wake the render host once"
        );

        let ready_scene = ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build ready async raster scene");
        assert!(
            count_image_marks(&ready_scene) > 0,
            "scene rebuilt after materialization should contain a raster image mark"
        );
    }

    #[tokio::test]
    async fn default_runtime_resources_constructor_installs_subscription() {
        let ctx = Arc::new(SessionContext::new());
        let compiled = compile_tiny_async_raster_plot(&ctx).await;

        let mut bundle = chart_avenger_app_with_default_runtime_resources(
            compiled,
            ctx,
            ChartAppOptions::default(),
        )
        .await
        .expect("build bundled chart app");

        assert!(
            bundle
                .app
                .app_state_mut()
                .has_evaluation_invalidation_subscription_for_testing()
                .await
        );
        assert_eq!(bundle.runtime_resources.render_invalidation_hub.epoch(), 0);
        assert!(
            bundle
                .runtime_resources
                .native_widget_runtime
                .registry
                .factory("text-input")
                .is_some(),
            "default chart runtime resources must install built-in native widgets"
        );
    }

    #[tokio::test]
    async fn missing_runtime_resources_warns_for_async_materialization() {
        use avenger_app::app::SceneGraphBuilder;

        let ctx = Arc::new(SessionContext::new());
        let compiled = compile_tiny_async_raster_plot(&ctx).await;
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(ctx);
        let mut state = ChartAppState::new(session, policy, ChartAppOptions::default());
        assert!(
            !state
                .has_evaluation_invalidation_subscription_for_testing()
                .await
        );

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build initial async raster scene");

        assert!(
            state
                .missing_materialization_wakeup_warning_emitted_for_testing()
                .await
        );
    }

    #[test]
    fn materialization_deferred_evaluation_maps_to_delayed_render_invalidation() {
        let request = render_invalidation_request_for_evaluation(EvaluationInvalidation {
            epoch: 7,
            reason: EvaluationInvalidationReason::MaterializationDeferred {
                kind: MaterializationKind::new("rasterize-2d"),
            },
            schedule: EvaluationInvalidationSchedule::After(Duration::from_millis(25)),
        });

        assert_eq!(
            request.reason,
            RenderInvalidationReason::EvaluationChanged {
                kind: "materialization-deferred:rasterize-2d".to_string()
            }
        );
        assert_eq!(
            request.schedule,
            RenderInvalidationSchedule::After(Duration::from_millis(25))
        );
    }

    fn count_image_marks(scene: &SceneGraph) -> usize {
        scene.marks.iter().map(count_image_marks_in_mark).sum()
    }

    fn count_image_marks_in_mark(mark: &SceneMark) -> usize {
        match mark {
            SceneMark::Image(_) => 1,
            SceneMark::Group(group) => group.marks.iter().map(count_image_marks_in_mark).sum(),
            _ => 0,
        }
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

        let result = state.set_param("width", 720.0).expect("set width");

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

        let result = state.set_param("width", 640.0).expect("set width");

        assert!(!result.changed);
        assert_eq!(result.revision, 0);
        assert!(result.change.is_none());
        assert!(state.param_changes_since(0).is_empty());
        assert_eq!(state.param_revision(), 0);
    }

    #[tokio::test]
    async fn rapid_set_param_calls_keep_latest_value_and_revision_order() {
        let state = resize_test_state().await;

        state.set_param("width", 700.0).expect("set width");
        state.set_param("width", 710.0).expect("set width");
        state.set_param("width", 720.0).expect("set width");

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

        state.set_param("width", 700.0).expect("set width");
        state.set_param("height", 500.0).expect("set height");

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
        let compiled = Chart::<Cartesian>::new()
            .param(enabled)
            .compile(&ctx)
            .await
            .expect("compile bool param test plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, ChartAppOptions::default());

        assert_eq!(state.param_bool("enabled"), Some(true));
        let result = state.set_param("enabled", false).expect("set enabled");

        assert!(result.changed);
        assert_eq!(state.param_bool("enabled"), Some(false));
        assert_eq!(state.param_f64("enabled"), None);
    }

    #[tokio::test]
    async fn set_param_queues_when_runtime_is_busy() {
        use avenger_app::app::SceneGraphBuilder;

        let mut state = resize_test_state().await;
        let runtime = state.runtime.lock().await;

        let result = state.set_param("width", 700.0).expect("set width");

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
        let compiled = Chart::<Cartesian>::new()
            .param(width.clone())
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
    async fn preview_policy_persists_across_non_param_rebuilds_until_settle() {
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
            .expect("first preview build");
        assert_eq!(
            state
                .last_metrics()
                .await
                .expect("first preview metrics")
                .mode,
            EvaluationMode::Preview
        );
        assert_eq!(
            state.runtime.lock().await.next_evaluation_mode,
            EvaluationMode::Preview
        );

        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("second preview build");
        assert_eq!(
            state
                .last_metrics()
                .await
                .expect("second preview metrics")
                .mode,
            EvaluationMode::Preview
        );

        ChartResizeSettleHandler
            .handle(
                &SceneGraphEvent::CanvasResizeSettled(CanvasResizeEvent {
                    size: [800.0, 600.0],
                }),
                &mut state,
                &rtree,
            )
            .await;
        ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("settled exact build");
        assert_eq!(
            state.last_metrics().await.expect("settled metrics").mode,
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

    async fn reaction_test_state(chart: Chart<Cartesian>, ctx: SessionContext) -> ChartAppState {
        let compiled = chart.compile(&ctx).await.expect("compile reaction chart");
        let graph =
            Arc::new(param_change_graph_for_plot(&compiled, &ctx).expect("compile reaction graph"));
        let resize_policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let mut state = ChartAppState::new(session, resize_policy, ChartAppOptions::default());
        state.has_param_reactions = true;
        state.runtime.lock().await.param_change_graph = Some(graph);
        state
    }

    #[tokio::test]
    async fn param_transaction_cascades_in_typed_waves() {
        let a = Param::new("a", ScalarValue::Int64(Some(0)));
        let b = Param::new("b", ScalarValue::Int64(Some(10)));
        let c = Param::new("c", ScalarValue::Int64(Some(100)));
        let chart = Chart::<Cartesian>::new()
            .param(a.clone())
            .param(b.clone())
            .param(c.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&a)
                    .filter(param_change::previous_value().eq(lit(0_i64)))
                    .set_param(&b, param_change::value() + lit(1_i64)),
            )
            .param_change_binding(
                ChartParamChangeBinding::on(&b).set_param(&c, a.expr() + param_change::value()),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let mut runtime = state.runtime.lock().await;
        let outcome = state
            .apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    state.next_param_transaction_id(),
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Preview,
                ),
                IndexMap::from([("a".to_string(), ScalarValue::Int64(Some(2)))]),
                Vec::new(),
                Vec::new(),
            )
            .expect("apply reaction transaction");

        assert!(outcome.param_changed);
        assert_eq!(
            runtime.session.params(),
            &IndexMap::from([
                ("a".to_string(), ScalarValue::Int64(Some(2))),
                ("b".to_string(), ScalarValue::Int64(Some(3))),
                ("c".to_string(), ScalarValue::Int64(Some(5))),
            ])
        );
        assert_eq!(
            outcome
                .changes
                .iter()
                .map(|change| change.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[tokio::test]
    async fn param_reaction_actions_observe_preceding_writes() {
        let source = Param::new("source", ScalarValue::Int64(Some(0)));
        let first = Param::new("first", ScalarValue::Int64(Some(0)));
        let second = Param::new("second", ScalarValue::Int64(Some(0)));
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(first.clone())
            .param(second.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source).then(
                    ChartAction::new()
                        .set_param(&first, param_change::value() + lit(1_i64))
                        .set_param(&second, first.expr() + lit(1_i64)),
                ),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;

        state
            .set_param("source", 4_i64)
            .expect("run ordered reaction");

        assert_eq!(state.param_f64("first"), Some(5.0));
        assert_eq!(state.param_f64("second"), Some(6.0));
    }

    #[tokio::test]
    async fn public_set_param_reports_complete_reaction_batch() {
        let source = Param::new("source", ScalarValue::Int64(Some(0)));
        let sink = Param::new("sink", ScalarValue::Int64(Some(0)));
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(sink.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source)
                    .set_param(&sink, param_change::value() + lit(1_i64)),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let result = state
            .set_param("source", 4_i64)
            .expect("apply public parameter transaction");

        assert!(result.changed);
        assert_eq!(
            result.change.as_ref().map(|change| change.name.as_str()),
            Some("source")
        );
        assert_eq!(
            result
                .changes
                .iter()
                .map(|change| (change.name.as_str(), change.transaction_id))
                .collect::<Vec<_>>(),
            vec![
                ("source", result.transaction_id),
                ("sink", result.transaction_id),
            ]
        );
        assert_eq!(state.param_f64("source"), Some(4.0));
        assert_eq!(state.param_f64("sink"), Some(5.0));
    }

    #[tokio::test]
    async fn param_transaction_rolls_back_on_reaction_error() {
        let source = Param::new("source", ScalarValue::Int64(Some(0)));
        let sink = Param::new("sink", ScalarValue::Int64(Some(9)));
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(sink.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source)
                    .set_param_required(&sink, lit(ScalarValue::Int64(None))),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let mut runtime = state.runtime.lock().await;
        let error = state
            .apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    state.next_param_transaction_id(),
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Exact,
                ),
                IndexMap::from([("source".to_string(), ScalarValue::Int64(Some(1)))]),
                Vec::new(),
                Vec::new(),
            )
            .err()
            .expect("required null reaction must fail");
        assert!(error.to_string().contains("required assignment"), "{error}");
        assert_eq!(
            runtime.session.params().get("source"),
            Some(&ScalarValue::Int64(Some(0)))
        );
        assert_eq!(
            runtime.session.params().get("sink"),
            Some(&ScalarValue::Int64(Some(9)))
        );
        assert!(state.param_changes_since(0).is_empty());
        drop(runtime);
        let error = state
            .set_param("source", 1_i64)
            .expect_err("public reaction error must propagate");
        assert!(error.to_string().contains("required assignment"), "{error}");
        assert!(state.param_changes_since(0).is_empty());
    }

    #[tokio::test]
    async fn button_action_failure_rolls_back_activation_and_never_latches() {
        let sink = Param::new("sink", ScalarValue::Int64(Some(9)));
        let button = avenger_chart_widgets::Button::new("clear")
            .label("Clear")
            .action(ChartAction::new().set_param_required(&sink, lit(ScalarValue::Int64(None))));
        let activation = button.activation_param();
        let chart = Chart::<Cartesian>::new()
            .param(sink)
            .widget(button.position(ChromePosition::Left));
        let state = reaction_test_state(chart, SessionContext::new()).await;

        let error = state
            .set_param(&activation.name, 1_u64)
            .expect_err("button action failure must reject the whole activation");
        assert!(error.to_string().contains("required assignment"), "{error}");
        assert_eq!(
            state.param_snapshot().params.get(&activation.name),
            Some(&ScalarValue::UInt64(Some(0)))
        );
        assert_eq!(state.param_f64("sink"), Some(9.0));
        assert!(state.param_changes_since(0).is_empty());
    }

    #[tokio::test]
    async fn one_reaction_atomically_resets_copies_and_clears_all_state_kinds() {
        let source = Param::new("source", 0_u64);
        let scalar = Param::new("scalar", 7_i64);
        let audit = Param::new("audit", 0_u64);
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let initial_store =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])
                .expect("initial store batch");
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(scalar.clone())
            .param(audit.clone())
            .selection(Selection::new("picked"))
            .store(Store::from_record_batch("cache", initial_store))
            .param_change_binding(
                ChartParamChangeBinding::on(&source).then(
                    ChartAction::new()
                        .reset_param(&scalar)
                        .set_param(&audit, param_change::value())
                        .clear_selection("picked")
                        .set_store("cache", StoreUpdate::clear()),
                ),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        state
            .set_param(&scalar.name, 42_i64)
            .expect("move scalar away from its registered default");
        let baseline_revision = state.param_revision();
        {
            let mut runtime = state.runtime.lock().await;
            runtime
                .session
                .apply_selection_patch(vec![SelectionAssignment {
                    selection_id: "picked".to_string(),
                    update: SelectionStateUpdate::ReplaceAllClauses {
                        clauses: vec![SelectionClause {
                            id: "seed".to_string(),
                            scope: ResolvedSelectionClauseScope {
                                sharing: CoordinationScope::Shared,
                                owner_path: Vec::new(),
                            },
                            predicate: SelectionPredicateSpec::Equality {
                                dimensions: vec![SelectionEqualityDimensionValue {
                                    id: "id".to_string(),
                                    field_expr: LogicalExprNode::from_expr(lit(1_i64))
                                        .expect("selection field expression"),
                                    value: ScalarValue::Int64(Some(1)),
                                }],
                            },
                            facet_context: Vec::new(),
                        }],
                    },
                }])
                .expect("seed selection");
            assert_eq!(
                runtime
                    .session
                    .selection_clauses_for_diagnostics("picked")
                    .len(),
                1
            );
            assert_eq!(
                runtime.session.store_rows_for_diagnostics("cache")[0]
                    .1
                    .len(),
                1
            );
        }

        let result = state
            .set_param(&source.name, 3_u64)
            .expect("run composed state reaction");
        assert!(result.changed);
        assert_eq!(
            result
                .changes
                .iter()
                .map(|change| (change.name.as_str(), change.transaction_id))
                .collect::<Vec<_>>(),
            vec![
                ("source", result.transaction_id),
                ("scalar", result.transaction_id),
                ("audit", result.transaction_id),
            ]
        );
        assert_eq!(state.param_f64("source"), Some(3.0));
        assert_eq!(state.param_f64("scalar"), Some(7.0));
        assert_eq!(state.param_f64("audit"), Some(3.0));
        assert_eq!(state.param_changes_since(baseline_revision), result.changes);
        let runtime = state.runtime.lock().await;
        assert!(
            runtime
                .session
                .selection_clauses_for_diagnostics("picked")
                .is_empty()
        );
        assert!(
            runtime
                .session
                .store_rows_for_diagnostics("cache")
                .iter()
                .all(|(_, rows)| rows.is_empty())
        );
    }

    #[tokio::test]
    async fn busy_runtime_preserves_fifo_reaction_transactions() {
        let source = Param::new("source", ScalarValue::Int64(Some(0)));
        let sink = Param::new("sink", ScalarValue::Int64(Some(0)));
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(sink.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source).set_param(&sink, param_change::value()),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let runtime = state.runtime.lock().await;

        assert!(state.set_param("source", 1_i64).expect("queue one").changed);
        assert!(state.set_param("source", 2_i64).expect("queue two").changed);
        assert_eq!(
            state.param_snapshot().params.get("source"),
            Some(&ScalarValue::Int64(Some(0))),
            "queued reaction transactions are not published optimistically"
        );
        drop(runtime);

        let mut runtime = state.runtime.lock().await;
        assert!(state.drain_pending_params_into_runtime(&mut runtime));
        assert_eq!(
            runtime.session.params().get("source"),
            Some(&ScalarValue::Int64(Some(2)))
        );
        assert_eq!(
            runtime.session.params().get("sink"),
            Some(&ScalarValue::Int64(Some(2)))
        );
        let changes = state.param_changes_since(0);
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.name.as_str(), change.value.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("source", ScalarValue::Int64(Some(1))),
                ("sink", ScalarValue::Int64(Some(1))),
                ("source", ScalarValue::Int64(Some(2))),
                ("sink", ScalarValue::Int64(Some(2))),
            ]
        );
        assert_eq!(changes[0].transaction_id, changes[1].transaction_id);
        assert_eq!(changes[2].transaction_id, changes[3].transaction_id);
        assert_ne!(changes[0].transaction_id, changes[2].transaction_id);
    }

    #[tokio::test]
    async fn param_transaction_filters_and_equal_writes_are_noops() {
        let source = Param::new("source", ScalarValue::Int64(Some(0)));
        let sink = Param::new("sink", ScalarValue::Int64(Some(7)));
        let chart = Chart::<Cartesian>::new()
            .param(source.clone())
            .param(sink.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source)
                    .filter(param_change::value().gt(lit(5_i64)))
                    .set_param(&sink, param_change::value()),
            );
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let mut runtime = state.runtime.lock().await;

        let equal = state
            .apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    state.next_param_transaction_id(),
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Exact,
                ),
                IndexMap::from([("source".to_string(), ScalarValue::Int64(Some(0)))]),
                Vec::new(),
                Vec::new(),
            )
            .expect("equal transaction");
        assert!(!equal.param_changed);
        assert!(equal.changes.is_empty());

        let filtered = state
            .apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    state.next_param_transaction_id(),
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Exact,
                ),
                IndexMap::from([("source".to_string(), ScalarValue::Int64(Some(3)))]),
                Vec::new(),
                Vec::new(),
            )
            .expect("filtered transaction");
        assert!(filtered.param_changed);
        assert_eq!(
            runtime.session.params().get("sink"),
            Some(&ScalarValue::Int64(Some(7)))
        );
    }

    #[tokio::test]
    async fn simultaneous_reaction_sink_collision_rolls_back_batch() {
        let a = Param::new("a", ScalarValue::Int64(Some(0)));
        let b = Param::new("b", ScalarValue::Int64(Some(0)));
        let chart = Chart::<Cartesian>::new()
            .param(a.clone())
            .param(b.clone())
            .selection(Selection::new("brush"))
            .param_change_binding(ChartParamChangeBinding::on(&a).clear_selection("brush"))
            .param_change_binding(ChartParamChangeBinding::on(&b).clear_selection("brush"));
        let state = reaction_test_state(chart, SessionContext::new()).await;
        let mut runtime = state.runtime.lock().await;
        let error = state
            .apply_root_param_transaction_to_runtime(
                &mut runtime,
                ParamTransactionContext::new(
                    state.next_param_transaction_id(),
                    ParamTransactionOrigin::ExternalHost,
                    EvaluationMode::Exact,
                ),
                IndexMap::from([
                    ("a".to_string(), ScalarValue::Int64(Some(1))),
                    ("b".to_string(), ScalarValue::Int64(Some(1))),
                ]),
                Vec::new(),
                Vec::new(),
            )
            .err()
            .expect("selection sink collision must fail");
        assert!(
            error.to_string().contains("targeting selection 'brush'"),
            "{error}"
        );
        assert_eq!(
            runtime.session.params().get("a"),
            Some(&ScalarValue::Int64(Some(0)))
        );
        assert_eq!(
            runtime.session.params().get("b"),
            Some(&ScalarValue::Int64(Some(0)))
        );
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
