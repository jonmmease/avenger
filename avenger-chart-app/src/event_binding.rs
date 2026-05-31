use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use avenger_app::error::AvengerAppError;
use avenger_chart::{
    event::{
        self, ChartEventBinding, ChartEventEvaluationMode, ChartEventStream, ChartEventType,
        InteractionColumnRequests,
    },
    plot::{CompiledPlot, ScopedParamAssignment, ScopedParamStoreSnapshot},
    render::{EvaluatedInteractionScope, EvaluationMode},
    serialization::LogicalExprNodeExt,
};
use avenger_chart_core::{
    CompiledParamSpec, CompiledScalarExpressionProgram, InteractionPointInversionRequest,
    PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions, PlaceholderColumn, Sharing,
    collect_placeholder_ids, one_row_batch_from_scalars, schema_from_fields,
};
use avenger_common::time::Instant;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{ModifiersState, SceneGraphEvent, SceneGraphEventType},
    stream::{
        EventStreamConfig, EventStreamContext, EventStreamEventSnapshot, EventStreamFilter,
        UpdateStatus,
    },
    window::{Key, MouseButton, MouseScrollDelta},
};
use avenger_geometry::rtree::SceneGraphRTree;
use datafusion::{
    arrow::{
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    error::DataFusionError,
    prelude::SessionContext,
    scalar::ScalarValue,
};
use indexmap::IndexMap;

use crate::ChartAppState;

pub(crate) fn event_streams_for_plot_bindings(
    compiled_plot: &CompiledPlot,
    ctx: &SessionContext,
) -> Result<
    Vec<(
        EventStreamConfig,
        Arc<dyn EventStreamHandler<ChartAppState>>,
    )>,
    AvengerAppError,
> {
    event_streams_for_bindings(
        compiled_plot.event_bindings(),
        ctx,
        compiled_plot.param_specs(),
    )
}

pub(crate) fn event_streams_for_bindings(
    bindings: &[ChartEventBinding],
    ctx: &SessionContext,
    param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<
    Vec<(
        EventStreamConfig,
        Arc<dyn EventStreamHandler<ChartAppState>>,
    )>,
    AvengerAppError,
> {
    let mut streams = Vec::new();
    for (binding_index, binding) in bindings.iter().enumerate() {
        let runtime = Arc::new(CompiledChartEventBinding::compile(
            binding_index,
            binding,
            ctx,
            param_specs,
        )?);
        streams.push((
            runtime.event_stream_config.clone(),
            Arc::new(ChartEventBindingHandler {
                runtime: runtime.clone(),
                state: Mutex::new(ChartEventBindingState::default()),
            }) as Arc<dyn EventStreamHandler<ChartAppState>>,
        ));

        if binding.settle_exact {
            if let Some(between) = &binding.between {
                let end_config =
                    stream_config_for_chart_stream(&between.end, None, false, ctx, param_specs)?;
                streams.push((
                    end_config,
                    Arc::new(ChartEventExactOnlyHandler)
                        as Arc<dyn EventStreamHandler<ChartAppState>>,
                ));
            }
        }
    }
    Ok(streams)
}

struct CompiledChartEventBinding {
    binding_index: usize,
    event_stream_config: EventStreamConfig,
    program: CompiledScalarExpressionProgram,
    filter_count: usize,
    assignments: Vec<CompiledParamAssignment>,
    evaluation_mode: ChartEventEvaluationMode,
    interaction_requests: InteractionColumnRequests,
}

struct CompiledParamAssignment {
    param_name: String,
    sharing: Sharing,
}

impl CompiledChartEventBinding {
    fn compile(
        binding_index: usize,
        binding: &ChartEventBinding,
        ctx: &SessionContext,
        param_specs: &IndexMap<String, CompiledParamSpec>,
    ) -> Result<Self, AvengerAppError> {
        binding
            .validate()
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        for assignment in &binding.assignments {
            if !param_specs.contains_key(&assignment.param_name) {
                return Err(AvengerAppError::InternalError(format!(
                    "Chart event binding assigns unknown param '{}'",
                    assignment.param_name
                )));
            }
        }

        // Convert filter and assignment expressions once so we can scan them for
        // reserved interaction columns before building the event schema.
        let mut filter_exprs = Vec::new();
        for filter in &binding.filters {
            filter_exprs.push(
                filter
                    .to_expr(ctx)
                    .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
            );
        }
        let mut assignment_exprs = Vec::new();
        for assignment in &binding.assignments {
            let expr = assignment
                .expr
                .to_expr(ctx)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            assignment_exprs.push((assignment.param_name.clone(), expr));
        }

        let mut scan_exprs = filter_exprs.clone();
        scan_exprs.extend(assignment_exprs.iter().map(|(_, expr)| expr.clone()));
        let interaction_requests = event::scan_interaction_columns(&scan_exprs);

        let schema = event_schema(param_specs, &interaction_requests);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let placeholder_columns = param_specs.keys().map(|param| {
            PlaceholderColumn::new(format!("${param}"), event::param_column_name(param))
        });
        let mut specs = Vec::new();
        for (index, expr) in filter_exprs.into_iter().enumerate() {
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("filter_{index}"), expr)
                    .with_expected_type(DataType::Boolean),
            );
        }
        let filter_count = specs.len();
        let mut assignments = Vec::new();
        for (param_name, expr) in assignment_exprs {
            let spec = param_specs
                .get(&param_name)
                .expect("assignment param validated");
            let target_type = spec.default.data_type();
            let sharing = spec.sharing;
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("assign_{param_name}"), expr)
                    .with_expected_type(target_type)
                    .with_nullable_cast(),
            );
            assignments.push(CompiledParamAssignment {
                param_name,
                sharing,
            });
        }
        let program = CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(allowed_columns)
                .with_placeholder_columns(placeholder_columns),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let event_stream_config = event_stream_config_for_binding(binding, ctx, param_specs)?;

        Ok(Self {
            binding_index,
            event_stream_config,
            program,
            filter_count,
            assignments,
            evaluation_mode: binding.evaluation_mode,
            interaction_requests,
        })
    }
}

#[derive(Default)]
struct ChartEventBindingState {
    active_start: Option<Instant>,
    start_params: Option<ScopedParamStoreSnapshot>,
    previous_params: Option<ScopedParamStoreSnapshot>,
    start_scope: Option<EvaluatedInteractionScope>,
    previous_scope: Option<EvaluatedInteractionScope>,
}

struct ChartEventBindingHandler {
    runtime: Arc<CompiledChartEventBinding>,
    state: Mutex<ChartEventBindingState>,
}

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartEventBindingHandler {
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
        let mut app = state.runtime.lock().await;
        let eval_start = Instant::now();

        let requests = &self.runtime.interaction_requests;
        let required_channels = requests.all_channels();
        let routing_enabled = !requests.is_empty();

        // Route the current event position to a coordinate scope.
        let current_scope: Option<EvaluatedInteractionScope> = if routing_enabled {
            match route_interaction_scope(
                &app.last_interaction_state.scopes,
                event.position(),
                &required_channels,
            ) {
                InteractionRoute::Scope(scope) => Some(scope.clone()),
                InteractionRoute::None | InteractionRoute::Ambiguous => None,
            }
        } else {
            None
        };

        // Freeze scoped params and the start scope at a new gesture start.
        {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            match &context.start_event {
                Some(start) if binding_state.active_start != Some(start.instant) => {
                    binding_state.active_start = Some(start.instant);
                    binding_state.start_params = Some(app.session.snapshot_scoped_params());
                    binding_state.start_scope = if routing_enabled {
                        match route_interaction_scope(
                            &app.last_interaction_state.scopes,
                            start.event.position(),
                            &required_channels,
                        ) {
                            InteractionRoute::Scope(scope) => Some(scope.clone()),
                            InteractionRoute::None | InteractionRoute::Ambiguous => None,
                        }
                    } else {
                        None
                    };
                    binding_state.previous_params = None;
                    binding_state.previous_scope = None;
                }
                None => {
                    binding_state.active_start = None;
                    binding_state.start_params = None;
                    binding_state.start_scope = None;
                }
                _ => {}
            }
        }

        let (start_snapshot, previous_snapshot, start_scope, previous_scope) = {
            let binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            (
                binding_state.start_params.clone(),
                binding_state.previous_params.clone(),
                binding_state.start_scope.clone(),
                binding_state.previous_scope.clone(),
            )
        };

        // Resolve effective params for the current/start/previous scopes.
        let current_owner_paths = current_scope
            .as_ref()
            .map(|scope| scope.sharing_owner_paths.clone())
            .unwrap_or_default();
        let current_params = app
            .session
            .effective_params_for_owner_paths(&current_owner_paths);
        let start_params = start_snapshot.as_ref().map(|snapshot| {
            let owner_paths = start_scope
                .as_ref()
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            app.session
                .effective_params_from_snapshot(snapshot, &owner_paths)
        });
        let previous_params = previous_snapshot.as_ref().map(|snapshot| {
            let owner_paths = previous_scope
                .as_ref()
                .map(|scope| scope.sharing_owner_paths.clone())
                .unwrap_or_default();
            app.session
                .effective_params_from_snapshot(snapshot, &owner_paths)
        });

        // Derive requested coordinate/domain columns from the routed scopes.
        let interaction_values = compute_interaction_values(
            requests,
            event.position(),
            context
                .start_event
                .as_ref()
                .and_then(|s| s.event.position()),
            context
                .previous_event
                .as_ref()
                .and_then(|p| p.event.position()),
            current_scope.as_ref(),
            start_scope.as_ref(),
            previous_scope.as_ref(),
        );

        let batch = match event_record_batch(
            self.runtime.program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &current_params,
                start_params: start_params.as_ref(),
                previous_params: previous_params.as_ref(),
                interaction_values: &interaction_values,
            },
        ) {
            Ok(batch) => batch,
            Err(err) => {
                app.event_metrics.evaluation_errors += 1;
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    error = %err,
                    "failed to build chart event batch"
                );
                return UpdateStatus::default();
            }
        };
        app.event_metrics.event_batches_evaluated += 1;

        let values = match self.runtime.program.evaluate_values(&batch) {
            Ok(values) => values,
            Err(err) => {
                app.event_metrics.evaluation_errors += 1;
                record_event_eval_elapsed(&mut app.event_metrics, eval_start);
                tracing::warn!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    error = %err,
                    "failed to evaluate chart event expressions"
                );
                return UpdateStatus::default();
            }
        };
        app.event_metrics.physical_expression_evaluations +=
            self.runtime.program.expression_count();

        if !filters_pass(&values[..self.runtime.filter_count]) {
            app.event_metrics.filter_failures += 1;
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus::default();
        }
        app.event_metrics.filter_passes += 1;

        let mut patch: Vec<ScopedParamAssignment> = Vec::new();
        for (assignment, value) in self
            .runtime
            .assignments
            .iter()
            .zip(values[self.runtime.filter_count..].iter())
        {
            // Derived interaction columns are null when the gesture has no routed
            // scope (e.g. a drag that started outside any plot area). Such an
            // assignment evaluates to a null or null-element value; writing it
            // would corrupt the target param, so treat it as a no-op.
            if !assignment_value_is_writable(value) {
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    param = %assignment.param_name,
                    "skipping null/degenerate assignment value"
                );
                continue;
            }
            let Some(owner_path) =
                assignment_owner_path(assignment.sharing, current_scope.as_ref())
            else {
                // Non-shared param with no routed scope: skip rather than write
                // to the wrong owner.
                tracing::debug!(
                    target: "avenger_chart_app::event_binding",
                    binding = self.runtime.binding_index,
                    param = %assignment.param_name,
                    "skipping scoped assignment with no routed scope"
                );
                continue;
            };
            if current_params.get(&assignment.param_name) != Some(value) {
                patch.push(ScopedParamAssignment {
                    name: assignment.param_name.clone(),
                    owner_path,
                    value: value.clone(),
                });
            }
        }

        let mut should_rerender = !patch.is_empty();
        if !patch.is_empty() {
            app.event_metrics.param_patch_events += 1;
            app.event_metrics.params_patched += patch.len();
            app.session.apply_scoped_param_patch(patch);
        }
        if self.runtime.assignments.is_empty()
            && self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact
        {
            should_rerender = true;
        }
        if !should_rerender {
            if !self.runtime.assignments.is_empty() {
                app.event_metrics.unchanged_patch_skips += 1;
            }
            record_event_eval_elapsed(&mut app.event_metrics, eval_start);
            return UpdateStatus::default();
        }
        if matches!(event, SceneGraphEvent::CanvasResize(_)) {
            app.accepted_resize_count += 1;
            tracing::debug!(
                target: "avenger_chart_app::resize",
                seq = app.accepted_resize_count,
                mode = ?self.runtime.evaluation_mode,
                "canvas resize accepted"
            );
        }

        app.next_evaluation_mode = match self.runtime.evaluation_mode {
            ChartEventEvaluationMode::Preview => EvaluationMode::Preview,
            ChartEventEvaluationMode::Exact => EvaluationMode::Exact,
        };
        record_event_eval_elapsed(&mut app.event_metrics, eval_start);

        {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            binding_state.previous_params = Some(app.session.snapshot_scoped_params());
            binding_state.previous_scope = current_scope.clone();
        }

        UpdateStatus {
            rerender: true,
            rebuild_geometry: self.runtime.evaluation_mode == ChartEventEvaluationMode::Exact,
        }
    }
}

fn record_event_eval_elapsed(metrics: &mut crate::ChartEventMetrics, start: Instant) {
    metrics.total_eval_us += start.elapsed().as_micros() as u64;
}

struct ChartEventExactOnlyHandler;

#[async_trait]
impl EventStreamHandler<ChartAppState> for ChartEventExactOnlyHandler {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut app = state.runtime.lock().await;
        app.next_evaluation_mode = EvaluationMode::Exact;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
        }
    }
}

/// Whether an assignment's evaluated value is safe to write to a param.
///
/// Derived interaction columns are null when a gesture has no routed scope, which
/// makes domain-list expressions evaluate to a null or null-element list. Writing
/// those would corrupt the target param (and can fail downstream scale building),
/// so they are treated as no-ops, matching the "null derived columns => no-op"
/// contract.
fn assignment_value_is_writable(value: &ScalarValue) -> bool {
    use datafusion::arrow::array::Array;
    match value {
        ScalarValue::Null => false,
        ScalarValue::List(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.len() >= 2 && elements.null_count() == 0
        }
        ScalarValue::LargeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.len() >= 2 && elements.null_count() == 0
        }
        ScalarValue::FixedSizeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return false;
            }
            let elements = array.value(0);
            elements.null_count() == 0
        }
        other => !other.is_null(),
    }
}

/// Resolve the owner path an assignment should write, given the routed scope.
///
/// `Shared` params always write the root path. Non-shared params require a
/// routed scope; without one, returns `None` so the caller skips the write.
fn assignment_owner_path(
    sharing: Sharing,
    scope: Option<&EvaluatedInteractionScope>,
) -> Option<Vec<ScalarValue>> {
    let level = sharing.to_level();
    if level == u8::MAX {
        return Some(Vec::new());
    }
    match scope {
        Some(scope) => Some(
            scope
                .sharing_owner_paths
                .get(&level)
                .cloned()
                .unwrap_or_default(),
        ),
        None => None,
    }
}

/// Invert a scene-space point through a scope's coordinate transform.
fn invert_scene_point(
    scope: &EvaluatedInteractionScope,
    scene_point: [f32; 2],
    channels: &[&str],
) -> Option<IndexMap<String, ScalarValue>> {
    let local_point = [
        scene_point[0] - scope.bounds.x,
        scene_point[1] - scope.bounds.y,
    ];
    scope
        .coord_transform
        .invert_interaction_point(InteractionPointInversionRequest {
            local_point,
            plot_area_width: scope.plot_area_width,
            plot_area_height: scope.plot_area_height,
            channels,
            scales: &scope.scales,
        })
        .ok()
}

/// Build a two-element `List(Float64)` domain scalar.
fn domain_list_scalar(min: f32, max: f32) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min as f64)),
            ScalarValue::Float64(Some(max as f64)),
        ],
        &DataType::Float64,
        true,
    ))
}

/// Compute the requested derived coordinate/domain columns from routed scopes.
#[allow(clippy::too_many_arguments)]
fn compute_interaction_values(
    requests: &InteractionColumnRequests,
    current_point: Option<[f32; 2]>,
    start_point: Option<[f32; 2]>,
    previous_point: Option<[f32; 2]>,
    current_scope: Option<&EvaluatedInteractionScope>,
    start_scope: Option<&EvaluatedInteractionScope>,
    previous_scope: Option<&EvaluatedInteractionScope>,
) -> HashMap<String, ScalarValue> {
    let mut values = HashMap::new();

    let fill_coords = |values: &mut HashMap<String, ScalarValue>,
                       channels: &std::collections::BTreeSet<String>,
                       point: Option<[f32; 2]>,
                       scope: Option<&EvaluatedInteractionScope>,
                       name_fn: fn(&str) -> String| {
        if channels.is_empty() {
            return;
        }
        let (Some(point), Some(scope)) = (point, scope) else {
            return;
        };
        let channel_refs: Vec<&str> = channels.iter().map(String::as_str).collect();
        if let Some(inverted) = invert_scene_point(scope, point, &channel_refs) {
            for channel in channels {
                if let Some(value) = inverted.get(channel) {
                    values.insert(name_fn(channel), value.clone());
                }
            }
        }
    };

    fill_coords(
        &mut values,
        &requests.current_coord,
        current_point,
        current_scope,
        event::event_coord_column_name,
    );
    fill_coords(
        &mut values,
        &requests.start_coord,
        start_point,
        start_scope,
        event::start_coord_column_name,
    );
    // event_at_start uses the CURRENT point through the FROZEN start scope.
    fill_coords(
        &mut values,
        &requests.event_at_start_coord,
        current_point,
        start_scope,
        event::event_at_start_coord_column_name,
    );
    fill_coords(
        &mut values,
        &requests.previous_coord,
        previous_point,
        previous_scope,
        event::previous_coord_column_name,
    );

    let fill_domains = |values: &mut HashMap<String, ScalarValue>,
                        channels: &std::collections::BTreeSet<String>,
                        scope: Option<&EvaluatedInteractionScope>,
                        name_fn: fn(&str) -> String| {
        let Some(scope) = scope else {
            return;
        };
        for channel in channels {
            if let Some(scale) = scope.scales.get(channel)
                && let Ok((min, max)) = scale.numeric_interval_domain()
            {
                values.insert(name_fn(channel), domain_list_scalar(min, max));
            }
        }
    };

    fill_domains(
        &mut values,
        &requests.current_domain,
        current_scope,
        event::event_domain_column_name,
    );
    fill_domains(
        &mut values,
        &requests.start_domain,
        start_scope,
        event::start_domain_column_name,
    );

    values
}

fn filters_pass(values: &[ScalarValue]) -> bool {
    values.iter().all(|value| match value {
        ScalarValue::Boolean(Some(value)) => *value,
        _ => false,
    })
}

/// Result of routing a pointer event against interaction scopes.
#[allow(dead_code)]
pub(crate) enum InteractionRoute<'a> {
    /// No event position, or no scope contained the point / supported the channels.
    None,
    /// A unique scope was selected.
    Scope(&'a EvaluatedInteractionScope),
    /// Multiple equal-priority scopes matched; coordinate columns stay null.
    Ambiguous,
}

fn scope_contains_point(scope: &EvaluatedInteractionScope, point: [f32; 2]) -> bool {
    let bounds = &scope.bounds;
    point[0] >= bounds.x
        && point[0] <= bounds.x + bounds.width
        && point[1] >= bounds.y
        && point[1] <= bounds.y + bounds.height
}

fn scope_area(scope: &EvaluatedInteractionScope) -> f32 {
    scope.bounds.width * scope.bounds.height
}

/// Route a pointer position to the unique smallest-area coordinate scope that
/// contains it and supports every requested channel.
///
/// Returns `None` when the event has no position or nothing matches, and
/// `Ambiguous` when multiple equal-smallest-area scopes match.
#[allow(dead_code)]
pub(crate) fn route_interaction_scope<'a>(
    scopes: &'a [EvaluatedInteractionScope],
    point: Option<[f32; 2]>,
    required_channels: &std::collections::BTreeSet<String>,
) -> InteractionRoute<'a> {
    let Some(point) = point else {
        return InteractionRoute::None;
    };
    let mut candidates: Vec<&EvaluatedInteractionScope> = scopes
        .iter()
        .filter(|scope| {
            scope_contains_point(scope, point)
                && required_channels
                    .iter()
                    .all(|channel| scope.channels.iter().any(|c| c == channel))
        })
        .collect();
    if candidates.is_empty() {
        return InteractionRoute::None;
    }
    candidates.sort_by(|a, b| scope_area(a).total_cmp(&scope_area(b)));
    let smallest = scope_area(candidates[0]);
    let tied = candidates
        .iter()
        .filter(|scope| (scope_area(scope) - smallest).abs() < f32::EPSILON)
        .count();
    if tied > 1 {
        return InteractionRoute::Ambiguous;
    }
    InteractionRoute::Scope(candidates[0])
}

fn event_stream_config_for_binding(
    binding: &ChartEventBinding,
    ctx: &SessionContext,
    param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: vec![scene_event_type_from_chart(binding.event_type)],
        throttle: binding.throttle_ms,
        consume: binding.consume,
        ..Default::default()
    };
    if let Some(between) = &binding.between {
        config.between = Some((
            Box::new(stream_config_for_chart_stream(
                &between.start,
                None,
                false,
                ctx,
                param_specs,
            )?),
            Box::new(stream_config_for_chart_stream(
                &between.end,
                None,
                false,
                ctx,
                param_specs,
            )?),
        ));
    }
    Ok(config)
}

fn stream_config_for_chart_stream(
    stream: &ChartEventStream,
    throttle: Option<u64>,
    consume: bool,
    ctx: &SessionContext,
    param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: stream
            .event_type
            .map(|event_type| vec![scene_event_type_from_chart(event_type)])
            .unwrap_or_default(),
        source_group: stream.source_group.clone(),
        mark_paths: stream.mark_paths.clone(),
        throttle,
        consume,
        ..Default::default()
    };
    if config.types.is_empty() {
        return Err(AvengerAppError::InternalError(
            "Chart event stream requires an event type".to_string(),
        ));
    }
    if !stream.filters.is_empty() {
        config.filter = Some(vec![compile_low_level_stream_filter(
            stream,
            ctx,
            param_specs,
        )?]);
    }
    Ok(config)
}

fn compile_low_level_stream_filter(
    stream: &ChartEventStream,
    ctx: &SessionContext,
    _param_specs: &IndexMap<String, CompiledParamSpec>,
) -> Result<EventStreamFilter, AvengerAppError> {
    let schema = event_schema(
        &IndexMap::new(),
        &event::InteractionColumnRequests::default(),
    );
    let allowed_columns = schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let mut specs = Vec::new();
    for (index, filter) in stream.filters.iter().enumerate() {
        let expr = filter
            .to_expr(ctx)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        let placeholders = collect_placeholder_ids(&expr)
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        if !placeholders.is_empty() {
            return Err(AvengerAppError::InternalError(
                "Chart event stream start/end filters cannot reference params yet".to_string(),
            ));
        }
        specs.push(
            PhysicalScalarExpressionSpec::new(format!("stream_filter_{index}"), expr)
                .with_expected_type(DataType::Boolean),
        );
    }
    let program = Arc::new(
        CompiledScalarExpressionProgram::compile(
            ctx,
            schema,
            specs,
            PhysicalScalarProgramOptions::default().with_allowed_columns(allowed_columns),
        )
        .map_err(|err| AvengerAppError::InternalError(err.to_string()))?,
    );
    Ok(EventStreamFilter::context(move |event, context, _rtree| {
        let params = IndexMap::new();
        let interaction_values = HashMap::new();
        let batch = match event_record_batch(
            program.schema().clone(),
            event,
            context,
            EventBatchInputs {
                current_params: &params,
                start_params: None,
                previous_params: None,
                interaction_values: &interaction_values,
            },
        ) {
            Ok(batch) => batch,
            Err(_) => return false,
        };
        let Ok(values) = program.evaluate_values(&batch) else {
            return false;
        };
        filters_pass(&values)
    }))
}

fn event_schema(
    param_specs: &IndexMap<String, CompiledParamSpec>,
    interaction: &event::InteractionColumnRequests,
) -> Arc<Schema> {
    let mut fields = vec![
        Field::new(event::EVENT_TYPE_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_X_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_Y_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_CANVAS_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_CANVAS_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WINDOW_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WINDOW_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WHEEL_DELTA_X_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_WHEEL_DELTA_Y_FIELD, DataType::Float64, true),
        Field::new(event::EVENT_BUTTON_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_KEY_FIELD, DataType::Utf8, true),
        Field::new(event::EVENT_SHIFT_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_CONTROL_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_ALT_FIELD, DataType::Boolean, true),
        Field::new(event::EVENT_META_FIELD, DataType::Boolean, true),
        Field::new(event::START_X_FIELD, DataType::Float64, true),
        Field::new(event::START_Y_FIELD, DataType::Float64, true),
        Field::new(event::START_CANVAS_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::START_CANVAS_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::START_WINDOW_WIDTH_FIELD, DataType::Float64, true),
        Field::new(event::START_WINDOW_HEIGHT_FIELD, DataType::Float64, true),
        Field::new(event::START_TIME_MS_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_X_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_Y_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_TIME_MS_FIELD, DataType::Float64, true),
        Field::new(event::ELAPSED_MS_FIELD, DataType::Float64, true),
        Field::new(event::PREVIOUS_ELAPSED_MS_FIELD, DataType::Float64, true),
    ];
    for (name, spec) in param_specs {
        let data_type = spec.default.data_type();
        fields.push(Field::new(
            event::param_column_name(name),
            data_type.clone(),
            true,
        ));
        fields.push(Field::new(
            event::start_param_column_name(name),
            data_type.clone(),
            true,
        ));
        fields.push(Field::new(
            event::previous_param_column_name(name),
            data_type,
            true,
        ));
    }

    // Derived coordinate columns invert to a single channel value (Float64).
    for channel in interaction.current_coord.iter() {
        fields.push(Field::new(
            event::event_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.start_coord.iter() {
        fields.push(Field::new(
            event::start_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.event_at_start_coord.iter() {
        fields.push(Field::new(
            event::event_at_start_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    for channel in interaction.previous_coord.iter() {
        fields.push(Field::new(
            event::previous_coord_column_name(channel),
            DataType::Float64,
            true,
        ));
    }
    // Derived domain columns are two-element numeric lists.
    let domain_list_type = DataType::List(Arc::new(Field::new("item", DataType::Float64, true)));
    for channel in interaction.current_domain.iter() {
        fields.push(Field::new(
            event::event_domain_column_name(channel),
            domain_list_type.clone(),
            true,
        ));
    }
    for channel in interaction.start_domain.iter() {
        fields.push(Field::new(
            event::start_domain_column_name(channel),
            domain_list_type.clone(),
            true,
        ));
    }

    schema_from_fields(fields)
}

/// Pre-resolved inputs for building a one-row event batch.
struct EventBatchInputs<'a> {
    current_params: &'a IndexMap<String, ScalarValue>,
    start_params: Option<&'a IndexMap<String, ScalarValue>>,
    previous_params: Option<&'a IndexMap<String, ScalarValue>>,
    interaction_values: &'a HashMap<String, ScalarValue>,
}

fn event_record_batch(
    schema: Arc<Schema>,
    event: &SceneGraphEvent,
    context: &EventStreamContext,
    inputs: EventBatchInputs<'_>,
) -> Result<RecordBatch, DataFusionError> {
    let mut values = HashMap::new();
    push_event_values(&mut values, event, "");
    if let Some(start) = &context.start_event {
        push_snapshot_values(&mut values, start, "start");
    }
    if let Some(previous) = &context.previous_event {
        push_snapshot_values(&mut values, previous, "previous");
    }
    if let (Some(current), Some(start)) = (&context.current_event, &context.start_event) {
        values.insert(
            event::ELAPSED_MS_FIELD.to_string(),
            ScalarValue::Float64(Some(duration_ms(current.instant, start.instant))),
        );
    }
    if let (Some(current), Some(previous)) = (&context.current_event, &context.previous_event) {
        values.insert(
            event::PREVIOUS_ELAPSED_MS_FIELD.to_string(),
            ScalarValue::Float64(Some(duration_ms(current.instant, previous.instant))),
        );
    }
    for (name, value) in inputs.current_params {
        values.insert(event::param_column_name(name), value.clone());
    }
    if let Some(start_params) = inputs.start_params {
        for (name, value) in start_params {
            values.insert(event::start_param_column_name(name), value.clone());
        }
    }
    if let Some(previous_params) = inputs.previous_params {
        for (name, value) in previous_params {
            values.insert(event::previous_param_column_name(name), value.clone());
        }
    }
    // Derived interaction columns are filled by name; absent columns become null.
    for (name, value) in inputs.interaction_values {
        values.insert(name.clone(), value.clone());
    }
    one_row_batch_from_scalars(schema, &values)
}

fn push_snapshot_values(
    values: &mut HashMap<String, ScalarValue>,
    snapshot: &EventStreamEventSnapshot,
    prefix: &str,
) {
    let mut event_values = HashMap::new();
    push_event_values(&mut event_values, &snapshot.event, "");
    let pairs: &[(&str, &str)] = match prefix {
        "start" => &[
            (event::EVENT_X_FIELD, event::START_X_FIELD),
            (event::EVENT_Y_FIELD, event::START_Y_FIELD),
            (
                event::EVENT_CANVAS_WIDTH_FIELD,
                event::START_CANVAS_WIDTH_FIELD,
            ),
            (
                event::EVENT_CANVAS_HEIGHT_FIELD,
                event::START_CANVAS_HEIGHT_FIELD,
            ),
            (
                event::EVENT_WINDOW_WIDTH_FIELD,
                event::START_WINDOW_WIDTH_FIELD,
            ),
            (
                event::EVENT_WINDOW_HEIGHT_FIELD,
                event::START_WINDOW_HEIGHT_FIELD,
            ),
        ],
        "previous" => &[
            (event::EVENT_X_FIELD, event::PREVIOUS_X_FIELD),
            (event::EVENT_Y_FIELD, event::PREVIOUS_Y_FIELD),
        ],
        _ => unreachable!(),
    };
    for (source, target) in pairs.iter().copied() {
        if let Some(value) = event_values.get(source) {
            values.insert(target.to_string(), value.clone());
        }
    }
    match prefix {
        "start" => {
            values.insert(
                event::START_TIME_MS_FIELD.to_string(),
                ScalarValue::Float64(Some(0.0)),
            );
        }
        "previous" => {
            values.insert(
                event::PREVIOUS_TIME_MS_FIELD.to_string(),
                ScalarValue::Float64(Some(0.0)),
            );
        }
        _ => {}
    }
}

fn push_event_values(
    values: &mut HashMap<String, ScalarValue>,
    event: &SceneGraphEvent,
    _prefix: &str,
) {
    values.insert(
        event::EVENT_TYPE_FIELD.to_string(),
        ScalarValue::Utf8(Some(event_type_name(event.event_type()).to_string())),
    );
    if let Some(position) = event.position() {
        values.insert(
            event::EVENT_X_FIELD.to_string(),
            ScalarValue::Float64(Some(position[0] as f64)),
        );
        values.insert(
            event::EVENT_Y_FIELD.to_string(),
            ScalarValue::Float64(Some(position[1] as f64)),
        );
    }
    if let Some(modifiers) = event_modifiers(event) {
        push_modifiers(values, modifiers);
    }
    match event {
        SceneGraphEvent::CanvasResize(e) | SceneGraphEvent::CanvasResizeSettled(e) => {
            values.insert(
                event::EVENT_CANVAS_WIDTH_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[0] as f64)),
            );
            values.insert(
                event::EVENT_CANVAS_HEIGHT_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[1] as f64)),
            );
        }
        SceneGraphEvent::WindowResize(e) | SceneGraphEvent::WindowResizeSettled(e) => {
            values.insert(
                event::EVENT_WINDOW_WIDTH_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[0] as f64)),
            );
            values.insert(
                event::EVENT_WINDOW_HEIGHT_FIELD.to_string(),
                ScalarValue::Float64(Some(e.size[1] as f64)),
            );
        }
        SceneGraphEvent::MouseDown(e) => push_button(values, e.button),
        SceneGraphEvent::MouseUp(e) => push_button(values, e.button),
        SceneGraphEvent::Click(e) => push_button(values, e.button),
        SceneGraphEvent::MouseWheel(e) => match e.delta {
            MouseScrollDelta::LineDelta(x, y) => {
                values.insert(
                    event::EVENT_WHEEL_DELTA_X_FIELD.to_string(),
                    ScalarValue::Float64(Some(x as f64)),
                );
                values.insert(
                    event::EVENT_WHEEL_DELTA_Y_FIELD.to_string(),
                    ScalarValue::Float64(Some(y as f64)),
                );
            }
            MouseScrollDelta::PixelDelta(x, y) => {
                values.insert(
                    event::EVENT_WHEEL_DELTA_X_FIELD.to_string(),
                    ScalarValue::Float64(Some(x)),
                );
                values.insert(
                    event::EVENT_WHEEL_DELTA_Y_FIELD.to_string(),
                    ScalarValue::Float64(Some(y)),
                );
            }
        },
        SceneGraphEvent::KeyPress(e) => push_key(values, e.key),
        SceneGraphEvent::KeyRelease(e) => push_key(values, e.key),
        _ => {}
    }
}

fn push_button(values: &mut HashMap<String, ScalarValue>, button: MouseButton) {
    values.insert(
        event::EVENT_BUTTON_FIELD.to_string(),
        ScalarValue::Utf8(Some(button_name(button).to_string())),
    );
}

fn push_key(values: &mut HashMap<String, ScalarValue>, key: Key) {
    values.insert(
        event::EVENT_KEY_FIELD.to_string(),
        ScalarValue::Utf8(Some(format!("{key:?}"))),
    );
}

fn push_modifiers(values: &mut HashMap<String, ScalarValue>, modifiers: ModifiersState) {
    values.insert(
        event::EVENT_SHIFT_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.shift)),
    );
    values.insert(
        event::EVENT_CONTROL_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.control)),
    );
    values.insert(
        event::EVENT_ALT_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.alt)),
    );
    values.insert(
        event::EVENT_META_FIELD.to_string(),
        ScalarValue::Boolean(Some(modifiers.meta)),
    );
}

fn event_modifiers(event: &SceneGraphEvent) -> Option<ModifiersState> {
    match event {
        SceneGraphEvent::MouseDown(e) => Some(e.modifiers),
        SceneGraphEvent::MouseUp(e) => Some(e.modifiers),
        SceneGraphEvent::Click(e) => Some(e.modifiers),
        SceneGraphEvent::DoubleClick(e) => Some(e.modifiers),
        SceneGraphEvent::MouseWheel(e) => Some(e.modifiers),
        SceneGraphEvent::KeyPress(e) => Some(e.modifiers),
        SceneGraphEvent::KeyRelease(e) => Some(e.modifiers),
        SceneGraphEvent::CursorMoved(e) => Some(e.modifiers),
        SceneGraphEvent::MouseEnter(e) => Some(e.modifiers),
        SceneGraphEvent::MouseLeave(e) => Some(e.modifiers),
        _ => None,
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::Back => "back",
        MouseButton::Forward => "forward",
        MouseButton::Other(_) => "other",
    }
}

fn event_type_name(event_type: SceneGraphEventType) -> &'static str {
    match event_type {
        SceneGraphEventType::MouseDown => "mouse_down",
        SceneGraphEventType::MouseUp => "mouse_up",
        SceneGraphEventType::Click => "click",
        SceneGraphEventType::DoubleClick => "double_click",
        SceneGraphEventType::MouseWheel => "mouse_wheel",
        SceneGraphEventType::KeyPress => "key_press",
        SceneGraphEventType::KeyRelease => "key_release",
        SceneGraphEventType::CursorMoved => "cursor_moved",
        SceneGraphEventType::MarkMouseEnter => "mark_mouse_enter",
        SceneGraphEventType::MarkMouseLeave => "mark_mouse_leave",
        SceneGraphEventType::WindowResize => "window_resize",
        SceneGraphEventType::WindowResizeSettled => "window_resize_settled",
        SceneGraphEventType::CanvasResize => "canvas_resize",
        SceneGraphEventType::CanvasResizeSettled => "canvas_resize_settled",
        SceneGraphEventType::WindowMoved => "window_moved",
        SceneGraphEventType::WindowFocused => "window_focused",
        SceneGraphEventType::WindowCloseRequested => "window_close_requested",
        SceneGraphEventType::FileChanged(_) => "file_changed",
    }
}

fn scene_event_type_from_chart(event_type: ChartEventType) -> SceneGraphEventType {
    match event_type {
        ChartEventType::MouseDown => SceneGraphEventType::MouseDown,
        ChartEventType::MouseUp => SceneGraphEventType::MouseUp,
        ChartEventType::Click => SceneGraphEventType::Click,
        ChartEventType::DoubleClick => SceneGraphEventType::DoubleClick,
        ChartEventType::MouseWheel => SceneGraphEventType::MouseWheel,
        ChartEventType::KeyPress => SceneGraphEventType::KeyPress,
        ChartEventType::KeyRelease => SceneGraphEventType::KeyRelease,
        ChartEventType::CursorMoved => SceneGraphEventType::CursorMoved,
        ChartEventType::MarkMouseEnter => SceneGraphEventType::MarkMouseEnter,
        ChartEventType::MarkMouseLeave => SceneGraphEventType::MarkMouseLeave,
        ChartEventType::WindowResize => SceneGraphEventType::WindowResize,
        ChartEventType::WindowResizeSettled => SceneGraphEventType::WindowResizeSettled,
        ChartEventType::CanvasResize => SceneGraphEventType::CanvasResize,
        ChartEventType::CanvasResizeSettled => SceneGraphEventType::CanvasResizeSettled,
        ChartEventType::WindowMoved => SceneGraphEventType::WindowMoved,
        ChartEventType::WindowFocused => SceneGraphEventType::WindowFocused,
        ChartEventType::WindowCloseRequested => SceneGraphEventType::WindowCloseRequested,
    }
}

fn duration_ms(current: Instant, previous: Instant) -> f64 {
    current.duration_since(previous).as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use avenger_chart::layout::LayoutBounds;
    use avenger_chart::prelude::*;
    use avenger_chart::render::{InteractionScopeId, InteractionScopeKind};
    use avenger_eventstream::{
        scene::{SceneCursorMovedEvent, SceneMouseDownEvent},
        window::{CanvasResizeEvent, MouseButton},
    };

    use super::*;

    fn coord_scope(
        id: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        channels: &[&str],
    ) -> EvaluatedInteractionScope {
        EvaluatedInteractionScope {
            id: InteractionScopeId(id),
            kind: InteractionScopeKind::Coordinate,
            bounds: LayoutBounds {
                x,
                y,
                width,
                height,
            },
            plot_area_width: width,
            plot_area_height: height,
            facet_path: Vec::new(),
            coord_node_path: Vec::new(),
            coord_transform: Box::new(Cartesian),
            channels: channels.iter().map(|c| c.to_string()).collect(),
            scales: HashMap::new(),
            sharing_owner_paths: HashMap::new(),
        }
    }

    fn channel_set(channels: &[&str]) -> BTreeSet<String> {
        channels.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn route_returns_none_for_point_outside_scope() {
        let scopes = vec![coord_scope(0, 50.0, 40.0, 300.0, 200.0, &["x", "y"])];
        let channels = channel_set(&["x", "y"]);
        assert!(matches!(
            route_interaction_scope(&scopes, Some([100.0, 100.0]), &channels),
            InteractionRoute::Scope(_)
        ));
        assert!(matches!(
            route_interaction_scope(&scopes, Some([10.0, 10.0]), &channels),
            InteractionRoute::None
        ));
        assert!(matches!(
            route_interaction_scope(&scopes, None, &channels),
            InteractionRoute::None
        ));
    }

    #[test]
    fn route_requires_all_channels_supported() {
        let scopes = vec![coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x"])];
        // The scope only supports x, so a binding needing y must not match it.
        assert!(matches!(
            route_interaction_scope(&scopes, Some([10.0, 10.0]), &channel_set(&["x", "y"])),
            InteractionRoute::None
        ));
    }

    #[test]
    fn route_picks_smallest_area_scope() {
        let scopes = vec![
            coord_scope(0, 0.0, 0.0, 400.0, 400.0, &["x", "y"]),
            coord_scope(1, 50.0, 50.0, 100.0, 100.0, &["x", "y"]),
        ];
        match route_interaction_scope(&scopes, Some([100.0, 100.0]), &channel_set(&["x"])) {
            InteractionRoute::Scope(scope) => assert_eq!(scope.id, InteractionScopeId(1)),
            _ => panic!("expected the smaller nested scope to win"),
        }
    }

    #[test]
    fn route_is_ambiguous_for_equal_area_overlap() {
        let scopes = vec![
            coord_scope(0, 0.0, 0.0, 100.0, 100.0, &["x", "y"]),
            coord_scope(1, 0.0, 0.0, 100.0, 100.0, &["x", "y"]),
        ];
        assert!(matches!(
            route_interaction_scope(&scopes, Some([50.0, 50.0]), &channel_set(&["x"])),
            InteractionRoute::Ambiguous
        ));
    }

    fn x_pan_binding() -> ChartEventBinding {
        let dx = event::event_at_start_coord("x") - event::start_coord("x");
        ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param(
                "x_domain",
                event::interval(
                    event::interval_start(event::start_domain("x")) - dx.clone(),
                    event::interval_end(event::start_domain("x")) - dx,
                ),
            )
            .preview()
            .settle_exact()
    }

    async fn pan_state_and_handler() -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let df = ctx
            .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 10.0)) AS t(x, y)")
            .await
            .expect("data");
        let binding = x_pan_binding();
        let compiled = Plot::<Cartesian>::new()
            .canvas_size(400.0, 300.0)
            .data(df)
            .add_param(x_domain.clone())
            .mark(
                Symbol::new()
                    .x_with(col("x"), move |c| {
                        c.scale_with::<Linear>(move |s| {
                            s.raw_domain(raw.clone()).nice(false).zero(false)
                        })
                    })
                    .y(col("y"))
                    .size(20.0),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile pan plot");
        let policy = compiled.resize_policy();
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        )
        .expect("compile binding runtime");
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handler)
    }

    /// A two-column FacetColumn plot whose leaf x scale reads a Shared raw-domain
    /// param, with the same x_pan_binding. A pan in any cell writes the Shared
    /// (root) domain, so every cell pans together.
    async fn faceted_pan_state_and_handler() -> (ChartAppState, ChartEventBindingHandler) {
        faceted_pan_state_and_handler_with_sharing(Sharing::Shared).await
    }

    async fn faceted_pan_state_and_handler_with_sharing(
        sharing: Sharing,
    ) -> (ChartAppState, ChartEventBindingHandler) {
        let ctx = SessionContext::new();
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        // A `Shared` param drives one root domain for all cells; a `Free` param
        // pans only the cell under the pointer. Match the scale's `share_scale`
        // to the param sharing so validation passes (param must be at least as
        // broad as the scale it drives).
        let share_scale = sharing.to_level() == u8::MAX;
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('A', 0.0, 0.0), ('A', 10.0, 10.0),
                    ('B', 0.0, 1.0), ('B', 10.0, 9.0)
                ) AS t(group_name, x, y)",
            )
            .await
            .expect("data");
        let binding = x_pan_binding();
        let compiled = Plot::<FacetColumn>::new()
            .canvas_size(640.0, 320.0)
            .data(df)
            .add_param_with_sharing(x_domain.clone(), sharing)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), move |c| {
                                let c = c.scale_with::<Linear>(move |s| {
                                    s.raw_domain(raw.clone()).nice(false).zero(false)
                                });
                                if share_scale { c.share_scale() } else { c }
                            })
                            .y(col("y"))
                            .size(20.0),
                    ),
                )
                .column(col("group_name")),
            )
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile faceted pan plot");
        let policy = compiled.resize_policy();
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        )
        .expect("compile binding runtime");
        let handler = ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        };
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        let state = ChartAppState::new(session, policy, crate::ChartAppOptions::default());
        (state, handler)
    }

    fn drag_x_domain_value(value: Option<&ScalarValue>) -> [f32; 2] {
        use avenger_chart_core::ScalarValueHelpers;
        match value {
            Some(scalar) => scalar
                .as_f32x2()
                .expect("x_domain should be a 2-element list"),
            None => panic!("x_domain missing"),
        }
    }

    async fn pan_move(
        state: &mut ChartAppState,
        handler: &ChartEventBindingHandler,
        gesture_instant: Instant,
        start_pos: [f32; 2],
        current_pos: [f32; 2],
    ) -> UpdateStatus {
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: start_pos,
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: gesture_instant,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: current_pos,
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                state,
                &empty_rtree(),
            )
            .await
    }

    #[tokio::test]
    async fn root_pan_updates_x_domain_param() {
        use avenger_app::app::SceneGraphBuilder;

        let (mut state, handler) = pan_state_and_handler().await;
        // First evaluation populates the interaction scope.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            1,
            "root Cartesian plot should export one scope"
        );
        let bounds = scopes[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;

        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(status.rerender, "a drag inside the plot should rerender");

        let params = state.params().await;
        let domain = drag_x_domain_value(params.get("x_domain"));
        // Dragging the pointer to the right pans the view right, so the domain
        // shifts left (toward negative) from the inferred [0, 10].
        assert!(
            domain[0] < 0.0 && domain[1] < 10.0,
            "expected a left-shifted domain, got {domain:?}"
        );
        // The re-evaluation with the new raw domain should succeed.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("preview build after pan");
    }

    #[tokio::test]
    async fn root_pan_outside_plot_is_noop() {
        use avenger_app::app::SceneGraphBuilder;

        let (mut state, handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let bounds = state.interaction_scopes().await[0].bounds;
        let default_domain = state.params().await.get("x_domain").cloned();

        // Start the gesture well outside the plot area: the start point routes to
        // no scope, so the derived start-domain columns are null. This must not
        // corrupt the domain param or crash a subsequent build.
        let outside = [bounds.x - 80.0, bounds.y - 80.0];
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            outside,
            [outside[0] + 40.0, outside[1]],
        )
        .await;
        assert!(!status.rerender, "an outside-start drag should be a no-op");
        assert_eq!(
            state.params().await.get("x_domain"),
            default_domain.as_ref(),
            "x_domain must be unchanged after an outside-start drag"
        );
        // A subsequent build must still succeed (no degenerate raw domain).
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after outside drag");
    }

    #[tokio::test]
    async fn root_pan_preview_moves_match_single_move() {
        use avenger_app::app::SceneGraphBuilder;

        // One continuous gesture: three preview moves with a rebuild between each
        // (as a real preview loop would do). The shared gesture instant keeps the
        // start scope/domain frozen even as the raw-domain param changes.
        let (mut multi_state, multi_handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut multi_state)
            .await
            .expect("multi initial build");
        let bounds = multi_state.interaction_scopes().await[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let gesture = Instant::now();
        for offset in [10.0_f32, 25.0, 50.0] {
            pan_move(
                &mut multi_state,
                &multi_handler,
                gesture,
                [cx, cy],
                [cx + offset, cy],
            )
            .await;
            // Rebuild in Preview mode, updating last_interaction_state with the
            // shifted domain. The frozen start scope must avoid feedback.
            crate::ChartSceneGraphBuilder
                .build(&mut multi_state)
                .await
                .expect("preview build");
        }
        let multi_domain = drag_x_domain_value(multi_state.params().await.get("x_domain"));

        // A fresh single-move gesture straight to the final position.
        let (mut single_state, single_handler) = pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut single_state)
            .await
            .expect("single initial build");
        pan_move(
            &mut single_state,
            &single_handler,
            Instant::now(),
            [cx, cy],
            [cx + 50.0, cy],
        )
        .await;
        let single_domain = drag_x_domain_value(single_state.params().await.get("x_domain"));

        // Because each preview move recomputes from the frozen start domain and
        // start scale, the cumulative gesture (even with rebuilds) matches a
        // single move to the same final position. This proves start framing does
        // not feed back as the raw domain updates during the drag.
        assert!(
            (multi_domain[0] - single_domain[0]).abs() < 1e-3,
            "multi {multi_domain:?} vs single {single_domain:?}"
        );
        assert!((multi_domain[1] - single_domain[1]).abs() < 1e-3);
    }

    #[tokio::test]
    async fn faceted_shared_pan_updates_all_cells_via_root_domain() {
        use avenger_app::app::SceneGraphBuilder;

        let (mut state, handler) = faceted_pan_state_and_handler().await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(
            scopes.len(),
            2,
            "two facet columns should export two coordinate scopes"
        );

        // Pan inside the first cell; the Shared param writes the root domain.
        let bounds = scopes[0].bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(
            status.rerender,
            "a drag inside a facet cell should rerender"
        );

        let params = state.params().await;
        let domain = drag_x_domain_value(params.get("x_domain"));
        assert!(
            domain[0] < 0.0 && domain[1] < 10.0,
            "expected a left-shifted shared domain, got {domain:?}"
        );
        // Re-evaluating applies the shared domain to every cell without error.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after shared facet pan");
    }

    /// First component of a scope's facet path, as a `&str` cell value.
    fn scope_cell(scope: &EvaluatedInteractionScope) -> Option<String> {
        match scope.facet_path.first() {
            Some(ScalarValue::Utf8(Some(value))) => Some(value.clone()),
            _ => None,
        }
    }

    /// Numeric x domain of the scope for `cell`.
    fn scope_x_domain(scopes: &[EvaluatedInteractionScope], cell: &str) -> (f32, f32) {
        scopes
            .iter()
            .find(|scope| scope_cell(scope).as_deref() == Some(cell))
            .unwrap_or_else(|| panic!("no scope for cell {cell}"))
            .scales
            .get("x")
            .expect("scope has x scale")
            .numeric_interval_domain()
            .expect("x domain is numeric")
    }

    #[tokio::test]
    async fn faceted_free_pan_updates_only_active_cell() {
        use avenger_app::app::SceneGraphBuilder;

        let (mut state, handler) = faceted_pan_state_and_handler_with_sharing(Sharing::Free).await;
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("initial build");
        let scopes = state.interaction_scopes().await;
        assert_eq!(scopes.len(), 2, "two facet columns export two scopes");

        // Pan inside cell "A". A Free param routes the write to cell A's owner.
        let cell_a = scopes
            .iter()
            .find(|scope| scope_cell(scope).as_deref() == Some("A"))
            .expect("cell A scope")
            .clone();
        let bounds = cell_a.bounds;
        let cx = bounds.x + bounds.width * 0.5;
        let cy = bounds.y + bounds.height * 0.5;
        let status = pan_move(
            &mut state,
            &handler,
            Instant::now(),
            [cx, cy],
            [cx + 40.0, cy],
        )
        .await;
        assert!(status.rerender, "a drag inside cell A should rerender");

        // The root param is untouched: a Free write targets the cell owner path,
        // not the root, so `params()` still reports the default null-list domain.
        let root_domain = state.params().await.get("x_domain").cloned();
        assert!(
            matches!(root_domain, Some(ScalarValue::List(_))),
            "root x_domain should remain a (default) list for a Free pan, got {root_domain:?}"
        );

        // Re-evaluate and confirm only cell A moved; cell B keeps its inferred
        // domain.
        crate::ChartSceneGraphBuilder
            .build(&mut state)
            .await
            .expect("build after free facet pan");
        let scopes = state.interaction_scopes().await;
        let a = scope_x_domain(&scopes, "A");
        let b = scope_x_domain(&scopes, "B");
        assert!(
            a.0 < 0.0 && a.1 < 10.0,
            "cell A should be panned left, got {a:?}"
        );
        assert!(
            (b.0 - 0.0).abs() < 0.5 && (b.1 - 10.0).abs() < 0.5,
            "cell B should keep its inferred [0, 10] domain, got {b:?}"
        );
    }

    fn empty_rtree() -> SceneGraphRTree {
        use avenger_scenegraph::scene_graph::SceneGraph;
        SceneGraphRTree::from_scene_graph(&SceneGraph {
            marks: Vec::new(),
            width: 1.0,
            height: 1.0,
            origin: [0.0, 0.0],
        })
    }

    async fn bound_state(binding: ChartEventBinding) -> ChartAppState {
        let ctx = SessionContext::new();
        let width = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let compiled = Plot::<Cartesian>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile event binding plot");
        let policy = compiled.resize_policy();
        let session = Arc::new(compiled).instantiate(Arc::new(ctx));
        ChartAppState::new(session, policy, crate::ChartAppOptions::default())
    }

    async fn handler_for_binding(binding: ChartEventBinding) -> ChartEventBindingHandler {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        )
        .expect("compile binding runtime");
        ChartEventBindingHandler {
            runtime: Arc::new(runtime),
            state: Mutex::new(ChartEventBindingState::default()),
        }
    }

    #[tokio::test]
    async fn event_binding_canvas_resize_updates_param() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [800.0, 400.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(800.0)))
        );
        let metrics = state.event_metrics().await;
        assert_eq!(metrics.event_batches_evaluated, 1);
        assert_eq!(metrics.param_patch_events, 1);
        assert_eq!(metrics.params_patched, 1);
    }

    #[tokio::test]
    async fn event_binding_drag_uses_start_event() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param("width", event::start_param("width") + event::dx())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;
        let now = Instant::now();
        let start_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                position: [100.0, 50.0],
                button: MouseButton::Left,
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: now,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: Some(start_event),
            previous_event: None,
        };
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [125.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(status.rerender);
        assert!(!status.rebuild_geometry);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(665.0)))
        );
    }

    #[tokio::test]
    async fn event_binding_previous_event_can_update_param() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(
                "width",
                event::previous_param("width") + (event::x() - event::previous_x()),
            )
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;
        {
            let mut binding_state = handler.state.lock().unwrap();
            binding_state.previous_params = Some(ScopedParamStoreSnapshot::from_root_params(
                IndexMap::from([("width".to_string(), ScalarValue::Float64(Some(640.0)))]),
            ));
        }
        let now = Instant::now();
        let previous_event = EventStreamEventSnapshot {
            event: SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                position: [100.0, 50.0],
                mark_instance: None,
                modifiers: Default::default(),
            }),
            mark_instance: None,
            instant: now,
        };
        let context = EventStreamContext {
            mark_instance: None,
            current_event: None,
            start_event: None,
            previous_event: Some(previous_event),
        };
        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [125.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &context,
                &mut state,
                &empty_rtree(),
            )
            .await;
        assert!(status.rerender);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(665.0)))
        );
    }

    #[tokio::test]
    async fn event_binding_filter_false_skips_patch() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .filter(event::shift().eq(lit(true)))
            .set_param("width", event::x())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [800.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(!status.rerender);
        assert_eq!(
            state.params().await.get("width"),
            Some(&ScalarValue::Float64(Some(640.0)))
        );
        assert_eq!(state.event_metrics().await.filter_failures, 1);
    }

    #[tokio::test]
    async fn event_binding_unchanged_patch_skips_rerender() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CanvasResize(CanvasResizeEvent {
                    size: [640.0, 400.0],
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(!status.rerender);
        assert_eq!(state.accepted_resize_count().await, 0);
        assert_eq!(state.event_metrics().await.unchanged_patch_skips, 1);
    }

    #[tokio::test]
    async fn exact_event_binding_requests_exact_evaluation() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param("width", event::x())
            .exact();
        let mut state = bound_state(binding.clone()).await;
        let handler = handler_for_binding(binding).await;

        let status = handler
            .handle_with_context(
                &SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: [800.0, 50.0],
                    mark_instance: None,
                    modifiers: Default::default(),
                }),
                &EventStreamContext::default(),
                &mut state,
                &empty_rtree(),
            )
            .await;

        assert!(status.rerender);
        assert!(status.rebuild_geometry);
        assert_eq!(
            state.runtime.lock().await.next_evaluation_mode,
            EvaluationMode::Exact
        );
    }

    #[tokio::test]
    async fn low_level_stream_filter_rejects_param_columns() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(event::start_param("width").gt(lit(0.0))),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param("width", event::x())
            .preview();
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");

        let err = match CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        ) {
            Ok(_) => panic!("param columns should be rejected in low-level filters"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("Unknown physical scalar expression column")
        );
    }

    #[tokio::test]
    async fn binding_with_coordinate_helper_adds_derived_schema_column() {
        let x_domain = Param::raw_domain("x_domain");
        let raw = x_domain.expr();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .set_param(
                &x_domain,
                event::interval(
                    event::interval_start(raw.clone()) - event::event_at_start_coord("x"),
                    event::interval_end(raw) - event::event_at_start_coord("x"),
                ),
            )
            .preview();
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(x_domain.clone())
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        )
        .expect("compile binding runtime");

        let has_column = |name: &str| {
            runtime
                .program
                .schema()
                .fields()
                .iter()
                .any(|f| f.name() == name)
        };
        assert!(
            has_column("__event_at_start_coord_x"),
            "expected derived event_at_start coord column in schema"
        );
        assert!(
            runtime
                .interaction_requests
                .event_at_start_coord
                .contains("x")
        );
    }

    #[tokio::test]
    async fn binding_without_coordinate_helpers_adds_no_derived_columns() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", event::canvas_width())
            .preview();
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .add_param(Param::new("width", ScalarValue::Float64(Some(640.0))))
            .event_binding(binding)
            .compile(&ctx)
            .await
            .expect("compile");
        let runtime = CompiledChartEventBinding::compile(
            0,
            compiled.event_bindings().first().unwrap(),
            &ctx,
            compiled.param_specs(),
        )
        .expect("compile binding runtime");

        assert!(runtime.interaction_requests.is_empty());
        let has_derived = runtime.program.schema().fields().iter().any(|f| {
            f.name().starts_with("__event_coord_")
                || f.name().starts_with("__start_coord_")
                || f.name().starts_with("__event_at_start_coord_")
                || f.name().starts_with("__previous_coord_")
                || f.name().starts_with("__event_domain_")
                || f.name().starts_with("__start_domain_")
        });
        assert!(!has_derived, "expected no derived interaction columns");
    }
}
