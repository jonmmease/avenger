//! Chart tool compilation support and built-in tool re-exports.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    Auto, AvengerChartError, ChartEventBinding, CompiledParamSpec, CompiledSelectionSpec,
    CompiledStoreSpec, CoordinateSystemCore, CoordinateSystemTransform, DefaultLogicalExprNodeExt,
    Param, Scale, Selection, Sharing, Store,
};
use avenger_chart_scales::PlotScaleSpec;
use datafusion::prelude::lit;
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;

pub use avenger_chart_core::{
    ChartTool, ToolExpansion, ToolExpansionContext, ToolMetadata, ToolParamExpansion,
    ToolParamSharing, ToolScaleEdit,
};
pub use avenger_chart_tools::{BoxZoom, PanScrollZoom, PointSelection};

pub(crate) struct ToolCompileContext {
    state: Arc<Mutex<ToolCompileState>>,
    coord_node_path: Vec<usize>,
}

impl ToolCompileContext {
    pub(crate) fn root() -> Self {
        Self {
            state: Arc::new(Mutex::new(ToolCompileState::default())),
            coord_node_path: Vec::new(),
        }
    }

    pub(crate) fn from_parent(parent: Option<&ToolCompileContext>) -> Self {
        Self {
            state: parent
                .map(|ctx| ctx.state.clone())
                .unwrap_or_else(|| Arc::new(Mutex::new(ToolCompileState::default()))),
            coord_node_path: parent
                .map(|ctx| ctx.coord_node_path.clone())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn with_coord_node_path_appended(&self, child_index: usize) -> Self {
        let mut coord_node_path = self.coord_node_path.clone();
        coord_node_path.push(child_index);
        Self {
            state: self.state.clone(),
            coord_node_path,
        }
    }

    pub(crate) fn expand_local_tools<C: CoordinateSystemCore>(
        &self,
        tools: &[Arc<dyn ChartTool<C>>],
    ) -> Result<Vec<ActiveToolExpansion<C>>, AvengerChartError> {
        let mut active = Vec::new();
        for tool in tools {
            let id = tool.id().to_string();
            validate_tool_id(&id)?;
            let mut expansion = tool.expand(ToolExpansionContext { tool_id: &id })?;
            self.localize_event_bindings(&mut expansion.event_bindings);
            let active_expansion = ActiveToolExpansion {
                id: id.clone(),
                expansion: expansion.clone(),
            };
            self.state
                .lock()
                .expect("tool compile state lock poisoned")
                .register_expansion(&id, &expansion)?;
            active.push(active_expansion);
        }
        Ok(active)
    }

    pub(crate) fn register_local_event_bindings(
        &self,
        bindings: &[ChartEventBinding],
    ) -> Result<(), AvengerChartError> {
        if bindings.is_empty() {
            return Ok(());
        }
        let mut bindings = bindings.to_vec();
        self.localize_event_bindings(&mut bindings);
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .event_bindings
            .extend(bindings);
        Ok(())
    }

    pub(crate) fn register_local_stores(&self, stores: &[Store]) -> Result<(), AvengerChartError> {
        let mut state = self.state.lock().expect("tool compile state lock poisoned");
        for store in stores {
            state.register_store(store.compile()?)?;
        }
        Ok(())
    }

    pub(crate) fn downcast(ctx: avenger_chart_core::CompileContext<'_>) -> Option<&Self> {
        ctx.downcast_ref::<Self>()
    }

    pub(crate) fn apply_scale_edits<C: CoordinateSystemCore>(
        &self,
        active_expansions: &[ActiveToolExpansion<C>],
        coord_transform: &dyn CoordinateSystemTransform,
        scale_to_coord_channel: &HashMap<String, String>,
        scale_specs: &mut HashMap<String, PlotScaleSpec>,
        scale_sharing: &HashMap<String, Sharing>,
    ) -> Result<(), AvengerChartError> {
        let invertible = coord_transform.interaction_invertible_channels();
        if invertible.is_empty() {
            return Ok(());
        }

        for active in active_expansions {
            for edit in &active.expansion.scale_edits {
                match edit {
                    ToolScaleEdit::RawDomain {
                        channel,
                        param_name,
                        override_existing,
                        disable_nice_zero,
                    } => {
                        if !invertible.contains(&channel.as_str()) {
                            continue;
                        }
                        let targets = scale_to_coord_channel
                            .iter()
                            .filter_map(|(scale_name, coord_channel)| {
                                (coord_channel == channel).then_some(scale_name.clone())
                            })
                            .collect::<Vec<_>>();
                        for scale_name in targets {
                            let sharing = scale_sharing
                                .get(&scale_name)
                                .copied()
                                .unwrap_or(Sharing::Free)
                                .to_normalized();
                            self.state
                                .lock()
                                .expect("tool compile state lock poisoned")
                                .record_scale_target(&active.id, channel, param_name, sharing)?;
                            let param = self
                                .state
                                .lock()
                                .expect("tool compile state lock poisoned")
                                .param(param_name)?
                                .clone();
                            apply_raw_domain_scale_edit(
                                &scale_name,
                                scale_specs,
                                &param,
                                *override_existing,
                                *disable_nice_zero,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub(crate) fn finalize_root(&self) -> Result<ToolArtifacts, AvengerChartError> {
        self.state
            .lock()
            .expect("tool compile state lock poisoned")
            .finalize()
    }

    fn localize_event_bindings(&self, bindings: &mut [ChartEventBinding]) {
        if self.coord_node_path.is_empty() {
            return;
        }
        for binding in bindings {
            *binding = binding
                .clone()
                .with_coord_node_path_target(self.coord_node_path.clone());
        }
    }
}

#[derive(Clone)]
pub(crate) struct ActiveToolExpansion<C: CoordinateSystemCore> {
    id: String,
    pub(crate) expansion: ToolExpansion<C>,
}

#[derive(Default)]
struct ToolCompileState {
    tool_ids: HashSet<String>,
    params: IndexMap<String, GeneratedParamState>,
    stores: IndexMap<String, CompiledStoreSpec>,
    selections: IndexMap<String, CompiledSelectionSpec>,
    event_bindings: Vec<ChartEventBinding>,
    metadata: Vec<ToolMetadata>,
    expected_targets: BTreeMap<(String, String, String), usize>,
}

impl ToolCompileState {
    fn register_expansion<C: CoordinateSystemCore>(
        &mut self,
        id: &str,
        expansion: &ToolExpansion<C>,
    ) -> Result<(), AvengerChartError> {
        if !self.tool_ids.insert(id.to_string()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate chart tool id '{id}'"
            )));
        }

        for param in &expansion.params {
            self.register_param(param)?;
        }
        for store in &expansion.stores {
            self.register_store(store.compile()?)?;
        }
        for selection in &expansion.selections {
            self.register_selection(selection)?;
        }
        for edit in &expansion.scale_edits {
            let ToolScaleEdit::RawDomain {
                channel,
                param_name,
                ..
            } = edit;
            self.expected_targets
                .entry((id.to_string(), channel.clone(), param_name.clone()))
                .or_insert(0);
        }
        self.event_bindings
            .extend(expansion.event_bindings.iter().cloned());
        self.metadata.extend(expansion.metadata.iter().cloned());
        Ok(())
    }

    fn register_store(&mut self, spec: CompiledStoreSpec) -> Result<(), AvengerChartError> {
        if self.stores.contains_key(&spec.name) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Store '{}' was declared more than once",
                spec.name
            )));
        }
        self.stores.insert(spec.name.clone(), spec);
        Ok(())
    }

    fn register_selection(&mut self, selection: &Selection) -> Result<(), AvengerChartError> {
        let spec = selection.compile()?;
        if self.selections.contains_key(&spec.id) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{}' was declared more than once",
                spec.id
            )));
        }
        self.selections.insert(spec.id.clone(), spec);
        Ok(())
    }

    fn register_param(&mut self, expansion: &ToolParamExpansion) -> Result<(), AvengerChartError> {
        match self.params.get_mut(&expansion.param.name) {
            Some(existing) => {
                if existing.param.default != expansion.param.default
                    || existing.requested != expansion.sharing
                {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Generated tool parameter '{}' was declared more than once with \
                         incompatible defaults or sharing",
                        expansion.param.name
                    )));
                }
            }
            None => {
                self.params.insert(
                    expansion.param.name.clone(),
                    GeneratedParamState {
                        param: expansion.param.clone(),
                        requested: expansion.sharing.clone(),
                        resolved_sharing: match expansion.sharing {
                            ToolParamSharing::Explicit(sharing) => Some(sharing.to_normalized()),
                            ToolParamSharing::MirrorScale { .. } => None,
                        },
                    },
                );
            }
        }
        Ok(())
    }

    fn param(&self, name: &str) -> Result<&Param, AvengerChartError> {
        self.params
            .get(name)
            .map(|state| &state.param)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Tool scale edit references unknown generated parameter '{name}'"
                ))
            })
    }

    fn record_scale_target(
        &mut self,
        tool_id: &str,
        channel: &str,
        param_name: &str,
        sharing: Sharing,
    ) -> Result<(), AvengerChartError> {
        if let Some(count) = self.expected_targets.get_mut(&(
            tool_id.to_string(),
            channel.to_string(),
            param_name.to_string(),
        )) {
            *count += 1;
        }

        let param = self.params.get_mut(param_name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Tool scale edit references unknown generated parameter '{param_name}'"
            ))
        })?;

        match &param.requested {
            ToolParamSharing::Explicit(explicit) => {
                if explicit.to_level() < sharing.to_level() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool raw-domain param '{param_name}' is shared at {} but target \
                         channel '{channel}' is shared at {}; the param must be shared at \
                         least as broadly as the scale",
                        describe_sharing(*explicit),
                        describe_sharing(sharing)
                    )));
                }
            }
            ToolParamSharing::MirrorScale {
                channel: mirror_channel,
            } => {
                if mirror_channel != channel {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool raw-domain param '{param_name}' mirrors channel '{mirror_channel}' \
                         but was used for channel '{channel}'"
                    )));
                }
                match param.resolved_sharing {
                    Some(existing) if existing != sharing => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "tool raw-domain param '{param_name}' targets channel '{channel}' \
                             with conflicting scale sharing values: {} and {}",
                            describe_sharing(existing),
                            describe_sharing(sharing)
                        )));
                    }
                    Some(_) => {}
                    None => param.resolved_sharing = Some(sharing),
                }
            }
        }
        Ok(())
    }

    fn finalize(&self) -> Result<ToolArtifacts, AvengerChartError> {
        for ((tool_id, channel, _param), count) in &self.expected_targets {
            if *count == 0 {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "tool '{tool_id}' did not find an invertible scale target for channel \
                     '{channel}'"
                )));
            }
        }

        let mut param_specs = Vec::new();
        for (name, param) in &self.params {
            let Some(sharing) = param.resolved_sharing else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "tool parameter '{name}' did not resolve a sharing scope"
                )));
            };
            param_specs.push(CompiledParamSpec::new(&param.param, sharing));
        }
        Ok(ToolArtifacts {
            param_specs,
            store_specs: self.stores.values().cloned().collect(),
            selection_specs: self.selections.values().cloned().collect(),
            event_bindings: self.event_bindings.clone(),
            metadata: self.metadata.clone(),
        })
    }
}

#[derive(Clone)]
struct GeneratedParamState {
    param: Param,
    requested: ToolParamSharing,
    resolved_sharing: Option<Sharing>,
}

pub(crate) struct ToolArtifacts {
    pub param_specs: Vec<CompiledParamSpec>,
    pub store_specs: Vec<CompiledStoreSpec>,
    pub selection_specs: Vec<CompiledSelectionSpec>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub metadata: Vec<ToolMetadata>,
}

fn apply_raw_domain_scale_edit(
    scale_name: &str,
    scale_specs: &mut HashMap<String, PlotScaleSpec>,
    param: &Param,
    override_existing: bool,
    disable_nice_zero: bool,
) -> Result<(), AvengerChartError> {
    let raw_domain = LogicalExprNode::from_expr(param.expr()).map_err(|err| {
        AvengerChartError::InternalError(format!("serialize tool raw-domain expression: {err}"))
    })?;
    let spec = scale_specs
        .entry(scale_name.to_string())
        .or_insert_with(|| PlotScaleSpec::Local(Scale::<Auto>::new().into_config()));
    let PlotScaleSpec::Local(config) = spec;
    if let Some(existing) = config
        .domain
        .as_option()
        .and_then(|domain| domain.raw_domain.as_ref())
        && !override_existing
        && existing != &raw_domain
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "tool cannot install raw_domain for scale '{scale_name}' because it already \
             has a different raw_domain"
        )));
    }

    let mut scale = Scale::<Auto>::from_config(config.clone()).raw_domain(param.expr());
    if disable_nice_zero {
        scale = scale.option("nice", lit(false)).option("zero", lit(false));
    }
    *config = scale.into_config();
    Ok(())
}

fn validate_tool_id(id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty()
        || id.contains('.')
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid chart tool id '{id}'; ids must be non-empty ASCII identifiers without periods"
        )));
    }
    Ok(())
}

fn describe_sharing(sharing: Sharing) -> String {
    match sharing.to_normalized() {
        Sharing::Free => "Free".to_string(),
        Sharing::Shared => "Shared".to_string(),
        Sharing::Level(level) => format!("Level({level})"),
    }
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::DefaultLogicalExprNodeExt;
    use avenger_chart_scales::Linear;
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::arrow::datatypes::DataType;
    use datafusion::prelude::{SessionContext, col, lit};

    use super::*;
    use crate::event as ev;
    use crate::event::ChartEventType;
    use crate::prelude::*;

    async fn data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        ctx.sql(
            "SELECT * FROM (VALUES
                ('A', 1.0, 2.0),
                ('A', 2.0, 3.0),
                ('B', 3.0, 4.0),
                ('B', 4.0, 5.0)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("data")
    }

    fn raw_domain_debug(compiled: &crate::plot::CompiledPlot, scale: &str) -> String {
        let PlotScaleSpec::Local(config) = compiled
            .scale_specs()
            .get(scale)
            .unwrap_or_else(|| panic!("missing scale {scale}"));
        let raw = config
            .domain
            .as_option()
            .and_then(|domain| domain.raw_domain.as_ref())
            .unwrap_or_else(|| panic!("missing raw_domain for {scale}"));
        format!(
            "{:?}",
            raw.to_expr(&SessionContext::new()).expect("raw expr")
        )
    }

    fn total_symbol_len(mark: &SceneMark) -> usize {
        match mark {
            SceneMark::Group(group) => group.marks.iter().map(total_symbol_len).sum(),
            SceneMark::Symbol(symbol) => symbol.len as usize,
            _ => 0,
        }
    }

    #[tokio::test]
    async fn pan_scroll_zoom_plain_cartesian_generates_params_bindings_and_raw_domains() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__enabled")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_pan_scroll_zoom__y_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 3);
        assert!(
            compiled
                .event_bindings()
                .iter()
                .any(|binding| binding.event_type == ChartEventType::DoubleClick)
        );
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_pan_scroll_zoom__x_domain"));
        assert!(raw_domain_debug(&compiled, "y").contains("__tool_pan_scroll_zoom__y_domain"));
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_column_mirrors_shared_scale() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_scale())
                    .y_with(col("y"), |c| c.free_scale()),
            )
            .tool(PanScrollZoom::cartesian());
        let compiled = Plot::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(leaf).column(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            Sharing::Level(u8::MAX)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__y_domain"].sharing,
            Sharing::Level(0)
        );
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_wrap_compiles() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_scale())
                    .y_with(col("y"), |c| c.share_scale()),
            )
            .tool(PanScrollZoom::cartesian());
        let compiled = Plot::<FacetWrap>::new()
            .data(df)
            .mark(Subplot::new(leaf).wrap(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            Sharing::Level(u8::MAX)
        );
        assert!(compiled.event_bindings().len() >= 2);
    }

    #[tokio::test]
    async fn leaf_tool_inside_nested_row_column_uses_level_sharing() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.with_scale_sharing(Sharing::Level(1)))
                    .y_with(col("y"), |c| c.with_scale_sharing(Sharing::Level(1))),
            )
            .tool(PanScrollZoom::cartesian());
        let column = Plot::<FacetColumn>::new().mark(Subplot::new(leaf).column(col("group_name")));
        let compiled = Plot::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(column).row(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__x_domain"].sharing,
            Sharing::Level(1)
        );
        assert_eq!(
            compiled.param_specs()["__tool_pan_scroll_zoom__y_domain"].sharing,
            Sharing::Level(1)
        );
    }

    #[derive(Clone)]
    struct CustomXTool;

    impl ChartTool<Cartesian> for CustomXTool {
        fn id(&self) -> &str {
            "custom_x"
        }

        fn expand(
            &self,
            _ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
            let param = Param::raw_domain("__tool_custom_x__x_domain");
            Ok(ToolExpansion::new()
                .param(param.clone(), ToolParamSharing::mirror_scale("x"))
                .scale_edit(ToolScaleEdit::raw_domain("x", param.name.clone()))
                .event_binding(
                    ChartEventBinding::on(ChartEventType::CanvasResize)
                        .set_param(&param, ev::interval(lit(0.0), lit(1.0)))
                        .preview(),
                ))
        }
    }

    #[tokio::test]
    async fn public_custom_tool_can_expand_to_param_binding_and_scale_edit() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(CustomXTool)
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_custom_x__x_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 1);
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_custom_x__x_domain"));
    }

    #[derive(Clone)]
    struct CustomSelectionTool;

    impl ChartTool<Cartesian> for CustomSelectionTool {
        fn id(&self) -> &str {
            "custom_selection"
        }

        fn expand(
            &self,
            _ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
            let store = Store::empty("__tool_custom_selection__boxes")
                .field("id", DataType::Utf8, false)
                .field("x_min", DataType::Float64, false)
                .field("x_max", DataType::Float64, false)
                .field("y_min", DataType::Float64, false)
                .field("y_max", DataType::Float64, false)
                .primary_key(["id"]);
            let selection =
                Selection::new("__tool_custom_selection__brush").empty_selects_nothing();
            Ok(ToolExpansion::new().store(store).selection(selection))
        }
    }

    #[tokio::test]
    async fn leaf_tool_inside_facet_can_expand_to_store_and_selection() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new()
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(CustomSelectionTool);
        let compiled = Plot::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(leaf).column(col("group_name")))
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .store_specs()
                .contains_key("__tool_custom_selection__boxes")
        );
        assert!(
            compiled
                .selection_specs()
                .contains_key("__tool_custom_selection__brush")
        );
    }

    #[tokio::test]
    async fn point_selection_tool_contributes_selection_bindings_and_metadata() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let picked = PointSelection::new("picked").field("group_name");
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y(col("y"))
                    .fill_with(lit("#b8beca"), |c| {
                        c.no_scale()
                            .when_value(picked.predicate(), lit("#2563eb"))
                            .no_legend()
                    }),
            )
            .tool(picked)
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_picked__enabled")
        );
        assert!(compiled.selection_specs().contains_key("picked"));
        assert_eq!(compiled.event_bindings().len(), 3);
        assert_eq!(compiled.tool_metadata().len(), 1);
        assert_eq!(compiled.tool_metadata()[0].id, "picked");
        assert_eq!(
            compiled.event_datum_types().get("group_name"),
            Some(&DataType::Utf8),
            "point selection should request clicked datum values"
        );
    }

    #[tokio::test]
    async fn box_zoom_cartesian_injects_unit_rect_mark_and_raw_domains() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(BoxZoom::cartesian())
            .compile(&ctx)
            .await
            .expect("compile");

        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_box_zoom__active")
        );
        assert!(
            compiled
                .param_specs()
                .contains_key("__tool_box_zoom__x_domain")
        );
        assert_eq!(compiled.event_bindings().len(), 5);
        assert!(
            compiled
                .marks()
                .iter()
                .any(|mark| mark.mark_type() == "rect")
        );
        assert!(raw_domain_debug(&compiled, "x").contains("__tool_box_zoom__x_domain"));
        assert!(raw_domain_debug(&compiled, "y").contains("__tool_box_zoom__y_domain"));
    }

    #[tokio::test]
    async fn unit_data_mark_renders_once_without_inherited_rows() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .unit_data()
                    .x_with(lit(10.0), |c| c.no_scale())
                    .y_with(lit(10.0), |c| c.no_scale()),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
        let symbol_count: usize = evaluated
            .scene_graph
            .marks
            .iter()
            .map(total_symbol_len)
            .sum();
        assert_eq!(symbol_count, 1);
    }

    #[tokio::test]
    async fn visible_false_suppresses_mark_rendering() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .unit_data()
                    .x_with(lit(10.0), |c| c.no_scale())
                    .y_with(lit(10.0), |c| c.no_scale())
                    .visible(lit(false)),
            )
            .compile(&ctx)
            .await
            .expect("compile");

        let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
        let symbol_count: usize = evaluated
            .scene_graph
            .marks
            .iter()
            .map(total_symbol_len)
            .sum();
        assert_eq!(symbol_count, 0);
    }

    #[tokio::test]
    async fn duplicate_tool_ids_error() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian())
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("duplicate ids should fail");
        };

        assert!(err.to_string().contains("Duplicate chart tool id"));
    }

    #[tokio::test]
    async fn invalid_tool_id_errors() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = Plot::<Cartesian>::new()
            .data(df)
            .mark(Symbol::new().x(col("x")).y(col("y")))
            .tool(PanScrollZoom::cartesian().id("bad.id"))
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("invalid tool id should fail");
        };

        assert!(err.to_string().contains("Invalid chart tool id"));
    }

    #[tokio::test]
    async fn pan_scroll_zoom_errors_without_matching_target() {
        let ctx = SessionContext::new();
        let result = Plot::<Cartesian>::new()
            .tool(PanScrollZoom::cartesian())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("no x/y scales should fail");
        };

        assert!(
            err.to_string()
                .contains("did not find an invertible scale target")
        );
    }

    #[tokio::test]
    async fn explicit_tool_param_sharing_must_cover_target_scale_sharing() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| c.share_scale())
                    .y(col("y")),
            )
            .tool(PanScrollZoom::cartesian().x_only().x_sharing(Sharing::Free))
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("too-narrow explicit sharing should fail");
        };

        assert!(
            err.to_string()
                .contains("must be shared at least as broadly")
        );
    }

    #[tokio::test]
    async fn existing_different_raw_domain_conflicts() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let result = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.raw_domain(ev::interval(lit(0.0), lit(1.0))))
                    })
                    .y(col("y")),
            )
            .tool(PanScrollZoom::cartesian().x_only())
            .compile(&ctx)
            .await;
        let Err(err) = result else {
            panic!("existing raw_domain should conflict");
        };

        assert!(
            err.to_string()
                .contains("already has a different raw_domain")
        );
    }
}
