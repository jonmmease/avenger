//! Plot builder for creating visualizations

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::{
    arrow::{
        array::{ArrayRef, RecordBatch, UInt64Array},
        compute::can_cast_types,
        datatypes::{DataType, Field, Schema},
    },
    common::{
        ScalarValue,
        tree_node::{Transformed, TreeNode},
    },
    dataframe::DataFrame,
    functions_window::expr_fn::row_number,
    logical_expr::{Expr, LogicalPlan, TableScan},
    prelude::{SessionContext, col, lit},
};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;

use avenger_chart_core::{
    AvengerChartError, Axis, AxisSpec, CanonicalJson, ChannelValue, ChartActionStep,
    ChartEventAction, ChartEventCursorAction, ChartEventParamAction, ChartEventSelectionAction,
    ChartEventStoreAction, ChartParamChangeBinding, ChartTool, ChartWidget, ChildPlotFurnishings,
    CompileContext, CompiledComposedWidget, CompiledDataContext, CompiledIdentityAllocator,
    CompiledMark, CompiledMarkIdentity, CompiledMarkState, CompiledNativeWidgetSpec,
    CompiledParamSpec, CompiledScalarExpressionProgram, CompiledSelectionSpec,
    CompiledSubplotChildPlot, CompiledViewSpec, CompiledWidget, CompiledWidgetAttachment,
    CompiledWidgetItemPlan, CoordinateGuide, CoordinateSystem, CoordinateSystemTransformCore,
    DataContext, DefaultLogicalExprNodeExt, DomainCoordination, DomainCoordinationGroup,
    FormattingContext, IntoPlotMark, Legend, LegendSurfaceKind, Mark, MarkDataMode, MarkState,
    NativeWidget, PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions, PixelFrame,
    PlaceholderColumn, PlotMark, PlotMarkKind, PositionedChartWidget, PositionedNativeWidget,
    RepeatContext, RepeatVariable, ResolvedStateTarget, ScaleInferenceHint, SceneGeometryTarget,
    Selection, SelectionSceneQuery, SelectionUpdate, Store, SubplotChildPlotSpec, Theme,
    TimeContext, WidgetAttachment, WidgetExpansionContext, WidgetItemValidation, WidgetItems,
    WidgetPlacement, compile_selections, resolved_selection_placeholder_id, schema_from_fields,
    selection_target_from_placeholder, store_target_from_placeholder, validate_mark_target_path,
    validate_structural_id,
};
use avenger_chart_marks::Subplot;
use avenger_chart_scales::{PlotScaleSpec as ScaleSpec, serialization::LogicalPlanNodeExt};

use crate::{
    concat::{ConcatOrigin, GridConcat, HConcat, VConcat, WrapConcat},
    event::{ChartEventBinding, ChartEventStream, rewrite_reserved_event_binding_local_datums},
    layout::{LayoutSpec, SizeMode},
    legend::ColorbarOverlay,
    repeat::{RepeatColumns, RepeatGrid, RepeatResolvedChildPlotSpec, RepeatRows, RepeatWrap},
    tools::{ToolCompileContext, discover_tool_scale_targets},
};

use super::{
    compiled::{CompiledColorbarOverlayMarks, CompiledPlot},
    title::{PlotSubtitle, PlotTitle},
};

#[derive(Clone)]
pub struct Plot<C: CoordinateSystem> {
    coord_system: C,

    /// Recursive plot elements stored until compilation.
    marks: Vec<PlotMark<C>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Plot-level scale configurations (set via .scale())
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Plot-level legend configurations (set via .legend())
    pub(crate) legends: IndexMap<String, Legend>,

    /// Guide configuration
    pub(crate) guide_config: Option<C::Guide>,

    /// Plot-level event bindings that patch params in chart apps
    pub(crate) event_bindings: Vec<ChartEventBinding>,

    /// Plot-level reactions that run when registered shared params change.
    pub(crate) param_change_bindings: Vec<ChartParamChangeBinding>,

    /// Authoring-time tools that expand during compilation.
    pub(crate) tools: Vec<Arc<dyn ChartTool<C>>>,

    /// Authoring-time widgets and their host placement.
    pub(crate) widgets: Vec<WidgetAttachment>,
}

#[derive(Default)]
pub(crate) struct RootChartFurnishings {
    pub(crate) theme: Option<Arc<Theme>>,
    pub(crate) time_context: TimeContext,
    pub(crate) formatting_context: FormattingContext,
    pub(crate) layout_spec: LayoutSpec,
    pub(crate) title: Option<PlotTitle>,
    pub(crate) subtitle: Option<PlotSubtitle>,
    pub(crate) param_specs: Vec<CompiledParamSpec>,
    pub(crate) selections: Vec<Selection>,
    pub(crate) stores: Vec<Store>,
}

fn child_layout_spec(furnishings: &ChildPlotFurnishings) -> LayoutSpec {
    LayoutSpec {
        plot_area: match (&furnishings.size.width, &furnishings.size.height) {
            (Some(width), Some(height)) => SizeMode::Fixed {
                width: width.clone().into(),
                height: height.clone().into(),
            },
            (Some(width), None) => SizeMode::Width(width.clone().into()),
            (None, Some(height)) => SizeMode::Height(height.clone().into()),
            (None, None) => SizeMode::Auto,
        },
        ..Default::default()
    }
}

pub(crate) fn compile_widget_items(
    widget_id: &str,
    items: WidgetItems,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<CompiledWidgetItemPlan, AvengerChartError> {
    const ORDER: &str = "__order";
    const INDEX: &str = "__idx";
    const VALUE: &str = "__value";
    const LABEL: &str = "__label";

    let mut source = items;
    let mut value_projection = None;
    let mut label_projection = None;
    let mut identity = None;
    let mut configured_validations = Vec::new();
    loop {
        match source {
            WidgetItems::Configured {
                source: inner,
                value,
                label,
                identity: configured_identity,
                validations,
            } => {
                if (value_projection.is_some() && value.is_some())
                    || (label_projection.is_some() && label.is_some())
                {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' item source has more than one canonical projection"
                    )));
                }
                value_projection = value_projection.or(value);
                label_projection = label_projection.or(label);
                if identity.is_some() && configured_identity.is_some() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' item source has more than one identity derivation"
                    )));
                }
                identity = identity.or(configured_identity);
                configured_validations.extend(validations);
                source = *inner;
            }
            base => {
                source = base;
                break;
            }
        }
    }
    if value_projection.is_some() != label_projection.is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Widget '{widget_id}' item source must project both canonical value and label columns"
        )));
    }

    let (mut data, mut validations) = match source {
        WidgetItems::Static(rows) => {
            let names = rows
                .first()
                .map(|row| row.values.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            if let Some(name) = names
                .iter()
                .find(|name| name.starts_with(avenger_chart_core::WIDGET_RUNTIME_INPUT_PREFIX))
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget '{widget_id}' static items use reserved internal column prefix in '{name}'"
                )));
            }
            if names.iter().any(|name| name == ORDER || name == INDEX) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget '{widget_id}' static items use reserved column '{ORDER}' or '{INDEX}'"
                )));
            }
            for row in &rows {
                if row.values.keys().ne(names.iter()) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' static item rows must have identical ordered fields"
                    )));
                }
            }

            let mut fields = Vec::new();
            let mut arrays: Vec<ArrayRef> = Vec::new();
            for name in &names {
                let array =
                    ScalarValue::iter_to_array(rows.iter().map(|row| row.values[name].clone()))?;
                fields.push(Field::new(name, array.data_type().clone(), true));
                arrays.push(array);
            }
            // Match DataFusion's one-based `row_number()` used by dynamic
            // sources so the shared `__idx = __order - 1` projection is
            // zero-based without underflow for the first static item.
            let order = Arc::new(UInt64Array::from_iter_values(1..=rows.len() as u64)) as ArrayRef;
            fields.push(Field::new(ORDER, order.data_type().clone(), false));
            arrays.push(order);
            let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)?;
            let dataframe = session_context.read_batch(batch)?;
            (
                dataframe,
                vec![WidgetItemValidation::NonNullUnique {
                    columns: vec![ORDER.to_string()],
                    role: "static declaration order".to_string(),
                }],
            )
        }
        WidgetItems::DataFrame {
            mut data,
            order_key,
        } => {
            if order_key.is_empty() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget '{widget_id}' DataFrame items require a nonempty total order key"
                )));
            }
            if let Some(field) = data.schema().fields().iter().find(|field| {
                field
                    .name()
                    .starts_with(avenger_chart_core::WIDGET_RUNTIME_INPUT_PREFIX)
            }) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Widget '{widget_id}' DataFrame items use reserved internal column prefix in '{}'",
                    field.name()
                )));
            }
            for reserved in [ORDER, INDEX] {
                if data.schema().field_with_unqualified_name(reserved).is_ok() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' DataFrame items contain reserved column '{reserved}'"
                    )));
                }
            }
            let mut key_columns = Vec::with_capacity(order_key.len());
            for (index, expr) in order_key.into_iter().enumerate() {
                let name = format!("__widget_order_key_{index}");
                if data.schema().field_with_unqualified_name(&name).is_ok() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Widget '{widget_id}' DataFrame items contain reserved column '{name}'"
                    )));
                }
                data = data.with_column(&name, expr)?;
                key_columns.push(name);
            }
            let mut row_number_expr = row_number();
            let datafusion::logical_expr::Expr::WindowFunction(window) = &mut row_number_expr
            else {
                return Err(AvengerChartError::InternalError(
                    "DataFusion row_number did not produce a window expression".to_string(),
                ));
            };
            window.params.order_by = key_columns
                .iter()
                .map(|name| col(name).sort(true, false))
                .collect();
            data = data.with_column(ORDER, row_number_expr)?;
            (
                data,
                vec![WidgetItemValidation::NonNullUnique {
                    columns: key_columns,
                    role: "DataFrame total order key".to_string(),
                }],
            )
        }
        WidgetItems::Configured { .. } => unreachable!("configured widget items were unwrapped"),
    };

    if let (Some(value), Some(label)) = (value_projection, label_projection) {
        // Evaluate both author expressions against the same input schema. Two
        // sequential `with_column` calls would let the label expression observe
        // a newly replaced `__value` (or vice versa) instead of the source
        // column, violating the canonical projection boundary.
        let mut projection = data
            .schema()
            .columns()
            .into_iter()
            .filter(|column| column.name != VALUE && column.name != LABEL)
            .map(datafusion::logical_expr::Expr::Column)
            .collect::<Vec<_>>();
        projection.push(value.alias(VALUE));
        projection.push(label.alias(LABEL));
        data = data.select(projection)?;
    }
    if let Some(identity) = &identity {
        if identity.output_column == ORDER || identity.output_column == INDEX {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Widget '{widget_id}' item identity cannot replace reserved column '{}'",
                identity.output_column
            )));
        }
        data.schema()
            .field_with_unqualified_name(&identity.source_column)
            .map_err(|_| {
                AvengerChartError::InvalidArgument(format!(
                    "Widget '{widget_id}' item identity source column '{}' is missing",
                    identity.source_column
                ))
            })?;
        // Reserve the final string column in the logical schema so mark data
        // and event-datum validation can see it. The prepared-base seam
        // replaces these sentinels with typed codec output after the relation's
        // one collect and before any consumer observes the batch.
        data = data.with_column(&identity.output_column, lit(String::new()))?;
    }
    data = data.with_column(INDEX, col(ORDER) - lit(1_u64))?;
    validations.extend(configured_validations);

    Ok(CompiledWidgetItemPlan {
        data: CompiledDataContext::new(Some(data), Vec::new(), IndexMap::new()),
        order_column: ORDER.to_string(),
        identity,
        validations,
    })
}

pub(crate) struct CompiledComposedWidgetOutput {
    pub(crate) widget: CompiledComposedWidget,
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,
}

pub(crate) async fn compile_composed_widget(
    widget: &dyn ChartWidget,
    public_widget_path: &str,
    session_context: &SessionContext,
    tool_context: &ToolCompileContext,
    widget_scene_index: usize,
    target_path_prefix: Option<&[usize]>,
    widget_instance_id: avenger_chart_core::WidgetInstanceId,
    _identity_allocator: &mut CompiledIdentityAllocator,
) -> Result<CompiledComposedWidgetOutput, AvengerChartError> {
    let id = widget.id().to_string();
    validate_structural_id("widget", &id)?;
    let expansion = widget.expand(
        WidgetExpansionContext::new(&id).with_resolved_instance(widget_instance_id.clone()),
    )?;
    if expansion.instance_id != widget_instance_id {
        return Err(AvengerChartError::InvalidArgument(format!(
            "widget '{id}' returned behavior for a different widget instance"
        )));
    }
    let items = expansion
        .items
        .clone()
        .map(|items| compile_widget_items(&id, items, session_context))
        .transpose()?;
    let widget_item_dataframe = items
        .as_ref()
        .and_then(|items| items.data.dataframe_with_context(session_context));
    let identity = widget as *const dyn ChartWidget as *const () as usize;
    let mark_runtime_ids = expansion
        .behavior
        .marks
        .iter()
        .map(|mark| mark.runtime_id.clone())
        .collect::<Vec<_>>();
    let target_paths = target_path_prefix.map(|prefix| {
        expansion
            .behavior
            .marks
            .iter()
            .enumerate()
            .filter_map(|(mark_index, resolved_mark)| {
                resolved_mark.mark.state().id.as_deref().map(|part| {
                    let mut path = Vec::with_capacity(prefix.len() + 1);
                    path.extend_from_slice(prefix);
                    path.push(mark_index);
                    (format!("{public_widget_path}.{part}"), vec![path])
                })
            })
            .collect()
    });
    let target_ids = target_path_prefix.map(|_| {
        expansion
            .behavior
            .marks
            .iter()
            .enumerate()
            .filter_map(|(mark_index, resolved_mark)| {
                resolved_mark.mark.state().id.as_deref().map(|part| {
                    (
                        format!("{public_widget_path}.{part}"),
                        vec![mark_runtime_ids[mark_index].clone()],
                    )
                })
            })
            .collect()
    });
    if target_paths.is_some() || public_widget_path != id {
        tool_context.register_widget_expansion_with_public_path(
            &id,
            public_widget_path,
            identity,
            &expansion.behavior,
            target_paths,
            target_ids,
        )?;
    } else {
        tool_context.register_widget_expansion(
            &id,
            identity,
            widget_scene_index,
            &expansion.behavior,
            &mark_runtime_ids,
        )?;
    }

    let mut compiled_marks = Vec::with_capacity(expansion.behavior.marks.len());
    let mut local_scale_specs = HashMap::new();
    let mut suppressed_axes = HashMap::new();
    let mut suppressed_legends = IndexMap::new();
    let mut local_scale_channels = HashMap::new();
    let mut relative_target_paths = std::collections::BTreeMap::new();
    let mut relative_target_ids = std::collections::BTreeMap::new();
    for (mark_index, resolved_mark) in expansion.behavior.marks.iter().enumerate() {
        let mark = &resolved_mark.mark;
        let part = mark.state().id.as_ref().ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Widget '{id}' marks must declare stable part ids"
            ))
        })?;
        validate_structural_id("widget part", part)?;
        if matches!(
            mark.state().view.as_ref().map(|view| &view.spec),
            Some(CompiledViewSpec::Cartesian(_))
        ) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Widget '{id}' part '{part}' uses a Cartesian view; PixelFrame widget parts must use View::pixel_frame()"
            )));
        }
        crate::plot::channel::extract_channel_configs_from_state(
            mark.state(),
            session_context,
            &PixelFrame,
            &mut suppressed_axes,
            &mut suppressed_legends,
            &mut local_scale_specs,
            &mut local_scale_channels,
        )?;
        let public_target_path = format!("{public_widget_path}.{part}");
        if relative_target_paths
            .insert(public_target_path.clone(), vec![vec![mark_index]])
            .is_some()
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Widget '{id}' declares duplicate part '{part}'"
            )));
        }
        let runtime_id = mark_runtime_ids[mark_index].clone();
        relative_target_ids.insert(public_target_path.clone(), vec![runtime_id.clone()]);
        let state = CompiledMarkState::from_mark_state(
            mark.state(),
            widget_item_dataframe
                .clone()
                .or_else(|| mark.state().data.dataframe().cloned()),
        )
        .with_mark_index(mark_index)
        .with_identity(CompiledMarkIdentity {
            runtime_id,
            source_name: Some(part.clone()),
            public_aliases: vec![public_target_path],
            private_ancestry: Vec::new(),
            component: Some(avenger_chart_core::CompiledComponentProvenance {
                component_kind: widget.kind().to_string(),
                component_id: Some(id.clone()),
                part_alias: part.clone(),
            }),
        });
        compiled_marks.push(mark.compile(state, session_context).await?);
    }
    crate::plot::channel::extract_channel_configs_from_compiled_domain_channels(
        &compiled_marks,
        &PixelFrame,
        &mut suppressed_axes,
        &mut suppressed_legends,
        &mut local_scale_specs,
        &mut local_scale_channels,
    )?;
    let presentation = expansion.presentation.compile()?;
    Ok(CompiledComposedWidgetOutput {
        widget: CompiledComposedWidget {
            id,
            kind: widget.kind().to_string(),
            behavior_instance_id: expansion.behavior.instance_id.clone(),
            behavior_exports: expansion.behavior.exports.clone(),
            marks: compiled_marks,
            relative_target_paths,
            relative_target_ids,
            measure: expansion.measure,
            items,
            presentation,
        },
        scale_specs: local_scale_specs,
    })
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<C> SubplotChildPlotSpec for Plot<C>
where
    C: CoordinateSystem + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec> {
        Box::new(self.clone())
    }

    fn has_plot_level_data(&self) -> bool {
        self.data.is_some()
    }

    async fn compile_boxed(
        &self,
        session_context: &datafusion::prelude::SessionContext,
        furnishings: &ChildPlotFurnishings,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let tool_context = ToolCompileContext::from_parent(None);
        Ok(Arc::new(
            self.clone()
                .compile_with_tool_context(
                    session_context,
                    Some(&tool_context),
                    None,
                    furnishings.clone(),
                )
                .await?,
        ))
    }

    async fn compile_boxed_with_context(
        &self,
        session_context: &datafusion::prelude::SessionContext,
        compile_context: Option<CompileContext<'_>>,
        furnishings: &ChildPlotFurnishings,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let tool_context = compile_context.and_then(ToolCompileContext::downcast);
        let furnishings = if let Some(repeat_context) =
            tool_context.and_then(ToolCompileContext::repeat_context)
        {
            furnishings.resolve_repeat(repeat_context)?
        } else {
            furnishings.clone()
        };
        Ok(Arc::new(
            self.clone()
                .compile_with_tool_context(session_context, tool_context, None, furnishings)
                .await?,
        ))
    }
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            marks: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            legends: IndexMap::new(),
            guide_config: None,
            event_bindings: Vec::new(),
            param_change_bindings: Vec::new(),
            tools: Vec::new(),
            widgets: Vec::new(),
        }
    }

    /// Add an erased tool produced by a language or host registry.
    #[doc(hidden)]
    pub fn tool_arc(mut self, tool: Arc<dyn ChartTool<C>>) -> Self {
        self.tools.push(tool);
        self
    }

    /// Add an erased widget attachment produced by a language or host registry.
    #[doc(hidden)]
    pub fn widget_attachment(mut self, widget: WidgetAttachment) -> Self {
        self.widgets.push(widget);
        self
    }
}

fn validate_param_change_binding_registry(
    bindings: &[ChartParamChangeBinding],
    param_specs: &IndexMap<String, CompiledParamSpec>,
    store_specs: &IndexMap<String, avenger_chart_core::CompiledStoreSpec>,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
) -> Result<(), AvengerChartError> {
    let mut param_writers = HashMap::<String, String>::new();
    for binding in bindings {
        binding.validate()?;
        let source = param_specs.get(&binding.source_param_name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Parameter-change binding references unknown source param '{}'",
                binding.source_param_name
            ))
        })?;
        if !source.sharing.is_fully_shared() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Parameter-change binding source '{}' must be shared at root scope",
                binding.source_param_name
            )));
        }
        for step in binding.action.ordered_steps() {
            match step {
                ChartActionStep::SetParam(assignment) => {
                    let target = param_specs.get(&assignment.param_name).ok_or_else(|| {
                        AvengerChartError::InvalidArgument(format!(
                            "Parameter-change binding from '{}' assigns unknown param '{}'",
                            binding.source_param_name, assignment.param_name
                        ))
                    })?;
                    if !target.sharing.is_fully_shared() {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Parameter-change binding target '{}' must be shared at root scope",
                            assignment.param_name
                        )));
                    }
                    if let Some(previous_source) = param_writers.insert(
                        assignment.param_name.clone(),
                        binding.source_param_name.clone(),
                    ) {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Parameter '{}' has multiple reactive writers from '{}' and '{}'",
                            assignment.param_name, previous_source, binding.source_param_name
                        )));
                    }
                }
                ChartActionStep::SetStore(assignment) => {
                    let target = store_specs.get(&assignment.store_name).ok_or_else(|| {
                        AvengerChartError::InvalidArgument(format!(
                            "Parameter-change binding from '{}' updates unknown store '{}'",
                            binding.source_param_name, assignment.store_name
                        ))
                    })?;
                    if !target.sharing.is_fully_shared() {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Parameter-change binding store target '{}' must be shared at root scope",
                            assignment.store_name
                        )));
                    }
                }
                ChartActionStep::SetSelection(assignment) => {
                    if !selection_specs.contains_key(&assignment.selection_id) {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Parameter-change binding from '{}' updates unknown selection '{}'",
                            binding.source_param_name, assignment.selection_id
                        )));
                    }
                }
                ChartActionStep::SetCursor(_) => {
                    unreachable!("parameter-change binding validation rejects cursor actions")
                }
            }
        }
    }
    Ok(())
}

pub(super) fn resolve_event_binding_state_targets(
    binding: ChartEventBinding,
    param_specs: &IndexMap<String, CompiledParamSpec>,
    store_specs: &IndexMap<String, avenger_chart_core::CompiledStoreSpec>,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
) -> Result<ChartEventBinding, AvengerChartError> {
    let actions = binding
        .action
        .ordered_steps()
        .into_iter()
        .map(|step| match step {
            ChartActionStep::SetParam(assignment) => {
                let spec = param_specs.get(&assignment.param_name).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Chart event binding assigns unknown param '{}'",
                        assignment.param_name
                    ))
                })?;
                Ok(ChartEventAction::SetParam(ChartEventParamAction {
                    target: ResolvedStateTarget::new(
                        spec.runtime_id.clone(),
                        assignment.param_name,
                    ),
                    value: assignment.value,
                    scope: assignment.scope,
                    replace_scoped_values: assignment.replace_scoped_values,
                    reject_null: assignment.reject_null,
                }))
            }
            ChartActionStep::SetStore(assignment) => {
                let spec = store_specs.get(&assignment.store_name).ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Chart event binding updates unknown store '{}'",
                        assignment.store_name
                    ))
                })?;
                Ok(ChartEventAction::SetStore(ChartEventStoreAction {
                    target: ResolvedStateTarget::new(
                        spec.runtime_id.clone(),
                        assignment.store_name,
                    ),
                    update: assignment.update,
                    scope: assignment.scope,
                    replace_scoped_values: assignment.replace_scoped_values,
                }))
            }
            ChartActionStep::SetSelection(assignment) => {
                let spec = selection_specs
                    .get(&assignment.selection_id)
                    .ok_or_else(|| {
                        AvengerChartError::InvalidArgument(format!(
                            "Chart event binding updates unknown selection '{}'",
                            assignment.selection_id
                        ))
                    })?;
                Ok(ChartEventAction::SetSelection(Box::new(
                    ChartEventSelectionAction {
                        target: ResolvedStateTarget::new(
                            spec.runtime_id.clone(),
                            assignment.selection_id,
                        ),
                        update: assignment.update,
                        scope: assignment.scope,
                    },
                )))
            }
            ChartActionStep::SetCursor(action) => {
                Ok(ChartEventAction::SetCursor(ChartEventCursorAction {
                    value: action.value,
                }))
            }
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?
        .into_iter()
        .map(|action| {
            action.map_exprs(&mut |expr| {
                resolve_state_relation_placeholders(expr, store_specs, selection_specs)
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    Ok(binding.with_resolved_actions(actions))
}

fn resolve_param_change_binding_state_targets(
    binding: ChartParamChangeBinding,
    param_specs: &IndexMap<String, CompiledParamSpec>,
    store_specs: &IndexMap<String, avenger_chart_core::CompiledStoreSpec>,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
) -> Result<ChartParamChangeBinding, AvengerChartError> {
    let source = param_specs.get(&binding.source_param_name).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "Parameter-change binding references unknown source param '{}'",
            binding.source_param_name
        ))
    })?;
    let source_target =
        ResolvedStateTarget::new(source.runtime_id.clone(), binding.source_param_name.clone());
    let actions = binding
        .action
        .ordered_steps()
        .into_iter()
        .map(|step| match step {
            ChartActionStep::SetParam(assignment) => {
                let spec = param_specs
                    .get(&assignment.param_name)
                    .expect("parameter-change target validated");
                Ok(ChartEventAction::SetParam(ChartEventParamAction {
                    target: ResolvedStateTarget::new(
                        spec.runtime_id.clone(),
                        assignment.param_name,
                    ),
                    value: assignment.value,
                    scope: assignment.scope,
                    replace_scoped_values: assignment.replace_scoped_values,
                    reject_null: assignment.reject_null,
                }))
            }
            ChartActionStep::SetStore(assignment) => {
                let spec = store_specs
                    .get(&assignment.store_name)
                    .expect("parameter-change store target validated");
                Ok(ChartEventAction::SetStore(ChartEventStoreAction {
                    target: ResolvedStateTarget::new(
                        spec.runtime_id.clone(),
                        assignment.store_name,
                    ),
                    update: assignment.update,
                    scope: assignment.scope,
                    replace_scoped_values: assignment.replace_scoped_values,
                }))
            }
            ChartActionStep::SetSelection(assignment) => {
                let spec = selection_specs
                    .get(&assignment.selection_id)
                    .expect("parameter-change selection target validated");
                Ok(ChartEventAction::SetSelection(Box::new(
                    ChartEventSelectionAction {
                        target: ResolvedStateTarget::new(
                            spec.runtime_id.clone(),
                            assignment.selection_id,
                        ),
                        update: assignment.update,
                        scope: assignment.scope,
                    },
                )))
            }
            ChartActionStep::SetCursor(_) => {
                unreachable!("parameter-change binding validation rejects cursor actions")
            }
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?
        .into_iter()
        .map(|action| {
            action.map_exprs(&mut |expr| {
                resolve_state_relation_placeholders(expr, store_specs, selection_specs)
            })
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    Ok(binding.with_resolved_state(source_target, actions))
}

fn resolve_state_relation_placeholders(
    expr: Expr,
    store_specs: &IndexMap<String, avenger_chart_core::CompiledStoreSpec>,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Expr::ScalarSubquery(mut subquery) = candidate {
            subquery.subquery = Arc::new(resolve_store_relation_plan(
                subquery.subquery.as_ref().clone(),
                store_specs,
            )?);
            return Ok(Transformed::yes(Expr::ScalarSubquery(subquery)));
        }
        let mut placeholder = match candidate {
            Expr::Placeholder(placeholder) => placeholder,
            other => return Ok(Transformed::no(other)),
        };
        if let Some(source_name) = store_target_from_placeholder(&placeholder.id) {
            let spec = store_specs.get(source_name).ok_or_else(|| {
                datafusion::error::DataFusionError::Plan(format!(
                    "Expression references unknown store '{source_name}'"
                ))
            })?;
            placeholder.id = format!(
                "{}{}",
                avenger_chart_core::STORE_RELATION_PLACEHOLDER_PREFIX,
                spec.runtime_id.as_opaque_str()
            );
            return Ok(Transformed::yes(Expr::Placeholder(placeholder)));
        }
        if let Some(source_name) = selection_target_from_placeholder(&placeholder.id) {
            let spec = selection_specs.get(source_name).ok_or_else(|| {
                datafusion::error::DataFusionError::Plan(format!(
                    "Expression references unknown selection '{source_name}'"
                ))
            })?;
            placeholder.id = resolved_selection_placeholder_id(&placeholder.id, &spec.runtime_id)
                .expect("decoded selection placeholder must preserve its operation");
            return Ok(Transformed::yes(Expr::Placeholder(placeholder)));
        }
        Ok(Transformed::no(Expr::Placeholder(placeholder)))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

fn resolve_store_relation_plan(
    plan: LogicalPlan,
    store_specs: &IndexMap<String, avenger_chart_core::CompiledStoreSpec>,
) -> Result<LogicalPlan, datafusion::error::DataFusionError> {
    plan.transform_up_with_subqueries(|candidate| {
        let LogicalPlan::TableScan(scan) = candidate else {
            return Ok(Transformed::no(candidate));
        };
        let Some(spec) = store_specs.get(scan.table_name.table()) else {
            return Ok(Transformed::no(LogicalPlan::TableScan(scan)));
        };
        let scan = TableScan::try_new(
            spec.runtime_id.as_opaque_str(),
            scan.source,
            scan.projection,
            scan.filters,
            scan.fetch,
        )?;
        Ok(Transformed::yes(LogicalPlan::TableScan(scan)))
    })
    .map(|transformed| transformed.data)
}

fn validate_param_change_binding_expressions(
    bindings: &[ChartParamChangeBinding],
    param_specs: &IndexMap<String, CompiledParamSpec>,
    session_context: &SessionContext,
) -> Result<(), AvengerChartError> {
    enum ExpectedResult {
        BooleanFilter,
        Assignment {
            target_name: String,
            target_type: DataType,
        },
        Any,
    }

    for binding in bindings {
        let source = param_specs
            .get(&binding.source_param_name)
            .expect("parameter-change source validated");
        let source_type = source.data_type.clone();
        let mut fields = param_specs
            .iter()
            .map(|(name, spec)| {
                Field::new(
                    avenger_chart_core::event::param_column_name(name),
                    spec.data_type.clone(),
                    true,
                )
            })
            .collect::<Vec<_>>();
        fields.push(Field::new(
            avenger_chart_core::param_change::VALUE_FIELD,
            source_type.clone(),
            true,
        ));
        fields.push(Field::new(
            avenger_chart_core::param_change::PREVIOUS_VALUE_FIELD,
            source_type,
            true,
        ));
        let schema = schema_from_fields(fields);
        let allowed_columns = schema
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<HashSet<_>>();
        let placeholder_columns = param_specs
            .keys()
            .map(|name| {
                PlaceholderColumn::new(
                    format!("${name}"),
                    avenger_chart_core::event::param_column_name(name),
                )
            })
            .collect::<Vec<_>>();

        let mut specs = Vec::new();
        let mut expected_results = Vec::new();
        for (index, filter) in binding.filters.iter().enumerate() {
            let expr = filter.to_expr(session_context).map_err(|error| {
                AvengerChartError::InvalidArgument(format!(
                    "Parameter-change binding from '{}' has an invalid filter: {error}",
                    binding.source_param_name
                ))
            })?;
            specs.push(PhysicalScalarExpressionSpec::new(
                format!("filter_{index}"),
                expr,
            ));
            expected_results.push(ExpectedResult::BooleanFilter);
        }
        for assignment in binding
            .action
            .ordered_steps()
            .into_iter()
            .filter_map(|step| {
                if let ChartActionStep::SetParam(assignment) = step {
                    Some(assignment)
                } else {
                    None
                }
            })
        {
            let avenger_chart_core::ChartActionParamValue::Expr { expr } = assignment.value else {
                continue;
            };
            let expr = expr.to_expr(session_context).map_err(|error| {
                AvengerChartError::InvalidArgument(format!(
                    "Parameter-change binding from '{}' has an invalid assignment to '{}': {error}",
                    binding.source_param_name, assignment.param_name
                ))
            })?;
            let target_type = param_specs
                .get(&assignment.param_name)
                .expect("parameter-change target validated")
                .default
                .data_type();
            specs.push(PhysicalScalarExpressionSpec::new(
                format!("assign_{}", assignment.param_name),
                expr,
            ));
            expected_results.push(ExpectedResult::Assignment {
                target_name: assignment.param_name.clone(),
                target_type,
            });
        }

        // Store and selection updates can contain expressions too. They do not
        // have one uniform result type, but compiling them into the same
        // one-row schema rejects event/datum/coordinate columns here.
        let mut side_effect_action = avenger_chart_core::ChartAction::new();
        side_effect_action.steps = binding
            .action
            .ordered_steps()
            .into_iter()
            .filter(|step| !matches!(step, ChartActionStep::SetParam(_)))
            .collect();
        let mut side_effect_index = 0usize;
        side_effect_action.map_exprs(&mut |expr| {
            specs.push(PhysicalScalarExpressionSpec::new(
                format!("side_effect_{side_effect_index}"),
                expr.clone(),
            ));
            expected_results.push(ExpectedResult::Any);
            side_effect_index += 1;
            Ok(expr)
        })?;

        let program = CompiledScalarExpressionProgram::compile(
            session_context,
            schema,
            specs,
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(allowed_columns)
                .with_placeholder_columns(placeholder_columns),
        )
        .map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Parameter-change binding from '{}' failed expression validation: {error}",
                binding.source_param_name
            ))
        })?;
        let result_types = program.expression_data_types().map_err(|error| {
            AvengerChartError::InvalidArgument(format!(
                "Parameter-change binding from '{}' failed expression validation: {error}",
                binding.source_param_name
            ))
        })?;
        for (result_type, expected) in result_types.into_iter().zip(expected_results) {
            match expected {
                ExpectedResult::BooleanFilter if result_type != DataType::Boolean => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Parameter-change binding from '{}' failed expression validation: filter result has type {result_type:?}, expected Boolean",
                        binding.source_param_name
                    )));
                }
                ExpectedResult::Assignment {
                    target_name,
                    target_type,
                } if !param_change_types_compatible(&result_type, &target_type) => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Parameter-change binding from '{}' failed expression validation: assignment to '{}' has incompatible type {result_type:?}, expected {target_type:?}",
                        binding.source_param_name, target_name
                    )));
                }
                ExpectedResult::BooleanFilter
                | ExpectedResult::Assignment { .. }
                | ExpectedResult::Any => {}
            }
        }
    }
    Ok(())
}

fn param_change_types_compatible(source: &DataType, target: &DataType) -> bool {
    if source == target || matches!(source, DataType::Null) {
        return true;
    }
    if source.is_numeric() || target.is_numeric() {
        return source.is_numeric() && target.is_numeric() && can_cast_types(source, target);
    }
    if source.is_temporal() || target.is_temporal() {
        return source.is_temporal() && target.is_temporal() && can_cast_types(source, target);
    }
    if source.is_nested() || target.is_nested() {
        return source.is_nested() && target.is_nested() && can_cast_types(source, target);
    }
    let is_string = |data_type: &DataType| {
        matches!(
            data_type,
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
        )
    };
    let is_binary = |data_type: &DataType| {
        matches!(
            data_type,
            DataType::Binary
                | DataType::LargeBinary
                | DataType::BinaryView
                | DataType::FixedSizeBinary(_)
        )
    };
    (is_string(source) && is_string(target) || is_binary(source) && is_binary(target))
        && can_cast_types(source, target)
}

impl Plot<PixelFrame> {
    pub fn host_widget<W: ChartWidget>(mut self, widget: W) -> Self {
        self.widgets.push(WidgetAttachment {
            source: avenger_chart_core::WidgetSource::composed(widget),
            placement: WidgetPlacement::ExplicitFrame,
        });
        self
    }

    pub fn host_native_widget<N: NativeWidget>(mut self, widget: N) -> Self {
        self.widgets.push(WidgetAttachment {
            source: avenger_chart_core::WidgetSource::native(widget),
            placement: WidgetPlacement::ExplicitFrame,
        });
        self
    }
}

impl<C: CoordinateSystem + Default> Default for Plot<C> {
    fn default() -> Self {
        Self::with_coord(C::default())
    }
}

impl<C: CoordinateSystem + Default> Plot<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    pub(crate) async fn compile(
        self,
        session_context: &datafusion::prelude::SessionContext,
        root_furnishings: RootChartFurnishings,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let root_tool_context = ToolCompileContext::root(
            root_furnishings.theme.clone(),
            root_furnishings.time_context.clone(),
            root_furnishings.formatting_context.clone(),
        );
        Box::pin(self.compile_with_tool_context(
            session_context,
            Some(&root_tool_context),
            Some(root_furnishings),
            ChildPlotFurnishings::default(),
        ))
        .await
    }

    pub(crate) async fn compile_with_tool_context(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        root_furnishings: Option<RootChartFurnishings>,
        child_furnishings: ChildPlotFurnishings,
    ) -> Result<CompiledPlot, AvengerChartError> {
        match try_lower_repeat_plot(self, session_context)? {
            MaybeLoweredRepeatPlot::Lowered(lowered) => {
                return Box::pin(lowered.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await;
            }
            MaybeLoweredRepeatPlot::Original(plot) => {
                return Box::pin(plot.compile_without_repeat_lowering(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await;
            }
        }
    }

    async fn compile_without_repeat_lowering(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        root_furnishings: Option<RootChartFurnishings>,
        child_furnishings: ChildPlotFurnishings,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let is_root = root_furnishings.is_some();
        let (root_param_specs, root_selections, root_stores, layout_spec, title, subtitle) =
            match root_furnishings {
                Some(root) => (
                    root.param_specs,
                    root.selections,
                    root.stores,
                    root.layout_spec,
                    root.title,
                    root.subtitle,
                ),
                None => (
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    child_layout_spec(&child_furnishings),
                    child_furnishings.caption,
                    None,
                ),
            };
        let effective_time_context = inherited_tool_context
            .map(|context| context.time_context())
            .cloned()
            .unwrap_or_default();
        let effective_formatting_context = inherited_tool_context
            .map(|context| context.formatting_context())
            .cloned()
            .unwrap_or_default();
        let tool_context = ToolCompileContext::from_parent(inherited_tool_context);
        // Source names and public aliases are deliberately absent from this
        // seed. The structural coordinate-node path plus stable traversal order
        // supplies deterministic identity across nested plot compilation.
        let mut identity_allocator =
            CompiledIdentityAllocator::new(tool_context.compiled_identity_seed());
        if is_root {
            tool_context.register_root_stores(&root_stores)?;
        }
        let coord_system = if let Some(repeat_context) = tool_context.repeat_context() {
            self.coord_system.resolve_repeat(repeat_context)?
        } else {
            self.coord_system.clone()
        };
        let mut pre_tool_axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut pre_tool_legends: IndexMap<String, Legend> = self.legends.clone();
        let mut pre_tool_scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut pre_tool_scale_to_coord_channel: HashMap<String, String> = HashMap::new();
        let pre_tool_flat_marks = flatten_plot_marks(&self.marks, tool_context.repeat_context())?;
        let mut pre_tool_mark_states =
            resolve_mark_states(&pre_tool_flat_marks.marks, tool_context.repeat_context())?;
        lower_group_views(&pre_tool_flat_marks, &mut pre_tool_mark_states)?;
        let pre_tool_mark_states = pre_tool_mark_states;
        let pre_tool_coord_system = coord_system.resolve_from_mark_states(&pre_tool_mark_states)?;
        pre_tool_coord_system.validate()?;
        let pre_tool_coord_transform = pre_tool_coord_system.create_transform();
        crate::plot::channel::extract_axis_configs_from_coordinate(
            &pre_tool_coord_system,
            &mut pre_tool_axis_specs,
        );
        for mark_state in &pre_tool_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                pre_tool_coord_transform.as_ref(),
                &mut pre_tool_axis_specs,
                &mut pre_tool_legends,
                &mut pre_tool_scale_specs,
                &mut pre_tool_scale_to_coord_channel,
            )?;
        }
        crate::plot::channel::extract_channel_configs_from_mark_domain_channels(
            &pre_tool_flat_marks.marks,
            pre_tool_coord_transform.as_ref(),
            &mut pre_tool_axis_specs,
            &mut pre_tool_legends,
            &mut pre_tool_scale_specs,
            &mut pre_tool_scale_to_coord_channel,
        )?;
        crate::plot::channel::extract_axis_configs_from_coordinate(
            &pre_tool_coord_system,
            &mut pre_tool_axis_specs,
        );
        let pre_tool_scale_coordination = scale_domain_coordinations_from_marks_and_states(
            &pre_tool_mark_states,
            &pre_tool_flat_marks.marks,
            pre_tool_coord_transform.as_ref(),
        )?;
        let tool_scale_targets = discover_tool_scale_targets(
            pre_tool_coord_transform.as_ref(),
            &pre_tool_scale_to_coord_channel,
            &pre_tool_scale_coordination,
        )?;
        let tool_coordinate_metrics = pre_tool_coord_transform
            .domain_provider()
            .map(|provider| {
                provider
                    .domain_descriptors()
                    .into_iter()
                    .flat_map(|descriptor| descriptor.metrics)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active_tool_expansions = tool_context.expand_local_tools(
            &self.tools,
            &tool_scale_targets,
            &tool_coordinate_metrics,
            &mut identity_allocator,
        )?;
        if tool_context.is_multiplied_host() && !self.widgets.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Widgets cannot be attached to a facet- or repeat-multiplied child plot; attach shared chrome to the parent or use a one-shot concat cell"
                    .to_string(),
            ));
        }
        let mut widget_ids = HashSet::new();
        for attachment in &self.widgets {
            let (id, kind) = if let Some(widget) = attachment.source.composed_widget() {
                (widget.id(), widget.kind())
            } else if let Some(widget) = attachment.source.native_widget() {
                (widget.id(), widget.kind())
            } else {
                return Err(AvengerChartError::InternalError(
                    "Widget source has no authoring variant".to_string(),
                ));
            };
            validate_structural_id("widget", id)?;
            validate_structural_id("widget kind", kind)?;
            if !widget_ids.insert(id.to_string()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate widget id '{id}'"
                )));
            }
        }
        let mut compiled_widgets = Vec::with_capacity(self.widgets.len());
        let mut widget_scale_specs = HashMap::new();
        let mut composed_widget_scene_index = 0usize;
        for (declaration_order, attachment) in self.widgets.iter().enumerate() {
            let instance_id = identity_allocator.allocate_widget_instance();
            let compiled_widget = if let Some(widget) = attachment.source.composed_widget() {
                let output = compile_composed_widget(
                    widget,
                    widget.id(),
                    session_context,
                    &tool_context,
                    composed_widget_scene_index,
                    None,
                    instance_id.clone(),
                    &mut identity_allocator,
                )
                .await?;
                widget_scale_specs.insert(output.widget.id.clone(), output.scale_specs);
                composed_widget_scene_index += 1;
                CompiledWidget::Composed(output.widget)
            } else if let Some(widget) = attachment.source.native_widget() {
                let id = widget.id().to_string();
                validate_structural_id("widget", &id)?;
                let state = widget.state().with_instance_identity(&instance_id);
                let identity = widget as *const dyn NativeWidget as *const () as usize;
                tool_context.register_native_widget(&id, identity, &state)?;
                CompiledWidget::Native(CompiledNativeWidgetSpec {
                    id,
                    kind: widget.kind().to_string(),
                    schema_version: widget.schema_version(),
                    payload: CanonicalJson::from_value(widget.payload())?,
                    measure: widget.measure(),
                    state,
                })
            } else {
                return Err(AvengerChartError::InternalError(
                    "Widget source has no authoring variant".to_string(),
                ));
            };
            compiled_widgets.push(CompiledWidgetAttachment {
                instance_id,
                widget: compiled_widget,
                placement: attachment.placement,
                declaration_order: declaration_order as u64,
            });
        }
        if !is_root {
            tool_context.register_local_event_bindings(&self.event_bindings)?;
            tool_context.register_local_param_change_bindings(&self.param_change_bindings)?;
        }
        let erased_tool_context: CompileContext<'_> = &tool_context;

        let mut selection_specs: IndexMap<String, CompiledSelectionSpec> =
            compile_selections(&root_selections)?;
        for spec in selection_specs.values_mut() {
            spec.runtime_id = identity_allocator.allocate_selection();
        }

        // Start with plot-level configurations
        let mut axis_specs: HashMap<String, AxisSpec> = HashMap::new();
        let mut legends: IndexMap<String, Legend> = self.legends.clone();
        let mut scale_specs: HashMap<String, ScaleSpec> = self.scale_specs.clone();
        let mut scale_to_coord_channel: HashMap<String, String> = HashMap::new();

        let user_flat_mark_count = pre_tool_flat_marks.marks.len();
        let tool_mark_metadata = active_tool_expansions
            .iter()
            .flat_map(|active| {
                active
                    .expansion
                    .marks
                    .iter()
                    .enumerate()
                    .map(|(ordinal, mark)| {
                        (
                            mark.runtime_id.clone(),
                            avenger_chart_core::CompiledComponentProvenance {
                                component_kind: active.expansion.component_kind.clone(),
                                component_id: active.expansion.component_id.clone(),
                                part_alias: mark
                                    .part_alias
                                    .clone()
                                    .unwrap_or_else(|| format!("mark-{ordinal}")),
                            },
                        )
                    })
            })
            .collect::<Vec<_>>();
        let mut marks = self.marks;
        for active in &active_tool_expansions {
            marks.extend(
                active
                    .expansion
                    .marks
                    .iter()
                    .map(|mark| PlotMark::from_mark_arc(mark.mark.clone())),
            );
            marks.extend(active.expansion.chrome.iter().cloned());
        }
        let mut flat_marks = flatten_plot_marks(&marks, tool_context.repeat_context())?;
        let mut composed_widget_index = 0usize;
        for attachment in &compiled_widgets {
            let CompiledWidget::Composed(widget) = &attachment.widget else {
                continue;
            };
            for (target, relative_paths) in &widget.relative_target_paths {
                let paths = relative_paths
                    .iter()
                    .map(|path| {
                        let mut rebased = Vec::with_capacity(path.len() + 1);
                        rebased.push(composed_widget_index);
                        rebased.extend(path);
                        rebased
                    })
                    .collect();
                flat_marks
                    .mark_target_registry
                    .insert(target.clone(), paths)?;
                let target_ids = widget.relative_target_ids.get(target).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "composed widget target '{target}' lacks compiled mark identities"
                    ))
                })?;
                flat_marks
                    .mark_target_registry
                    .bind_target_ids(target, target_ids.clone())?;
            }
            composed_widget_index += 1;
        }
        let mut resolved_mark_states =
            resolve_mark_states(&flat_marks.marks, tool_context.repeat_context())?;
        lower_group_views(&flat_marks, &mut resolved_mark_states)?;
        resolve_inline_view_identities(
            &mut flat_marks,
            &mut resolved_mark_states,
            &mut identity_allocator,
        )?;
        let resolved_mark_states = resolved_mark_states;
        let coord_system = coord_system.resolve_from_mark_states(&resolved_mark_states)?;
        coord_system.validate()?;
        let coord_transform = coord_system.create_transform();

        // 1. Extract and merge channel configs from all marks with proper SessionContext
        crate::plot::channel::extract_axis_configs_from_coordinate(&coord_system, &mut axis_specs);
        for mark_state in &resolved_mark_states {
            crate::plot::channel::extract_channel_configs_from_state(
                mark_state,
                session_context,
                coord_transform.as_ref(),
                &mut axis_specs,
                &mut legends,
                &mut scale_specs,
                &mut scale_to_coord_channel,
            )?;
        }
        crate::plot::channel::extract_channel_configs_from_mark_domain_channels(
            &flat_marks.marks,
            coord_transform.as_ref(),
            &mut axis_specs,
            &mut legends,
            &mut scale_specs,
            &mut scale_to_coord_channel,
        )?;
        crate::plot::channel::extract_axis_configs_from_coordinate(&coord_system, &mut axis_specs);

        let scale_coordination = scale_domain_coordinations_from_marks_and_states(
            &resolved_mark_states,
            &flat_marks.marks,
            coord_transform.as_ref(),
        )?;
        tool_context.apply_scale_edits(
            &active_tool_expansions,
            coord_transform.as_ref(),
            &scale_to_coord_channel,
            &mut scale_specs,
            &scale_coordination,
        )?;

        // 2. Compile all marks. Aggregate channels are intentionally prepared at
        // runtime so faceted marks aggregate after mark-level data scope has been
        // resolved.
        let mut compiled_marks: Vec<Arc<dyn CompiledMark>> = Vec::new();
        let mut regular_mark_ids = Vec::with_capacity(flat_marks.marks.len());
        for (mark_index, (m, mark_state)) in flat_marks
            .marks
            .iter()
            .zip(resolved_mark_states.iter())
            .enumerate()
        {
            // Get the DataFrame (or use plot-level data)
            let df_opt = if mark_state.data_mode == MarkDataMode::Unit {
                None
            } else if flat_marks.mark_group_indices[mark_index].is_some() {
                mark_state.data.dataframe().cloned()
            } else {
                mark_state
                    .data
                    .dataframe()
                    .cloned()
                    .or_else(|| self.data.clone())
            };

            let tool_metadata = mark_index
                .checked_sub(user_flat_mark_count)
                .and_then(|index| tool_mark_metadata.get(index));
            let runtime_id = tool_metadata
                .map(|(runtime_id, _)| runtime_id.clone())
                .unwrap_or_else(|| identity_allocator.allocate_mark());
            let compiled_state = CompiledMarkState::from_mark_state(mark_state, df_opt)
                .with_mark_index(mark_index)
                .with_identity(CompiledMarkIdentity {
                    runtime_id: runtime_id.clone(),
                    source_name: mark_state.id.clone(),
                    public_aliases: flat_marks.public_target_aliases[mark_index].clone(),
                    private_ancestry: flat_marks.private_ancestries[mark_index].clone(),
                    component: tool_metadata
                        .map(|(_, component)| component.clone())
                        .or_else(|| flat_marks.component_provenance[mark_index].clone()),
                });
            let compiled_mark = m
                .compile_with_context(compiled_state, session_context, Some(erased_tool_context))
                .await?;
            regular_mark_ids.push(runtime_id);
            compiled_marks.push(compiled_mark);
        }
        flat_marks
            .mark_target_registry
            .bind_regular_mark_ids(&regular_mark_ids)?;
        let mark_groups = compile_mark_group_states(&flat_marks.group_states);
        crate::plot::channel::extract_channel_configs_from_compiled_domain_channels(
            &compiled_marks,
            coord_transform.as_ref(),
            &mut axis_specs,
            &mut legends,
            &mut scale_specs,
            &mut scale_to_coord_channel,
        )?;

        // 3. Build guide renderer - either from config or default
        let mut guide = if let Some(config) = &self.guide_config {
            config.clone()
        } else {
            // Create default guide for the coordinate system
            C::Guide::default()
        };

        // We need to populate axes from axis_specs before building
        // Create axes map for the guide
        let mut guide_axes = HashMap::new();

        // Add user-specified axes from axis_specs
        for (channel, axis_spec) in &axis_specs {
            let AxisSpec::Local(axis_config) = axis_spec;
            // The axis_config is already the correct type for this coordinate system
            // We need to downcast it to the specific axis type for the guide
            // This is safe because the axis type matches the coordinate system
            if let Some(typed_axis) = axis_config
                .as_any()
                .downcast_ref::<<C::Guide as CoordinateGuide>::Axis>()
            {
                guide_axes.insert(channel.clone(), typed_axis.clone());
            }
        }

        // Set the axes on the guide
        guide.set_axes(guide_axes);

        // Pass compiled mark metadata to the guide so it can extract titles
        // without owning the render-capable compiled mark collection.
        guide.set_compiled_marks(&compiled_marks, session_context);

        let compiled_guide = Arc::from(guide.build());

        // 4. Prepare serialized logical plan for plot-level data (for rebuilds)
        let data_plan_node = match &self.data {
            Some(df) => {
                let plan = df.logical_plan().clone();
                Some(LogicalPlanNode::from_logical_plan(&plan).map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to serialize logical plan: {}",
                        e
                    ))
                })?)
            }
            None => None,
        };

        // Build param specs in stable declaration order and reject duplicates
        // across explicit root declarations and tool-generated state.
        let mut param_source_specs = root_param_specs;
        let mut store_source_specs = Vec::new();
        let legend_colorbar_overlays = compile_colorbar_overlays(&legends, session_context).await?;
        for legend in legends.values_mut() {
            legend.colorbar_overlays.clear();
        }
        let legend_event_bindings = legend_event_bindings(&legends, session_context)?;
        if !is_root {
            tool_context.register_local_legend_event_bindings(&legend_event_bindings)?;
        }

        let mut event_bindings = if is_root {
            self.event_bindings
                .iter()
                .cloned()
                .map(ChartEventBinding::with_plot_surface_target)
                .collect()
        } else {
            Vec::new()
        };
        let mut param_change_bindings = if is_root {
            self.param_change_bindings.clone()
        } else {
            Vec::new()
        };
        if is_root {
            event_bindings.extend(legend_event_bindings);
        }
        let mut tool_metadata = Vec::new();
        let mut tool_behaviors = Vec::new();
        if is_root {
            let artifacts = tool_context.finalize_root()?;
            param_source_specs.extend(artifacts.param_specs);
            param_source_specs.extend(artifacts.native_widget_param_specs);
            store_source_specs.extend(artifacts.store_specs);
            for spec in artifacts.selection_specs {
                if selection_specs.contains_key(&spec.id) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Duplicate plot selection '{}'",
                        spec.id
                    )));
                }
                let mut spec = spec;
                if spec.runtime_id.is_unresolved() {
                    spec.runtime_id = identity_allocator.allocate_selection();
                }
                selection_specs.insert(spec.id.clone(), spec);
            }
            event_bindings.extend(artifacts.event_bindings);
            param_change_bindings.extend(artifacts.param_change_bindings);
            tool_metadata.extend(artifacts.metadata);
            tool_behaviors.extend(artifacts.behaviors);
            for (target, paths) in artifacts.child_widget_target_paths {
                let ids = artifacts
                    .child_widget_target_ids
                    .get(&target)
                    .cloned()
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "child widget target '{target}' lacks compiled mark identities"
                        ))
                    })?;
                flat_marks
                    .mark_target_registry
                    .insert(target.clone(), paths)?;
                flat_marks
                    .mark_target_registry
                    .bind_target_ids(&target, ids)?;
            }
        }

        event_bindings = event_bindings
            .into_iter()
            .map(|binding| rewrite_reserved_event_binding_local_datums(binding, session_context))
            .collect::<Result<_, AvengerChartError>>()?;

        for binding in &event_bindings {
            binding.validate()?;
        }
        for binding in &param_change_bindings {
            binding.validate()?;
        }
        event_bindings = event_bindings
            .into_iter()
            .map(|binding| {
                resolve_event_binding_mark_targets(binding, &flat_marks.mark_target_registry)
            })
            .collect::<Result<_, AvengerChartError>>()?;

        let mut param_specs: IndexMap<String, CompiledParamSpec> = IndexMap::new();
        for spec in &param_source_specs {
            if spec
                .name
                .starts_with(avenger_chart_core::WIDGET_RUNTIME_INPUT_PREFIX)
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Plot parameter '{}' uses reserved widget runtime prefix '{}'",
                    spec.name,
                    avenger_chart_core::WIDGET_RUNTIME_INPUT_PREFIX
                )));
            }
            if param_specs.contains_key(&spec.name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate plot parameter '{}'",
                    spec.name
                )));
            }
            let mut spec = spec.clone();
            if spec.runtime_id.is_unresolved() {
                spec.runtime_id = identity_allocator.allocate_param();
            }
            param_specs.insert(spec.name.clone(), spec);
        }

        // Derive the flat default-param map for existing callers from the specs.
        let default_params: IndexMap<String, ScalarValue> = param_specs
            .values()
            .map(|spec| (spec.name.clone(), spec.default.clone()))
            .collect();

        let mut store_specs = IndexMap::new();
        for mut spec in store_source_specs {
            if store_specs.contains_key(&spec.name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate plot store '{}'",
                    spec.name
                )));
            }
            if spec.runtime_id.is_unresolved() {
                spec.runtime_id = identity_allocator.allocate_store();
            }
            store_specs.insert(spec.name.clone(), spec);
        }

        event_bindings = event_bindings
            .into_iter()
            .map(|binding| {
                resolve_event_binding_state_targets(
                    binding,
                    &param_specs,
                    &store_specs,
                    &selection_specs,
                )
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        validate_param_change_binding_registry(
            &param_change_bindings,
            &param_specs,
            &store_specs,
            &selection_specs,
        )?;
        validate_param_change_binding_expressions(
            &param_change_bindings,
            &param_specs,
            session_context,
        )?;
        param_change_bindings = param_change_bindings
            .into_iter()
            .map(|binding| {
                resolve_param_change_binding_state_targets(
                    binding,
                    &param_specs,
                    &store_specs,
                    &selection_specs,
                )
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        let param_specs = avenger_chart_core::CompiledStateRegistry::try_from_specs(
            param_specs.into_values(),
            |spec| &spec.runtime_id,
            |spec| spec.name.as_str(),
        )
        .map_err(|err| AvengerChartError::InvalidArgument(err.to_string()))?;
        let store_specs = avenger_chart_core::CompiledStateRegistry::try_from_specs(
            store_specs.into_values(),
            |spec| &spec.runtime_id,
            |spec| spec.name.as_str(),
        )
        .map_err(|err| AvengerChartError::InvalidArgument(err.to_string()))?;
        let selection_specs = avenger_chart_core::CompiledStateRegistry::try_from_specs(
            selection_specs.into_values(),
            |spec| &spec.runtime_id,
            |spec| spec.id.as_str(),
        )
        .map_err(|err| AvengerChartError::InvalidArgument(err.to_string()))?;
        let mark_runtime_paths = flat_marks
            .mark_target_registry
            .runtime_index(&regular_mark_ids)?;

        // 5. Build CompiledPlot (we do not store a persistent ScaleBuilder; it is
        // rebuilt per evaluation using current params for correctness.)
        let mut compiled = CompiledPlot {
            coord_transform,
            compiled_guide: Some(compiled_guide),
            marks: compiled_marks,
            mark_groups,
            mark_group_index_by_mark: flat_marks.mark_group_indices,
            mark_runtime_paths,
            axis_specs,
            legends,
            legend_colorbar_overlays,
            layout_spec,
            title,
            subtitle,
            theme: tool_context.theme().cloned(),
            time_context: effective_time_context,
            formatting_context: effective_formatting_context,
            scale_to_coord_channel,
            scale_specs,
            widget_scale_specs,
            data: data_plan_node,
            default_params,
            param_specs,
            store_specs,
            event_bindings,
            param_change_bindings,
            event_datum_fields: Vec::new(),
            event_coord_fields: Vec::new(),
            selection_specs,
            tool_metadata,
            tool_behaviors,
            widgets: compiled_widgets,
            baked_tables: Vec::new(),
            bake_report: None,
        };
        compiled.event_datum_fields = compiled.infer_event_datum_fields(session_context).await?;
        compiled.event_coord_fields = compiled.infer_event_coord_fields(session_context).await?;

        // 6. Validate scoped raw-domain params are shared at least as broadly as
        // the scales they drive (catches Free/Level pan misconfigurations early).
        compiled.validate_coordinate_domain_metrics()?;
        compiled.validate_scoped_raw_domain_sharing(session_context)?;
        compiled.validate_transform_output_scale_sharing()?;

        Ok(compiled)
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    /// Apply coordinate-system configuration through the system's own
    /// builder. Coordinate options live on `C`; `Plot` carries no
    /// coordinate-specific methods.
    pub fn configure_coord(mut self, f: impl FnOnce(C) -> C) -> Self {
        self.coord_system = f(self.coord_system);
        self
    }

    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: IntoPlotMark<C>,
    {
        // Just store plot elements - config extraction happens during compile().
        self.marks.extend(mark.into_plot_marks());
        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Add a plot-level event binding that can patch one or more params in chart apps.
    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    /// Add multiple plot-level event bindings.
    pub fn event_bindings(mut self, bindings: impl IntoIterator<Item = ChartEventBinding>) -> Self {
        self.event_bindings.extend(bindings);
        self
    }

    /// Add a reaction to a registered shared parameter change.
    pub fn param_change_binding(mut self, binding: ChartParamChangeBinding) -> Self {
        self.param_change_bindings.push(binding);
        self
    }

    /// Add multiple reactions to registered shared parameter changes.
    pub fn param_change_bindings(
        mut self,
        bindings: impl IntoIterator<Item = ChartParamChangeBinding>,
    ) -> Self {
        self.param_change_bindings.extend(bindings);
        self
    }

    /// Add an authoring-time chart tool.
    pub fn tool<T: ChartTool<C>>(mut self, tool: T) -> Self {
        self.tools.push(Arc::new(tool));
        self
    }

    /// Add multiple authoring-time chart tools of the same concrete type.
    pub fn tools<T: ChartTool<C>>(mut self, tools: impl IntoIterator<Item = T>) -> Self {
        self.tools.extend(
            tools
                .into_iter()
                .map(|tool| Arc::new(tool) as Arc<dyn ChartTool<C>>),
        );
        self
    }

    pub fn widget<W: ChartWidget>(mut self, widget: PositionedChartWidget<W>) -> Self {
        self.widgets.push(WidgetAttachment::composed(widget));
        self
    }

    pub fn native_widget<N: NativeWidget>(mut self, widget: PositionedNativeWidget<N>) -> Self {
        self.widgets.push(WidgetAttachment::native(widget));
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.guide_config = match self.guide_config {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.update(guide);
                Some(updated)
            }
            None => Some(guide),
        };
        self
    }
}

enum MaybeLoweredRepeatPlot<C: CoordinateSystem> {
    Original(Box<Plot<C>>),
    Lowered(Box<LoweredRepeatPlot>),
}

enum LoweredRepeatPlot {
    Columns(Box<Plot<HConcat>>),
    Rows(Box<Plot<VConcat>>),
    Grid(Box<Plot<GridConcat>>),
    Wrap(Box<Plot<WrapConcat>>),
}

impl LoweredRepeatPlot {
    async fn compile_with_tool_context(
        self,
        session_context: &datafusion::prelude::SessionContext,
        inherited_tool_context: Option<&ToolCompileContext>,
        root_furnishings: Option<RootChartFurnishings>,
        child_furnishings: ChildPlotFurnishings,
    ) -> Result<CompiledPlot, AvengerChartError> {
        match self {
            Self::Columns(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await
            }
            Self::Rows(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await
            }
            Self::Grid(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await
            }
            Self::Wrap(plot) => {
                Box::pin(plot.compile_with_tool_context(
                    session_context,
                    inherited_tool_context,
                    root_furnishings,
                    child_furnishings,
                ))
                .await
            }
        }
    }
}

fn try_lower_repeat_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<MaybeLoweredRepeatPlot<C>, AvengerChartError> {
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatColumns>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Columns(Box::new(lower_repeat_columns_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatRows>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Rows(Box::new(lower_repeat_rows_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatGrid>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Grid(Box::new(lower_repeat_grid_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }
    if let Some(repeat) = (&plot.coord_system as &dyn Any)
        .downcast_ref::<RepeatWrap>()
        .cloned()
    {
        return Ok(MaybeLoweredRepeatPlot::Lowered(Box::new(
            LoweredRepeatPlot::Wrap(Box::new(lower_repeat_wrap_plot(
                plot,
                repeat,
                session_context,
            )?)),
        )));
    }

    Ok(MaybeLoweredRepeatPlot::Original(Box::new(plot)))
}

#[allow(clippy::type_complexity)]
struct RepeatPlotParts<C: CoordinateSystem> {
    data: Option<DataFrame>,
    scale_specs: HashMap<String, ScaleSpec>,
    legends: IndexMap<String, Legend>,
    event_bindings: Vec<ChartEventBinding>,
    param_change_bindings: Vec<ChartParamChangeBinding>,
    _phantom: std::marker::PhantomData<fn() -> C>,
}

fn split_repeat_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    kind: &str,
) -> Result<RepeatPlotParts<C>, AvengerChartError> {
    let Plot {
        coord_system: _,
        marks,
        data,
        scale_specs,
        legends,
        guide_config,
        event_bindings,
        param_change_bindings,
        tools,
        widgets,
    } = plot;

    if !marks.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} uses `.cell(...)` for its repeated child plot and does not support root-level marks"
        )));
    }
    if guide_config.is_some() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} does not support root-level guide configuration in this repeat stage"
        )));
    }
    if !tools.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} does not support root-level tools in this repeat stage; attach tools to the repeated cell plot"
        )));
    }
    if !widgets.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} does not support root-level widgets in this repeat stage; attach widgets outside the repeated template"
        )));
    }

    Ok(RepeatPlotParts {
        data,
        scale_specs,
        legends,
        event_bindings,
        param_change_bindings,
        _phantom: std::marker::PhantomData,
    })
}

fn finish_lowered_repeat_plot<C, P>(
    parts: RepeatPlotParts<C>,
    coord_system: P,
    marks: Vec<Arc<dyn Mark<P>>>,
) -> Plot<P>
where
    C: CoordinateSystem,
    P: CoordinateSystem,
{
    Plot {
        coord_system,
        marks: marks.into_iter().map(PlotMark::from_mark_arc).collect(),
        data: parts.data,
        scale_specs: parts.scale_specs,
        legends: parts.legends,
        guide_config: None,
        event_bindings: parts.event_bindings,
        param_change_bindings: parts.param_change_bindings,
        tools: Vec::new(),
        widgets: Vec::new(),
    }
}

#[derive(Clone)]
struct AuthoringMarkGroupState {
    id: Option<String>,
    parent_group_index: Option<usize>,
    scale_inference_hints: Vec<ScaleInferenceHint>,
    data: DataContext,
    data_mode: MarkDataMode,
    facet_data_scope: avenger_chart_core::FacetDataScope,
    view: Option<avenger_chart_core::ViewScopeState>,
}

#[derive(Clone)]
struct AuthoringComponentContext {
    kind: String,
    id: Option<String>,
}

struct FlattenedPlotMarks<C: CoordinateSystem> {
    marks: Vec<Arc<dyn Mark<C>>>,
    group_states: Vec<AuthoringMarkGroupState>,
    mark_group_indices: Vec<Option<usize>>,
    public_target_aliases: Vec<Vec<String>>,
    private_ancestries: Vec<Vec<usize>>,
    component_provenance: Vec<Option<avenger_chart_core::CompiledComponentProvenance>>,
    /// Root marks and children of idless groups share this unqualified id
    /// namespace even when the child id is not exposed as a public target.
    unqualified_mark_ids: HashSet<String>,
    mark_target_registry: MarkTargetRegistry,
}

impl<C: CoordinateSystem> FlattenedPlotMarks<C> {
    fn reserve_unqualified_mark_id(&mut self, id: &str) -> Result<(), AvengerChartError> {
        if self.unqualified_mark_ids.insert(id.to_string()) {
            Ok(())
        } else {
            Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate mark id '{id}' among sibling marks"
            )))
        }
    }
}

#[derive(Clone, Debug, Default)]
struct MarkTargetRegistry {
    paths: HashMap<String, Vec<Vec<usize>>>,
    ids: HashMap<String, Vec<avenger_chart_core::MarkId>>,
}

impl MarkTargetRegistry {
    fn insert(
        &mut self,
        target: String,
        mark_paths: Vec<Vec<usize>>,
    ) -> Result<(), AvengerChartError> {
        if self.paths.contains_key(&target) {
            let message = if target.contains('.') {
                format!("Duplicate mark target path '{target}'")
            } else {
                format!("Duplicate mark id '{target}' among sibling marks")
            };
            return Err(AvengerChartError::InvalidArgument(message));
        }
        self.paths.insert(target, mark_paths);
        Ok(())
    }

    fn bind_target_ids(
        &mut self,
        target: &str,
        mut ids: Vec<avenger_chart_core::MarkId>,
    ) -> Result<(), AvengerChartError> {
        if !self.paths.contains_key(target) {
            return Err(AvengerChartError::InternalError(format!(
                "cannot bind identities for unknown mark target '{target}'"
            )));
        }
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Err(AvengerChartError::InternalError(format!(
                "mark target '{target}' resolved to no compiled identities"
            )));
        }
        self.ids.insert(target.to_string(), ids);
        Ok(())
    }

    fn bind_regular_mark_ids(
        &mut self,
        regular_mark_ids: &[avenger_chart_core::MarkId],
    ) -> Result<(), AvengerChartError> {
        let bindings = self
            .paths
            .iter()
            .filter(|(target, _)| !self.ids.contains_key(*target))
            .filter_map(|(target, paths)| {
                let ids = paths
                    .iter()
                    .map(|path| match path.as_slice() {
                        [mark_index] => regular_mark_ids.get(*mark_index).cloned(),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((target.clone(), ids))
            })
            .collect::<Vec<_>>();
        for (target, ids) in bindings {
            self.bind_target_ids(&target, ids)?;
        }
        Ok(())
    }

    fn resolve_ids(
        &self,
        target: &str,
    ) -> Result<Vec<avenger_chart_core::MarkId>, AvengerChartError> {
        self.ids.get(target).cloned().ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!("Unknown mark target '{target}'"))
        })
    }

    fn runtime_index(
        &self,
        regular_mark_ids: &[avenger_chart_core::MarkId],
    ) -> Result<
        std::collections::BTreeMap<avenger_chart_core::MarkId, Vec<Vec<usize>>>,
        AvengerChartError,
    > {
        let mut index = std::collections::BTreeMap::new();
        for (mark_index, id) in regular_mark_ids.iter().enumerate() {
            index
                .entry(id.clone())
                .or_insert_with(Vec::new)
                .push(vec![mark_index]);
        }
        for (target, ids) in &self.ids {
            let paths = self.paths.get(target).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "compiled mark target '{target}' lacks runtime paths"
                ))
            })?;
            if ids.len() != paths.len() {
                return Err(AvengerChartError::InternalError(format!(
                    "compiled mark target '{target}' has {} identities but {} runtime paths",
                    ids.len(),
                    paths.len()
                )));
            }
            for (id, path) in ids.iter().zip(paths) {
                index
                    .entry(id.clone())
                    .or_insert_with(Vec::new)
                    .push(path.clone());
            }
        }
        for paths in index.values_mut() {
            paths.sort();
            paths.dedup();
        }
        Ok(index)
    }
}

fn flatten_plot_marks<C: CoordinateSystem>(
    elements: &[PlotMark<C>],
    repeat_context: Option<&RepeatContext>,
) -> Result<FlattenedPlotMarks<C>, AvengerChartError> {
    let empty_repeat_context;
    let repeat_context = match repeat_context {
        Some(repeat_context) => repeat_context,
        None => {
            empty_repeat_context = RepeatContext::default();
            &empty_repeat_context
        }
    };
    let mut flat = FlattenedPlotMarks {
        marks: Vec::new(),
        group_states: Vec::new(),
        mark_group_indices: Vec::new(),
        public_target_aliases: Vec::new(),
        private_ancestries: Vec::new(),
        component_provenance: Vec::new(),
        unqualified_mark_ids: HashSet::new(),
        mark_target_registry: MarkTargetRegistry::default(),
    };
    flatten_plot_mark_elements(elements, None, &[], &[], None, repeat_context, &mut flat)?;
    Ok(flat)
}

fn flatten_plot_mark_elements<C: CoordinateSystem>(
    elements: &[PlotMark<C>],
    parent_group_index: Option<usize>,
    public_path_prefix: &[String],
    private_ancestry: &[usize],
    component: Option<&AuthoringComponentContext>,
    repeat_context: &RepeatContext,
    flat: &mut FlattenedPlotMarks<C>,
) -> Result<Vec<usize>, AvengerChartError> {
    let mut descendant_mark_indices = Vec::new();
    for element in elements {
        match element.kind() {
            PlotMarkKind::InvalidArgument(message) => {
                return Err(AvengerChartError::InvalidArgument(message.clone()));
            }
            PlotMarkKind::Primitive(mark) => {
                for alias in &mark.state().public_aliases {
                    validate_mark_target_path("mark alias", alias)?;
                }
                for alias in element.language_public_aliases() {
                    validate_mark_target_path("mark alias", alias)?;
                }
                if let Some(id) = mark.state().id.as_deref() {
                    validate_structural_id("mark", id)?;
                    if public_path_prefix.is_empty() {
                        flat.reserve_unqualified_mark_id(id)?;
                    }
                }
                if parent_group_index.is_some() {
                    let state = mark.state();
                    if state.has_explicit_data_source() {
                        return Err(AvengerChartError::InvalidArgument(
                            "Primitive child marks inside MarkGroup may not set explicit data or data_store"
                                .to_string(),
                        ));
                    }
                    if state.data_mode == MarkDataMode::Unit {
                        return Err(AvengerChartError::InvalidArgument(
                            "Primitive child marks inside MarkGroup may not use unit_data()"
                                .to_string(),
                        ));
                    }
                }
                if component.is_some() && mark.state().id.is_none() {
                    return Err(AvengerChartError::InvalidArgument(
                        "Primitive marks in a component group must declare stable part ids"
                            .to_string(),
                    ));
                }
                let mark_index = flat.marks.len();
                let mut public_target_aliases = mark_public_target_aliases(
                    element
                        .publishes_source_id()
                        .then_some(mark.state().id.as_deref())
                        .flatten(),
                    &mark.state().public_aliases,
                    parent_group_index,
                    public_path_prefix,
                );
                // Compiler-resolved aliases are already exact paths relative
                // to the chart root; native Rust aliases remain relative to
                // the mark's containing public group.
                public_target_aliases.extend(element.language_public_aliases().iter().cloned());
                public_target_aliases.sort();
                public_target_aliases.dedup();
                flat.marks.push(mark.clone());
                flat.mark_group_indices.push(parent_group_index);
                flat.public_target_aliases
                    .push(public_target_aliases.clone());
                flat.private_ancestries.push(private_ancestry.to_vec());
                flat.component_provenance.push(component.map(|component| {
                    avenger_chart_core::CompiledComponentProvenance {
                        component_kind: component.kind.clone(),
                        component_id: component.id.clone(),
                        part_alias: element
                            .component_part_alias()
                            .map(ToString::to_string)
                            .or_else(|| mark.state().id.clone())
                            .expect("component part validated"),
                    }
                }));
                for public_target_alias in public_target_aliases {
                    flat.mark_target_registry
                        .insert(public_target_alias, vec![vec![mark_index]])?;
                }
                descendant_mark_indices.push(mark_index);
            }
            PlotMarkKind::Group(group) => {
                group.validate_id()?;
                if group.children().is_empty() {
                    return Err(AvengerChartError::InvalidArgument(
                        "MarkGroup must contain at least one child mark or group".to_string(),
                    ));
                }
                let group_public_path_prefix = group_public_path_prefix(
                    public_path_prefix,
                    element
                        .publishes_source_id()
                        .then_some(group.id_ref())
                        .flatten(),
                );
                let group_index = flat.group_states.len();
                flat.group_states.push(AuthoringMarkGroupState {
                    id: group.id_ref().map(ToString::to_string),
                    parent_group_index,
                    scale_inference_hints: group.scale_inference_hints().to_vec(),
                    data: group.data_context().resolve_repeat(repeat_context)?,
                    data_mode: group.data_mode(),
                    facet_data_scope: group.facet_data_scope_value(),
                    view: group
                        .view_scope_state()
                        .map(|view| view.resolve_repeat(repeat_context))
                        .transpose()?,
                });
                let mut child_private_ancestry = private_ancestry.to_vec();
                child_private_ancestry.push(group_index);
                let nested_component =
                    group
                        .component_kind_ref()
                        .map(|kind| AuthoringComponentContext {
                            kind: kind.to_string(),
                            id: group.id_ref().map(ToString::to_string),
                        });
                let group_descendants = flatten_plot_mark_elements(
                    group.children(),
                    Some(group_index),
                    &group_public_path_prefix,
                    &child_private_ancestry,
                    nested_component.as_ref().or(component),
                    repeat_context,
                    flat,
                )?;
                if element.publishes_source_id()
                    && group.id_ref().is_some()
                    && !group_public_path_prefix.is_empty()
                {
                    let group_target = group_public_path_prefix.join(".");
                    let paths = group_descendants
                        .iter()
                        .map(|mark_index| vec![*mark_index])
                        .collect();
                    flat.mark_target_registry.insert(group_target, paths)?;
                }
                descendant_mark_indices.extend(group_descendants);
            }
        }
    }
    Ok(descendant_mark_indices)
}

fn group_public_path_prefix(parent: &[String], group_id: Option<&str>) -> Vec<String> {
    let mut prefix = parent.to_vec();
    if let Some(group_id) = group_id {
        prefix.push(group_id.to_string());
    }
    prefix
}

fn mark_public_target_aliases(
    mark_id: Option<&str>,
    explicit_aliases: &[String],
    parent_group_index: Option<usize>,
    public_path_prefix: &[String],
) -> Vec<String> {
    let mut aliases = Vec::new();
    if let Some(mark_id) = mark_id {
        if public_path_prefix.is_empty() {
            if parent_group_index.is_none() {
                aliases.push(mark_id.to_string());
            }
        } else {
            let mut segments = public_path_prefix.to_vec();
            segments.push(mark_id.to_string());
            aliases.push(segments.join("."));
        }
    }
    for explicit in explicit_aliases {
        let mut segments = public_path_prefix.to_vec();
        segments.extend(explicit.split('.').map(ToString::to_string));
        aliases.push(segments.join("."));
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn resolve_event_binding_mark_targets(
    mut binding: ChartEventBinding,
    registry: &MarkTargetRegistry,
) -> Result<ChartEventBinding, AvengerChartError> {
    let ids = resolve_mark_target_ids(binding.mark_ids(), registry)?;
    if !ids.is_empty() {
        binding = binding.with_resolved_mark_ids(ids);
    }
    if let Some(mut between) = binding.between.take() {
        between.start = resolve_event_stream_mark_targets(between.start, registry)?;
        between.end = resolve_event_stream_mark_targets(between.end, registry)?;
        binding.between = Some(between);
    }
    binding.action = binding.action.try_map_selection_updates(|update| {
        resolve_selection_update_scene_query_mark_targets(update, registry)
    })?;
    Ok(binding)
}

fn resolve_event_stream_mark_targets(
    stream: ChartEventStream,
    registry: &MarkTargetRegistry,
) -> Result<ChartEventStream, AvengerChartError> {
    let ids = resolve_mark_target_ids(stream.mark_ids(), registry)?;
    Ok(if ids.is_empty() {
        stream
    } else {
        stream.with_resolved_mark_ids(ids)
    })
}

fn resolve_selection_update_scene_query_mark_targets(
    update: SelectionUpdate,
    registry: &MarkTargetRegistry,
) -> Result<SelectionUpdate, AvengerChartError> {
    Ok(match update {
        SelectionUpdate::ReplaceAllFromSceneQuery { query } => {
            SelectionUpdate::ReplaceAllFromSceneQuery {
                query: resolve_selection_scene_query_mark_targets(query, registry)?,
            }
        }
        SelectionUpdate::ReplaceFromSceneQueryInScope { query } => {
            SelectionUpdate::ReplaceFromSceneQueryInScope {
                query: resolve_selection_scene_query_mark_targets(query, registry)?,
            }
        }
        SelectionUpdate::UpsertFromSceneQuery { query } => SelectionUpdate::UpsertFromSceneQuery {
            query: resolve_selection_scene_query_mark_targets(query, registry)?,
        },
        SelectionUpdate::ToggleFromSceneQuery { query } => SelectionUpdate::ToggleFromSceneQuery {
            query: resolve_selection_scene_query_mark_targets(query, registry)?,
        },
        other => other,
    })
}

fn resolve_selection_scene_query_mark_targets(
    mut query: SelectionSceneQuery,
    registry: &MarkTargetRegistry,
) -> Result<SelectionSceneQuery, AvengerChartError> {
    query.query.target = resolve_scene_geometry_target_mark_targets(query.query.target, registry)?;
    Ok(query)
}

fn resolve_scene_geometry_target_mark_targets(
    target: SceneGeometryTarget,
    registry: &MarkTargetRegistry,
) -> Result<SceneGeometryTarget, AvengerChartError> {
    let ids = resolve_mark_target_ids(target.mark_ids(), registry)?;
    Ok(if ids.is_empty() {
        target
    } else {
        target.with_resolved_mark_ids(ids)
    })
}

fn resolve_mark_target_ids(
    targets: &[String],
    registry: &MarkTargetRegistry,
) -> Result<Vec<avenger_chart_core::MarkId>, AvengerChartError> {
    let mut ids = Vec::new();
    for target in targets {
        ids.extend(registry.resolve_ids(target)?);
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Lower group view scopes onto child marks.
///
/// A mark inside a viewed group becomes a view-scoped mark: its ordinary
/// data context (transforms and channels authored on the child) moves into a
/// per-mark view scope sharing the group's compiled view spec. Every
/// existing view consumer (domain-inference gating, view param resolution,
/// tool target discovery, preview rebuild forcing) then applies to group
/// children without further threading. The group's own view-local chain is
/// retained on the group state and executed once per evaluation by the mark
/// data runtime, which feeds its output to the children as their view-chain
/// input.
fn lower_group_views<C: CoordinateSystem>(
    flat: &FlattenedPlotMarks<C>,
    mark_states: &mut [MarkState],
) -> Result<(), AvengerChartError> {
    // Validate that view source names are unique across the plot (group scopes plus
    // mark-level scopes), and that viewed groups do not nest.
    let mut seen_view_names: HashSet<String> = HashSet::new();
    let mut register_view_name = |name: &str| -> Result<(), AvengerChartError> {
        if !seen_view_names.insert(name.to_string()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate inline view name '{name}': view scopes must be unique within a plot"
            )));
        }
        Ok(())
    };
    for group in &flat.group_states {
        let Some(view) = group.view.as_ref() else {
            continue;
        };
        register_view_name(view.spec.source_name())?;
        let mut ancestor = group.parent_group_index;
        while let Some(index) = ancestor {
            let parent = &flat.group_states[index];
            if let Some(parent_view) = parent.view.as_ref() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Group view scope '{}' is nested inside group view scope '{}'; nested view scopes are not supported",
                    view.spec.source_name(),
                    parent_view.spec.source_name()
                )));
            }
            ancestor = parent.parent_group_index;
        }
    }
    for state in mark_states.iter() {
        if let Some(view) = state.view.as_ref() {
            register_view_name(view.spec.source_name())?;
        }
    }

    for (mark_index, state) in mark_states.iter_mut().enumerate() {
        let mut ancestor = flat.mark_group_indices[mark_index];
        let mut group_view = None;
        while let Some(index) = ancestor {
            let group = &flat.group_states[index];
            if let Some(view) = group.view.as_ref() {
                group_view = Some(view);
                break;
            }
            ancestor = group.parent_group_index;
        }
        let Some(group_view) = group_view else {
            continue;
        };
        if let Some(mark_view) = state.view.as_ref() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Mark view scope '{}' is nested inside group view scope '{}'; nested view scopes are not supported",
                mark_view.spec.source_name(),
                group_view.spec.source_name()
            )));
        }
        let view_data = std::mem::take(&mut state.data);
        state.view = Some(avenger_chart_core::ViewScopeState::new(
            group_view.spec.clone(),
            view_data,
        ));
    }

    for group in &flat.group_states {
        avenger_chart_core::validate_inline_view_references(&group.data, None, "group base data")?;
        if let Some(view) = &group.view {
            avenger_chart_core::validate_inline_view_references(
                &view.data,
                Some(view.spec.source_name()),
                "group view-local data",
            )?;
        }
    }
    for state in mark_states {
        avenger_chart_core::validate_inline_view_references(&state.data, None, "mark base data")?;
        if let Some(view) = &state.view {
            avenger_chart_core::validate_inline_view_references(
                &view.data,
                Some(view.spec.source_name()),
                "mark view-local data",
            )?;
        }
    }
    Ok(())
}

/// Assign one opaque identity to every unique inline view declaration after
/// group views have been lowered onto their descendant mark states.
fn resolve_inline_view_identities<C: CoordinateSystem>(
    flat: &mut FlattenedPlotMarks<C>,
    mark_states: &mut [MarkState],
    allocator: &mut CompiledIdentityAllocator,
) -> Result<(), AvengerChartError> {
    let mut ids_by_source_name = HashMap::new();
    for group in &mut flat.group_states {
        let Some(view) = group.view.as_mut() else {
            continue;
        };
        let source_name = view.spec.source_name().to_string();
        let runtime_id = ids_by_source_name
            .entry(source_name)
            .or_insert_with(|| allocator.allocate_view())
            .clone();
        view.spec.set_runtime_id(runtime_id);
    }
    for state in mark_states {
        let Some(view) = state.view.as_mut() else {
            continue;
        };
        let source_name = view.spec.source_name().to_string();
        let runtime_id = ids_by_source_name
            .entry(source_name)
            .or_insert_with(|| allocator.allocate_view())
            .clone();
        view.spec.set_runtime_id(runtime_id);
        if view.spec.runtime_id().is_unresolved() {
            return Err(AvengerChartError::InternalError(
                "inline view identity remained unresolved after plot compilation".to_string(),
            ));
        }
    }
    Ok(())
}

fn compile_mark_group_states(
    groups: &[AuthoringMarkGroupState],
) -> Vec<super::compiled::CompiledMarkGroupState> {
    groups
        .iter()
        .map(|group| {
            let data = if let Some(store_data) = group.data.store_data_ref() {
                CompiledDataContext::new_store_data(
                    store_data.clone(),
                    group.data.transforms().to_vec(),
                    group.data.channels().clone(),
                )
            } else {
                CompiledDataContext::new(
                    group.data.dataframe().cloned(),
                    group.data.transforms().to_vec(),
                    group.data.channels().clone(),
                )
            };
            super::compiled::CompiledMarkGroupState {
                id: group.id.clone(),
                parent_group_index: group.parent_group_index,
                scale_inference_hints: group.scale_inference_hints.clone(),
                data,
                data_mode: group.data_mode,
                facet_data_scope: group.facet_data_scope,
                view: group
                    .view
                    .as_ref()
                    .map(avenger_chart_core::CompiledViewScope::from_view_scope_state),
            }
        })
        .collect()
}

fn lower_repeat_columns_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatColumns,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<HConcat>, AvengerChartError> {
    let variables = resolve_repeat_variables(
        "RepeatColumns",
        "columns",
        repeat.columns_config(),
        session_context,
    )?;
    let column_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(column_index, column)| {
            let key = format!("repeat_col:{}", column.id);
            let id = format!("repeat_col_{}", column.id);
            let label = column.title.clone();
            let repeat_context = RepeatContext::new()
                .with_column(column, column_index, column_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let Some(cell) = repeat.cell_templates().select(
                "RepeatColumns",
                &key,
                &repeat_context,
                session_context,
            )?
            else {
                return Ok(None);
            };
            Ok(Some(Arc::new(
                Subplot::<HConcat>::new(RepeatResolvedChildPlotSpec::new(
                    cell.plot,
                    repeat_context,
                ))
                .with_furnishings(cell.furnishings)
                .name(key)
                .id(id)
                .label(label),
            ) as Arc<dyn Mark<HConcat>>))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?
        .into_iter()
        .flatten()
        .collect();
    let parts = split_repeat_plot(plot, "RepeatColumns")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        HConcat::new().with_origin(ConcatOrigin::RepeatColumns),
        marks,
    ))
}

fn lower_repeat_rows_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatRows,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<VConcat>, AvengerChartError> {
    let variables =
        resolve_repeat_variables("RepeatRows", "rows", repeat.rows_config(), session_context)?;
    let row_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let key = format!("repeat_row:{}", row.id);
            let id = format!("repeat_row_{}", row.id);
            let label = row.title.clone();
            let repeat_context = RepeatContext::new()
                .with_row(row, row_index, row_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let Some(cell) = repeat.cell_templates().select(
                "RepeatRows",
                &key,
                &repeat_context,
                session_context,
            )?
            else {
                return Ok(None);
            };
            Ok(Some(Arc::new(
                Subplot::<VConcat>::new(RepeatResolvedChildPlotSpec::new(
                    cell.plot,
                    repeat_context,
                ))
                .with_furnishings(cell.furnishings)
                .name(key)
                .id(id)
                .label(label),
            ) as Arc<dyn Mark<VConcat>>))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?
        .into_iter()
        .flatten()
        .collect();
    let parts = split_repeat_plot(plot, "RepeatRows")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        VConcat::new().with_origin(ConcatOrigin::RepeatRows),
        marks,
    ))
}

fn lower_repeat_grid_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatGrid,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<GridConcat>, AvengerChartError> {
    let rows =
        resolve_repeat_variables("RepeatGrid", "rows", repeat.rows_config(), session_context)?;
    let columns = resolve_repeat_variables(
        "RepeatGrid",
        "columns",
        repeat.columns_config(),
        session_context,
    )?;
    let row_count = rows.len();
    let column_count = columns.len();
    let mut marks = Vec::new();
    for (row_index, row) in rows.iter().cloned().enumerate() {
        for (column_index, column) in columns.iter().cloned().enumerate() {
            let key = format!("repeat_cell:{}:{}", row.id, column.id);
            let id = format!("repeat_cell_{}_{}", row.id, column.id);
            let repeat_context = RepeatContext::new()
                .with_row(row.clone(), row_index, row_count)
                .with_column(column.clone(), column_index, column_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone())
                .with_matrix_axis_defaults(repeat.matrix_axis_defaults());
            let Some(cell) = repeat.cell_templates().select(
                "RepeatGrid",
                &key,
                &repeat_context,
                session_context,
            )?
            else {
                continue;
            };
            marks.push(Arc::new(
                Subplot::<GridConcat>::new(RepeatResolvedChildPlotSpec::new(
                    cell.plot,
                    repeat_context,
                ))
                .with_furnishings(cell.furnishings)
                .name(key)
                .id(id)
                .at(row_index, column_index),
            ) as Arc<dyn Mark<GridConcat>>);
        }
    }
    let parts = split_repeat_plot(plot, "RepeatGrid")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        GridConcat::new()
            .rows(row_count)
            .columns(column_count)
            .with_axis_guide_visibility_config(repeat.axis_guide_visibility_config())
            .with_origin(ConcatOrigin::RepeatGrid),
        marks,
    ))
}

fn lower_repeat_wrap_plot<C: CoordinateSystem>(
    plot: Plot<C>,
    repeat: RepeatWrap,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Plot<WrapConcat>, AvengerChartError> {
    let variables = resolve_repeat_variables(
        "RepeatWrap",
        "items",
        repeat.items_config(),
        session_context,
    )?;
    let item_count = variables.len();
    let marks = variables
        .into_iter()
        .enumerate()
        .map(|(item_index, item)| {
            let key = format!("repeat_item:{}", item.id);
            let id = format!("repeat_item_{}", item.id);
            let label = item.title.clone();
            let repeat_context = RepeatContext::new()
                .with_item(item, item_index, item_count)
                .with_domain_coordination(repeat.domain_coordination_config().clone());
            let Some(cell) = repeat.cell_templates().select(
                "RepeatWrap",
                &key,
                &repeat_context,
                session_context,
            )?
            else {
                return Ok(None);
            };
            Ok(Some(Arc::new(
                Subplot::<WrapConcat>::new(RepeatResolvedChildPlotSpec::new(
                    cell.plot,
                    repeat_context,
                ))
                .with_furnishings(cell.furnishings)
                .name(key)
                .id(id)
                .label(label),
            ) as Arc<dyn Mark<WrapConcat>>))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?
        .into_iter()
        .flatten()
        .collect();
    let parts = split_repeat_plot(plot, "RepeatWrap")?;
    Ok(finish_lowered_repeat_plot(
        parts,
        WrapConcat::new()
            .with_column_mode(repeat.column_mode_config().clone())
            .with_origin(ConcatOrigin::RepeatWrap),
        marks,
    ))
}

fn resolve_repeat_variables(
    kind: &str,
    axis: &str,
    variables: &[RepeatVariable],
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<avenger_chart_core::ResolvedRepeatVariable>, AvengerChartError> {
    if variables.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{kind} requires at least one repeat {axis} variable"
        )));
    }
    let mut ids = HashSet::new();
    let mut resolved = Vec::with_capacity(variables.len());
    for variable in variables {
        let variable = variable.resolve(session_context)?;
        if !ids.insert(variable.id.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{kind} has duplicate repeat {axis} variable id '{}'",
                variable.id
            )));
        }
        resolved.push(variable);
    }
    Ok(resolved)
}

fn scale_domain_coordinations_from_marks_and_states<C: CoordinateSystem>(
    states: &[MarkState],
    marks: &[Arc<dyn Mark<C>>],
    coord_transform: &dyn CoordinateSystemTransformCore,
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let state_refs = states.iter().collect::<Vec<_>>();
    scale_domain_coordinations_from_state_refs_and_marks(&state_refs, marks, coord_transform)
}

fn merge_scale_domain_coordination(
    result: &mut HashMap<String, DomainCoordination>,
    coord_transform: &dyn CoordinateSystemTransformCore,
    channel_name: &str,
    channel_value: &ChannelValue,
) -> Result<(), AvengerChartError> {
    if !coord_transform.channel_uses_scale(channel_name) {
        return Ok(());
    }
    let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
        return Ok(());
    };
    let Some(coordination) = channel_value.get_domain_coordination() else {
        return Ok(());
    };
    let coordination = coordination.clone();
    match result.get_mut(&scale_name) {
        Some(existing) => {
            if existing.group != coordination.group {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "scale '{scale_name}' has incompatible domain groups"
                )));
            }
            if coordination.scope.to_level() > existing.scope.to_level() {
                existing.scope = coordination.scope;
            }
        }
        None => {
            result.insert(scale_name, coordination);
        }
    }
    Ok(())
}

fn scale_domain_coordinations_from_state_refs_and_marks<C: CoordinateSystem>(
    states: &[&MarkState],
    marks: &[Arc<dyn Mark<C>>],
    coord_transform: &dyn CoordinateSystemTransformCore,
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let mut result: HashMap<String, DomainCoordination> = HashMap::new();

    for state in states {
        for (channel_name, channel_value) in state.data.channels() {
            merge_scale_domain_coordination(
                &mut result,
                coord_transform,
                channel_name,
                channel_value,
            )?;
        }
        if let Some(view) = state.view.as_ref() {
            for (channel_name, channel_value) in view.data.channels() {
                merge_scale_domain_coordination(
                    &mut result,
                    coord_transform,
                    channel_name,
                    channel_value,
                )?;
            }
        }
    }

    for mark in marks {
        for source in mark.scale_domain_channels()? {
            merge_scale_domain_coordination(
                &mut result,
                coord_transform,
                &source.channel,
                &source.channel_value,
            )?;
        }
    }
    Ok(result)
}

fn resolve_mark_states<C: CoordinateSystem>(
    marks: &[Arc<dyn Mark<C>>],
    repeat_context: Option<&RepeatContext>,
) -> Result<Vec<MarkState>, AvengerChartError> {
    let empty_repeat_context;
    let repeat_context = match repeat_context {
        Some(repeat_context) => repeat_context,
        None => {
            empty_repeat_context = RepeatContext::default();
            &empty_repeat_context
        }
    };
    marks
        .iter()
        .map(|mark| {
            let mut state = mark.state().resolve_repeat(repeat_context)?;
            apply_repeat_matrix_axis_defaults::<C>(&mut state, repeat_context)?;
            Ok(state)
        })
        .collect()
}

fn apply_repeat_matrix_axis_defaults<C: CoordinateSystem>(
    state: &mut MarkState,
    repeat_context: &RepeatContext,
) -> Result<(), AvengerChartError>
where
    <C::Guide as CoordinateGuide>::Axis: 'static,
{
    if !repeat_context.matrix_axis_defaults {
        return Ok(());
    }

    let updates = state
        .data
        .channels()
        .iter()
        .filter_map(|(channel, value)| {
            if value.get_axis_config().is_some() || state.axis_configs.contains_key(channel) {
                return None;
            }
            repeat_matrix_axis_title(channel, value, repeat_context)
                .map(|title| (channel.clone(), value.clone(), title))
        })
        .collect::<Vec<_>>();

    for (channel, value, title) in updates {
        let mut axis = <C::Guide as CoordinateGuide>::Axis::default();
        if axis.set_default_title_expr(lit(title))? {
            state.data = state
                .data
                .clone()
                .with_channel_value(&channel, value.with_boxed_axis_config(Box::new(axis)));
        }
    }

    Ok(())
}

fn repeat_matrix_axis_title(
    channel: &str,
    value: &avenger_chart_core::ChannelValue,
    repeat_context: &RepeatContext,
) -> Option<String> {
    let variable = match channel {
        "x" => repeat_context.column.as_ref()?,
        "y" => repeat_context.row.as_ref()?,
        _ => return None,
    };
    let coordination = value.get_domain_coordination()?;
    if coordination.group == DomainCoordinationGroup::Named(variable.id.clone()) {
        Some(variable.title.clone())
    } else {
        None
    }
}

fn legend_event_bindings(
    legends: &IndexMap<String, Legend>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<ChartEventBinding>, AvengerChartError> {
    let mut bindings = Vec::new();
    for (channel_name, legend) in legends {
        legend.validate_event_surface()?;
        for binding in &legend.event_bindings {
            let binding = binding.clone().with_legend_surface_target(
                vec![channel_name.clone()],
                vec![
                    LegendSurfaceKind::DiscreteItem,
                    LegendSurfaceKind::ContinuousColorbar,
                ],
            );
            let binding = rewrite_reserved_event_binding_local_datums(binding, session_context)?;
            bindings.push(binding);
        }
    }
    Ok(bindings)
}

async fn compile_colorbar_overlays(
    legends: &IndexMap<String, Legend>,
    session_context: &datafusion::prelude::SessionContext,
) -> Result<Vec<CompiledColorbarOverlayMarks>, AvengerChartError> {
    let mut compiled = Vec::new();
    for (channel_name, legend) in legends {
        if legend.colorbar_overlays.is_empty() {
            continue;
        }
        let mut marks = Vec::new();
        for overlay in &legend.colorbar_overlays {
            let Some(overlay) = overlay.downcast_ref::<ColorbarOverlay>() else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Legend '{}' has an unsupported colorbar overlay type",
                    channel_name
                )));
            };
            marks.extend(overlay.compile(session_context).await?);
        }
        compiled.push(CompiledColorbarOverlayMarks {
            channel_name: channel_name.clone(),
            marks,
        });
    }
    Ok(compiled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::Cartesian;
    use crate::concat::{GridConcat, HConcat, VConcat, WrapConcat, compiled_subplot};
    use crate::event::{
        ChartEventBinding, ChartEventStream, ChartEventType, ChartParamChangeBinding, param_change,
    };
    use crate::facet::coord::{FacetColumn, FacetWrap};
    use crate::facet::marks::{FacetColumnSubplotChannels, FacetWrapSubplotChannels};
    use crate::plot::{EvaluationRequest, SelectionAssignment, SelectionStateUpdate};
    use crate::repeat::{RepeatColumns, RepeatGrid, RepeatRows, RepeatWrap};
    use crate::tools::ToolCompileContext;
    use crate::zerod::ZeroDCoord;
    use avenger_chart_cartesian::{
        CartesianAxis, CartesianRectPositionChannels, CartesianSymbolPositionChannels,
    };
    use avenger_chart_core::{
        AxisGuideVisibilityPolicy, CompiledDataTransform, CoordinationScope, DataTransform,
        DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
        DefaultLogicalExprNodeExt, DomainCoordinationGroup, FormattingContext, IntoPlotMark,
        MarkGroup, Param, PlotMark, RepeatContext, RepeatDomainCoordination, RepeatVariable,
        ResolvedRepeatVariable, ResolvedSelectionClauseScope, ScaleChannelConfig,
        ScaleInferenceHint, ScaleTypePreference, SceneGeometryQuery, SceneQueryDatumField,
        Selection, SelectionClause, SelectionClauseUpdate, SelectionEqualityDimensionValue,
        SelectionPredicateSpec, SelectionPredicateUpdate, SelectionSceneQuery, SelectionUpdate,
        Store, StoreRow, StoreUpdate, SubplotDataSource, collect_placeholder_ids,
        collect_repeat_placeholder_kinds, repeat, simplify_to_scalar_sync, store_placeholder_expr,
    };
    use avenger_chart_marks::{Rect, Subplot, Symbol};
    use avenger_chart_parallel::{
        Parallel, ParallelLine, ParallelSymbol,
        event::{
            PARALLEL_DIMENSION_ID_FIELD, PARALLEL_SURFACE_KIND_DIMENSION_TITLE,
            PARALLEL_SURFACE_KIND_FIELD, PARALLEL_SURFACE_KIND_POINT, PARALLEL_TITLE_FIELD,
            parallel_dimension_id, parallel_surface_kind, parallel_title,
        },
        generated_dimension_channel,
    };
    use avenger_chart_tools::PanScrollZoom;
    use avenger_chart_transforms::{Bin, Calculate, Filter};
    use avenger_scenegraph::marks::{
        line::SceneLineMark,
        mark::{MarkInstance, SceneMark},
    };

    use datafusion::{
        arrow::{
            array::Float64Array,
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        functions_aggregate::expr_fn::count,
        prelude::{SessionContext, col, lit},
    };
    use datafusion_proto::protobuf::LogicalExprNode;
    use serde::{Deserialize, Serialize};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Mutex;

    #[test]
    fn event_state_relation_placeholders_normalize_to_opaque_ids() {
        let mut allocator = CompiledIdentityAllocator::new("placeholder-test");
        let mut store = Store::empty("rows").compile().unwrap();
        store.runtime_id = allocator.allocate_store();
        let mut selection = Selection::new("picked").compile().unwrap();
        selection.runtime_id = allocator.allocate_selection();
        let stores = IndexMap::from([("rows".to_string(), store.clone())]);
        let selections = IndexMap::from([("picked".to_string(), selection.clone())]);

        let expr = store_placeholder_expr("rows")
            .is_not_null()
            .and(Selection::new("picked").predicate());
        let resolved = resolve_state_relation_placeholders(expr, &stores, &selections).unwrap();
        let placeholders = collect_placeholder_ids(&resolved).unwrap();

        assert!(
            placeholders
                .iter()
                .any(|id| id.contains(store.runtime_id.as_opaque_str()))
        );
        assert!(
            placeholders
                .iter()
                .any(|id| id.contains(selection.runtime_id.as_opaque_str()))
        );
        assert!(placeholders.iter().all(|id| !id.ends_with("rows")));
        assert!(placeholders.iter().all(|id| !id.ends_with("picked")));
    }

    fn resolved_repeat(id: &str) -> ResolvedRepeatVariable {
        ResolvedRepeatVariable {
            id: id.to_string(),
            expr: col(id),
            title: id.to_string(),
            type_hint: None,
        }
    }

    fn column_repeat_context(id: &str, index: usize, count: usize) -> RepeatContext {
        RepeatContext::new().with_column(resolved_repeat(id), index, count)
    }

    fn repeat_vars(names: &[&str]) -> Vec<RepeatVariable> {
        names
            .iter()
            .map(|name| RepeatVariable::new(*name, col(*name)).title(format!("Title {name}")))
            .collect()
    }

    fn repeated_column_cell() -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(repeat::column()).y(lit(1.0)).size(64.0))
    }

    fn repeated_row_cell() -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(lit(1.0)).y(repeat::row()).size(64.0))
    }

    #[tokio::test]
    async fn compile_preserves_formatting_context_number_locale() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let number_locale_spec = avenger_text::NumberLocaleSpec {
            decimal: Some("~".to_string()),
            group: Some("_".to_string()),
            ..Default::default()
        };
        let datetime_locale_spec = avenger_text::DateTimeLocaleSpec {
            date_patterns: Some(avenger_text::LengthsSpec {
                long: Some("y'~'MM'~'dd".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .formatting_context(
                FormattingContext::new()
                    .number_locale("custom")
                    .number_locale_spec("custom", number_locale_spec.clone())
                    .datetime_locale("custom-datetime")
                    .datetime_timezone("America/New_York")
                    .datetime_locale_spec("custom-datetime", datetime_locale_spec.clone()),
            )
            .mark(Symbol::new().x(lit(1.0)).y(lit(2.0)))
            .compile(&ctx)
            .await?;

        assert_eq!(
            compiled.formatting_context.resolved_number_locale(),
            "custom"
        );
        assert_eq!(
            compiled
                .formatting_context
                .number_locale_specs()
                .get("custom"),
            Some(&number_locale_spec)
        );
        assert_eq!(
            compiled.formatting_context.resolved_datetime_locale(),
            "custom-datetime"
        );
        assert_eq!(
            compiled.formatting_context.resolved_datetime_timezone(),
            "America/New_York"
        );
        assert_eq!(
            compiled
                .formatting_context
                .datetime_locale_specs()
                .get("custom-datetime"),
            Some(&datetime_locale_spec)
        );

        let serialized =
            bincode::serialize(&compiled).expect("serialize compiled plot with formatting context");
        let decoded: CompiledPlot = bincode::deserialize(&serialized)
            .expect("deserialize compiled plot with formatting context");
        assert_eq!(
            decoded.formatting_context.resolved_number_locale(),
            "custom"
        );
        assert_eq!(
            decoded
                .formatting_context
                .number_locale_specs()
                .get("custom"),
            Some(&number_locale_spec)
        );
        assert_eq!(
            decoded.formatting_context.resolved_datetime_locale(),
            "custom-datetime"
        );
        assert_eq!(
            decoded.formatting_context.resolved_datetime_timezone(),
            "America/New_York"
        );
        assert_eq!(
            decoded
                .formatting_context
                .datetime_locale_specs()
                .get("custom-datetime"),
            Some(&datetime_locale_spec)
        );
        Ok(())
    }

    fn repeated_grid_cell() -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(repeat::column())
                .y(repeat::row())
                .size(64.0),
        )
    }

    fn collect_scene_line_marks<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneLineMark>) {
        for mark in marks {
            match mark {
                SceneMark::Line(line) => out.push(line),
                SceneMark::Group(group) => collect_scene_line_marks(&group.marks, out),
                _ => {}
            }
        }
    }

    fn diagonal_histogram_count_shared_cell() -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new().mark(Rect::new().transform(
            Bin::new(repeat::column()).maxbins(5),
            |mark, bin| {
                mark.x(bin.start())
                    .x2(bin.end())
                    .y_with(lit(0.0), |c| {
                        c.with_domain_group("hist_count").share_domain()
                    })
                    .y2_with(count(lit(1)), |c| {
                        c.with_domain_group("hist_count").share_domain()
                    })
            },
        ))
    }

    fn constant_y_grid_cell(value: f64) -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(repeat::column()).y(lit(value)).size(64.0))
    }

    struct TestCompoundMark;

    impl IntoPlotMark<Cartesian> for TestCompoundMark {
        fn into_plot_marks(self) -> Vec<PlotMark<Cartesian>> {
            vec![
                PlotMark::from_mark(Symbol::new().id("compound_symbol").x(lit(1.0)).y(lit(2.0))),
                PlotMark::from_mark(Rect::new().id("compound_rect").x(lit(0.0)).y(lit(0.0))),
            ]
        }
    }

    static COUNTING_GROUP_TRANSFORM_APPLIES: AtomicUsize = AtomicUsize::new(0);
    static COUNTING_GROUP_TRANSFORM_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    #[derive(Clone)]
    struct CountingGroupTransform;

    impl DataTransform for CountingGroupTransform {
        type Output = ();

        fn into_compiled_and_output(
            self,
            _ctx: DataTransformCompileContext,
        ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
            Ok((Box::new(CompiledCountingGroupTransform), ()))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct CompiledCountingGroupTransform;

    #[typetag::serde(name = "test_mark_group_counting")]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    impl CompiledDataTransform for CompiledCountingGroupTransform {
        fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: datafusion::dataframe::DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataTransformResult, AvengerChartError> {
            COUNTING_GROUP_TRANSFORM_APPLIES.fetch_add(1, Ordering::SeqCst);
            Ok(DataTransformResult::dataframe(dataframe))
        }
    }

    async fn xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.sql("SELECT * FROM (VALUES (1.0, 2.0), (2.0, 4.0), (3.0, 6.0)) AS t(x, y)")
            .await
            .expect("dataframe")
    }

    #[tokio::test]
    async fn mark_group_flattening_preserves_author_order() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("a").x(lit(1.0)).y(lit(1.0)))
            .mark(
                MarkGroup::new()
                    .id("summary")
                    .mark(Symbol::new().id("b").x(lit(2.0)).y(lit(2.0)))
                    .mark(Rect::new().id("c").x(lit(3.0)).y(lit(3.0))),
            )
            .mark(TestCompoundMark)
            .compile(&ctx)
            .await
            .expect("compile");

        let ids = compiled
            .marks
            .iter()
            .map(|mark| mark.state().id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                Some("a".to_string()),
                Some("b".to_string()),
                Some("c".to_string()),
                Some("compound_symbol".to_string()),
                Some("compound_rect".to_string()),
            ]
        );
        assert_eq!(compiled.mark_groups.len(), 1);
        assert_eq!(
            compiled.mark_group_index_by_mark,
            vec![None, Some(0), Some(0), None, None]
        );
        let public_paths = compiled
            .marks
            .iter()
            .map(|mark| mark.state().identity.public_aliases.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            public_paths,
            vec![
                vec!["a".to_string()],
                vec!["summary.b".to_string()],
                vec!["summary.c".to_string()],
                vec!["compound_symbol".to_string()],
                vec!["compound_rect".to_string()],
            ]
        );
    }

    #[tokio::test]
    async fn nested_mark_group_records_nearest_parent_group() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new().id("outer").mark(
                    MarkGroup::new()
                        .id("inner")
                        .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_groups.len(), 2);
        assert_eq!(compiled.mark_groups[0].parent_group_index, None);
        assert_eq!(compiled.mark_groups[1].parent_group_index, Some(0));
        assert_eq!(compiled.mark_group_index_by_mark, vec![Some(1)]);
        assert_eq!(
            compiled.marks[0].state().identity.public_aliases,
            vec!["outer.inner.leaf"]
        );
    }

    #[tokio::test]
    async fn compiled_mark_identity_separates_aliases_private_structure_and_component_parts() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .id("points")
                    .alias("primary")
                    .alias("secondary")
                    .x(lit(1.0))
                    .y(lit(1.0)),
            )
            .mark(MarkGroup::new().mark(Symbol::new().id("private_leaf").x(lit(2.0)).y(lit(2.0))))
            .mark(
                MarkGroup::new()
                    .id("summary")
                    .component_kind("box-plot")
                    .mark(Rect::new().id("box").x(lit(3.0)).y(lit(3.0))),
            )
            .compile(&ctx)
            .await
            .expect("compile identity fixture");

        let exported = &compiled.marks[0].state().identity;
        assert!(!exported.runtime_id.is_unresolved());
        assert_eq!(
            exported.public_aliases,
            vec!["points", "primary", "secondary"]
        );

        let private = &compiled.marks[1].state().identity;
        assert!(private.public_aliases.is_empty());
        assert_eq!(private.private_ancestry, vec![0]);

        let component = compiled.marks[2]
            .state()
            .identity
            .component
            .as_ref()
            .expect("component provenance");
        assert_eq!(component.component_kind, "box-plot");
        assert_eq!(component.component_id.as_deref(), Some("summary"));
        assert_eq!(component.part_alias, "box");

        let runtime_names = compiled.mark_runtime_name_index();
        assert_eq!(
            runtime_names.get(&exported.runtime_id),
            Some(&vec!["points".to_string()]),
            "secondary public aliases resolve through the canonical rendered name"
        );
        assert!(!runtime_names.contains_key(&private.runtime_id));
        assert_eq!(
            runtime_names.get(&compiled.marks[2].state().identity.runtime_id),
            Some(&vec!["summary.box".to_string()])
        );

        let restored: CompiledPlot = bincode::deserialize(
            &bincode::serialize(&compiled).expect("serialize identity fixture"),
        )
        .expect("deserialize identity fixture");
        assert_eq!(
            restored.marks[0].state().identity,
            compiled.marks[0].state().identity
        );
        assert_eq!(
            restored
                .runtime_paths_for_mark_ids(&[restored.marks[0]
                    .state()
                    .identity
                    .runtime_id
                    .clone()])
                .unwrap(),
            vec![vec![0]]
        );

        let (baked, _) = compiled
            .bake(&ctx, &crate::bake::BakePolicy::default())
            .await
            .expect("bake identity fixture");
        assert_eq!(
            baked.marks[2].state().identity,
            compiled.marks[2].state().identity
        );
    }

    #[tokio::test]
    async fn mark_group_event_targets_resolve_to_public_paths() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("manual_box_plot.outliers"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            between.start.resolved_mark_ids(),
            &[compiled.marks[1].state().identity.runtime_id.clone()]
        );
        assert_eq!(
            compiled.marks[1].state().identity.public_aliases,
            vec!["manual_box_plot.outliers"]
        );
    }

    #[tokio::test]
    async fn root_primitive_event_target_resolves_to_mark_path() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("points"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            compiled
                .runtime_paths_for_mark_ids(between.start.resolved_mark_ids())
                .unwrap(),
            vec![vec![0usize]]
        );
    }

    #[tokio::test]
    async fn mark_group_root_target_resolves_to_descendants() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("manual_box_plot"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            compiled
                .runtime_paths_for_mark_ids(between.start.resolved_mark_ids())
                .unwrap(),
            vec![vec![0usize], vec![1usize]]
        );
    }

    #[tokio::test]
    async fn nested_local_mark_target_without_root_group_is_invalid() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(MarkGroup::new().mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0))))
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("outliers"),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("unrooted nested target should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Unknown mark target 'outliers'"));
    }

    #[tokio::test]
    async fn same_local_child_ids_under_different_roots_are_valid_targets() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("a")
                    .mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0))),
            )
            .mark(
                MarkGroup::new()
                    .id("b")
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .event_binding(ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).marks(["a.outliers", "b.outliers"]),
                ChartEventStream::on(ChartEventType::MouseUp),
            ))
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let between = binding.between.as_ref().expect("between binding");
        assert_eq!(
            compiled
                .runtime_paths_for_mark_ids(between.start.resolved_mark_ids())
                .unwrap(),
            vec![vec![0usize], vec![1usize]]
        );
        assert_eq!(
            compiled
                .marks
                .iter()
                .map(|mark| mark.state().identity.public_aliases.clone())
                .collect::<Vec<_>>(),
            vec![
                vec!["a.outliers".to_string()],
                vec!["b.outliers".to_string()]
            ]
        );
    }

    #[tokio::test]
    async fn duplicate_child_target_paths_under_same_root_error() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("a")
                    .mark(Symbol::new().id("outliers").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate nested public target should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Duplicate mark target path 'a.outliers'"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn scene_query_mark_targets_resolve_to_public_paths() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("manual_box_plot")
                    .mark(Symbol::new().id("box").x(lit(1.0)).y(lit(1.0)))
                    .mark(Symbol::new().id("outliers").x(lit(2.0)).y(lit(2.0))),
            )
            .selection(Selection::new("picked"))
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click).set_selection(
                    "picked",
                    SelectionUpdate::replace_all_from_scene_query(SelectionSceneQuery::new(
                        SceneGeometryQuery::rect(lit(0.0), lit(0.0), lit(10.0), lit(10.0))
                            .mark("manual_box_plot.outliers"),
                    )),
                ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let binding = compiled.event_bindings().first().expect("event binding");
        let SelectionUpdate::ReplaceAllFromSceneQuery { query } =
            &binding.action.selection_steps().next().unwrap().update
        else {
            panic!("expected scene query update");
        };
        assert_eq!(
            query.query.target.resolved_mark_ids(),
            &[compiled.marks[1].state().identity.runtime_id.clone()]
        );
    }

    #[tokio::test]
    async fn empty_mark_group_is_invalid() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(MarkGroup::new())
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("empty group should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("MarkGroup must contain"), "{err}");
    }

    #[tokio::test]
    async fn primitive_group_child_cannot_set_explicit_data() {
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(MarkGroup::new().mark(Symbol::new().data(data).x(col("x")).y(col("y"))))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("explicit child data should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Primitive child marks inside MarkGroup may not set explicit data"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn flattened_group_child_ids_must_be_unique() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("duplicate").x(lit(1.0)).y(lit(1.0)))
            .mark(MarkGroup::new().mark(Symbol::new().id("duplicate").x(lit(2.0)).y(lit(2.0))))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate ids should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Duplicate mark id"), "{err}");
    }

    #[tokio::test]
    async fn two_marks_in_one_group_share_transform_preparation() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group
                        .mark(Symbol::new().x(col("x")).y(col("y")))
                        .mark(Symbol::new().x(col("x")).y(col("y")))
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sibling_groups_prepare_independent_transform_branches() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let branch = || {
            MarkGroup::<Cartesian>::new().transform_no_output(CountingGroupTransform, |group| {
                group.mark(Symbol::new().x(col("x")).y(col("y")))
            })
        };
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(data.clone())
            .mark(branch())
            .mark(branch())
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn data_transparent_groups_do_not_shadow_child_mark_data() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("transparent")
                    .scale_inference_hint(ScaleInferenceHint::new("x", ScaleTypePreference::Point))
                    .mark(Symbol::new().id("leaf").x(col("x")).y(col("y"))),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_group_index_for_mark(0), Some(0));
        assert_eq!(compiled.data_group_index_for_mark(0), None);
        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![ScaleInferenceHint::new("x", ScaleTypePreference::Point)]
        );
        assert_eq!(
            compiled.marks[0].state().identity.public_aliases,
            vec!["transparent.leaf"]
        );

        let data = xy_dataframe(&ctx).await;
        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("transparent group does not hide plot data");
    }

    #[tokio::test]
    async fn data_transparent_nested_group_uses_parent_data_group() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx).await;
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group.mark(
                        MarkGroup::new()
                            .id("transparent")
                            .mark(Symbol::new().id("leaf").x(col("x")).y(col("y"))),
                    )
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(compiled.mark_group_index_for_mark(0), Some(1));
        assert_eq!(compiled.data_group_index_for_mark(0), Some(0));
        let _scales = compiled
            .build_scales_for_dataframe(&data, 320.0, 240.0, &ctx, compiled.get_default_params())
            .await
            .expect("build scales");
    }

    #[tokio::test]
    async fn nested_explicit_group_data_resets_parent_inheritance() {
        let _guard = COUNTING_GROUP_TRANSFORM_TEST_LOCK.lock().await;
        let ctx = SessionContext::new();
        let parent_data = xy_dataframe(&ctx).await;
        let child_data = ctx
            .sql("SELECT * FROM (VALUES (10.0, 20.0), (30.0, 40.0)) AS t(x, y)")
            .await
            .expect("child dataframe");
        COUNTING_GROUP_TRANSFORM_APPLIES.store(0, Ordering::SeqCst);
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .data(parent_data.clone())
            .mark(
                MarkGroup::new().transform_no_output(CountingGroupTransform, |group| {
                    group.mark(
                        MarkGroup::new()
                            .data(child_data)
                            .mark(Symbol::new().x(col("x")).y(col("y"))),
                    )
                }),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let _scales = compiled
            .build_scales_for_dataframe(
                &parent_data,
                320.0,
                240.0,
                &ctx,
                compiled.get_default_params(),
            )
            .await
            .expect("build scales");
        assert_eq!(COUNTING_GROUP_TRANSFORM_APPLIES.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn compiled_plot_serializes_mark_group_metadata() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("outer")
                    .scale_inference_hint(ScaleInferenceHint::new("x", ScaleTypePreference::Point))
                    .mark(
                        MarkGroup::new()
                            .id("inner")
                            .scale_inference_hint(ScaleInferenceHint::new(
                                "y",
                                ScaleTypePreference::Band,
                            ))
                            .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
                    ),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![
                ScaleInferenceHint::new("x", ScaleTypePreference::Point),
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
            ]
        );
        let bytes = bincode::serialize(&compiled).expect("serialize");
        let restored: CompiledPlot = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored.mark_groups.len(), 2);
        assert_eq!(restored.mark_groups[0].id.as_deref(), Some("outer"));
        assert_eq!(
            restored.mark_groups[0].scale_inference_hints,
            vec![ScaleInferenceHint::new("x", ScaleTypePreference::Point)]
        );
        assert_eq!(restored.mark_groups[1].id.as_deref(), Some("inner"));
        assert_eq!(restored.mark_groups[1].parent_group_index, Some(0));
        assert_eq!(
            restored.mark_groups[1].scale_inference_hints,
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        assert_eq!(restored.mark_group_index_by_mark, vec![Some(1)]);
        assert_eq!(
            restored.scale_inference_hints_for_mark(0).expect("hints"),
            vec![
                ScaleInferenceHint::new("x", ScaleTypePreference::Point),
                ScaleInferenceHint::new("y", ScaleTypePreference::Band),
            ]
        );
    }

    #[tokio::test]
    async fn compiled_plot_serializes_single_mark_group_metadata() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .mark(
                MarkGroup::new()
                    .id("group")
                    .scale_inference_hint(ScaleInferenceHint::new("y", ScaleTypePreference::Band))
                    .mark(Symbol::new().id("leaf").x(lit(1.0)).y(lit(1.0))),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.scale_inference_hints_for_mark(0).expect("hints"),
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        let bytes = bincode::serialize(&compiled).expect("serialize");
        let restored: CompiledPlot = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored.mark_groups.len(), 1);
        assert_eq!(restored.mark_groups[0].id.as_deref(), Some("group"));
        assert_eq!(
            restored.mark_groups[0].scale_inference_hints,
            vec![ScaleInferenceHint::new("y", ScaleTypePreference::Band)]
        );
        assert_eq!(restored.mark_group_index_by_mark, vec![Some(0)]);
    }

    fn repeated_item_cell() -> Plot<Cartesian> {
        crate::plot::Plot::<Cartesian>::new()
            .mark(Symbol::new().x(lit(1.0)).y(repeat::item()).size(64.0))
    }

    fn zerod_branch_cell() -> Plot<ZeroDCoord> {
        crate::plot::Plot::<ZeroDCoord>::new().mark(Symbol::new().fill("#2f7ed8").size(64.0))
    }

    fn repeat_histogram_domain_data(ctx: &SessionContext) -> DataFrame {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
            Field::new("c", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0])),
                Arc::new(Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0])),
                Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 3.0])),
            ],
        )
        .expect("repeat histogram domain batch");
        ctx.read_batch(batch).expect("repeat histogram domain df")
    }

    fn child_channel_expr(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
        ctx: &SessionContext,
    ) -> String {
        subplot.compiled_subplot().marks[0]
            .data_context()
            .channels()
            .get(channel)
            .expect("child channel")
            .expr(ctx)
            .expect("child expr")
            .to_string()
    }

    fn child_channel_domain_coordination(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
    ) -> Option<DomainCoordination> {
        subplot.compiled_subplot().marks[0]
            .data_context()
            .channels()
            .get(channel)
            .expect("child channel")
            .get_domain_coordination()
            .cloned()
    }

    fn child_axis_title(
        subplot: &crate::concat::CompiledConcatSubplot,
        channel: &str,
        ctx: &SessionContext,
    ) -> Option<String> {
        let AxisSpec::Local(axis) = subplot.compiled_subplot().axis_specs.get(channel)?;
        let axis = axis.as_any().downcast_ref::<CartesianAxis>()?;
        let title = axis.title.as_option()?.as_ref()?;
        let scalar = simplify_to_scalar_sync(title.to_default_expr(ctx).ok()?).ok()?;
        match scalar {
            ScalarValue::Utf8(Some(value)) => Some(value),
            other => Some(other.to_string()),
        }
    }

    fn lowered_children(compiled: &CompiledPlot) -> Vec<&crate::concat::CompiledConcatSubplot> {
        compiled
            .marks
            .iter()
            .map(|mark| compiled_subplot(mark.as_ref()).expect("lowered concat subplot"))
            .collect()
    }

    async fn compile_with_repeat_context(
        plot: Plot<Cartesian>,
        ctx: &SessionContext,
        repeat_context: RepeatContext,
    ) -> CompiledPlot {
        let tool_context =
            ToolCompileContext::root(None, TimeContext::default(), FormattingContext::default())
                .with_repeat_context(repeat_context);
        plot.compile_with_tool_context(
            ctx,
            Some(&tool_context),
            Some(RootChartFurnishings::default()),
            ChildPlotFurnishings::default(),
        )
        .await
        .expect("plot compiles with repeat context")
    }

    #[tokio::test]
    async fn repeat_columns_lower_to_hconcat_children() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatColumns>::new()
            .configure_coord(|c| {
                c.columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_column_cell())
            })
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<HConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), Some("repeat_col:a"));
        assert_eq!(children[1].key(), Some("repeat_col:b"));
        assert_eq!(children[0].label(), Some("Title a"));
        assert_eq!(children[1].label(), Some("Title b"));
        assert_eq!(
            children[0].compiled_state().id.as_deref(),
            Some("repeat_col_a")
        );
        assert_eq!(
            children[1].compiled_state().id.as_deref(),
            Some("repeat_col_b")
        );
        assert_eq!(children[0].data_source(), SubplotDataSource::InheritParent);
        assert_eq!(child_channel_expr(children[0], "x", &ctx), "a");
        assert_eq!(child_channel_expr(children[1], "x", &ctx), "b");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_rows_lower_to_vconcat_children() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatRows>::new()
            .configure_coord(|c| c.rows(repeat_vars(&["a", "b"])).cell(repeated_row_cell()))
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<VConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].key(), Some("repeat_row:a"));
        assert_eq!(children[1].key(), Some("repeat_row:b"));
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[1], "y", &ctx), "b");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_lowers_to_grid_concat_with_cell_placement() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["r1", "r2"]))
                    .columns(repeat_vars(&["c1", "c2", "c3"]))
                    .cell(repeated_grid_cell())
            })
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<GridConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 6);
        let placements = children
            .iter()
            .map(|child| {
                let placement = child.grid_placement().expect("grid placement");
                (placement.row, placement.column)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            placements,
            vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]
        );
        assert_eq!(children[0].key(), Some("repeat_cell:r1:c1"));
        assert_eq!(children[5].key(), Some("repeat_cell:r2:c3"));
        assert_eq!(child_channel_expr(children[0], "x", &ctx), "c1");
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "r1");
        assert_eq!(child_channel_expr(children[5], "x", &ctx), "c3");
        assert_eq!(child_channel_expr(children[5], "y", &ctx), "r2");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_generate_variable_groups() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .matrix_domains()
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        let top_left_x = child_channel_domain_coordination(children[0], "x").expect("x domain");
        let top_left_y = child_channel_domain_coordination(children[0], "y").expect("y domain");
        let top_right_x = child_channel_domain_coordination(children[1], "x").expect("x domain");
        assert_eq!(top_left_x.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            top_left_x.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            top_left_y.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            top_right_x.group,
            DomainCoordinationGroup::Named("b".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_preserve_explicit_scope_and_independent_mode()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let level_compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["b"]))
                    .cell(repeated_grid_cell())
                    .matrix_domains_with_scope(CoordinationScope::Level(1))
            })
            .compile(&ctx)
            .await?;
        let level_child = lowered_children(&level_compiled);
        let x_coordination =
            child_channel_domain_coordination(level_child[0], "x").expect("x domain");
        assert_eq!(x_coordination.scope, CoordinationScope::Level(1));
        assert_eq!(
            x_coordination.group,
            DomainCoordinationGroup::Named("b".to_string())
        );

        let independent_compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["b"]))
                    .cell(repeated_grid_cell())
            })
            .compile(&ctx)
            .await?;
        let independent_child = lowered_children(&independent_compiled);
        assert!(child_channel_domain_coordination(independent_child[0], "x").is_none());
        assert!(child_channel_domain_coordination(independent_child[0], "y").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_reject_conflicting_authored_domain_group() {
        let ctx = SessionContext::new();
        let conflicting_cell = crate::plot::Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(repeat::column(), |c| c.with_domain_group("other"))
                .y(repeat::row())
                .size(64.0),
        );
        let err = match crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["a"]))
                    .cell(conflicting_cell)
                    .matrix_domains()
            })
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("conflicting repeat domain group should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("Repeat-generated domain coordination target"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn repeat_grid_conditional_histogram_preserves_count_domain_group()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        repeat::row_index().eq(repeat::column_index()),
                        diagonal_histogram_count_shared_cell(),
                    )
                    .matrix_domains()
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        for index in [0, 3] {
            let y = child_channel_domain_coordination(children[index], "y").expect("y domain");
            let y2 = child_channel_domain_coordination(children[index], "y2").expect("y2 domain");
            assert_eq!(y.scope, CoordinationScope::Level(u8::MAX));
            assert_eq!(y2.scope, CoordinationScope::Level(u8::MAX));
            assert_eq!(
                y.group,
                DomainCoordinationGroup::Named("hist_count".to_string())
            );
            assert_eq!(
                y2.group,
                DomainCoordinationGroup::Named("hist_count".to_string())
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_conditional_histogram_coordinates_count_domains()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .data(repeat_histogram_domain_data(&ctx))
            .plot_size(180.0, 140.0)
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b", "c"]))
                    .columns(repeat_vars(&["a", "b", "c"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        repeat::row_index().eq(repeat::column_index()),
                        diagonal_histogram_count_shared_cell(),
                    )
                    .matrix_domains()
            })
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let histogram_y_domains = evaluated
            .interaction
            .scopes
            .iter()
            .filter_map(|scope| {
                let child = scope.child_frame_path.last()?;
                let key = child.key.as_deref()?;
                matches!(
                    key,
                    "repeat_cell:a:a" | "repeat_cell:b:b" | "repeat_cell:c:c"
                )
                .then(|| {
                    scope
                        .scales
                        .get("y")
                        .expect("y scale")
                        .numeric_interval_domain()
                        .expect("numeric y domain")
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(histogram_y_domains.len(), 3);
        assert_eq!(histogram_y_domains[0], histogram_y_domains[1]);
        assert_eq!(histogram_y_domains[0], histogram_y_domains[2]);
        assert!(
            histogram_y_domains[0].1 >= 6.0,
            "shared count domain should include the densest histogram bin, got {:?}",
            histogram_y_domains[0]
        );

        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_item_domains_generate_item_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatWrap>::new()
            .configure_coord(|c| {
                c.items(repeat_vars(&["a", "b"]))
                    .columns(2)
                    .cell(repeated_item_cell())
                    .item_domains_with_scope(CoordinationScope::Level(1))
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        let first_y = child_channel_domain_coordination(children[0], "y").expect("y domain");
        let second_y = child_channel_domain_coordination(children[1], "y").expect("y domain");
        assert_eq!(first_y.scope, CoordinationScope::Level(1));
        assert_eq!(
            first_y.group,
            DomainCoordinationGroup::Named("a".to_string())
        );
        assert_eq!(
            second_y.group,
            DomainCoordinationGroup::Named("b".to_string())
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_match_manual_grid_concat() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let repeat_compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .matrix_domains()
            })
            .compile(&ctx)
            .await?;

        let manual_cell = |x: &'static str, y: &'static str| {
            crate::plot::Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col(x), move |c| c.with_domain_group(x).share_domain())
                    .y_with(col(y), move |c| c.with_domain_group(y).share_domain())
                    .size(64.0),
            )
        };
        let manual_compiled = crate::plot::Chart::<GridConcat>::new()
            .configure_coord(|c| c.rows(2).columns(2))
            .mark(Subplot::new(manual_cell("a", "a")).at(0, 0))
            .mark(Subplot::new(manual_cell("b", "a")).at(0, 1))
            .mark(Subplot::new(manual_cell("a", "b")).at(1, 0))
            .mark(Subplot::new(manual_cell("b", "b")).at(1, 1))
            .compile(&ctx)
            .await?;

        let repeat_children = lowered_children(&repeat_compiled);
        let manual_children = lowered_children(&manual_compiled);
        let repeat_targets = repeat_children
            .iter()
            .map(|child| {
                (
                    child_channel_domain_coordination(child, "x"),
                    child_channel_domain_coordination(child, "y"),
                )
            })
            .collect::<Vec<_>>();
        let manual_targets = manual_children
            .iter()
            .map(|child| {
                (
                    child_channel_domain_coordination(child, "x"),
                    child_channel_domain_coordination(child, "y"),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(repeat_targets, manual_targets);
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_domains_are_visible_to_pan_scroll_zoom()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let repeat_context = RepeatContext::new()
            .with_row(resolved_repeat("a"), 0, 2)
            .with_column(resolved_repeat("b"), 1, 2)
            .with_domain_coordination(RepeatDomainCoordination::by_variable(
                CoordinationScope::Shared,
            ));
        let compiled = compile_with_repeat_context(
            repeated_grid_cell().tool(PanScrollZoom::cartesian()),
            &ctx,
            repeat_context,
        )
        .await;

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__domain__a")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__domain__b")
        );
        assert!(
            !compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            !compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__y_domain")
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__domain__a"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__domain__b"].sharing,
            CoordinationScope::Level(u8::MAX)
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_pan_scroll_zoom_expands_across_cells() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .param(Param::new("explicit_root", true))
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell().tool(PanScrollZoom::cartesian()))
                    .matrix_domains()
            })
            .compile(&ctx)
            .await?;

        let tool_param_names = compiled
            .param_specs()
            .keys()
            .filter(|name| name.starts_with("__tool_pan_scroll_zoom__"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            tool_param_names,
            vec![
                "__tool_pan_scroll_zoom__enabled".to_string(),
                "__tool_pan_scroll_zoom__domain__a".to_string(),
                "__tool_pan_scroll_zoom__domain__b".to_string(),
            ]
        );
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert!(compiled.param_specs().contains_key("explicit_root"));

        let drag_bindings = compiled
            .event_bindings()
            .iter()
            .filter(|binding| binding.event_type == ChartEventType::CursorMoved)
            .collect::<Vec<_>>();
        assert_eq!(drag_bindings.len(), 4);
        let mut targets = drag_bindings
            .iter()
            .map(|binding| {
                binding
                    .scope_target
                    .as_ref()
                    .and_then(|target| target.resolved_coord_node_path_prefix())
                    .map(|target| target.to_vec())
            })
            .collect::<Vec<_>>();
        targets.sort();
        assert_eq!(
            targets,
            vec![Some(vec![0]), Some(vec![1]), Some(vec![2]), Some(vec![3]),]
        );
        assert!(drag_bindings.iter().any(|binding| {
            binding
                .action
                .param_steps()
                .any(|assignment| assignment.param_name == "__tool_pan_scroll_zoom__domain__a")
                && binding
                    .action
                    .param_steps()
                    .any(|assignment| assignment.param_name == "__tool_pan_scroll_zoom__domain__b")
        }));

        assert_eq!(
            compiled
                .event_bindings()
                .iter()
                .filter(|binding| binding.event_type == ChartEventType::MouseWheel)
                .count(),
            4
        );
        assert_eq!(
            compiled
                .event_bindings()
                .iter()
                .filter(|binding| binding.event_type == ChartEventType::DoubleClick)
                .count(),
            4
        );
        Ok(())
    }

    #[tokio::test]
    async fn explicit_root_state_coexists_with_ordinary_plot_tool_state()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .param(Param::new("explicit_root", true))
            .mark(Symbol::new().x(lit(1.0)).y(lit(2.0)).size(64.0))
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await?;

        assert!(compiled.param_specs().contains_key("explicit_root"));
        assert!(
            compiled
                .param_specs()
                .keys()
                .any(|name| name.starts_with("__tool_pan_scroll_zoom__"))
        );
        assert_eq!(compiled.tool_metadata().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_free_pan_scroll_zoom_uses_cell_specific_params()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell().tool(PanScrollZoom::cartesian()))
                    .matrix_domains_with_scope(CoordinationScope::Free)
            })
            .compile(&ctx)
            .await?;

        let tool_param_names = compiled
            .param_specs()
            .keys()
            .filter(|name| name.starts_with("__tool_pan_scroll_zoom__"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a__repeat_cell_b_a"),
            "expected a cell-specific a-domain param, got {tool_param_names:?}"
        );
        assert!(
            tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a__repeat_cell_a_b"),
            "expected a distinct cross-orientation a-domain param, got {tool_param_names:?}"
        );
        assert!(
            !tool_param_names
                .iter()
                .any(|name| name == "__tool_pan_scroll_zoom__domain__a"),
            "free repeat should not use a shared a-domain param: {tool_param_names:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_axes_generate_title_defaults_and_policy()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .matrix_domains()
                    .matrix_axes()
            })
            .compile(&ctx)
            .await?;

        let grid = compiled
            .coord_transform
            .as_any()
            .downcast_ref::<GridConcat>()
            .expect("lowered to GridConcat");
        assert_eq!(
            grid.axis_guide_visibility_config().labels,
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups
        );
        assert_eq!(
            grid.axis_guide_visibility_config().title,
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups
        );

        let children = lowered_children(&compiled);
        assert_eq!(
            child_axis_title(children[0], "x", &ctx).as_deref(),
            Some("Title a")
        );
        assert_eq!(
            child_axis_title(children[0], "y", &ctx).as_deref(),
            Some("Title a")
        );
        assert_eq!(
            child_axis_title(children[1], "x", &ctx).as_deref(),
            Some("Title b")
        );
        assert_eq!(
            child_axis_title(children[2], "y", &ctx).as_deref(),
            Some("Title b")
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_matrix_axes_preserve_explicit_titles_and_skip_non_repeat_axes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let explicit_cell = crate::plot::Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(repeat::column(), |c| c.axis(|a| a.title("authored x")))
                .y(lit(1.0))
                .size(64.0),
        );
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["b"]))
                    .cell(explicit_cell)
                    .matrix_domains()
                    .matrix_axes()
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(
            child_axis_title(children[0], "x", &ctx).as_deref(),
            Some("authored x")
        );
        assert_eq!(child_axis_title(children[0], "y", &ctx), None);
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_cell_when_selects_diagonal_branch() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        repeat::row_index().eq(repeat::column_index()),
                        constant_y_grid_cell(99.0),
                    )
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 4);
        assert_eq!(children[0].key(), Some("repeat_cell:a:a"));
        assert_eq!(children[1].key(), Some("repeat_cell:a:b"));
        assert_eq!(children[3].key(), Some("repeat_cell:b:b"));
        assert!(
            child_channel_expr(children[0], "y", &ctx).contains("99"),
            "diagonal cell should use branch template"
        );
        assert_eq!(child_channel_expr(children[1], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[2], "y", &ctx), "b");
        assert!(
            child_channel_expr(children[3], "y", &ctx).contains("99"),
            "diagonal cell should use branch template"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_cell_when_uses_author_order_priority() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        repeat::row_index().eq(repeat::column_index()),
                        constant_y_grid_cell(10.0),
                    )
                    .cell_when(lit(true), constant_y_grid_cell(20.0))
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert!(
            child_channel_expr(children[0], "y", &ctx).contains("10"),
            "first matching branch should win on diagonal cells"
        );
        assert!(
            child_channel_expr(children[1], "y", &ctx).contains("20"),
            "later broad branch should handle off-diagonal cells"
        );
        assert!(
            child_channel_expr(children[3], "y", &ctx).contains("10"),
            "first matching branch should win on diagonal cells"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_cell_when_allows_heterogeneous_child_plot_specs()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["a"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        repeat::row_index().eq(repeat::column_index()),
                        zerod_branch_cell(),
                    )
            })
            .compile(&ctx)
            .await?;

        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 1);
        assert!(
            children[0]
                .compiled_subplot()
                .coord_transform
                .as_any()
                .is::<ZeroDCoord>(),
            "diagonal branch should compile as the selected ZeroD child plot"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_lowers_to_wrap_concat_with_item_context() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::<RepeatWrap>::new()
            .configure_coord(|c| {
                c.items(repeat_vars(&["a", "b", "c"]))
                    .columns(2)
                    .cell(repeated_item_cell())
            })
            .compile(&ctx)
            .await?;

        assert!(compiled.coord_transform.as_any().is::<WrapConcat>());
        let children = lowered_children(&compiled);
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].key(), Some("repeat_item:a"));
        assert_eq!(children[2].key(), Some("repeat_item:c"));
        assert_eq!(children[0].label(), Some("Title a"));
        assert_eq!(children[2].label(), Some("Title c"));
        assert_eq!(child_channel_expr(children[0], "y", &ctx), "a");
        assert_eq!(child_channel_expr(children[2], "y", &ctx), "c");
        Ok(())
    }

    #[tokio::test]
    async fn repeat_generated_cells_preserve_event_datum_translation()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
            Field::new("group_name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![3.0, 4.0])),
                Arc::new(datafusion::arrow::array::StringArray::from(vec![
                    "Alpha", "Beta",
                ])),
            ],
        )
        .expect("record batch");
        let df = ctx.read_batch(batch).expect("dataframe");

        let compiled = crate::plot::Chart::<RepeatColumns>::new()
            .data(df)
            .configure_coord(|c| {
                c.columns(vec![
                    RepeatVariable::new("a", col("a")),
                    RepeatVariable::new("b", col("b")),
                ])
                .cell(repeated_column_cell())
            })
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .filter(crate::event::datum("group_name").is_not_null()),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.event_datums.rows.len(), 2);
        let subplot_ids = evaluated
            .event_datums
            .rows
            .iter()
            .map(|rows| rows.subplot_id_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            subplot_ids,
            vec![
                vec!["repeat_col_a".to_string()],
                vec!["repeat_col_b".to_string()]
            ]
        );
        for rows in &evaluated.event_datums.rows {
            assert_eq!(rows.rows.num_rows(), 2);
            assert!(rows.rows.column_by_name("group_name").is_some());
        }
        Ok(())
    }

    #[tokio::test]
    async fn parallel_axis_title_event_datums_support_scene_query_fields()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed, 100.0 AS cost").await?;
        let compiled = crate::plot::Chart::with_coord(
            Parallel::new()
                .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
                .dimension_with("cost", |d| d.axis(|a| a.title("Cost"))),
        )
        .plot_size(120.0, 100.0)
        .data(data)
        .mark(
            ParallelLine::new()
                .dimension("speed", col("speed"))
                .dimension("cost", col("cost")),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(parallel_surface_kind().eq(lit(PARALLEL_SURFACE_KIND_DIMENSION_TITLE)))
                .filter(parallel_dimension_id().is_not_null())
                .filter(parallel_title().is_not_null()),
        )
        .compile(&ctx)
        .await?;

        let event_datum_types = compiled.event_datum_types();
        assert_eq!(
            event_datum_types.get(PARALLEL_SURFACE_KIND_FIELD),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get(PARALLEL_DIMENSION_ID_FIELD),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get(PARALLEL_TITLE_FIELD),
            Some(&DataType::Utf8)
        );

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let axis_title_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| {
                rows.rows
                    .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
                    .is_some()
            })
            .expect("parallel axis title event datum rows");
        let queried = evaluated.event_datums.datums_for_mark_instances(
            [MarkInstance {
                name: "parallel_axis_title_hit".to_string(),
                mark_path: axis_title_rows.mark_path.clone(),
                instance_index: Some(0),
            }],
            &[
                SceneQueryDatumField::new("dimension").datum(PARALLEL_DIMENSION_ID_FIELD),
                SceneQueryDatumField::new("title").datum(PARALLEL_TITLE_FIELD),
            ],
            &[],
        )?;

        assert_eq!(queried.num_rows(), 1);
        let dimension =
            ScalarValue::try_from_array(queried.column_by_name("dimension").unwrap(), 0)?;
        let title = ScalarValue::try_from_array(queried.column_by_name("title").unwrap(), 0)?;
        assert_eq!(dimension, ScalarValue::Utf8(Some("speed".to_string())));
        assert_eq!(title, ScalarValue::Utf8(Some("Speed".to_string())));
        Ok(())
    }

    #[tokio::test]
    async fn parallel_symbol_event_datums_support_source_row_and_dimension_scene_query()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES \
                    ('r0', 10.0, 100.0), \
                    ('r1', 20.0, 200.0) \
                ) AS t(row_id, speed, cost)",
            )
            .await?;
        let compiled = crate::plot::Chart::<Parallel>::new()
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(
                ParallelSymbol::new()
                    .dimension("speed", col("speed"))
                    .dimension("cost", col("cost")),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .filter(crate::event::datum("row_id").is_not_null())
                    .filter(parallel_dimension_id().is_not_null())
                    .filter(parallel_surface_kind().eq(lit(PARALLEL_SURFACE_KIND_POINT))),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let speed_point_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| {
                let Some(dimension_column) = rows.rows.column_by_name(PARALLEL_DIMENSION_ID_FIELD)
                else {
                    return false;
                };
                matches!(
                    ScalarValue::try_from_array(dimension_column, 0),
                    Ok(ScalarValue::Utf8(Some(value))) if value == "speed"
                )
            })
            .expect("parallel speed point event datum rows");
        let queried = evaluated.event_datums.datums_for_mark_instances(
            [MarkInstance {
                name: "parallel_symbol".to_string(),
                mark_path: speed_point_rows.mark_path.clone(),
                instance_index: Some(1),
            }],
            &[
                SceneQueryDatumField::new("source_row").datum("row_id"),
                SceneQueryDatumField::new("dimension").datum(PARALLEL_DIMENSION_ID_FIELD),
                SceneQueryDatumField::new("surface").datum(PARALLEL_SURFACE_KIND_FIELD),
            ],
            &[],
        )?;

        assert_eq!(queried.num_rows(), 1);
        let source_row =
            ScalarValue::try_from_array(queried.column_by_name("source_row").unwrap(), 0)?;
        let dimension =
            ScalarValue::try_from_array(queried.column_by_name("dimension").unwrap(), 0)?;
        let surface = ScalarValue::try_from_array(queried.column_by_name("surface").unwrap(), 0)?;
        assert_eq!(source_row, ScalarValue::Utf8(Some("r1".to_string())));
        assert_eq!(dimension, ScalarValue::Utf8(Some("speed".to_string())));
        assert_eq!(
            surface,
            ScalarValue::Utf8(Some(PARALLEL_SURFACE_KIND_POINT.to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn parallel_line_zindex_lifts_selected_branch_above_context()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let data = ctx
            .sql(
                "SELECT * FROM (VALUES \
                    ('row0', 10.0, 30.0), \
                    ('row1', 20.0, 40.0) \
                ) AS t(id, speed, cost)",
            )
            .await?;
        let selected = Selection::new("picked").empty_selects_nothing();
        let selected_predicate = selected.predicate();
        let compiled = Arc::new(
            crate::plot::Chart::<Parallel>::new()
                .plot_size(120.0, 100.0)
                .data(data)
                .selection(selected)
                .mark(
                    ParallelLine::new()
                        .id("context_lines")
                        .dimension("speed", col("speed"))
                        .dimension("cost", col("cost"))
                        .stroke("#c4cbd5")
                        .zindex(1),
                )
                .mark(
                    ParallelLine::new()
                        .id("selected_lines")
                        .dimension("speed", col("speed"))
                        .dimension("cost", col("cost"))
                        .transform_no_output(Filter::new(selected_predicate), |mark| mark)
                        .stroke("#2563eb")
                        .zindex(20),
                )
                .compile(ctx.as_ref())
                .await?,
        );
        let mut session = compiled.instantiate(ctx);
        session.apply_selection_patch(vec![SelectionAssignment {
            selection_id: "picked".to_string(),
            update: SelectionStateUpdate::ReplaceAllClauses {
                clauses: vec![SelectionClause {
                    id: "row1".to_string(),
                    scope: ResolvedSelectionClauseScope {
                        sharing: CoordinationScope::Shared,
                        owner_path: Vec::new(),
                    },
                    predicate: SelectionPredicateSpec::Equality {
                        dimensions: vec![SelectionEqualityDimensionValue {
                            id: "id".to_string(),
                            field_expr: LogicalExprNode::from_expr(col("id"))?,
                            value: ScalarValue::Utf8(Some("row1".to_string())),
                        }],
                    },
                    facet_context: Vec::new(),
                }],
            },
        }])?;

        let (evaluated, _metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await?;
        let mut lines = Vec::new();
        collect_scene_line_marks(&evaluated.scene_graph.marks, &mut lines);
        let context_lines = lines
            .iter()
            .copied()
            .filter(|line| line.name == "context_lines")
            .collect::<Vec<_>>();
        let selected_lines = lines
            .iter()
            .copied()
            .filter(|line| line.name == "selected_lines")
            .collect::<Vec<_>>();

        assert_eq!(context_lines.len(), 2);
        assert_eq!(selected_lines.len(), 1);
        assert!(context_lines.iter().all(|line| line.zindex == Some(1)));
        assert!(selected_lines.iter().all(|line| line.zindex == Some(20)));
        assert!(
            selected_lines[0].zindex > context_lines[0].zindex,
            "selected-line branch should render above context branch"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_inside_hconcat_exports_prefixed_interaction_metadata()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0, 10.0), (2.0, 20.0)) AS t(a, b)")
            .await?;
        let repeat = crate::plot::Plot::<RepeatGrid>::new()
            .data(df)
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a", "b"]))
                    .columns(repeat_vars(&["a", "b"]))
                    .cell(repeated_grid_cell())
                    .matrix_domains()
                    .matrix_axes()
            });
        let compiled = crate::plot::Chart::<HConcat>::new()
            .canvas_size(620.0, 320.0)
            .mark(Subplot::new(repeat).name("matrix").id("matrix"))
            .mark(
                Subplot::new(crate::plot::Plot::<ZeroDCoord>::new())
                    .name("summary")
                    .id("summary"),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 4);
        let scope = evaluated
            .interaction
            .scopes
            .iter()
            .find(|scope| {
                scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some("repeat_cell:a:b")
            })
            .expect("repeat a/b scope");
        assert_eq!(
            scope.subplot_id_path,
            vec!["matrix".to_string(), "repeat_cell_a_b".to_string()]
        );
        assert_eq!(
            scope
                .child_frame_path
                .iter()
                .filter_map(|segment| segment.key.as_deref())
                .collect::<Vec<_>>(),
            vec!["matrix", "repeat_cell:a:b"]
        );
        assert_eq!(scope.child_frame_path[1].row, Some(0));
        assert_eq!(scope.child_frame_path[1].column, Some(1));
        assert!(scope.facet_path.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn repeat_grid_inside_facet_exports_facet_and_repeat_metadata()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 1.0, 10.0),
                    ('G1', 2.0, 20.0),
                    ('G2', 3.0, 30.0),
                    ('G2', 4.0, 40.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = crate::plot::Plot::<RepeatGrid>::new().configure_coord(|c| {
            c.rows(repeat_vars(&["a", "b"]))
                .columns(repeat_vars(&["a", "b"]))
                .cell(repeated_grid_cell())
                .matrix_domains()
                .matrix_axes()
        });
        let compiled = crate::plot::Chart::<FacetColumn>::new()
            .canvas_size(820.0, 320.0)
            .data(df)
            .mark(Subplot::new(repeat).id("matrix").column(col("group_name")))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 8);
        let mut groups = evaluated
            .interaction
            .scopes
            .iter()
            .map(|scope| match scope.facet_path.first() {
                Some(ScalarValue::Utf8(Some(value))) => value.clone(),
                other => panic!("expected facet value, got {other:?}"),
            })
            .collect::<Vec<_>>();
        groups.sort();
        groups.dedup();
        assert_eq!(groups, vec!["G1".to_string(), "G2".to_string()]);

        let g1_ab = evaluated
            .interaction
            .scopes
            .iter()
            .find(|scope| {
                matches!(
                    scope.facet_path.first(),
                    Some(ScalarValue::Utf8(Some(value))) if value == "G1"
                ) && scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    == Some("repeat_cell:a:b")
            })
            .expect("G1 repeat a/b scope");
        assert_eq!(
            g1_ab.subplot_id_path,
            vec!["matrix".to_string(), "repeat_cell_a_b".to_string()]
        );
        assert_eq!(
            g1_ab.sharing_owner_paths.get(&0),
            Some(&g1_ab.facet_path),
            "Free owner path should remain the outer facet cell"
        );
        assert_eq!(
            g1_ab.sharing_owner_paths.get(&1),
            Some(&Vec::new()),
            "Level(1) owner path should project to the root for a one-level facet"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_wrap_inside_facet_wrap_preserves_logical_facet_scope()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 1.0, 10.0),
                    ('G2', 2.0, 20.0),
                    ('G3', 3.0, 30.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = crate::plot::Plot::<RepeatWrap>::new().configure_coord(|c| {
            c.items(repeat_vars(&["a", "b"]))
                .columns(2)
                .cell(repeated_item_cell())
                .item_domains_with_scope(CoordinationScope::Free)
        });
        let compiled = crate::plot::Chart::<FacetWrap>::new()
            .canvas_size(760.0, 420.0)
            .data(df)
            .mark(
                Subplot::new(repeat)
                    .id("wrapped_repeat")
                    .wrap_with(col("group_name"), |c| c.columns(2)),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        assert_eq!(evaluated.interaction.scopes.len(), 6);
        for scope in &evaluated.interaction.scopes {
            assert_eq!(
                scope.logical_facet_values.len(),
                1,
                "FacetWrap should contribute one logical facet level"
            );
            assert_eq!(
                scope.subplot_id_path.first().map(String::as_str),
                Some("wrapped_repeat")
            );
            assert!(
                scope
                    .child_frame_path
                    .last()
                    .and_then(|segment| segment.key.as_deref())
                    .is_some_and(|key| key.starts_with("repeat_item:")),
                "scope should carry the repeated item child frame: {scope:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn repeat_container_validates_variables_and_cell_template() {
        let ctx = SessionContext::new();
        let missing_cell = match crate::plot::Chart::<RepeatColumns>::new()
            .configure_coord(|c| c.columns(repeat_vars(&["a"])))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("missing repeat cell should fail"),
            Err(err) => err,
        };
        assert!(missing_cell.to_string().contains("requires a default"));

        let duplicate = match crate::plot::Chart::<RepeatColumns>::new()
            .configure_coord(|c| {
                c.columns(vec![
                    RepeatVariable::new("a", col("a")),
                    RepeatVariable::new("a", col("b")),
                ])
                .cell(repeated_column_cell())
            })
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate repeat variables should fail"),
            Err(err) => err,
        };
        assert!(
            duplicate
                .to_string()
                .contains("duplicate repeat columns variable id 'a'")
        );

        let empty = match crate::plot::Chart::<RepeatRows>::new()
            .configure_coord(|c| {
                c.rows(Vec::<RepeatVariable>::new())
                    .cell(repeated_row_cell())
            })
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("empty repeat variables should fail"),
            Err(err) => err,
        };
        assert!(
            empty
                .to_string()
                .contains("requires at least one repeat rows variable")
        );

        let data_dependent_predicate = match crate::plot::Chart::<RepeatGrid>::new()
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["a"]))
                    .cell(repeated_grid_cell())
                    .cell_when(
                        col("datum_value").eq(lit(1_i64)),
                        constant_y_grid_cell(99.0),
                    )
            })
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("data-dependent repeat branch predicate should fail"),
            Err(err) => err,
        };
        assert!(
            data_dependent_predicate
                .to_string()
                .contains("branch 0 predicate failed"),
            "{data_dependent_predicate}"
        );
    }

    #[tokio::test]
    async fn mark_id_is_accepted_on_regular_mark() {
        let ctx = SessionContext::new();
        crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .compile(&ctx)
            .await
            .expect("mark id compiles");
    }

    #[tokio::test]
    async fn invalid_mark_id_errors() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("bad.id").x(lit(1.0)).y(lit(1.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("invalid mark id should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Invalid mark id"));
    }

    #[tokio::test]
    async fn duplicate_sibling_mark_ids_error() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().id("points").x(lit(1.0)).y(lit(1.0)))
            .mark(Symbol::new().id("points").x(lit(2.0)).y(lit(2.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("duplicate sibling mark ids should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("Duplicate mark id 'points'"));
    }

    #[tokio::test]
    async fn repeat_context_resolves_mark_channel_placeholders_during_compile() {
        let ctx = SessionContext::new();
        let template = || {
            crate::plot::Plot::<Cartesian>::new()
                .mark(Symbol::new().x(repeat::column()).y(lit(1.0)).size(64.0))
        };

        let compiled_a =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("a", 0, 2)).await;
        let compiled_b =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("b", 1, 2)).await;

        let x_a = compiled_a.marks[0]
            .data_context()
            .channels()
            .get("x")
            .expect("x channel")
            .expr(&ctx)
            .expect("x expr");
        let x_b = compiled_b.marks[0]
            .data_context()
            .channels()
            .get("x")
            .expect("x channel")
            .expr(&ctx)
            .expect("x expr");

        assert_eq!(x_a.to_string(), "a");
        assert_eq!(x_b.to_string(), "b");
        assert!(
            collect_repeat_placeholder_kinds(&x_a)
                .expect("collect")
                .is_empty()
        );
        assert!(
            collect_repeat_placeholder_kinds(&x_b)
                .expect("collect")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn coordinate_axis_configs_merge_into_compiled_axes() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::with_coord(
            Parallel::new()
                .dimension_with("mpg", |d| d.axis(|axis| axis.title("Miles per gallon"))),
        )
        .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
        .mark(ParallelLine::new().dimension("mpg", col("mpg")))
        .compile(&ctx)
        .await
        .expect("compile parallel plot");

        assert!(
            compiled
                .axis_specs
                .contains_key(&generated_dimension_channel("mpg")),
            "coordinate-level dimension axis should be merged into compiled axis specs"
        );
    }

    #[tokio::test]
    async fn configured_parallel_dimension_without_mark_binding_errors() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::with_coord(Parallel::new().dimension("mpg"))
            .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("configured dimension without mark binding should fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("no mark binds data"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn coordinate_axis_configs_round_trip_with_compiled_plot() {
        let ctx = SessionContext::new();
        let compiled = crate::plot::Chart::with_coord(
            Parallel::new()
                .dimension_with("mpg", |d| d.axis(|axis| axis.title("Miles per gallon"))),
        )
        .data(ctx.sql("SELECT 21.0 AS mpg").await.expect("data"))
        .mark(ParallelLine::new().dimension("mpg", col("mpg")))
        .compile(&ctx)
        .await
        .expect("compile parallel plot");

        let encoded = bincode::serialize(&compiled).expect("serialize compiled plot");
        let decoded: crate::plot::compiled::CompiledPlot =
            bincode::deserialize(&encoded).expect("deserialize compiled plot");
        assert!(
            decoded
                .axis_specs
                .contains_key(&generated_dimension_channel("mpg")),
            "coordinate-level axis configs should survive compiled plot serialization"
        );
    }

    #[tokio::test]
    async fn repeat_context_resolves_transform_inputs_during_evaluate() {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .expect("record batch");
        let df = ctx.read_batch(batch).expect("dataframe");

        let template = || {
            crate::plot::Plot::<Cartesian>::new().data(df.clone()).mark(
                Symbol::new().transform_no_output(
                    Calculate::new().expr("repeated", repeat::column()),
                    |mark| mark.x(col("repeated")).y(lit(1.0)).size(64.0),
                ),
            )
        };

        let compiled_a =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("a", 0, 2)).await;
        let evaluated_a = compiled_a
            .evaluate(&ctx, None)
            .await
            .expect("evaluates with repeat-resolved transform input");
        assert!(evaluated_a.scene_graph.width > 0.0);

        let compiled_b =
            compile_with_repeat_context(template(), &ctx, column_repeat_context("b", 1, 2)).await;
        let evaluated_b = compiled_b
            .evaluate(&ctx, None)
            .await
            .expect("evaluates with second repeat-resolved transform input");
        assert!(evaluated_b.scene_graph.width > 0.0);
    }

    #[tokio::test]
    async fn repeat_grid_resolves_event_binding_placeholders() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("a", DataType::Float64, false),
                Field::new("b", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![3.0, 4.0])),
            ],
        )?;
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(repeat::cell_id().eq(lit("repeat_cell:a:b")))
            .set_store_at_start_scope(
                "brush_boxes",
                StoreUpdate::upsert_rows([StoreRow::new()
                    .field("cell_id", repeat::cell_id())
                    .field("row_id", repeat::row_id())
                    .field("column_id", repeat::column_id())]),
            )
            .set_selection_at_start_scope(
                "brush",
                SelectionUpdate::upsert_clause(
                    SelectionClauseUpdate::interval(repeat::cell_id())
                        .dimension(repeat::column())
                        .endpoints(lit(1.0), lit(2.0))
                        .dimension(repeat::row())
                        .endpoints(lit(3.0), lit(4.0))
                        .build(),
                ),
            )
            .set_selection_at_start_scope(
                "field_pick",
                SelectionUpdate::replace_clause(
                    SelectionClauseUpdate::equality(lit("active"))
                        .dimension_datum_named("column", repeat::column_name())
                        .build(),
                ),
            );
        let compiled = crate::plot::Chart::<RepeatGrid>::new()
            .store(
                Store::empty("brush_boxes")
                    .field("cell_id", DataType::Utf8, false)
                    .field("row_id", DataType::Utf8, false)
                    .field("column_id", DataType::Utf8, false)
                    .primary_key(["cell_id"]),
            )
            .selection(Selection::new("brush"))
            .selection(Selection::new("field_pick"))
            .data(ctx.read_batch(batch)?)
            .configure_coord(|c| {
                c.rows(repeat_vars(&["a"]))
                    .columns(repeat_vars(&["b"]))
                    .cell(repeated_grid_cell().event_binding(binding))
            })
            .compile(&ctx)
            .await?;

        let bindings = compiled
            .event_bindings()
            .iter()
            .filter(|binding| binding.event_type == ChartEventType::Click)
            .collect::<Vec<_>>();
        assert_eq!(bindings.len(), 1);
        let binding = bindings[0];
        let filter = binding.filters[0].to_expr(&ctx)?;
        assert_eq!(
            simplify_to_scalar_sync(filter)?,
            ScalarValue::Boolean(Some(true))
        );

        let StoreUpdate::UpsertRows { rows } = &binding.action.store_steps().next().unwrap().update
        else {
            panic!("expected store upsert");
        };
        let cell_id = rows[0]
            .fields
            .get("cell_id")
            .expect("cell id field")
            .to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(cell_id)?,
            ScalarValue::Utf8(Some("repeat_cell:a:b".to_string()))
        );
        let row_id = rows[0]
            .fields
            .get("row_id")
            .expect("row id field")
            .to_expr()?;
        let column_id = rows[0]
            .fields
            .get("column_id")
            .expect("column id field")
            .to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(row_id)?,
            ScalarValue::Utf8(Some("a".to_string()))
        );
        assert_eq!(
            simplify_to_scalar_sync(column_id)?,
            ScalarValue::Utf8(Some("b".to_string()))
        );

        let SelectionUpdate::UpsertClauses { clauses } =
            &binding.action.selection_steps().next().unwrap().update
        else {
            panic!("expected selection upsert");
        };
        let clause_id = clauses[0].id.to_expr()?;
        assert_eq!(
            simplify_to_scalar_sync(clause_id)?,
            ScalarValue::Utf8(Some("repeat_cell:a:b".to_string()))
        );
        let SelectionPredicateUpdate::Interval { dimensions } = &clauses[0].predicate else {
            panic!("expected interval predicate");
        };
        assert_eq!(dimensions[0].field_expr.to_expr(&ctx)?.to_string(), "b");
        assert_eq!(dimensions[1].field_expr.to_expr(&ctx)?.to_string(), "a");

        let SelectionUpdate::ReplaceAllClauses { clauses } =
            &binding.action.selection_steps().nth(1).unwrap().update
        else {
            panic!("expected selection replacement");
        };
        let SelectionPredicateUpdate::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions[0].field_expr.to_expr(&ctx)?.to_string(), "b");
        assert_eq!(
            dimensions[0].value.to_expr()?.to_string(),
            "__event_datum_b"
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeat_placeholder_without_context_errors_during_compile() {
        let ctx = SessionContext::new();
        let err = match crate::plot::Chart::<Cartesian>::new()
            .mark(Symbol::new().x(repeat::column()).y(lit(1.0)))
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("repeat placeholder without context should error"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("repeat::column()"), "{err}");
    }

    #[tokio::test]
    async fn param_change_binding_compiles_and_round_trips_with_typed_value() {
        let ctx = SessionContext::new();
        let source = Param::new("source", 1_i64);
        let mirror = Param::new("mirror", 0_i64);
        let binding = ChartParamChangeBinding::on(&source)
            .filter(param_change::previous_value().not_eq(param_change::value()))
            .set_param(&mirror, param_change::value())
            .exact();
        let compiled = crate::plot::Chart::<Cartesian>::new()
            .param(source)
            .param(mirror)
            .param_change_binding(binding.clone())
            .compile(&ctx)
            .await
            .expect("compile reaction");
        let compiled_binding = compiled
            .param_change_bindings()
            .first()
            .expect("compiled reaction");
        assert_eq!(
            compiled_binding.source_param_name,
            binding.source_param_name
        );
        assert_eq!(compiled_binding.filters, binding.filters);
        assert_eq!(compiled_binding.action, binding.action);
        assert!(compiled_binding.resolved_source().is_some());
        assert_eq!(compiled_binding.resolved_actions().len(), 1);

        let bytes = bincode::serialize(&compiled).expect("serialize compiled reaction");
        let restored: CompiledPlot =
            bincode::deserialize(&bytes).expect("deserialize compiled reaction");
        assert_eq!(
            restored.param_change_bindings(),
            compiled.param_change_bindings()
        );
    }

    #[tokio::test]
    async fn child_param_change_binding_merges_once_across_structural_copies() {
        let ctx = SessionContext::new();
        let source = Param::new("source", 1_i64);
        let mirror = Param::new("mirror", 0_i64);
        let binding = ChartParamChangeBinding::on(&source)
            .set_param(&mirror, param_change::value())
            .exact();
        let child = crate::plot::Plot::<Cartesian>::new().param_change_binding(binding.clone());
        let compiled = crate::plot::Chart::<HConcat>::new()
            .param(source)
            .param(mirror)
            .mark(Subplot::new(child.clone()).name("left").id("left"))
            .mark(Subplot::new(child).name("right").id("right"))
            .compile(&ctx)
            .await
            .expect("compile child reactions");
        let compiled_binding = compiled
            .param_change_bindings()
            .first()
            .expect("merged compiled reaction");
        assert_eq!(compiled.param_change_bindings().len(), 1);
        assert_eq!(
            compiled_binding.source_param_name,
            binding.source_param_name
        );
        assert_eq!(compiled_binding.action, binding.action);
        assert!(compiled_binding.resolved_source().is_some());
        assert_eq!(compiled_binding.resolved_actions().len(), 1);
    }

    #[tokio::test]
    async fn param_change_binding_rejects_unknown_and_non_shared_params() {
        let ctx = SessionContext::new();
        let source = Param::new("source", 1_i64);
        let mirror = Param::new("mirror", 0_i64);

        let error = crate::plot::Chart::<Cartesian>::new()
            .param(mirror.clone())
            .param_change_binding(
                ChartParamChangeBinding::on("missing").set_param(&mirror, param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("unknown source should fail");
        assert!(error.to_string().contains("unknown source param 'missing'"));

        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source).set_param("missing", param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("unknown target should fail");
        assert!(
            error
                .to_string()
                .contains("assigns unknown param 'missing'")
        );

        let error = crate::plot::Chart::<Cartesian>::new()
            .param_with_sharing(source.clone(), CoordinationScope::Free)
            .param(mirror.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source).set_param(&mirror, param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("non-shared source should fail");
        assert!(error.to_string().contains("source 'source' must be shared"));

        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source.clone())
            .param_with_sharing(mirror.clone(), CoordinationScope::Free)
            .param_change_binding(
                ChartParamChangeBinding::on(&source).set_param(&mirror, param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("non-shared target should fail");
        assert!(error.to_string().contains("target 'mirror' must be shared"));
    }

    #[tokio::test]
    async fn param_change_binding_rejects_duplicate_writers_and_unknown_state_targets() {
        let ctx = SessionContext::new();
        let source_a = Param::new("source_a", 1_i64);
        let source_b = Param::new("source_b", 2_i64);
        let mirror = Param::new("mirror", 0_i64);
        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source_a.clone())
            .param(source_b.clone())
            .param(mirror.clone())
            .param_change_bindings([
                ChartParamChangeBinding::on(&source_a).set_param(&mirror, param_change::value()),
                ChartParamChangeBinding::on(&source_b).set_param(&mirror, param_change::value()),
            ])
            .compile(&ctx)
            .await
            .err()
            .expect("duplicate reactive writers should fail");
        assert!(error.to_string().contains("multiple reactive writers"));

        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source_a.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source_a).set_store("missing", StoreUpdate::clear()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("unknown store should fail");
        assert!(
            error
                .to_string()
                .contains("updates unknown store 'missing'")
        );

        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source_a.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source_a).clear_selection("missing_selection"),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("unknown selection should fail");
        assert!(
            error
                .to_string()
                .contains("updates unknown selection 'missing_selection'")
        );
    }

    #[tokio::test]
    async fn param_change_binding_rejects_incompatible_and_event_only_expressions() {
        let ctx = SessionContext::new();
        let source = Param::new("source", true);
        let domain = Param::raw_domain("domain");
        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source.clone())
            .param(domain.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source).set_param(&domain, param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("incompatible result type should fail");
        assert!(error.to_string().contains("failed expression validation"));

        let target = Param::new("target", 0.0_f64);
        let error = crate::plot::Chart::<Cartesian>::new()
            .param(source.clone())
            .param(target.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&source)
                    .set_param(&target, avenger_chart_core::event::x()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("event-only column should fail");
        assert!(error.to_string().contains("failed expression validation"));

        let number_source = Param::new("number_source", 1_i64);
        let error = crate::plot::Chart::<Cartesian>::new()
            .param(number_source.clone())
            .param_change_binding(
                ChartParamChangeBinding::on(&number_source).filter(param_change::value()),
            )
            .compile(&ctx)
            .await
            .err()
            .expect("non-boolean filter should fail");
        assert!(error.to_string().contains("failed expression validation"));
    }
}
