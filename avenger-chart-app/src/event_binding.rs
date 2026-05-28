use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use avenger_app::error::AvengerAppError;
use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventEvaluationMode, ChartEventStream, ChartEventType},
    plot::CompiledPlot,
    render::EvaluationMode,
    serialization::LogicalExprNodeExt,
};
use avenger_chart_core::{
    CompiledScalarExpressionProgram, PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions,
    PlaceholderColumn, collect_placeholder_ids, one_row_batch_from_scalars, schema_from_fields,
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
        compiled_plot.get_default_params(),
    )
}

pub(crate) fn event_streams_for_bindings(
    bindings: &[ChartEventBinding],
    ctx: &SessionContext,
    default_params: &IndexMap<String, ScalarValue>,
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
            default_params,
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
                    stream_config_for_chart_stream(&between.end, None, false, ctx, default_params)?;
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
}

struct CompiledParamAssignment {
    param_name: String,
}

impl CompiledChartEventBinding {
    fn compile(
        binding_index: usize,
        binding: &ChartEventBinding,
        ctx: &SessionContext,
        default_params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerAppError> {
        binding
            .validate()
            .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
        for assignment in &binding.assignments {
            if !default_params.contains_key(&assignment.param_name) {
                return Err(AvengerAppError::InternalError(format!(
                    "Chart event binding assigns unknown param '{}'",
                    assignment.param_name
                )));
            }
        }

        let schema = event_schema(default_params);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let placeholder_columns = default_params.keys().map(|param| {
            PlaceholderColumn::new(format!("${param}"), event::param_column_name(param))
        });
        let mut specs = Vec::new();
        for (index, filter) in binding.filters.iter().enumerate() {
            let expr = filter
                .to_expr(ctx)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            specs.push(
                PhysicalScalarExpressionSpec::new(format!("filter_{index}"), expr)
                    .with_expected_type(DataType::Boolean),
            );
        }
        let filter_count = specs.len();
        let mut assignments = Vec::new();
        for assignment in &binding.assignments {
            let expr = assignment
                .expr
                .to_expr(ctx)
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            let target_type = default_params
                .get(&assignment.param_name)
                .expect("assignment param validated")
                .data_type();
            specs.push(
                PhysicalScalarExpressionSpec::new(
                    format!("assign_{}", assignment.param_name),
                    expr,
                )
                .with_expected_type(target_type)
                .with_nullable_cast(),
            );
            assignments.push(CompiledParamAssignment {
                param_name: assignment.param_name.clone(),
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
        let event_stream_config = event_stream_config_for_binding(binding, ctx, default_params)?;

        Ok(Self {
            binding_index,
            event_stream_config,
            program,
            filter_count,
            assignments,
            evaluation_mode: binding.evaluation_mode,
        })
    }
}

#[derive(Default)]
struct ChartEventBindingState {
    active_start: Option<Instant>,
    start_params: Option<IndexMap<String, ScalarValue>>,
    previous_params: Option<IndexMap<String, ScalarValue>>,
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
        let current_params = app.session.params().clone();
        let (start_params, previous_params) = {
            let mut binding_state = self
                .state
                .lock()
                .expect("chart event binding lock poisoned");
            update_binding_gesture_state(&mut binding_state, context, &current_params);
            (
                binding_state.start_params.clone(),
                binding_state.previous_params.clone(),
            )
        };

        let batch = match event_record_batch(
            self.runtime.program.schema().clone(),
            event,
            context,
            &current_params,
            start_params.as_ref(),
            previous_params.as_ref(),
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

        let mut patch = IndexMap::new();
        for (assignment, value) in self
            .runtime
            .assignments
            .iter()
            .zip(values[self.runtime.filter_count..].iter())
        {
            if app.session.params().get(&assignment.param_name) != Some(value) {
                patch.insert(assignment.param_name.clone(), value.clone());
            }
        }

        let mut should_rerender = !patch.is_empty();
        if !patch.is_empty() {
            app.event_metrics.param_patch_events += 1;
            app.event_metrics.params_patched += patch.len();
            app.session.apply_param_patch(patch);
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
            binding_state.previous_params = Some(app.session.params().clone());
        }

        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
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

fn update_binding_gesture_state(
    state: &mut ChartEventBindingState,
    context: &EventStreamContext,
    current_params: &IndexMap<String, ScalarValue>,
) {
    if let Some(start) = &context.start_event {
        if state.active_start != Some(start.instant) {
            state.active_start = Some(start.instant);
            state.start_params = Some(current_params.clone());
            state.previous_params = None;
        }
    } else {
        state.active_start = None;
        state.start_params = None;
    }
}

fn filters_pass(values: &[ScalarValue]) -> bool {
    values.iter().all(|value| match value {
        ScalarValue::Boolean(Some(value)) => *value,
        _ => false,
    })
}

fn event_stream_config_for_binding(
    binding: &ChartEventBinding,
    ctx: &SessionContext,
    default_params: &IndexMap<String, ScalarValue>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: vec![SceneGraphEventType::from(binding.event_type)],
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
                default_params,
            )?),
            Box::new(stream_config_for_chart_stream(
                &between.end,
                None,
                false,
                ctx,
                default_params,
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
    default_params: &IndexMap<String, ScalarValue>,
) -> Result<EventStreamConfig, AvengerAppError> {
    let mut config = EventStreamConfig {
        types: stream
            .event_type
            .map(|event_type| vec![SceneGraphEventType::from(event_type)])
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
            default_params,
        )?]);
    }
    Ok(config)
}

fn compile_low_level_stream_filter(
    stream: &ChartEventStream,
    ctx: &SessionContext,
    _default_params: &IndexMap<String, ScalarValue>,
) -> Result<EventStreamFilter, AvengerAppError> {
    let schema = event_schema(&IndexMap::new());
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
        let batch = match event_record_batch(
            program.schema().clone(),
            event,
            context,
            &params,
            None,
            None,
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

fn event_schema(default_params: &IndexMap<String, ScalarValue>) -> Arc<Schema> {
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
    for (name, value) in default_params {
        let data_type = value.data_type();
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
    schema_from_fields(fields)
}

fn event_record_batch(
    schema: Arc<Schema>,
    event: &SceneGraphEvent,
    context: &EventStreamContext,
    current_params: &IndexMap<String, ScalarValue>,
    start_params: Option<&IndexMap<String, ScalarValue>>,
    previous_params: Option<&IndexMap<String, ScalarValue>>,
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
    for (name, value) in current_params {
        values.insert(event::param_column_name(name), value.clone());
    }
    if let Some(start_params) = start_params {
        for (name, value) in start_params {
            values.insert(event::start_param_column_name(name), value.clone());
        }
    }
    if let Some(previous_params) = previous_params {
        for (name, value) in previous_params {
            values.insert(event::previous_param_column_name(name), value.clone());
        }
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
    match ChartEventType::try_from(event_type) {
        Ok(ChartEventType::MouseDown) => "mouse_down",
        Ok(ChartEventType::MouseUp) => "mouse_up",
        Ok(ChartEventType::Click) => "click",
        Ok(ChartEventType::DoubleClick) => "double_click",
        Ok(ChartEventType::MouseWheel) => "mouse_wheel",
        Ok(ChartEventType::KeyPress) => "key_press",
        Ok(ChartEventType::KeyRelease) => "key_release",
        Ok(ChartEventType::CursorMoved) => "cursor_moved",
        Ok(ChartEventType::MarkMouseEnter) => "mark_mouse_enter",
        Ok(ChartEventType::MarkMouseLeave) => "mark_mouse_leave",
        Ok(ChartEventType::WindowResize) => "window_resize",
        Ok(ChartEventType::WindowResizeSettled) => "window_resize_settled",
        Ok(ChartEventType::CanvasResize) => "canvas_resize",
        Ok(ChartEventType::CanvasResizeSettled) => "canvas_resize_settled",
        Ok(ChartEventType::WindowMoved) => "window_moved",
        Ok(ChartEventType::WindowFocused) => "window_focused",
        Ok(ChartEventType::WindowCloseRequested) => "window_close_requested",
        Err(_) => "file_changed",
    }
}

fn duration_ms(current: Instant, previous: Instant) -> f64 {
    current.duration_since(previous).as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use avenger_chart::prelude::*;
    use avenger_eventstream::{
        scene::{SceneCursorMovedEvent, SceneMouseDownEvent},
        window::{CanvasResizeEvent, MouseButton},
    };

    use super::*;

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
            compiled.get_default_params(),
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
            binding_state.previous_params = Some(IndexMap::from([(
                "width".to_string(),
                ScalarValue::Float64(Some(640.0)),
            )]));
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
            compiled.get_default_params(),
        ) {
            Ok(_) => panic!("param columns should be rejected in low-level filters"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("Unknown physical scalar expression column")
        );
    }
}
