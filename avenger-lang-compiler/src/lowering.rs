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
        Auto, ChannelValue, LegendableChannelValue, Param, Scale, ScaleChannelValue, Selection,
        Store,
    },
};
use avenger_chart_core::{DataTransformExecutionContext, Param as ChartParam, TimeContext};
use avenger_chart_lang_registry::{
    NativeRegistry, ResolvedChildPlot, ResolvedDeclaration as NativeDeclaration, ResolvedMark,
    ResolvedMarkGroup, ResolvedPlot, ResolvedTransformStage, ResolvedValue as NativeValue,
};
use avenger_chart_schema::{NativeKindKey, NativeKindNamespace};
use avenger_lang_core::{
    DeclarationId, Diagnostic, IntervalUnit, ParamId, PhysicalField, PhysicalType, ResolvedBinding,
    ResolvedDeclaration, ResolvedExpression, ResolvedParam, ResolvedProject, ResolvedQuery,
    ResolvedSelectionCombine, ResolvedSelectionEmpty, ResolvedSqlReference, ResolvedStore,
    ResolvedTarget, ResolvedValue, SourceLabel, SourceSpan, StateSharing, StoreId, TimeUnit,
    ast::BindingTime,
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
    widget_owned_params: BTreeSet<ParamId>,
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
            widget_owned_params: widget_owned_params(project),
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
            && origin.export_role == "value"
            && let Some(ResolvedValue::String(id)) = declaration.properties.get("id")
        {
            return format!("{id}__value");
        }
        param.source_name.clone()
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
        plot.coordinate = self.native_declaration(chart, None, NativeKindNamespace::Coordinate)?;

        let data = match chart.properties.get("data") {
            Some(value) => Some(self.lower_data(value, chart).await?),
            None => None,
        };
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
        for selection in self.project.selections.values() {
            if belongs_to_chart(&selection.owner_ancestry, &chart.id)
                && selection.generated_by.is_none()
            {
                let mut lowered = Selection::new(selection.source_name.clone());
                lowered = match selection.empty {
                    ResolvedSelectionEmpty::All => lowered.empty_selects_all(),
                    ResolvedSelectionEmpty::None => lowered.empty_selects_nothing(),
                };
                lowered = lowered.combine(match selection.combine {
                    ResolvedSelectionCombine::Union => avenger_chart_core::SelectionCombine::Union,
                    ResolvedSelectionCombine::Intersect => {
                        avenger_chart_core::SelectionCombine::Intersect
                    }
                });
                plot.furnishings.selections.push(lowered);
            }
        }

        let coordinate_kind = plot.coordinate.kind.clone();
        let root_group = self
            .lower_container(chart, data.as_ref(), coordinate_kind.as_str(), &mut plot)
            .await?;
        if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
            plot.marks.push(ResolvedMark::Group(root_group));
        }
        Ok(plot)
    }

    fn lower_container<'b>(
        &'b mut self,
        container: &'b ResolvedDeclaration,
        inherited_data: Option<&'b DataFrame>,
        coordinate: &'b str,
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
                        let declaration = self.native_declaration(
                            child,
                            Some(input),
                            NativeKindNamespace::Transform,
                        )?;
                        let lowered = self
                            .registry
                            .lower_transform(
                                &declaration,
                                avenger_chart_core::DataTransformCompileContext::new(
                                    avenger_chart_core::CoordinationScope::Free,
                                ),
                            )
                            .map_err(|error| lowerer_error(child, error.to_string()))?;
                        let params = IndexMap::new();
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
                            .map_err(|error| lowerer_error(child, error.to_string()))?;
                        current_data = Some(result.dataframe);
                        if let Some(data) = &current_data {
                            self.record_schema(child, data);
                        }
                        group.transforms.push(ResolvedTransformStage {
                            scope: avenger_chart_core::CoordinationScope::Free,
                            transform: lowered.transform,
                        });
                    }
                    "group" => {
                        let child_group = self
                            .lower_container(child, current_data.as_ref(), coordinate, plot)
                            .await?;
                        group.marks.push(ResolvedMark::Group(child_group));
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
                        let native = ResolvedMark::Native(declaration);
                        if let Some(data) = mark_data {
                            let mut wrapper = ResolvedMarkGroup::new();
                            wrapper.data = Some(data);
                            wrapper.marks.push(native);
                            group.marks.push(ResolvedMark::Group(wrapper));
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
                        let mut declaration = self.native_declaration(
                            child,
                            current_data.as_ref(),
                            NativeKindNamespace::Widget,
                        )?;
                        if !declaration.properties.contains_key("value_param")
                            && let Some(ResolvedTarget::Param(id)) = child.exports.get("value")
                            && let Some(param) = self.params.get(id)
                        {
                            declaration.properties.insert(
                                "value_param".to_string(),
                                NativeValue::Param(param.clone()),
                            );
                        }
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
                    "param" | "store" | "selection" | "on" => {}
                    other => {
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
            plot.coordinate = self.native_declaration(
                declaration,
                inherited_data,
                NativeKindNamespace::Coordinate,
            )?;
            let explicit_data = match declaration.properties.get("data") {
                Some(value) => Some(self.lower_data(value, declaration).await?),
                None => None,
            };
            if let Some(data) = &explicit_data {
                self.record_schema(declaration, data);
            }
            plot.data = explicit_data.clone();
            let planning_data = explicit_data.as_ref().or(inherited_data);
            let coordinate_kind = plot.coordinate.kind.clone();
            let root_group = self
                .lower_container(
                    declaration,
                    planning_data,
                    coordinate_kind.as_str(),
                    &mut plot,
                )
                .await?;
            if !root_group.marks.is_empty() || !root_group.transforms.is_empty() {
                plot.marks.push(ResolvedMark::Group(root_group));
            }
            Ok(plot)
        })
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
        let mut native = NativeDeclaration::new(kind.clone());
        native.source_name.clone_from(&declaration.name);
        for (name, value) in &declaration.properties {
            if is_core_property(&declaration.keyword, name) {
                continue;
            }
            let lowered = if namespace == NativeKindNamespace::Mark {
                self.channel_value(value, data, declaration)?
            } else {
                self.native_value(value, data, declaration)?
            };
            native.properties.insert(name.clone(), lowered);
        }
        let key = match namespace {
            NativeKindNamespace::Mark => {
                NativeKindKey::mark(declaration.coordinate.as_deref().unwrap_or(""), kind)
            }
            _ => NativeKindKey::new(namespace, kind),
        };
        if !self.registry.snapshot().entries.contains_key(&key) {
            return Err(lowerer_error(
                declaration,
                format!("native registry has no schema/lowerer pair for {key:?}"),
            ));
        }
        Ok(native)
    }

    fn native_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<NativeValue, Diagnostic> {
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
            ResolvedValue::Call { .. }
            | ResolvedValue::Reference(_)
            | ResolvedValue::Dimension(_)
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
        let mut channel = match value {
            ResolvedValue::Object {
                head: Some(head),
                properties,
                ..
            } => {
                let mut channel = self.raw_channel_value(head, data, declaration)?;
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
                channel
            }
            _ => self.raw_channel_value(value, data, declaration)?,
        };
        if matches!(value, ResolvedValue::Visual(_)) {
            channel = channel.no_scale();
        }
        Ok(NativeValue::Channel(channel))
    }

    fn raw_channel_value(
        &self,
        value: &ResolvedValue,
        data: Option<&DataFrame>,
        declaration: &ResolvedDeclaration,
    ) -> Result<ChannelValue, Diagnostic> {
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
        for reference in &expression.references {
            sql = rewrite_reference_sql(sql, reference);
        }
        let mut parse_data = data.clone();
        let mut replacements = BTreeMap::new();
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
        if !query.bindings.is_empty() || !query.helpers.is_empty() || !query.references.is_empty() {
            return Err(lowerer_error(
                declaration,
                "bound/helper SQL queries are not yet supported by the Phase 5 query planner",
            ));
        }
        Ok(query.sql.clone())
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
    match declaration.properties.get("id") {
        Some(ResolvedValue::String(id)) => Some(id),
        _ => declaration.name.as_deref(),
    }
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

fn is_core_property(keyword: &str, name: &str) -> bool {
    matches!(
        (keyword, name),
        (
            "chart" | "plot",
            "data" | "title" | "subtitle" | "layout" | "theme"
        ) | ("cell", "at" | "data" | "label")
            | ("group", "data" | "component_kind" | "label")
            | ("mark", "data")
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

fn binding_spelling(binding: &ResolvedBinding) -> String {
    let suffix = match binding.time {
        BindingTime::Current => "",
        BindingTime::Start => "@start",
        BindingTime::Previous => "@previous",
    };
    format!("${}{suffix}", binding.authored_path.join("."))
}

fn rewrite_reference_sql(mut sql: String, reference: &ResolvedSqlReference) -> String {
    if let ResolvedTarget::Output(output) = &reference.target {
        sql = sql.replace(
            &reference.authored_path.join("."),
            &format!("\"{}\"", output.name.replace('"', "\"\"")),
        );
    }
    sql
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
