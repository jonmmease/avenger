//! Chart tools that expand into params, scale edits, and event bindings.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    Auto, AvengerChartError, CompiledParamSpec, CoordinateSystemTransform,
    DefaultLogicalExprNodeExt, Param, Scale, Sharing,
};
use avenger_chart_scales::PlotScaleSpec;
use datafusion::{
    common::ScalarValue,
    functions::expr_fn::power,
    prelude::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::event::{self as ev, ChartEventBinding, ChartEventStream, ChartEventType};

/// Public trait for chart tools.
///
/// Tools are authoring-time packages. During plot compilation they expand into
/// ordinary chart params, scale edits, event bindings, and metadata.
pub trait ChartTool: Send + Sync + 'static {
    fn id(&self) -> &str;

    fn expand(&self, ctx: ToolExpansionContext<'_>) -> Result<ToolExpansion, AvengerChartError>;
}

#[derive(Clone, Copy, Debug)]
pub struct ToolExpansionContext<'a> {
    pub tool_id: &'a str,
}

#[derive(Clone, Debug, Default)]
pub struct ToolExpansion {
    pub params: Vec<ToolParamExpansion>,
    pub event_bindings: Vec<ChartEventBinding>,
    pub scale_edits: Vec<ToolScaleEdit>,
    pub metadata: Vec<ToolMetadata>,
}

impl ToolExpansion {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn param(mut self, param: Param, sharing: ToolParamSharing) -> Self {
        self.params.push(ToolParamExpansion { param, sharing });
        self
    }

    pub fn event_binding(mut self, binding: ChartEventBinding) -> Self {
        self.event_bindings.push(binding);
        self
    }

    pub fn scale_edit(mut self, edit: ToolScaleEdit) -> Self {
        self.scale_edits.push(edit);
        self
    }

    pub fn metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata.push(metadata);
        self
    }
}

#[derive(Clone, Debug)]
pub struct ToolParamExpansion {
    pub param: Param,
    pub sharing: ToolParamSharing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolParamSharing {
    Explicit(Sharing),
    MirrorScale { channel: String },
}

impl ToolParamSharing {
    pub fn explicit(sharing: Sharing) -> Self {
        Self::Explicit(sharing)
    }

    pub fn mirror_scale(channel: impl Into<String>) -> Self {
        Self::MirrorScale {
            channel: channel.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ToolScaleEdit {
    RawDomain {
        channel: String,
        param_name: String,
        override_existing: bool,
        disable_nice_zero: bool,
    },
}

impl ToolScaleEdit {
    pub fn raw_domain(channel: impl Into<String>, param_name: impl Into<String>) -> Self {
        Self::RawDomain {
            channel: channel.into(),
            param_name: param_name.into(),
            override_existing: false,
            disable_nice_zero: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub id: String,
    pub label: String,
    pub enabled_param: Option<String>,
}

impl ToolMetadata {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled_param: None,
        }
    }

    pub fn enabled_param(mut self, name: impl Into<String>) -> Self {
        self.enabled_param = Some(name.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct PanScrollZoom {
    id: String,
    x_channel: Option<String>,
    y_channel: Option<String>,
    x_domain_param: Option<Param>,
    y_domain_param: Option<Param>,
    x_sharing: Option<Sharing>,
    y_sharing: Option<Sharing>,
    drag_button: String,
    scroll_zoom: bool,
    zoom_base: f64,
    consume_wheel: bool,
    settle_exact: bool,
    enabled_by_default: bool,
}

impl PanScrollZoom {
    pub fn cartesian() -> Self {
        Self {
            id: "pan_scroll_zoom".to_string(),
            x_channel: Some("x".to_string()),
            y_channel: Some("y".to_string()),
            x_domain_param: None,
            y_domain_param: None,
            x_sharing: None,
            y_sharing: None,
            drag_button: "left".to_string(),
            scroll_zoom: true,
            zoom_base: 1.02,
            consume_wheel: true,
            settle_exact: false,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn x_channel(mut self, channel: impl Into<String>) -> Self {
        self.x_channel = Some(channel.into());
        self
    }

    pub fn y_channel(mut self, channel: impl Into<String>) -> Self {
        self.y_channel = Some(channel.into());
        self
    }

    pub fn x_only(mut self) -> Self {
        self.y_channel = None;
        self
    }

    pub fn y_only(mut self) -> Self {
        self.x_channel = None;
        self
    }

    pub fn x_domain_param(mut self, param: Param) -> Self {
        self.x_domain_param = Some(param);
        self
    }

    pub fn y_domain_param(mut self, param: Param) -> Self {
        self.y_domain_param = Some(param);
        self
    }

    pub fn x_sharing(mut self, sharing: Sharing) -> Self {
        self.x_sharing = Some(sharing);
        self
    }

    pub fn y_sharing(mut self, sharing: Sharing) -> Self {
        self.y_sharing = Some(sharing);
        self
    }

    pub fn drag_button(mut self, button: impl Into<String>) -> Self {
        self.drag_button = button.into();
        self
    }

    pub fn scroll_zoom(mut self, enabled: bool) -> Self {
        self.scroll_zoom = enabled;
        self
    }

    pub fn zoom_base(mut self, zoom_base: f64) -> Self {
        self.zoom_base = zoom_base;
        self
    }

    pub fn consume_wheel(mut self, consume: bool) -> Self {
        self.consume_wheel = consume;
        self
    }

    pub fn settle_exact(mut self, settle: bool) -> Self {
        self.settle_exact = settle;
        self
    }

    pub fn enabled_by_default(mut self, enabled: bool) -> Self {
        self.enabled_by_default = enabled;
        self
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.id, "enabled")
    }

    fn default_domain_param(&self, channel: &str) -> Param {
        Param::raw_domain(generated_tool_name(&self.id, &format!("{channel}_domain")))
    }
}

impl ChartTool for PanScrollZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(&self, _ctx: ToolExpansionContext<'_>) -> Result<ToolExpansion, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let mut expansion = ToolExpansion::new()
            .param(enabled.clone(), ToolParamSharing::Explicit(Sharing::Shared))
            .metadata(
                ToolMetadata::new(self.id.clone(), "Pan/Zoom").enabled_param(enabled.name.clone()),
            );

        let mut channels = Vec::new();
        if let Some(channel) = &self.x_channel {
            let param = self
                .x_domain_param
                .clone()
                .unwrap_or_else(|| self.default_domain_param(channel));
            let sharing = self
                .x_sharing
                .map(ToolParamSharing::Explicit)
                .unwrap_or_else(|| ToolParamSharing::mirror_scale(channel));
            channels.push((channel.clone(), param.clone()));
            expansion = expansion
                .param(param.clone(), sharing)
                .scale_edit(ToolScaleEdit::raw_domain(channel.clone(), param.name));
        }
        if let Some(channel) = &self.y_channel {
            let param = self
                .y_domain_param
                .clone()
                .unwrap_or_else(|| self.default_domain_param(channel));
            let sharing = self
                .y_sharing
                .map(ToolParamSharing::Explicit)
                .unwrap_or_else(|| ToolParamSharing::mirror_scale(channel));
            channels.push((channel.clone(), param.clone()));
            expansion = expansion
                .param(param.clone(), sharing)
                .scale_edit(ToolScaleEdit::raw_domain(channel.clone(), param.name));
        }

        if channels.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' must enable at least one coordinate channel",
                self.id
            )));
        }

        expansion = expansion.event_binding(drag_pan_binding(
            &enabled.name,
            &self.drag_button,
            &channels,
            self.settle_exact,
        ));

        if self.scroll_zoom {
            expansion = expansion.event_binding(scroll_zoom_binding(
                &enabled.name,
                &channels,
                self.zoom_base,
                self.consume_wheel,
            ));
        }

        Ok(expansion)
    }
}

fn drag_pan_binding(
    enabled_param: &str,
    drag_button: &str,
    channels: &[(String, Param)],
    settle_exact: bool,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .between(
            ChartEventStream::on(ChartEventType::MouseDown)
                .filter(ev::button().eq(lit(drag_button.to_string()))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .preview();

    for (channel, param) in channels {
        let delta = ev::event_at_start_coord(channel) - ev::start_coord(channel);
        binding = binding.set_param(
            param,
            ev::interval(
                ev::interval_start(ev::start_domain(channel)) - delta.clone(),
                ev::interval_end(ev::start_domain(channel)) - delta,
            ),
        );
    }

    if settle_exact {
        binding.settle_exact()
    } else {
        binding
    }
}

fn scroll_zoom_binding(
    enabled_param: &str,
    channels: &[(String, Param)],
    zoom_base: f64,
    consume_wheel: bool,
) -> ChartEventBinding {
    let factor = power(lit(zoom_base), lit(-1.0_f64) * ev::wheel_delta_y());
    let mut binding = ChartEventBinding::on(ChartEventType::MouseWheel)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::wheel_delta_y().not_eq(lit(0.0_f64)))
        .preview()
        .consume(consume_wheel);

    for (channel, param) in channels {
        binding = binding
            .filter(ev::event_coord(channel).is_not_null())
            .set_param(param, zoom_interval(channel, factor.clone()));
    }

    binding
}

fn zoom_interval(channel: &str, factor: Expr) -> Expr {
    let domain = ev::event_domain(channel);
    let anchor = ev::event_coord(channel);
    ev::interval(
        anchor.clone() + (ev::interval_start(domain.clone()) - anchor.clone()) * factor.clone(),
        anchor.clone() + (ev::interval_end(domain) - anchor) * factor,
    )
}

fn generated_tool_name(id: &str, suffix: &str) -> String {
    format!("__tool_{id}__{suffix}")
}

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
        assert_eq!(compiled.event_bindings().len(), 2);
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
