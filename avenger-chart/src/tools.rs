//! Chart tool compilation support and built-in tool re-exports.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    Auto, AvengerChartError, ChartEventBinding, CompiledParamSpec, CoordinateSystemTransform,
    DefaultLogicalExprNodeExt, Param, Scale, Sharing,
};
use avenger_chart_scales::PlotScaleSpec;
use datafusion::prelude::lit;
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;

pub use avenger_chart_core::{
    ChartTool, ToolExpansion, ToolExpansionContext, ToolMetadata, ToolParamExpansion,
    ToolParamSharing, ToolScaleEdit,
};
pub use avenger_chart_tools::PanScrollZoom;

pub(crate) struct ToolCompileContext {
    state: Arc<Mutex<ToolCompileState>>,
    active: Arc<Vec<ActiveToolExpansion>>,
}

impl ToolCompileContext {
    pub(crate) fn root() -> Self {
        Self {
            state: Arc::new(Mutex::new(ToolCompileState::default())),
            active: Arc::new(Vec::new()),
        }
    }

    pub(crate) fn from_parent_with_tools(
        parent: Option<&ToolCompileContext>,
        tools: &[Arc<dyn ChartTool>],
    ) -> Result<Self, AvengerChartError> {
        let state = parent
            .map(|ctx| ctx.state.clone())
            .unwrap_or_else(|| Arc::new(Mutex::new(ToolCompileState::default())));
        let mut active = parent
            .map(|ctx| ctx.active.as_ref().clone())
            .unwrap_or_default();

        for tool in tools {
            let id = tool.id().to_string();
            validate_tool_id(&id)?;
            let expansion = tool.expand(ToolExpansionContext { tool_id: &id })?;
            let active_expansion = ActiveToolExpansion {
                id: id.clone(),
                expansion: expansion.clone(),
            };
            state
                .lock()
                .expect("tool compile state lock poisoned")
                .register_expansion(&id, &expansion)?;
            active.push(active_expansion);
        }

        Ok(Self {
            state,
            active: Arc::new(active),
        })
    }

    pub(crate) fn downcast(ctx: avenger_chart_core::CompileContext<'_>) -> Option<&Self> {
        ctx.downcast_ref::<Self>()
    }

    pub(crate) fn apply_scale_edits(
        &self,
        coord_transform: &dyn CoordinateSystemTransform,
        scale_to_coord_channel: &HashMap<String, String>,
        scale_specs: &mut HashMap<String, PlotScaleSpec>,
        scale_sharing: &HashMap<String, Sharing>,
    ) -> Result<(), AvengerChartError> {
        let invertible = coord_transform.interaction_invertible_channels();
        if invertible.is_empty() {
            return Ok(());
        }

        for active in self.active.iter() {
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
}

#[derive(Clone)]
struct ActiveToolExpansion {
    id: String,
    expansion: ToolExpansion,
}

#[derive(Default)]
struct ToolCompileState {
    tool_ids: HashSet<String>,
    params: IndexMap<String, GeneratedParamState>,
    event_bindings: Vec<ChartEventBinding>,
    metadata: Vec<ToolMetadata>,
    expected_targets: BTreeMap<(String, String, String), usize>,
}

impl ToolCompileState {
    fn register_expansion(
        &mut self,
        id: &str,
        expansion: &ToolExpansion,
    ) -> Result<(), AvengerChartError> {
        if !self.tool_ids.insert(id.to_string()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate chart tool id '{id}'"
            )));
        }

        for param in &expansion.params {
            self.register_param(param)?;
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
    async fn root_tool_reaches_faceted_cartesian_leaf_and_mirrors_shared_scale() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), |c| c.share_scale())
                .y_with(col("y"), |c| c.free_scale()),
        );
        let compiled = Plot::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(leaf).column(col("group_name")))
            .tool(PanScrollZoom::cartesian())
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
    async fn root_tool_reaches_facet_wrap_cartesian_leaf() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), |c| c.share_scale())
                .y_with(col("y"), |c| c.share_scale()),
        );
        let compiled = Plot::<FacetWrap>::new()
            .data(df)
            .mark(Subplot::new(leaf).wrap(col("group_name")))
            .tool(PanScrollZoom::cartesian())
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
    async fn root_tool_reaches_nested_row_column_leaf_with_level_sharing() {
        let ctx = SessionContext::new();
        let df = data(&ctx).await;
        let leaf = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), |c| c.with_scale_sharing(Sharing::Level(1)))
                .y_with(col("y"), |c| c.with_scale_sharing(Sharing::Level(1))),
        );
        let column = Plot::<FacetColumn>::new().mark(Subplot::new(leaf).column(col("group_name")));
        let compiled = Plot::<FacetRow>::new()
            .data(df)
            .mark(Subplot::new(column).row(col("group_name")))
            .tool(PanScrollZoom::cartesian())
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

    impl ChartTool for CustomXTool {
        fn id(&self) -> &str {
            "custom_x"
        }

        fn expand(
            &self,
            _ctx: ToolExpansionContext<'_>,
        ) -> Result<ToolExpansion, AvengerChartError> {
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
