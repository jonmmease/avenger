// Lowering returns source-rich diagnostics internally. Boxing every helper's
// error would add pervasive indirection without shrinking the public failure
// type or changing the one-diagnostic-at-a-time lowering control flow.
#![allow(clippy::result_large_err)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow::{
    datatypes::{
        DataType, Field, Fields, IntervalUnit as ArrowIntervalUnit, Schema,
        TimeUnit as ArrowTimeUnit,
    },
    record_batch::RecordBatch,
};
use avenger_chart::{
    layout::LayoutSpec,
    prelude::{
        Auto, ChannelExpr, ChannelValue, LegendableChannelValue, Param, Scale, ScaleChannelValue,
        Selection, Store, WidgetItemRow, WidgetItems,
    },
};
use avenger_chart_core::{
    DataTransformExecutionContext, DataTransformStage, FormattingContext, Param as ChartParam,
    PatternAnchor, PatternChannelValue, PatternFill, PatternInk, PatternLayer,
    PatternLayerOperation, StripeDash, StripePatternLayer, Theme, TimeContext, WeekStart,
};
use avenger_chart_lang_registry::{
    NativeOutputValue, NativeRegistry, NativeTransformMode, ResolvedChildPlot,
    ResolvedDeclaration as NativeDeclaration, ResolvedMark, ResolvedMarkGroup, ResolvedPlot,
    ResolvedTransformStage, ResolvedValue as NativeValue,
};
use avenger_chart_schema::{NativeKindKey, NativeKindNamespace, ValueShape};
use avenger_lang_core::{
    DeclarationId, Diagnostic, IntervalUnit, ParamId, PhysicalField, PhysicalType, ResolvedBinding,
    ResolvedDeclaration, ResolvedExpression, ResolvedOutputHandle, ResolvedOutputShape,
    ResolvedParam, ResolvedProject, ResolvedQuery, ResolvedSelection, ResolvedSelectionCombine,
    ResolvedSelectionEmpty, ResolvedSqlReference, ResolvedStore, ResolvedTarget, ResolvedValue,
    SelectionId, SourceLabel, SourceSpan, StateSharing, StoreId, TimeUnit, ast::BindingTime,
};
use datafusion::{
    common::{
        ScalarValue,
        tree_node::{Transformed, TreeNode},
    },
    dataframe::DataFrame,
    logical_expr::{Expr, col, lit},
    prelude::SessionContext,
};
use indexmap::IndexMap;

use crate::{
    CompiledChartArtifact, CompiledProject, DependencyFingerprint, ProjectChartId,
    ProjectFingerprint,
};

pub(crate) struct LoweredProject {
    pub charts: Vec<LoweredChart>,
    pub analysis_schemas: Vec<(DeclarationId, SourceSpan, Arc<Schema>)>,
}

pub(crate) struct LoweredChart {
    pub artifact: CompiledChartArtifact,
}

pub(crate) async fn lower_project(
    project: &ResolvedProject,
    registry: &NativeRegistry,
    context: &SessionContext,
) -> Result<LoweredProject, Vec<Diagnostic>> {
    let mut lowerer = ProjectLowerer::new(project, registry, context);
    match lowerer.lower().await {
        Ok(project) => Ok(project),
        Err(diagnostic) => Err(vec![diagnostic]),
    }
}

struct ProjectLowerer<'a> {
    project: &'a ResolvedProject,
    registry: &'a NativeRegistry,
    context: &'a SessionContext,
    params: BTreeMap<ParamId, Param>,
    stores: BTreeMap<StoreId, Store>,
    selections: BTreeMap<SelectionId, Selection>,
    widget_owned_params: BTreeSet<ParamId>,
    tool_owned_selections: BTreeSet<SelectionId>,
    transform_outputs: BTreeMap<ResolvedOutputHandle, NativeOutputValue>,
    analysis_schemas: Vec<(DeclarationId, SourceSpan, Arc<Schema>)>,
}

impl<'a> ProjectLowerer<'a> {
    fn new(
        project: &'a ResolvedProject,
        registry: &'a NativeRegistry,
        context: &'a SessionContext,
    ) -> Self {
        Self {
            project,
            registry,
            context,
            params: BTreeMap::new(),
            stores: BTreeMap::new(),
            selections: BTreeMap::new(),
            widget_owned_params: widget_owned_params(project),
            tool_owned_selections: tool_owned_selections(project),
            transform_outputs: BTreeMap::new(),
            analysis_schemas: Vec::new(),
        }
    }

    async fn lower(&mut self) -> Result<LoweredProject, Diagnostic> {
        self.lower_state()?;
        let mut charts = Vec::new();
        for chart_id in &self.project.charts {
            let declaration = find_declaration(self.project, chart_id).ok_or_else(|| {
                diagnostic(
                    SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                    "AVENGER-LOWER-001",
                    "resolved chart declaration is missing",
                    chart_id.to_string(),
                )
            })?;
            let plot = self.lower_chart(declaration).await?;
            let compiled = self
                .registry
                .compile_root(&plot, self.context)
                .await
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            let mut artifact = CompiledChartArtifact::new(
                ProjectChartId::new(declaration.id.as_str()),
                declaration.name.clone(),
                declaration.source,
                Arc::new(compiled),
                self.registry.profile_id().clone(),
                DependencyFingerprint::new(self.project.source_fingerprint.clone()),
            );
            self.enrich_interface(&mut artifact, declaration);
            charts.push(LoweredChart { artifact });
        }
        Ok(LoweredProject {
            charts,
            analysis_schemas: std::mem::take(&mut self.analysis_schemas),
        })
    }

    fn lower_state(&mut self) -> Result<(), Diagnostic> {
        for id in &self.project.param_default_order {
            self.lower_param(id)?;
        }
        // Native tool/widget exports have schema-provided defaults and do not
        // participate in the authored param-default DAG. They still need the
        // exact same typed parameter representation before their paired
        // lowerers run.
        for id in self.project.params.keys() {
            if !self.params.contains_key(id) {
                self.lower_param(id)?;
            }
        }
        for (id, store) in &self.project.stores {
            self.stores.insert(id.clone(), self.lower_store(store)?);
        }
        for (id, selection) in &self.project.selections {
            self.selections
                .insert(id.clone(), self.lower_selection(selection));
        }
        Ok(())
    }

    fn enrich_interface(&self, artifact: &mut CompiledChartArtifact, chart: &ResolvedDeclaration) {
        for (id, param) in &self.project.params {
            if !belongs_to_chart(&param.owner_ancestry, &chart.id) {
                continue;
            }
            let runtime_name = &self.params[id].name;
            if let Some(spec) = artifact.compiled.param_specs().get(runtime_name)
                && let Some(binding) = artifact.interface.params.get_mut(runtime_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = param.migration_key.as_ref().map(|key| {
                    avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
                });
            }
        }
        for (id, store) in &self.project.stores {
            if !belongs_to_chart(&store.owner_ancestry, &chart.id) {
                continue;
            }
            let runtime_name = &self.stores[id].name;
            if let Some(spec) = artifact.compiled.store_specs().get(runtime_name)
                && let Some(binding) = artifact.interface.stores.get_mut(runtime_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = store.migration_key.as_ref().map(|key| {
                    avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
                });
            }
        }
        for selection in self.project.selections.values() {
            if !belongs_to_chart(&selection.owner_ancestry, &chart.id) {
                continue;
            }
            if let Some(spec) = artifact
                .compiled
                .selection_specs()
                .get(&selection.source_name)
                && let Some(binding) = artifact
                    .interface
                    .selections
                    .get_mut(&selection.source_name)
            {
                binding.runtime_id = spec.runtime_id.as_opaque_str().to_string();
                binding.migration_key = selection.migration_key.as_ref().map(|key| {
                    avenger_chart_core::StateMigrationKey::from_compiler_identity(key.as_str())
                });
            }
        }

        let chart_path = chart.public_path.as_deref().or(chart.name.as_deref());
        let compiled_mark_aliases = compiled_mark_aliases(&artifact.compiled);
        for (path, target) in &self.project.public_targets {
            if !public_path_belongs_to_chart(path, chart_path) {
                continue;
            }
            let runtime_id = match target {
                ResolvedTarget::Param(id) => self.params.get(id).and_then(|param| {
                    artifact
                        .compiled
                        .param_specs()
                        .get(&param.name)
                        .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                }),
                ResolvedTarget::Store(id) => self.stores.get(id).and_then(|store| {
                    artifact
                        .compiled
                        .store_specs()
                        .get(&store.name)
                        .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                }),
                ResolvedTarget::Selection(id) => {
                    self.project.selections.get(id).and_then(|value| {
                        artifact
                            .compiled
                            .selection_specs()
                            .get(&value.source_name)
                            .map(|spec| spec.runtime_id.as_opaque_str().to_string())
                    })
                }
                ResolvedTarget::Mark(_) => {
                    compiled_mark_id_for_public_path(&compiled_mark_aliases, path, chart_path)
                }
                ResolvedTarget::Widget(_) => find_declaration_by_target(self.project, target)
                    .and_then(|declaration| widget_source_id(declaration))
                    .and_then(|id| {
                        artifact
                            .compiled
                            .widgets()
                            .iter()
                            .find(|attachment| attachment.widget.id() == id)
                            .map(|attachment| attachment.instance_id.as_opaque_str().to_string())
                    }),
                ResolvedTarget::Part { declaration, .. } => {
                    find_declaration(self.project, declaration).and_then(|owner| {
                        if owner.keyword == "widget" {
                            compiled_widget_part_id(&artifact.compiled, path, chart_path)
                        } else {
                            compiled_mark_id_for_public_path(
                                &compiled_mark_aliases,
                                path,
                                chart_path,
                            )
                        }
                    })
                }
                _ => None,
            };
            if let Some(runtime_id) = runtime_id {
                artifact
                    .interface
                    .public_targets
                    .insert(path.clone(), runtime_id.clone());
                if let ResolvedTarget::Param(id) = target
                    && self.project.params[id]
                        .generated_by
                        .as_ref()
                        .is_some_and(|origin| {
                            find_declaration(self.project, &origin.declaration)
                                .is_some_and(|declaration| declaration.keyword == "widget")
                        })
                {
                    artifact
                        .interface
                        .widget_exports
                        .insert(path.clone(), runtime_id);
                }
            }
        }
    }

    fn lower_param(&mut self, id: &ParamId) -> Result<(), Diagnostic> {
        let param = &self.project.params[id];
        let data_type = physical_data_type(&param.data_type);
        let default = self.typed_scalar(&param.default, &data_type, param)?;
        let runtime_name = self.param_runtime_name(param);
        self.params
            .insert(id.clone(), Param::new(runtime_name, default));
        Ok(())
    }

    fn param_runtime_name(&self, param: &ResolvedParam) -> String {
        let Some(origin) = &param.generated_by else {
            return param.source_name.clone();
        };
        let Some(declaration) = find_declaration(self.project, &origin.declaration) else {
            return param.source_name.clone();
        };
        if declaration.keyword == "widget"
            && let Some(id) = declaration.name.as_deref()
        {
            let role = match origin.export_role.as_str() {
                "cursor_position" => "cursor",
                role => role,
            };
            return format!("{id}__{role}");
        }
        param.source_name.clone()
    }

    fn lower_selection(&self, selection: &ResolvedSelection) -> Selection {
        let runtime_name = selection
            .generated_by
            .as_ref()
            .and_then(|origin| {
                let declaration = find_declaration(self.project, &origin.declaration)?;
                if declaration.keyword != "widget" {
                    return None;
                }
                let id = declaration.name.as_deref()?;
                Some(format!("{id}__{}", origin.export_role))
            })
            .unwrap_or_else(|| selection.source_name.clone());
        let mut lowered = Selection::new(runtime_name);
        lowered = match selection.empty {
            ResolvedSelectionEmpty::All => lowered.empty_selects_all(),
            ResolvedSelectionEmpty::None => lowered.empty_selects_nothing(),
        };
        lowered.combine(match selection.combine {
            ResolvedSelectionCombine::Union => avenger_chart_core::SelectionCombine::Union,
            ResolvedSelectionCombine::Intersect => avenger_chart_core::SelectionCombine::Intersect,
        })
    }

    fn typed_scalar(
        &self,
        value: &ResolvedValue,
        data_type: &DataType,
        param: &ResolvedParam,
    ) -> Result<ScalarValue, Diagnostic> {
        if let ResolvedValue::Binding(ResolvedBinding {
            target: ResolvedTarget::Param(id),
            time: BindingTime::Current,
            ..
        }) = value
        {
            return self
                .params
                .get(id)
                .map(|param| param.default.clone())
                .ok_or_else(|| {
                    lowerer_error_at(
                        param.declaration.clone(),
                        self.project,
                        "parameter default dependency was not lowered first",
                    )
                });
        }
        let result = match value {
            ResolvedValue::Null => ScalarValue::try_new_null(data_type),
            ResolvedValue::String(value)
            | ResolvedValue::Number(value)
            | ResolvedValue::Atom(value) => ScalarValue::try_from_string(value.clone(), data_type),
            ResolvedValue::Boolean(value) => {
                ScalarValue::try_from_string(value.to_string(), data_type)
            }
            _ => {
                return Err(lowerer_error_at(
                    param.declaration.clone(),
                    self.project,
                    "Phase 5 parameter defaults must currently be typed literals or earlier parameter bindings",
                ));
            }
        };
        result.map_err(|error| {
            lowerer_error_at(
                param.declaration.clone(),
                self.project,
                format!("invalid {:?} parameter default: {error}", data_type),
            )
        })
    }

    fn lower_store(&self, store: &ResolvedStore) -> Result<Store, Diagnostic> {
        let fields = store.fields.iter().map(physical_field).collect::<Vec<_>>();
        let schema = Arc::new(Schema::new(fields.clone()));
        let mut lowered = if store.rows.is_empty() {
            Store::new(store.source_name.clone(), schema)
        } else {
            let columns = fields
                .iter()
                .map(|field| {
                    let values = store
                        .rows
                        .iter()
                        .map(|row| {
                            let value = row.get(field.name()).unwrap_or(&ResolvedValue::Null);
                            scalar_literal(value, field.data_type())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    ScalarValue::iter_to_array(values).map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, String>>()
                .map_err(|error| {
                    lowerer_error_at(store.declaration.clone(), self.project, error)
                })?;
            let batch = RecordBatch::try_new(schema, columns).map_err(|error| {
                lowerer_error_at(store.declaration.clone(), self.project, error.to_string())
            })?;
            Store::from_record_batch(store.source_name.clone(), batch)
        };
        lowered = lowered
            .primary_key(store.primary_key.clone())
            .sharing(sharing(store.sharing));
        Ok(lowered)
    }

    async fn lower_chart(
        &mut self,
        chart: &ResolvedDeclaration,
    ) -> Result<ResolvedPlot, Diagnostic> {
        let coordinate = chart
            .kind
            .clone()
            .ok_or_else(|| lowerer_error(chart, "chart coordinate kind was not resolved"))?;
        let mut plot = ResolvedPlot::new(coordinate);
        let data = match chart.properties.get("data") {
            Some(value) => Some(self.lower_data(value, chart).await?),
            None => None,
        };
        plot.coordinate =
            self.native_declaration(chart, data.as_ref(), NativeKindNamespace::Coordinate)?;
        plot.data = data.clone();
        if let Some(data) = &data {
            self.record_schema(chart, data);
        }

        if let Some(value) = chart.properties.get("title") {
            plot.furnishings.title = Some(self.expression_value(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("subtitle") {
            plot.furnishings.subtitle = Some(self.expression_value(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("layout") {
            plot.furnishings.layout = Some(self.lower_layout(value, data.as_ref(), chart)?);
        }
        if let Some(value) = chart.properties.get("time") {
            plot.furnishings.time_context = self.lower_time_context(value, chart)?;
        }
        if let Some(value) = chart.properties.get("format") {
            plot.furnishings.formatting_context = self.lower_formatting_context(value, chart)?;
        }
        let mut theme = None;
        for declaration in chart
            .children
            .iter()
            .filter(|child| child.keyword == "theme")
        {
            let Some(ResolvedValue::String(css)) = declaration.properties.get("css") else {
                return Err(lowerer_error(
                    declaration,
                    "theme css from requires project resource loading, which is not available in this lowering context",
                ));
            };
            match &mut theme {
                None => {
                    theme = Some(Theme::from_css(css).map_err(|error| {
                        lowerer_error(declaration, format!("invalid theme CSS: {error}"))
                    })?);
                }
                Some(theme) => theme.append_css(css).map_err(|error| {
                    lowerer_error(declaration, format!("invalid theme CSS: {error}"))
                })?,
            }
        }
        plot.furnishings.theme = theme;

        for (id, param) in &self.project.params {
            if param.table_owner.is_none()
                && belongs_to_chart(&param.owner_ancestry, &chart.id)
                && !self.widget_owned_params.contains(id)
            {
                plot.furnishings
                    .params
                    .push((self.params[id].clone(), sharing(param.sharing)));
            }
        }
        for (id, store) in &self.project.stores {
            if belongs_to_chart(&store.owner_ancestry, &chart.id) {
                plot.furnishings.stores.push(self.stores[id].clone());
            }
        }
        for (id, selection) in &self.project.selections {
            if belongs_to_chart(&selection.owner_ancestry, &chart.id)
                && selection.generated_by.is_none()
                && !self.tool_owned_selections.contains(id)
            {
                plot.furnishings
                    .selections
                    .push(self.selections[id].clone());
            }
        }

        let root_group = self
            .lower_container(chart, data.as_ref(), &mut plot)
            .await?;
        if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
            plot.marks.push(ResolvedMark::Group(Box::new(root_group)));
        }
        Ok(plot)
    }

    fn lower_time_context(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<TimeContext, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(declaration, "time must be a property block"));
        };
        let mut context = TimeContext::new();
        if let Some(value) = properties.get("timezone").and_then(resolved_string) {
            context = context.timezone(value);
        }
        if let Some(value) = properties.get("week_start").and_then(resolved_atom) {
            let week_start = match value {
                "sunday" => WeekStart::Sunday,
                "monday" => WeekStart::Monday,
                "tuesday" => WeekStart::Tuesday,
                "wednesday" => WeekStart::Wednesday,
                "thursday" => WeekStart::Thursday,
                "friday" => WeekStart::Friday,
                "saturday" => WeekStart::Saturday,
                other => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported week_start `{other}`"),
                    ));
                }
            };
            context = context.week_start(week_start);
        }
        Ok(context)
    }

    fn lower_formatting_context(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<FormattingContext, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(
                declaration,
                "format must be a property block",
            ));
        };
        let mut context = FormattingContext::new();
        if let Some(value) = properties.get("number_locale").and_then(resolved_string) {
            context = context.number_locale(value);
        }
        if let Some(value) = properties.get("datetime_locale").and_then(resolved_string) {
            context = context.datetime_locale(value);
        }
        if let Some(value) = properties
            .get("datetime_timezone")
            .and_then(resolved_string)
        {
            context = context.datetime_timezone(value);
        }
        Ok(context)
    }

    fn lower_container<'b>(
        &'b mut self,
        container: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
        plot: &'b mut ResolvedPlot,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ResolvedMarkGroup, Diagnostic>> + 'b>,
    > {
        Box::pin(async move {
            let explicit_data = match container.properties.get("data") {
                Some(value) if container.keyword != "chart" => {
                    Some(self.lower_data(value, container).await?)
                }
                _ => None,
            };
            let mut current_data = explicit_data.as_ref().or(inherited_data).cloned();
            if let Some(data) = &explicit_data {
                self.record_schema(container, data);
            }
            let mut group = ResolvedMarkGroup::new();
            group.data = explicit_data;
            if container.keyword == "group" {
                group.id = container.name.clone();
                group.component_kind = container.component_kind.clone();
            }

            for child in &container.children {
                match child.keyword.as_str() {
                    "transform" => {
                        let input = current_data.as_ref().ok_or_else(|| {
                            lowerer_error(
                                child,
                                "transform has no inherited or explicit data source",
                            )
                        })?;
                        let (data, stage) = self.lower_transform_stage(child, input, None).await?;
                        current_data = Some(data);
                        group.transforms.push(ResolvedTransformStage {
                            scope: stage.scope,
                            transform: stage.transform,
                        });
                    }
                    "group" => {
                        let child_group = self
                            .lower_container(child, current_data.as_ref(), plot)
                            .await?;
                        group.marks.push(ResolvedMark::Group(Box::new(child_group)));
                    }
                    "mark" => {
                        let mark_data = match child.properties.get("data") {
                            Some(value) => Some(self.lower_data(value, child).await?),
                            None => None,
                        };
                        let planning_data = mark_data.as_ref().or(current_data.as_ref());
                        let declaration = self.native_declaration(
                            child,
                            planning_data,
                            NativeKindNamespace::Mark,
                        )?;
                        let key = NativeKindKey::mark(
                            child.coordinate.as_deref().unwrap_or(""),
                            child.kind.as_deref().unwrap_or(""),
                        );
                        let owns_plot_child = self
                            .registry
                            .snapshot()
                            .entries
                            .get(&key)
                            .is_some_and(|schema| {
                                schema.child_rules.iter().any(|rule| rule.role == "plot")
                            });
                        let native = if owns_plot_child {
                            let mut plots = child
                                .children
                                .iter()
                                .filter(|nested| nested.keyword == "plot");
                            let nested = plots.next().ok_or_else(|| {
                                lowerer_error(child, "registered child mark requires one `plot`")
                            })?;
                            if plots.next().is_some() {
                                return Err(lowerer_error(
                                    child,
                                    "registered child mark accepts only one `plot`",
                                ));
                            }
                            ResolvedMark::NativeWithChild {
                                declaration,
                                child: Box::new(
                                    self.lower_nested_plot(nested, planning_data).await?,
                                ),
                            }
                        } else {
                            ResolvedMark::Native(declaration)
                        };
                        if let Some(data) = mark_data {
                            let mut wrapper = ResolvedMarkGroup::new();
                            wrapper.data = Some(data);
                            wrapper.marks.push(native);
                            group.marks.push(ResolvedMark::Group(Box::new(wrapper)));
                        } else {
                            group.marks.push(native);
                        }
                    }
                    "tool" => plot.tools.push(self.native_declaration(
                        child,
                        current_data.as_ref(),
                        NativeKindNamespace::Tool,
                    )?),
                    "widget" => {
                        let widget_data = match child.properties.get("data") {
                            Some(value) => Some(self.lower_widget_items(value, child).await?),
                            None => None,
                        };
                        let mut declaration = self.native_declaration(
                            child,
                            widget_data
                                .as_ref()
                                .map(|(_, data)| data)
                                .or(current_data.as_ref()),
                            NativeKindNamespace::Widget,
                        )?;
                        if let Some((items, _)) = widget_data {
                            declaration
                                .properties
                                .insert("data".to_string(), NativeValue::WidgetItems(items));
                        }
                        self.install_native_state_bindings(child, &mut declaration)?;
                        plot.widgets.push(declaration);
                    }
                    "cell" | "plot" => {
                        let child_plot =
                            self.lower_nested_plot(child, current_data.as_ref()).await?;
                        let placement = self.child_placement(child, current_data.as_ref())?;
                        plot.children.push(ResolvedChildPlot {
                            plot: Box::new(child_plot),
                            placement,
                        });
                    }
                    "param" | "store" | "selection" | "on" | "theme" | "resource" => {}
                    other => {
                        let coordinate_key = NativeKindKey::new(
                            NativeKindNamespace::Coordinate,
                            plot.coordinate.kind.clone(),
                        );
                        if self
                            .registry
                            .snapshot()
                            .entries
                            .get(&coordinate_key)
                            .is_some_and(|schema| {
                                schema.child_rules.iter().any(|rule| rule.role == other)
                            })
                        {
                            continue;
                        }
                        return Err(lowerer_error(
                            child,
                            format!("Phase 5 cannot lower `{other}` declarations yet"),
                        ));
                    }
                }
            }
            Ok(group)
        })
    }

    fn lower_nested_plot<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolvedPlot, Diagnostic>> + 'b>>
    {
        Box::pin(async move {
            let coordinate = declaration.kind.clone().ok_or_else(|| {
                lowerer_error(declaration, "child plot coordinate kind was not resolved")
            })?;
            let mut plot = ResolvedPlot::new(coordinate);
            let explicit_data = match declaration.properties.get("data") {
                Some(value) => Some(self.lower_data(value, declaration).await?),
                None => None,
            };
            if let Some(data) = &explicit_data {
                self.record_schema(declaration, data);
            }
            // A nested plot is compiled as an independently typed child plot.
            // Preserve lexical data inheritance by installing the current
            // DataFrame when the child does not author an override; otherwise
            // the child compiler would have no schema or runtime relation even
            // though its DSL scope correctly resolved inherited columns.
            plot.data = explicit_data.clone().or_else(|| inherited_data.cloned());
            plot.data_is_inherited = explicit_data.is_none() && inherited_data.is_some();
            plot.coordinate = self.native_declaration(
                declaration,
                plot.data.as_ref(),
                NativeKindNamespace::Coordinate,
            )?;
            let planning_data = plot.data.clone();
            let root_group = self
                .lower_container(declaration, planning_data.as_ref(), &mut plot)
                .await?;
            if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
                plot.marks.push(ResolvedMark::Group(Box::new(root_group)));
            }
            Ok(plot)
        })
    }

    fn lower_transform_stage<'b>(
        &'b mut self,
        declaration: &'b ResolvedDeclaration,
        input: &'b DataFrame,
        inherited_scope: Option<avenger_chart_core::CoordinationScope>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(DataFrame, DataTransformStage), Diagnostic>>
                + 'b,
        >,
    > {
        Box::pin(async move {
            let kind = declaration.kind.as_deref().ok_or_else(|| {
                lowerer_error(declaration, "native transform kind was not resolved")
            })?;
            let scope = self.transform_scope(declaration, inherited_scope)?;
            let native =
                self.native_declaration(declaration, Some(input), NativeKindNamespace::Transform)?;
            let context = avenger_chart_core::DataTransformCompileContext::new(scope);
            let lowered = match self
                .registry
                .transform_mode(kind)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?
            {
                NativeTransformMode::Leaf => self
                    .registry
                    .lower_transform(&native, context)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?,
                NativeTransformMode::Pipeline => {
                    let mut child_data = input.clone();
                    let mut stages = Vec::new();
                    for child in declaration
                        .children
                        .iter()
                        .filter(|child| child.keyword == "transform")
                    {
                        let (next_data, stage) = self
                            .lower_transform_stage(child, &child_data, Some(scope))
                            .await?;
                        child_data = next_data;
                        stages.push(stage);
                    }
                    let mut outputs = BTreeMap::new();
                    for output in declaration
                        .children
                        .iter()
                        .filter(|child| child.keyword == "output")
                    {
                        let name = output.name.clone().ok_or_else(|| {
                            lowerer_error(output, "pipeline output has no resolved name")
                        })?;
                        let expr = match output.properties.get("value") {
                            Some(value) => {
                                self.expression_value(value, Some(&child_data), output)?
                            }
                            None => {
                                child_data
                                    .schema()
                                    .field_with_unqualified_name(&name)
                                    .map_err(|error| lowerer_error(output, error.to_string()))?;
                                col(&name)
                            }
                        };
                        outputs.insert(name, expr);
                    }
                    self.registry
                        .lower_transform_pipeline(&native, stages, outputs, context)
                        .map_err(|error| lowerer_error(declaration, error.to_string()))?
                }
            };

            let params = self
                .params
                .values()
                .map(|param| (param.name.clone(), param.default.clone()))
                .collect::<IndexMap<_, _>>();
            let execution = DataTransformExecutionContext {
                session_context: self.context,
                params: &params,
                time_context: TimeContext::default(),
                facet_context: None,
            };
            let result = lowered
                .transform
                .apply(input.clone(), &execution)
                .await
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            self.install_transform_outputs(declaration, &lowered.outputs)?;
            self.record_schema(declaration, &result.dataframe);
            Ok((
                result.dataframe,
                DataTransformStage::new(scope, lowered.transform),
            ))
        })
    }

    fn transform_scope(
        &self,
        declaration: &ResolvedDeclaration,
        inherited: Option<avenger_chart_core::CoordinationScope>,
    ) -> Result<avenger_chart_core::CoordinationScope, Diagnostic> {
        let Some(value) = declaration.properties.get("scope") else {
            return Ok(inherited.unwrap_or(avenger_chart_core::CoordinationScope::Free));
        };
        match value {
            ResolvedValue::Atom(value) if value == "shared" => {
                Ok(avenger_chart_core::CoordinationScope::Shared)
            }
            ResolvedValue::Atom(value) if value == "free" => {
                Ok(avenger_chart_core::CoordinationScope::Free)
            }
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "level(...) transform scope requires one integer",
                    ));
                };
                let level = level.parse::<u8>().map_err(|_| {
                    lowerer_error(
                        declaration,
                        "transform scope level must be an integer from 0 through 255",
                    )
                })?;
                Ok(avenger_chart_core::CoordinationScope::Level(level))
            }
            _ => Err(lowerer_error(
                declaration,
                "transform scope must be shared, free, or level(<integer>)",
            )),
        }
    }

    fn install_transform_outputs(
        &mut self,
        declaration: &ResolvedDeclaration,
        outputs: &BTreeMap<String, NativeOutputValue>,
    ) -> Result<(), Diagnostic> {
        for (name, handle) in &declaration.transform_outputs {
            let Some(value) = outputs.get(name) else {
                continue;
            };
            let matches = matches!(
                (&handle.shape, value),
                (
                    ResolvedOutputShape::Expression,
                    NativeOutputValue::Expr(_) | NativeOutputValue::Channel(_)
                ) | (
                    ResolvedOutputShape::RasterDimension,
                    NativeOutputValue::RasterDim(_)
                ) | (ResolvedOutputShape::Opaque, NativeOutputValue::Opaque(_))
            );
            if !matches {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "registered output `{name}` has native type incompatible with {:?}",
                        handle.shape
                    ),
                ));
            }
            self.transform_outputs.insert(handle.clone(), value.clone());
        }
        Ok(())
    }

    fn child_placement(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let mut placement = NativeDeclaration::new("child");
        placement.source_name.clone_from(&declaration.name);
        if let Some(ResolvedValue::Object { properties, .. }) = declaration.properties.get("at") {
            for (name, value) in properties {
                placement
                    .properties
                    .insert(name.clone(), self.native_value(value, data, declaration)?);
            }
        }
        if let Some(label) = declaration.properties.get("label") {
            placement.properties.insert(
                "label".to_string(),
                self.native_value(label, data, declaration)?,
            );
        }
        if let Some(predicate) = declaration.properties.get("when") {
            placement.properties.insert(
                "when".to_string(),
                NativeValue::Expr(self.expression_value(predicate, data, declaration)?),
            );
        }
        Ok(placement)
    }

    fn native_declaration(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
        namespace: NativeKindNamespace,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let kind = declaration.kind.clone().ok_or_else(|| {
            lowerer_error(declaration, "native declaration kind was not resolved")
        })?;
        let key = match namespace {
            NativeKindNamespace::Mark => {
                NativeKindKey::mark(declaration.coordinate.as_deref().unwrap_or(""), &kind)
            }
            _ => NativeKindKey::new(namespace, &kind),
        };
        let schema = self.registry.snapshot().entries.get(&key).ok_or_else(|| {
            lowerer_error(
                declaration,
                format!("native registry has no schema/lowerer pair for {key:?}"),
            )
        })?;
        let mut native = NativeDeclaration::new(kind.clone());
        native.source_name.clone_from(&declaration.name);
        native.live_exports = declaration.exports.keys().cloned().collect();
        for (name, value) in &declaration.properties {
            if is_core_property(&declaration.keyword, name) {
                continue;
            }
            let property_shape = schema.properties.get(name).map(|property| &property.shape);
            // Widget data is lowered asynchronously at the container call
            // site and installed after this synchronous schema-directed pass.
            if property_shape == Some(&ValueShape::WidgetData) {
                continue;
            }
            // A mark declaration contains both encoding channels and ordinary
            // native properties (for example Text.syntax and raster
            // dimensions). The registry schema, not the `mark` keyword,
            // decides which lowering path applies.
            let lowered = if let Some(channel) = schema.channels.get(name) {
                if channel.shape == ValueShape::PatternChannel {
                    self.pattern_channel_value(value, data, declaration)?
                } else {
                    self.channel_value(value, data, declaration)?
                }
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::ChannelMap)
            {
                self.channel_map_value(value, data, declaration)?
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::RasterDimensionChannel)
            {
                self.raster_dimension_channel_value(name, value, data, declaration)?
            } else if let Some(ValueShape::ConfiguredExpression(fields)) = property_shape {
                self.configured_expression_value(value, fields, data, declaration)?
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::FacetDataScope)
            {
                NativeValue::Integer(self.facet_data_scope_level(value, declaration)?.into())
            } else if property_shape == Some(&ValueShape::CoordinationScope) {
                match self.coordination_scope(value, declaration)? {
                    avenger_chart_core::CoordinationScope::Shared => {
                        NativeValue::String("shared".to_string())
                    }
                    avenger_chart_core::CoordinationScope::Free => {
                        NativeValue::String("free".to_string())
                    }
                    avenger_chart_core::CoordinationScope::Level(level) => {
                        NativeValue::Integer(level.into())
                    }
                }
            } else if matches!(
                property_shape,
                Some(ValueShape::Array(inner)) if inner.as_ref() == &ValueShape::SqlExpression
            ) {
                let ResolvedValue::Array(values) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must be an array of SQL expressions"),
                    ));
                };
                NativeValue::Array(
                    values
                        .iter()
                        .map(|value| {
                            self.expression_value(value, data, declaration)
                                .map(NativeValue::Expr)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            } else if schema
                .properties
                .get(name)
                .is_some_and(|property| property.shape == ValueShape::SqlExpression)
            {
                NativeValue::Expr(self.expression_value(value, data, declaration)?)
            } else if property_shape == Some(&ValueShape::SelectionBinding) {
                let ResolvedValue::Reference(reference) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must reference a selection"),
                    ));
                };
                let ResolvedTarget::Selection(id) = &reference.target else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must reference a selection"),
                    ));
                };
                NativeValue::Selection(self.selections.get(id).cloned().ok_or_else(|| {
                    lowerer_error(declaration, "resolved selection is unavailable")
                })?)
            } else if property_shape == Some(&ValueShape::ScalarBinding) {
                let ResolvedValue::Binding(binding) = value else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must bind a parameter"),
                    ));
                };
                let ResolvedTarget::Param(id) = &binding.target else {
                    return Err(lowerer_error(
                        declaration,
                        format!("property `{name}` must bind a parameter"),
                    ));
                };
                NativeValue::Param(self.params.get(id).cloned().ok_or_else(|| {
                    lowerer_error(declaration, "resolved parameter is unavailable")
                })?)
            } else {
                self.native_value(value, data, declaration)?
            };
            native.properties.insert(name.clone(), lowered);
        }
        for child in &declaration.children {
            // Coordinate-owned structural children (for example parallel
            // dimensions) are consumed directly by the coordinate lowerer.
            // Transform pipeline children have a dedicated ordered lowering
            // path below: forwarding them here would resolve output handles
            // before the preceding child stages have installed those outputs.
            if namespace == NativeKindNamespace::Coordinate
                && !matches!(child.keyword.as_str(), "cell" | "plot")
                && schema
                    .child_rules
                    .iter()
                    .any(|rule| rule.role == child.keyword)
            {
                native.children.push(self.native_owner_child(child, data)?);
            }
        }
        Ok(native)
    }

    fn configured_expression_value(
        &self,
        value: &ResolvedValue,
        fields: &std::collections::BTreeMap<String, avenger_chart_schema::PropertySchema>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let ResolvedValue::Object {
            head: Some(head),
            properties,
            ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "configured expression requires a value head and property block",
            ));
        };
        let properties = properties
            .iter()
            .map(|(name, value)| {
                let field = fields.get(name).ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        format!("unsupported configured-expression property `{name}`"),
                    )
                })?;
                let value = match &field.shape {
                    ValueShape::SqlExpression => {
                        NativeValue::Expr(self.expression_value(value, data, declaration)?)
                    }
                    ValueShape::CoordinationScope => {
                        match self.coordination_scope(value, declaration)? {
                            avenger_chart_core::CoordinationScope::Shared => {
                                NativeValue::String("shared".to_string())
                            }
                            avenger_chart_core::CoordinationScope::Free => {
                                NativeValue::String("free".to_string())
                            }
                            avenger_chart_core::CoordinationScope::Level(level) => {
                                NativeValue::Integer(level.into())
                            }
                        }
                    }
                    _ => self.native_value(value, data, declaration)?,
                };
                Ok((name.clone(), value))
            })
            .collect::<Result<_, Diagnostic>>()?;
        Ok(NativeValue::Configured {
            head: Box::new(self.channel_value(head, data, declaration)?),
            properties,
        })
    }

    fn facet_data_scope_level(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<u8, Diagnostic> {
        match value {
            ResolvedValue::Atom(value) if value == "filtered" => Ok(0),
            ResolvedValue::Atom(value) if value == "broadcast" => Ok(u8::MAX),
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "facet data scope level(...) requires one integer",
                    ));
                };
                level.parse::<u8>().map_err(|_| {
                    lowerer_error(
                        declaration,
                        "facet data scope level must be an integer from 0 through 255",
                    )
                })
            }
            _ => Err(lowerer_error(
                declaration,
                "facet_data_scope must be filtered, broadcast, or level(<integer>)",
            )),
        }
    }

    fn coordination_scope(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<avenger_chart_core::CoordinationScope, Diagnostic> {
        match value {
            ResolvedValue::Atom(value) if value == "shared" => {
                Ok(avenger_chart_core::CoordinationScope::Shared)
            }
            ResolvedValue::Atom(value) if value == "free" => {
                Ok(avenger_chart_core::CoordinationScope::Free)
            }
            ResolvedValue::Call { function, args } if function == "level" => {
                let [ResolvedValue::Number(level)] = args.as_slice() else {
                    return Err(lowerer_error(
                        declaration,
                        "coordination scope level(...) requires one integer",
                    ));
                };
                level
                    .parse::<u8>()
                    .map(avenger_chart_core::CoordinationScope::Level)
                    .map_err(|_| {
                        lowerer_error(
                            declaration,
                            "coordination scope level must be an integer from 0 through 255",
                        )
                    })
            }
            _ => Err(lowerer_error(
                declaration,
                "coordination scope must be shared, free, or level(<integer>)",
            )),
        }
    }

    fn install_native_state_bindings(
        &self,
        source: &ResolvedDeclaration,
        native: &mut NativeDeclaration,
    ) -> Result<(), Diagnostic> {
        let kind = source
            .kind
            .as_deref()
            .ok_or_else(|| lowerer_error(source, "native widget kind was not resolved"))?;
        let key = NativeKindKey::new(NativeKindNamespace::Widget, kind);
        let schema = &self.registry.snapshot().entries[&key];
        for export in schema.exports.values() {
            let Some(property) = &export.binding_property else {
                continue;
            };
            if native.properties.contains_key(property) {
                continue;
            }
            let Some(target) = source.exports.get(&export.alias) else {
                continue;
            };
            let value = match target {
                ResolvedTarget::Param(id)
                    if self.project.params[id].generated_by.is_some()
                        && self.params[id].default.is_null() =>
                {
                    None
                }
                ResolvedTarget::Param(id) => self.params.get(id).cloned().map(NativeValue::Param),
                ResolvedTarget::Selection(id) => {
                    self.selections.get(id).cloned().map(NativeValue::Selection)
                }
                _ => None,
            };
            if let Some(value) = value {
                native.properties.insert(property.clone(), value);
            }
        }
        Ok(())
    }

    fn native_owner_child(
        &self,
        declaration: &ResolvedDeclaration,
        data: Option<&DataFrame>,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let mut native = NativeDeclaration::new(declaration.keyword.clone());
        native.variant.clone_from(&declaration.kind);
        native.source_name.clone_from(&declaration.name);
        for (name, value) in &declaration.properties {
            native
                .properties
                .insert(name.clone(), self.native_value(value, data, declaration)?);
        }
        native.children = declaration
            .children
            .iter()
            .map(|child| self.native_owner_child(child, data))
            .collect::<Result<_, _>>()?;
        Ok(native)
    }

    fn channel_map_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let ResolvedValue::Object { properties, .. } = value else {
            return Err(lowerer_error(
                declaration,
                "configured channel map did not resolve to an object",
            ));
        };
        Ok(NativeValue::Object(
            properties
                .iter()
                .map(|(name, value)| {
                    Ok((name.clone(), self.channel_value(value, data, declaration)?))
                })
                .collect::<Result<_, Diagnostic>>()?,
        ))
    }

    fn raster_dimension_channel_value(
        &self,
        property: &str,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let (dimension_value, configs) = match value {
            ResolvedValue::Object {
                head: Some(head),
                properties,
                ..
            } => (head.as_ref(), Some(properties)),
            _ => (value, None),
        };
        let NativeValue::Output(NativeOutputValue::RasterDim(dimension)) =
            self.native_value(dimension_value, data, declaration)?
        else {
            return Err(lowerer_error(
                declaration,
                "configured raster channel requires a raster dimension handle",
            ));
        };
        // Categorical overlay dimensions use an explicitly typed null seed so
        // scale inference chooses ordinal without adding a placeholder domain
        // value. Position dimensions use the same numeric seed as the native
        // RasterPositionConfig builder.
        let mut channel = if property == "fill_by" {
            ChannelValue::from(lit(ScalarValue::Utf8(None)))
        } else {
            ChannelValue::from(lit(0.0_f64)).with_scale_name(property)
        };
        if let Some(configs) = configs {
            channel = self.apply_channel_configs(channel, configs, data, declaration)?;
        }
        Ok(NativeValue::RasterDimensionChannel {
            dimension,
            channel: Box::new(channel),
        })
    }

    fn pattern_channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        if let ResolvedValue::Pattern(value) = value {
            return Ok(NativeValue::Pattern(PatternChannelValue::value(Some(
                self.pattern_fill(value, declaration)?,
            ))));
        }
        let NativeValue::Channel(channel) = self.channel_value(value, data, declaration)? else {
            unreachable!("channel lowering always returns a channel")
        };
        Ok(NativeValue::Pattern((*channel).into()))
    }

    fn pattern_fill(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<PatternFill, Diagnostic> {
        let ResolvedValue::Object {
            properties,
            children,
            ..
        } = value
        else {
            return Err(lowerer_error(
                declaration,
                "pattern literal requires a property block",
            ));
        };
        let anchor = match properties.get("anchor").and_then(resolved_atom) {
            None | Some("plot") => PatternAnchor::Plot,
            Some("mark") => PatternAnchor::Mark,
            Some("chart") => PatternAnchor::Chart,
            Some(value) => {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported pattern anchor `{value}`"),
                ));
            }
        };
        let ink = match properties.get("ink") {
            None => PatternInk::default(),
            Some(ResolvedValue::Object {
                kind: Some(kind),
                properties,
                ..
            }) if kind == "auto_contrast" => PatternInk::AutoContrast {
                opacity: resolved_f32(properties.get("opacity"), 0.18, declaration, "ink.opacity")?,
            },
            Some(ResolvedValue::Object {
                kind: Some(kind),
                properties,
                ..
            }) if kind == "solid" => {
                let color = properties
                    .get("color")
                    .and_then(resolved_string)
                    .ok_or_else(|| {
                        lowerer_error(declaration, "solid pattern ink requires `color`")
                    })?;
                PatternInk::Solid {
                    color: avenger_color::parse_color_string_strict(color).map_err(|error| {
                        lowerer_error(declaration, format!("invalid pattern ink color: {error}"))
                    })?,
                    opacity: resolved_f32(
                        properties.get("opacity"),
                        0.18,
                        declaration,
                        "ink.opacity",
                    )?,
                }
            }
            Some(_) => {
                return Err(lowerer_error(
                    declaration,
                    "pattern ink must be `auto_contrast { ... }` or `solid { ... }`",
                ));
            }
        };
        let mut layers = Vec::new();
        for layer in children {
            if layer.keyword != "layer" {
                return Err(lowerer_error(
                    declaration,
                    format!("unsupported pattern child `{}`", layer.keyword),
                ));
            }
            match layer.kind.as_deref() {
                Some("stripe") => {
                    let angle = required_resolved_f32(layer, "angle", declaration)?;
                    let spacing =
                        required_resolved_f32_alias(layer, "spacing_px", "spacing", declaration)?;
                    let stroke_width = optional_resolved_f32_alias(
                        layer,
                        "stroke_width_px",
                        "stroke_width",
                        1.0,
                        declaration,
                    )?;
                    let mut stripe = StripePatternLayer::new(angle, spacing, stroke_width);
                    stripe.phase =
                        optional_resolved_f32_alias(layer, "phase_px", "phase", 0.0, declaration)?;
                    stripe.operation =
                        pattern_operation(layer.properties.get("operation"), declaration)?;
                    if let Some(ResolvedValue::Object { properties, .. }) =
                        layer.properties.get("dash")
                    {
                        stripe.dash = Some(StripeDash {
                            length: resolved_f32(
                                properties
                                    .get("length_px")
                                    .or_else(|| properties.get("length")),
                                f32::NAN,
                                declaration,
                                "dash.length_px",
                            )?,
                            gap: resolved_f32(
                                properties.get("gap_px").or_else(|| properties.get("gap")),
                                f32::NAN,
                                declaration,
                                "dash.gap_px",
                            )?,
                            phase: resolved_f32(
                                properties
                                    .get("phase_px")
                                    .or_else(|| properties.get("phase")),
                                0.0,
                                declaration,
                                "dash.phase_px",
                            )?,
                        });
                    }
                    layers.push(PatternLayer::Stripe(stripe));
                }
                Some(kind) => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported pattern layer `{kind}`"),
                    ));
                }
                None => {
                    return Err(lowerer_error(declaration, "pattern layer requires a kind"));
                }
            }
        }
        let pattern = PatternFill {
            anchor,
            ink,
            layers,
        };
        pattern
            .validate()
            .map_err(|error| lowerer_error(declaration, format!("{error:?}")))?;
        Ok(pattern)
    }

    fn native_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return Ok(NativeValue::Output(output.clone()));
        }
        Ok(match value {
            ResolvedValue::Boolean(value) => NativeValue::Boolean(*value),
            ResolvedValue::Number(value) => value
                .parse::<i64>()
                .map(NativeValue::Integer)
                .or_else(|_| value.parse::<f64>().map(NativeValue::Number))
                .map_err(|_| {
                    lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
                })?,
            ResolvedValue::String(value) | ResolvedValue::Atom(value) => {
                NativeValue::String(value.clone())
            }
            ResolvedValue::Null => NativeValue::Scalar(ScalarValue::Null),
            ResolvedValue::Column(_)
            | ResolvedValue::Expression(_)
            | ResolvedValue::Visual(_)
            | ResolvedValue::Binding(_) => {
                NativeValue::Expr(self.expression_value(value, data, declaration)?)
            }
            ResolvedValue::Query(query) => NativeValue::Query(self.query_sql(query, declaration)?),
            ResolvedValue::Array(values) => NativeValue::Array(
                values
                    .iter()
                    .map(|value| self.native_value(value, data, declaration))
                    .collect::<Result<_, _>>()?,
            ),
            ResolvedValue::Object { properties, .. } => NativeValue::Object(
                properties
                    .iter()
                    .map(|(name, value)| {
                        Ok((name.clone(), self.native_value(value, data, declaration)?))
                    })
                    .collect::<Result<_, Diagnostic>>()?,
            ),
            ResolvedValue::Call { function, args } => NativeValue::Call {
                function: function.clone(),
                args: args
                    .iter()
                    .map(|value| self.native_value(value, data, declaration))
                    .collect::<Result<_, _>>()?,
            },
            ResolvedValue::Dimension(dimension) => NativeValue::Output(
                self.transform_outputs
                    .get(&dimension.target)
                    .cloned()
                    .ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "raster dimension `{}` is unavailable at this dataflow position",
                                dimension.authored_path.join(".")
                            ),
                        )
                    })?,
            ),
            ResolvedValue::Reference(reference)
                if matches!(reference.target, ResolvedTarget::Output(_)) =>
            {
                let ResolvedTarget::Output(handle) = &reference.target else {
                    unreachable!()
                };
                NativeValue::Output(self.transform_outputs.get(handle).cloned().ok_or_else(
                    || {
                        lowerer_error(
                            declaration,
                            format!(
                                "transform output `{}` is unavailable at this dataflow position",
                                reference.authored_path.join(".")
                            ),
                        )
                    },
                )?)
            }
            ResolvedValue::Reference(_)
            | ResolvedValue::Pattern(_)
            | ResolvedValue::Environment(_)
            | ResolvedValue::None
            | ResolvedValue::DefinitionArgument(_)
            | ResolvedValue::Invalid => {
                return Err(lowerer_error(
                    declaration,
                    format!("value `{value:?}` is not lowerable in this native slot"),
                ));
            }
        })
    }

    fn channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
        let data_expr = match value {
            ResolvedValue::Object { head, .. } => match head {
                Some(head) => self.channel_data_expr(head, data, declaration)?,
                None => lit(1.0_f64),
            },
            _ => self.channel_data_expr(value, data, declaration)?,
        };
        let mut channel = match value {
            ResolvedValue::Object {
                head, properties, ..
            } => {
                // Configuration-only channels (for example a raster's
                // opacity-by-total scale) intentionally omit a data head. A
                // scaled numeric seed asks the native owner/runtime to supply
                // the real values while retaining ordinary scale metadata.
                let channel = match head {
                    Some(head) => self.raw_channel_value(head, data, declaration)?,
                    None => ChannelValue::from(lit(1.0_f64)),
                };
                self.apply_channel_configs(channel, properties, data, declaration)?
            }
            _ => self.raw_channel_value(value, data, declaration)?,
        };
        if matches!(value, ResolvedValue::Visual(_)) {
            channel = channel.no_scale();
        }
        Ok(NativeValue::Channel(Box::new(ChannelExpr::new(
            data_expr, channel,
        ))))
    }

    fn channel_data_expr(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(expr.clone()),
                NativeOutputValue::Channel(channel) => Ok(channel.data_expr().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a channel value"),
                ),
            };
        }
        match value {
            ResolvedValue::String(value) => Ok(lit(value.clone())),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                Ok(lit(value.parse::<i64>().unwrap()))
            }
            ResolvedValue::Number(value) => value.parse::<f64>().map(lit).map_err(|_| {
                lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
            }),
            ResolvedValue::Boolean(value) => Ok(lit(*value)),
            ResolvedValue::Null => Ok(lit(ScalarValue::Null)),
            ResolvedValue::Visual(inner) => self.expression_value(inner, data, declaration),
            _ => self.expression_value(value, data, declaration),
        }
    }

    fn apply_channel_configs(
        &self,
        mut channel: ChannelValue,
        properties: &std::collections::BTreeMap<String, ResolvedValue>,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        for (name, config) in properties {
            channel = match name.as_str() {
                "scale" => self.apply_scale(channel, config, data, declaration)?,
                "axis" => self.apply_axis(channel, config, data, declaration)?,
                "legend" => self.apply_legend(channel, config, data, declaration)?,
                other => {
                    return Err(lowerer_error(
                        declaration,
                        format!("unsupported channel configuration `{other}`"),
                    ));
                }
            };
        }
        Ok(channel)
    }

    fn raw_channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(ChannelValue::from(expr.clone())),
                NativeOutputValue::Channel(channel) => Ok(channel.channel_value().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a channel value"),
                ),
            };
        }
        Ok(match value {
            ResolvedValue::String(value) => ChannelValue::from(value.as_str()),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                ChannelValue::from(value.parse::<i64>().unwrap())
            }
            ResolvedValue::Number(value) => {
                ChannelValue::from(value.parse::<f64>().map_err(|_| {
                    lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
                })?)
            }
            ResolvedValue::Boolean(value) => ChannelValue::from(*value),
            ResolvedValue::Null => ChannelValue::from(lit(ScalarValue::Null)).no_scale(),
            ResolvedValue::Visual(inner) => {
                ChannelValue::from(self.expression_value(inner, data, declaration)?).no_scale()
            }
            _ => ChannelValue::from(self.expression_value(value, data, declaration)?),
        })
    }

    fn apply_scale(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel.no_scale());
        }
        let native = self.object_declaration(value, data, declaration, "linear")?;
        let key = NativeKindKey::new(NativeKindNamespace::Scale, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let scale = lowered.downcast::<Scale<Auto>>().map_err(|_| {
            lowerer_error(
                declaration,
                "registered scale lowerer did not return Scale<Auto>",
            )
        })?;
        Ok(channel.scale(move |_| (*scale).clone()))
    }

    fn apply_axis(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel);
        }
        let native = self.object_declaration(value, data, declaration, "cartesian")?;
        let key = NativeKindKey::new(NativeKindNamespace::Axis, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let axis = lowered
            .downcast::<Box<dyn avenger_chart_core::Axis>>()
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered axis lowerer did not return Box<dyn Axis>",
                )
            })?;
        Ok(channel.with_boxed_axis_config(*axis))
    }

    fn apply_legend(
        &self,
        channel: ChannelValue,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
        if matches!(value, ResolvedValue::None) {
            return Ok(channel.no_legend());
        }
        let native = self.object_declaration(value, data, declaration, "standard")?;
        let key = NativeKindKey::new(NativeKindNamespace::Legend, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        let legend = lowered
            .downcast::<avenger_chart_core::Legend>()
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered legend lowerer did not return Legend",
                )
            })?;
        Ok(channel.legend(*legend))
    }

    fn object_declaration(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
        default_kind: &str,
    ) -> Result<NativeDeclaration, Diagnostic> {
        let (kind, properties) = match value {
            ResolvedValue::Atom(kind) => (kind.clone(), None),
            ResolvedValue::Object {
                kind, properties, ..
            } => (
                kind.clone().unwrap_or_else(|| default_kind.to_string()),
                Some(properties),
            ),
            _ => {
                return Err(lowerer_error(
                    declaration,
                    "configuration must be a kind atom or property block",
                ));
            }
        };
        let mut native = NativeDeclaration::new(kind);
        if let Some(properties) = properties {
            for (name, value) in properties {
                native
                    .properties
                    .insert(name.clone(), self.native_value(value, data, declaration)?);
            }
        }
        Ok(native)
    }

    fn expression_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if let ResolvedValue::Expression(expression) = value
            && let Some(output) = self.direct_output(expression)
        {
            return match output {
                NativeOutputValue::Expr(expr) => Ok(expr.clone()),
                NativeOutputValue::Channel(channel) => Ok(channel.data_expr().clone()),
                NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => Err(
                    lowerer_error(declaration, "this transform output is not a SQL expression"),
                ),
            };
        }
        match value {
            ResolvedValue::String(value) | ResolvedValue::Atom(value) => Ok(lit(value.clone())),
            ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
                Ok(lit(value.parse::<i64>().unwrap()))
            }
            ResolvedValue::Number(value) => value.parse::<f64>().map(lit).map_err(|_| {
                lowerer_error(declaration, format!("invalid numeric literal `{value}`"))
            }),
            ResolvedValue::Boolean(value) => Ok(lit(*value)),
            ResolvedValue::Null => Ok(lit(ScalarValue::Null)),
            ResolvedValue::Column(name) => {
                let data = data.ok_or_else(|| {
                    lowerer_error(
                        declaration,
                        format!("column `{name}` has no data schema in scope"),
                    )
                })?;
                data.schema()
                    .field_with_unqualified_name(name)
                    .map_err(|error| lowerer_error(declaration, error.to_string()))?;
                Ok(col(name))
            }
            ResolvedValue::Expression(expression) => {
                self.planned_expression(expression, data, declaration)
            }
            ResolvedValue::Binding(binding) => self.binding_expr(binding, declaration),
            ResolvedValue::Visual(inner) => self.expression_value(inner, data, declaration),
            _ => Err(lowerer_error(
                declaration,
                format!("value `{value:?}` is not a SQL expression"),
            )),
        }
    }

    fn binding_expr(
        &self,
        binding: &ResolvedBinding,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if binding.time != BindingTime::Current {
            return Err(lowerer_error(
                declaration,
                "temporal parameter reads are only lowered with event bindings",
            ));
        }
        let ResolvedTarget::Param(id) = &binding.target else {
            return Err(lowerer_error(
                declaration,
                "table bindings are not scalar expressions",
            ));
        };
        self.params
            .get(id)
            .map(ChartParam::expr)
            .ok_or_else(|| lowerer_error(declaration, "resolved parameter is unavailable"))
    }

    fn planned_expression(
        &self,
        expression: &ResolvedExpression,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<Expr, Diagnostic> {
        if !expression.helpers.is_empty() {
            return Err(lowerer_error(
                declaration,
                "reserved SQL helpers in native expressions are not in the Phase 5 slice",
            ));
        }
        let data = data.ok_or_else(|| {
            lowerer_error(declaration, "SQL expression has no data schema in scope")
        })?;
        let mut sql = expression.sql.clone();
        let mut parse_data = data.clone();
        let mut replacements = BTreeMap::new();
        for (index, reference) in expression.references.iter().enumerate() {
            let (expr, planning_seed) = match &reference.target {
                ResolvedTarget::Output(handle) => {
                    let output = self.transform_outputs.get(handle).ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "transform output `{}` is unavailable at this dataflow position",
                                reference.authored_path.join(".")
                            ),
                        )
                    })?;
                    let expr = match output {
                        NativeOutputValue::Expr(expr) => expr.clone(),
                        NativeOutputValue::Channel(channel) => channel.data_expr().clone(),
                        NativeOutputValue::RasterDim(_) | NativeOutputValue::Opaque(_) => {
                            return Err(lowerer_error(
                                declaration,
                                format!(
                                    "transform output `{}` cannot be used in a SQL expression",
                                    reference.authored_path.join(".")
                                ),
                            ));
                        }
                    };
                    (expr.clone(), expr)
                }
                ResolvedTarget::Reserved { namespace, path } if namespace == "repeat" => {
                    repeat_reference_expr(path).ok_or_else(|| {
                        lowerer_error(
                            declaration,
                            format!(
                                "unsupported repeat reference `{}`",
                                reference.authored_path.join(".")
                            ),
                        )
                    })?
                }
                _ => continue,
            };
            let synthetic = format!("__avenger_output_{index:08}");
            sql = rewrite_reference_sql(sql, reference, &synthetic);
            parse_data = parse_data
                .with_column(&synthetic, planning_seed)
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic, expr);
        }
        for (index, binding) in expression.bindings.iter().enumerate() {
            let synthetic = format!("__avenger_binding_{index:08}");
            let authored = binding_spelling(binding);
            sql = sql.replace(&authored, &format!("\"{synthetic}\""));
            let expr = self.binding_expr(binding, declaration)?;
            parse_data = parse_data
                .with_column(&synthetic, expr.clone())
                .map_err(|error| lowerer_error(declaration, error.to_string()))?;
            replacements.insert(synthetic, expr);
        }
        let parsed = parse_data
            .parse_sql_expr(&sql)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        parsed
            .transform_up(|candidate| {
                if let Expr::Column(column) = &candidate
                    && let Some(replacement) = replacements.get(&column.name)
                {
                    return Ok(Transformed::yes(replacement.clone()));
                }
                Ok(Transformed::no(candidate))
            })
            .map(|result| result.data)
            .map_err(|error: datafusion::error::DataFusionError| {
                lowerer_error(declaration, error.to_string())
            })
    }

    fn query_sql(
        &self,
        query: &ResolvedQuery,
        declaration: &ResolvedDeclaration,
    ) -> Result<String, Diagnostic> {
        if !query.helpers.is_empty() {
            return Err(lowerer_error(
                declaration,
                "reserved helper calls are not valid in SQL transform queries",
            ));
        }
        let mut sql = query.sql.clone();
        for reference in &query.references {
            let ResolvedTarget::Output(handle) = &reference.target else {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "reference `{}` is not a relation-column output",
                        reference.authored_path.join(".")
                    ),
                ));
            };
            let output = self.transform_outputs.get(handle).ok_or_else(|| {
                lowerer_error(
                    declaration,
                    format!(
                        "transform output `{}` is unavailable at this dataflow position",
                        reference.authored_path.join(".")
                    ),
                )
            })?;
            if !matches!(
                output,
                NativeOutputValue::Expr(_) | NativeOutputValue::Channel(_)
            ) {
                return Err(lowerer_error(
                    declaration,
                    format!(
                        "transform output `{}` is not a SQL column",
                        reference.authored_path.join(".")
                    ),
                ));
            }
            sql = rewrite_reference_sql(sql, reference, &handle.name);
        }
        for binding in &query.bindings {
            if binding.time != BindingTime::Current {
                return Err(lowerer_error(
                    declaration,
                    "temporal parameter reads are only valid in event expressions",
                ));
            }
            let ResolvedTarget::Param(id) = &binding.target else {
                return Err(lowerer_error(
                    declaration,
                    "SQL query bindings must reference scalar parameters",
                ));
            };
            let param = self.params.get(id).ok_or_else(|| {
                lowerer_error(declaration, "resolved SQL parameter is unavailable")
            })?;
            sql = sql.replace(&binding_spelling(binding), &format!("${}", param.name));
        }
        Ok(sql)
    }

    fn direct_output(&self, expression: &ResolvedExpression) -> Option<&NativeOutputValue> {
        if !expression.bindings.is_empty()
            || !expression.helpers.is_empty()
            || expression.references.len() != 1
        {
            return None;
        }
        let reference = expression.references.first()?;
        if !is_direct_reference_sql(&expression.sql, &reference.authored_path) {
            return None;
        }
        let ResolvedTarget::Output(handle) = &reference.target else {
            return None;
        };
        self.transform_outputs.get(handle)
    }

    async fn lower_widget_items(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<(WidgetItems, DataFrame), Diagnostic> {
        let data = self.lower_data(value, declaration).await?;
        if let ResolvedValue::Object { properties, .. } = value
            && let Some(ResolvedValue::Array(rows)) = properties.get("values")
        {
            let mut items = Vec::with_capacity(rows.len());
            for row in rows {
                let ResolvedValue::Object { properties, .. } = row else {
                    return Err(lowerer_error(
                        declaration,
                        "widget inline data rows must be anonymous objects",
                    ));
                };
                let values = properties
                    .iter()
                    .map(|(name, value)| {
                        Ok((
                            name.clone(),
                            widget_item_scalar(value).map_err(|message| {
                                lowerer_error(
                                    declaration,
                                    format!("widget data field `{name}` {message}"),
                                )
                            })?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                items.push(WidgetItemRow::new(values));
            }
            return Ok((WidgetItems::Static(items), data));
        }

        let order_key = declaration
            .properties
            .get("order_by")
            .map(|value| match value {
                ResolvedValue::Array(values) => values
                    .iter()
                    .map(|value| self.expression_value(value, Some(&data), declaration))
                    .collect::<Result<Vec<_>, _>>(),
                value => self
                    .expression_value(value, Some(&data), declaration)
                    .map(|value| vec![value]),
            })
            .transpose()?
            .unwrap_or_default();
        if order_key.is_empty() {
            return Err(lowerer_error(
                declaration,
                "non-inline widget data requires a nonempty total `order_by` expression list",
            ));
        }
        Ok((
            WidgetItems::DataFrame {
                data: data.clone(),
                order_key,
            },
            data,
        ))
    }

    async fn lower_data(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<DataFrame, Diagnostic> {
        match value {
            ResolvedValue::Binding(ResolvedBinding {
                target: ResolvedTarget::Store(_),
                ..
            }) => Err(lowerer_error(
                declaration,
                "store-backed mark data requires the event-runtime store relation seam",
            )),
            ResolvedValue::Object { properties, .. } => {
                let sources = ["values", "table", "sql"]
                    .into_iter()
                    .filter(|name| properties.contains_key(*name))
                    .collect::<Vec<_>>();
                if sources.len() != 1 {
                    return Err(lowerer_error(
                        declaration,
                        "data block must contain exactly one of `values`, `table`, or `sql`",
                    ));
                }
                match sources[0] {
                    "values" => self.inline_values(&properties["values"], declaration).await,
                    "table" => {
                        let ResolvedValue::String(table) = &properties["table"] else {
                            return Err(lowerer_error(
                                declaration,
                                "data table name must be a string",
                            ));
                        };
                        self.context
                            .table(table.as_str())
                            .await
                            .map_err(|error| lowerer_error(declaration, error.to_string()))
                    }
                    "sql" => {
                        let ResolvedValue::Query(query) = &properties["sql"] else {
                            return Err(lowerer_error(
                                declaration,
                                "data sql source must be a query",
                            ));
                        };
                        let sql = self.query_sql(query, declaration)?;
                        self.context
                            .sql(&sql)
                            .await
                            .map_err(|error| lowerer_error(declaration, error.to_string()))
                    }
                    _ => unreachable!(),
                }
            }
            _ => Err(lowerer_error(
                declaration,
                "data must be a source block or table-valued store binding",
            )),
        }
    }

    async fn inline_values(
        &self,
        value: &ResolvedValue,
        declaration: &ResolvedDeclaration,
    ) -> Result<DataFrame, Diagnostic> {
        let ResolvedValue::Array(rows) = value else {
            return Err(lowerer_error(declaration, "inline values must be an array"));
        };
        if rows.is_empty() {
            return Err(lowerer_error(declaration, "inline values cannot be empty"));
        }
        let ResolvedValue::Object {
            properties: first, ..
        } = &rows[0]
        else {
            return Err(lowerer_error(
                declaration,
                "inline rows must be anonymous objects",
            ));
        };
        if first.is_empty() {
            return Err(lowerer_error(declaration, "inline rows cannot be empty"));
        }
        let columns = first.keys().cloned().collect::<Vec<_>>();
        let mut sql_rows = Vec::new();
        for row in rows {
            let ResolvedValue::Object { properties, .. } = row else {
                return Err(lowerer_error(
                    declaration,
                    "inline rows must be anonymous objects",
                ));
            };
            if properties.keys().ne(columns.iter()) {
                return Err(lowerer_error(
                    declaration,
                    "every inline row must contain the same fields",
                ));
            }
            sql_rows.push(format!(
                "({})",
                columns
                    .iter()
                    .map(|name| inline_literal_sql(&properties[name]))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ));
        }
        let aliases = columns
            .iter()
            .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT * FROM (VALUES {}) AS __avenger_inline({aliases})",
            sql_rows.join(", ")
        );
        self.context
            .sql(&sql)
            .await
            .map_err(|error| lowerer_error(declaration, error.to_string()))
    }

    fn lower_layout(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<LayoutSpec, Diagnostic> {
        let native = self.object_declaration(value, data, declaration, "chart")?;
        let key = NativeKindKey::new(NativeKindNamespace::Layout, native.kind.clone());
        let lowered = self
            .registry
            .lower_object(&key, &native)
            .map_err(|error| lowerer_error(declaration, error.to_string()))?;
        lowered
            .downcast::<LayoutSpec>()
            .map(|value| *value)
            .map_err(|_| {
                lowerer_error(
                    declaration,
                    "registered layout lowerer did not return LayoutSpec",
                )
            })
    }

    fn record_schema(&mut self, declaration: &ResolvedDeclaration, data: &DataFrame) {
        self.analysis_schemas.push((
            declaration.id.clone(),
            declaration.span,
            Arc::new(data.schema().as_arrow().clone()),
        ));
    }
}

fn repeat_reference_expr(path: &[String]) -> Option<(Expr, Expr)> {
    use avenger_chart_core::repeat;

    let [name] = path else {
        return None;
    };
    let (resolved, seed) = match name.as_str() {
        "row" => (repeat::row().into_data_expr(), lit(0.0_f64)),
        "column" => (repeat::column().into_data_expr(), lit(0.0_f64)),
        "item" => (repeat::item().into_data_expr(), lit(0.0_f64)),
        "row_name" => (lit(repeat::row_name()), lit("")),
        "column_name" => (lit(repeat::column_name()), lit("")),
        "item_name" => (lit(repeat::item_name()), lit("")),
        "row_index" => (repeat::row_index(), lit(0_i64)),
        "column_index" => (repeat::column_index(), lit(0_i64)),
        "item_index" => (repeat::item_index(), lit(0_i64)),
        "cell_id" => (repeat::cell_id(), lit("")),
        "row_id" => (repeat::row_id(), lit("")),
        "column_id" => (repeat::column_id(), lit("")),
        "item_id" => (repeat::item_id(), lit("")),
        "row_title" => (repeat::row_title(), lit("")),
        "column_title" => (repeat::column_title(), lit("")),
        "item_title" => (repeat::item_title(), lit("")),
        _ => return None,
    };
    Some((resolved, seed))
}

pub(crate) fn compiled_project_from_lowered(
    project: &ResolvedProject,
    registry: &NativeRegistry,
    lowered: LoweredProject,
) -> CompiledProject {
    let charts = lowered
        .charts
        .into_iter()
        .map(|chart| (chart.artifact.id.clone(), chart.artifact))
        .collect();
    CompiledProject {
        charts,
        sources: project.sources.clone(),
        native_registry_profile: registry.profile_id().clone(),
        project_fingerprint: ProjectFingerprint::new(project.source_fingerprint.clone()),
    }
}

fn widget_owned_params(project: &ResolvedProject) -> BTreeSet<ParamId> {
    project
        .files
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .filter(|declaration| matches!(declaration.keyword.as_str(), "widget" | "tool"))
        .flat_map(|declaration| declaration.exports.values())
        .filter_map(|target| match target {
            ResolvedTarget::Param(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn tool_owned_selections(project: &ResolvedProject) -> BTreeSet<SelectionId> {
    project
        .files
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .filter(|declaration| declaration.keyword == "tool")
        .flat_map(|declaration| declaration.exports.values())
        .filter_map(|target| match target {
            ResolvedTarget::Selection(id) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn declarations_depth_first(root: &ResolvedDeclaration) -> Vec<&ResolvedDeclaration> {
    let mut result = vec![root];
    for child in &root.children {
        result.extend(declarations_depth_first(child));
    }
    result
}

fn find_declaration<'a>(
    project: &'a ResolvedProject,
    id: &DeclarationId,
) -> Option<&'a ResolvedDeclaration> {
    project
        .files
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .find(|declaration| &declaration.id == id)
}

fn find_declaration_by_target<'a>(
    project: &'a ResolvedProject,
    target: &ResolvedTarget,
) -> Option<&'a ResolvedDeclaration> {
    project
        .files
        .values()
        .flat_map(|file| &file.roots)
        .flat_map(declarations_depth_first)
        .find(|declaration| declaration.runtime_target.as_ref() == Some(target))
}

fn widget_source_id(declaration: &ResolvedDeclaration) -> Option<&str> {
    declaration.name.as_deref()
}

fn public_path_belongs_to_chart(path: &str, chart_path: Option<&str>) -> bool {
    chart_path.is_none_or(|chart| path == chart || path.starts_with(&format!("{chart}.")))
}

fn relative_public_path<'a>(path: &'a str, chart_path: Option<&str>) -> &'a str {
    chart_path
        .and_then(|chart| path.strip_prefix(chart))
        .and_then(|path| path.strip_prefix('.'))
        .unwrap_or(path)
}

fn compiled_mark_aliases(compiled: &avenger_chart::plot::CompiledPlot) -> BTreeMap<String, String> {
    compiled
        .marks()
        .iter()
        .flat_map(|mark| {
            let runtime_id = mark.state().identity.runtime_id.as_opaque_str().to_string();
            mark.state()
                .identity
                .public_aliases
                .iter()
                .cloned()
                .map(move |alias| (alias, runtime_id.clone()))
        })
        .collect()
}

fn compiled_mark_id_for_public_path(
    aliases: &BTreeMap<String, String>,
    path: &str,
    chart_path: Option<&str>,
) -> Option<String> {
    aliases.get(relative_public_path(path, chart_path)).cloned()
}

fn compiled_widget_part_id(
    compiled: &avenger_chart::plot::CompiledPlot,
    path: &str,
    chart_path: Option<&str>,
) -> Option<String> {
    let relative = relative_public_path(path, chart_path);
    compiled.widgets().iter().find_map(|attachment| {
        let avenger_chart_core::CompiledWidget::Composed(widget) = &attachment.widget else {
            return None;
        };
        widget
            .relative_target_ids
            .get(relative)
            .and_then(|ids| ids.first())
            .map(|id| id.as_opaque_str().to_string())
    })
}

fn belongs_to_chart(ancestry: &[DeclarationId], chart: &DeclarationId) -> bool {
    ancestry.iter().any(|ancestor| ancestor == chart)
}

fn resolved_atom(value: &ResolvedValue) -> Option<&str> {
    match value {
        ResolvedValue::Atom(value) | ResolvedValue::String(value) => Some(value),
        _ => None,
    }
}

fn resolved_string(value: &ResolvedValue) -> Option<&str> {
    match value {
        ResolvedValue::String(value) => Some(value),
        _ => None,
    }
}

fn resolved_f32(
    value: Option<&ResolvedValue>,
    default: f32,
    declaration: &ResolvedDeclaration,
    property: &str,
) -> Result<f32, Diagnostic> {
    match value {
        Some(ResolvedValue::Number(value)) => value.parse::<f32>().map_err(|_| {
            lowerer_error(
                declaration,
                format!("pattern property `{property}` must be a finite number"),
            )
        }),
        None if !default.is_nan() => Ok(default),
        None => Err(lowerer_error(
            declaration,
            format!("pattern property `{property}` is required"),
        )),
        Some(_) => Err(lowerer_error(
            declaration,
            format!("pattern property `{property}` must be a number"),
        )),
    }
}

fn required_resolved_f32(
    declaration_value: &ResolvedDeclaration,
    property: &str,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value.properties.get(property),
        f32::NAN,
        owner,
        property,
    )
}

fn required_resolved_f32_alias(
    declaration_value: &ResolvedDeclaration,
    preferred: &str,
    fallback: &str,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value
            .properties
            .get(preferred)
            .or_else(|| declaration_value.properties.get(fallback)),
        f32::NAN,
        owner,
        preferred,
    )
}

fn optional_resolved_f32_alias(
    declaration_value: &ResolvedDeclaration,
    preferred: &str,
    fallback: &str,
    default: f32,
    owner: &ResolvedDeclaration,
) -> Result<f32, Diagnostic> {
    resolved_f32(
        declaration_value
            .properties
            .get(preferred)
            .or_else(|| declaration_value.properties.get(fallback)),
        default,
        owner,
        preferred,
    )
}

fn pattern_operation(
    value: Option<&ResolvedValue>,
    declaration: &ResolvedDeclaration,
) -> Result<PatternLayerOperation, Diagnostic> {
    match value.and_then(resolved_atom) {
        None | Some("add") => Ok(PatternLayerOperation::Add),
        Some("subtract") => Ok(PatternLayerOperation::Subtract),
        Some("xor") => Ok(PatternLayerOperation::Xor),
        Some(value) => Err(lowerer_error(
            declaration,
            format!("unsupported pattern operation `{value}`"),
        )),
    }
}

fn is_core_property(keyword: &str, name: &str) -> bool {
    matches!(
        (keyword, name),
        (
            "chart" | "plot",
            "data" | "title" | "subtitle" | "layout" | "theme" | "time" | "format" | "guide"
        ) | ("cell", "at" | "data" | "label" | "when")
            | ("group", "data" | "component_kind" | "label")
            | ("mark", "data")
            | ("transform", "scope")
            | ("tool", "id")
    )
}

fn sharing(value: StateSharing) -> avenger_chart_core::CoordinationScope {
    match value {
        StateSharing::Shared => avenger_chart_core::CoordinationScope::Shared,
        StateSharing::Free => avenger_chart_core::CoordinationScope::Free,
        StateSharing::Level(level) => {
            avenger_chart_core::CoordinationScope::Level(level.min(u8::MAX as u32) as u8)
        }
    }
}

fn physical_field(field: &PhysicalField) -> Field {
    Field::new(
        field.name.clone(),
        physical_data_type(&field.data_type),
        field.nullable,
    )
}

fn physical_data_type(value: &PhysicalType) -> DataType {
    match value {
        PhysicalType::Boolean => DataType::Boolean,
        PhysicalType::Int8 => DataType::Int8,
        PhysicalType::Int16 => DataType::Int16,
        PhysicalType::Int32 => DataType::Int32,
        PhysicalType::Int64 => DataType::Int64,
        PhysicalType::UInt8 => DataType::UInt8,
        PhysicalType::UInt16 => DataType::UInt16,
        PhysicalType::UInt32 => DataType::UInt32,
        PhysicalType::UInt64 => DataType::UInt64,
        PhysicalType::Float16 => DataType::Float16,
        PhysicalType::Float32 => DataType::Float32,
        PhysicalType::Float64 => DataType::Float64,
        PhysicalType::Utf8 => DataType::Utf8,
        PhysicalType::LargeUtf8 => DataType::LargeUtf8,
        PhysicalType::Binary => DataType::Binary,
        PhysicalType::LargeBinary => DataType::LargeBinary,
        PhysicalType::Date32 => DataType::Date32,
        PhysicalType::Date64 => DataType::Date64,
        PhysicalType::Time32(unit) => DataType::Time32(arrow_time_unit(*unit)),
        PhysicalType::Time64(unit) => DataType::Time64(arrow_time_unit(*unit)),
        PhysicalType::Timestamp { unit, timezone } => DataType::Timestamp(
            arrow_time_unit(*unit),
            timezone
                .as_ref()
                .map(|value| Arc::<str>::from(value.as_str())),
        ),
        PhysicalType::Duration(unit) => DataType::Duration(arrow_time_unit(*unit)),
        PhysicalType::Interval(unit) => DataType::Interval(match unit {
            IntervalUnit::YearMonth => ArrowIntervalUnit::YearMonth,
            IntervalUnit::DayTime => ArrowIntervalUnit::DayTime,
            IntervalUnit::MonthDayNano => ArrowIntervalUnit::MonthDayNano,
        }),
        PhysicalType::FixedSizeBinary(size) => DataType::FixedSizeBinary(*size),
        PhysicalType::Decimal128 { precision, scale } => DataType::Decimal128(*precision, *scale),
        PhysicalType::Decimal256 { precision, scale } => DataType::Decimal256(*precision, *scale),
        PhysicalType::List(element) => DataType::List(Arc::new(Field::new_list_field(
            physical_data_type(element),
            true,
        ))),
        PhysicalType::LargeList(element) => DataType::LargeList(Arc::new(Field::new_list_field(
            physical_data_type(element),
            true,
        ))),
        PhysicalType::FixedSizeList { element, length } => DataType::FixedSizeList(
            Arc::new(Field::new_list_field(physical_data_type(element), true)),
            *length,
        ),
        PhysicalType::Struct(fields) => DataType::Struct(Fields::from(
            fields.iter().map(physical_field).collect::<Vec<_>>(),
        )),
        PhysicalType::Map { key, value } => DataType::Map(
            Arc::new(Field::new(
                "entries",
                DataType::Struct(Fields::from(vec![
                    Field::new("keys", physical_data_type(key), false),
                    Field::new("values", physical_data_type(value), true),
                ])),
                false,
            )),
            false,
        ),
    }
}

fn arrow_time_unit(value: TimeUnit) -> ArrowTimeUnit {
    match value {
        TimeUnit::Second => ArrowTimeUnit::Second,
        TimeUnit::Millisecond => ArrowTimeUnit::Millisecond,
        TimeUnit::Microsecond => ArrowTimeUnit::Microsecond,
        TimeUnit::Nanosecond => ArrowTimeUnit::Nanosecond,
    }
}

fn scalar_literal(value: &ResolvedValue, data_type: &DataType) -> Result<ScalarValue, String> {
    match value {
        ResolvedValue::Null => {
            ScalarValue::try_new_null(data_type).map_err(|error| error.to_string())
        }
        ResolvedValue::String(value)
        | ResolvedValue::Number(value)
        | ResolvedValue::Atom(value) => ScalarValue::try_from_string(value.clone(), data_type)
            .map_err(|error| error.to_string()),
        ResolvedValue::Boolean(value) => ScalarValue::try_from_string(value.to_string(), data_type)
            .map_err(|error| error.to_string()),
        _ => Err("store row values must be typed literals in Phase 5".to_string()),
    }
}

fn inline_literal_sql(value: &ResolvedValue) -> Result<String, Diagnostic> {
    match value {
        ResolvedValue::String(value) => Ok(format!("'{}'", value.replace('\'', "''"))),
        ResolvedValue::Number(value) => Ok(value.clone()),
        ResolvedValue::Boolean(value) => Ok(value.to_string().to_uppercase()),
        ResolvedValue::Null => Ok("NULL".to_string()),
        ResolvedValue::Expression(expression)
            if expression.bindings.is_empty()
                && expression.helpers.is_empty()
                && expression.references.is_empty() =>
        {
            Ok(expression.sql.clone())
        }
        _ => Err(diagnostic(
            SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
            "AVENGER-LOWER-006",
            "inline data contains a non-literal value",
            format!("found {value:?}"),
        )),
    }
}

fn widget_item_scalar(value: &ResolvedValue) -> Result<ScalarValue, &'static str> {
    match value {
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Number(value) if value.parse::<i64>().is_ok() => {
            Ok(ScalarValue::Int64(Some(value.parse::<i64>().unwrap())))
        }
        ResolvedValue::Number(value) => value
            .parse::<f64>()
            .map(|value| ScalarValue::Float64(Some(value)))
            .map_err(|_| "must be a scalar literal"),
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Null => Ok(ScalarValue::Null),
        _ => Err("must be a scalar literal"),
    }
}

fn binding_spelling(binding: &ResolvedBinding) -> String {
    let suffix = match binding.time {
        BindingTime::Current => "",
        BindingTime::Start => "@start",
        BindingTime::Previous => "@previous",
    };
    format!("${}{suffix}", binding.authored_path.join("."))
}

fn rewrite_reference_sql(
    mut sql: String,
    reference: &ResolvedSqlReference,
    replacement: &str,
) -> String {
    let quoted_replacement = format!("\"{}\"", replacement.replace('"', "\"\""));
    let bare = reference.authored_path.join(".");
    let quoted = reference
        .authored_path
        .iter()
        .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".");
    sql = sql.replace(&quoted, &quoted_replacement);
    sql.replace(&bare, &quoted_replacement)
}

fn is_direct_reference_sql(sql: &str, path: &[String]) -> bool {
    let candidate = sql.trim().trim_end_matches(';').trim();
    let bare = path.join(".");
    let quoted = path
        .iter()
        .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".");
    candidate == bare || candidate == quoted
}

fn lowerer_error(declaration: &ResolvedDeclaration, message: impl Into<String>) -> Diagnostic {
    diagnostic(
        declaration.span,
        "AVENGER-LOWER-002",
        "native chart lowering failed",
        message,
    )
}

fn lowerer_error_at(
    declaration: DeclarationId,
    project: &ResolvedProject,
    message: impl Into<String>,
) -> Diagnostic {
    let message = message.into();
    find_declaration(project, &declaration).map_or_else(
        || {
            diagnostic(
                SourceSpan::empty(avenger_lang_core::SourceId::new(0), 0),
                "AVENGER-LOWER-002",
                "native chart lowering failed",
                message.clone(),
            )
        },
        |declaration| lowerer_error(declaration, message.clone()),
    )
}

fn diagnostic(span: SourceSpan, code: &str, message: &str, label: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, message, SourceLabel::new(span, label))
}
