//! Built-in chart tools.

use avenger_chart_cartesian::{Cartesian, CartesianRectPositionChannels};
use avenger_chart_core::{
    AvengerChartError, ChannelConfig, ChartEventBinding, ChartEventStream, ChartEventType,
    ChartTool, CoordinateSystemCore, CoordinationScope, DomainCoordination,
    DomainCoordinationGroup, EmptySelectionBehavior, IntoExpr, Param, SceneGeometryHitPolicy,
    SceneGeometryQuery, SceneQueryDatumField, Selection, SelectionClauseUpdate, SelectionCombine,
    SelectionSceneQuery, SelectionUpdate, Store, StoreData, StoreRow, StoreUpdate, ToolExpansion,
    ToolExpansionContext, ToolMetadata, ToolParamSharing, ToolScaleEdit, event as ev, repeat,
};
use avenger_chart_marks::Rect;
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    functions::expr_fn::power,
    prelude::{Expr, col, lit},
};

#[derive(Clone, Debug)]
pub struct PanScrollZoom {
    id: String,
    x_channel: Option<String>,
    y_channel: Option<String>,
    x_domain_param: Option<Param>,
    y_domain_param: Option<Param>,
    x_sharing: Option<CoordinationScope>,
    y_sharing: Option<CoordinationScope>,
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

    pub fn x_sharing(mut self, sharing: CoordinationScope) -> Self {
        self.x_sharing = Some(sharing);
        self
    }

    pub fn y_sharing(mut self, sharing: CoordinationScope) -> Self {
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

    fn default_domain_param_for_target(
        &self,
        channel: &str,
        coordination: Option<&DomainCoordination>,
    ) -> Param {
        let Some(coordination) = coordination else {
            return self.default_domain_param(channel);
        };
        let DomainCoordinationGroup::Named(group) = &coordination.group else {
            return self.default_domain_param(channel);
        };
        if group == channel {
            self.default_domain_param(channel)
        } else {
            Param::raw_domain(generated_tool_name(&self.id, &format!("domain__{group}")))
        }
    }
}

impl ChartTool<Cartesian> for PanScrollZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let mut expansion = ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .metadata(
                ToolMetadata::new(self.id.clone(), "Pan/Zoom").enabled_param(enabled.name.clone()),
            );

        let mut scale_channels = Vec::new();
        let mut event_targets: Vec<(Vec<String>, Param)> = Vec::new();
        if let Some(channel) = &self.x_channel {
            let target = ctx.single_domain_coordination_for_channel(channel);
            let param = self
                .x_domain_param
                .clone()
                .unwrap_or_else(|| self.default_domain_param_for_target(channel, target.as_ref()));
            let sharing = match (
                self.x_sharing,
                self.x_domain_param.is_none(),
                target.as_ref(),
            ) {
                (Some(scope), _, _) => ToolParamSharing::Explicit(scope),
                (None, true, Some(target)) => ToolParamSharing::Explicit(target.scope),
                (None, _, _) => ToolParamSharing::mirror_scale(channel),
            };
            scale_channels.push((channel.clone(), param.clone(), sharing));
            push_pan_zoom_event_target(
                &mut event_targets,
                channel.clone(),
                param.clone(),
                self.x_domain_param.is_none(),
                target,
            );
        }
        if let Some(channel) = &self.y_channel {
            let target = ctx.single_domain_coordination_for_channel(channel);
            let param = self
                .y_domain_param
                .clone()
                .unwrap_or_else(|| self.default_domain_param_for_target(channel, target.as_ref()));
            let sharing = match (
                self.y_sharing,
                self.y_domain_param.is_none(),
                target.as_ref(),
            ) {
                (Some(scope), _, _) => ToolParamSharing::Explicit(scope),
                (None, true, Some(target)) => ToolParamSharing::Explicit(target.scope),
                (None, _, _) => ToolParamSharing::mirror_scale(channel),
            };
            scale_channels.push((channel.clone(), param.clone(), sharing));
            push_pan_zoom_event_target(
                &mut event_targets,
                channel.clone(),
                param.clone(),
                self.y_domain_param.is_none(),
                target,
            );
        }

        if scale_channels.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' must enable at least one coordinate channel",
                self.id
            )));
        }

        let mut registered_domain_params: Vec<(String, ToolParamSharing)> = Vec::new();
        for (channel, param, sharing) in scale_channels {
            if let Some((_, existing_sharing)) = registered_domain_params
                .iter()
                .find(|(name, _)| name == &param.name)
            {
                if existing_sharing != &sharing {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool '{}' generated raw-domain param '{}' with incompatible sharing",
                        self.id, param.name
                    )));
                }
            } else {
                registered_domain_params.push((param.name.clone(), sharing.clone()));
                expansion = expansion.param(param.clone(), sharing);
            }
            expansion = expansion.scale_edit(ToolScaleEdit::raw_domain(channel, param.name));
        }

        expansion = expansion.event_binding(drag_pan_binding(
            &enabled.name,
            &self.drag_button,
            &event_targets,
            self.settle_exact,
        ));

        if self.scroll_zoom {
            expansion = expansion.event_binding(scroll_zoom_binding(
                &enabled.name,
                &event_targets,
                self.zoom_base,
                self.consume_wheel,
            ));
        }
        expansion = expansion.event_binding(reset_view_binding(&enabled.name, &event_targets));

        Ok(expansion)
    }
}

fn push_pan_zoom_event_target(
    targets: &mut Vec<(Vec<String>, Param)>,
    channel: String,
    param: Param,
    generated_param: bool,
    coordination: Option<DomainCoordination>,
) {
    if generated_param
        && coordination.is_some()
        && let Some((channels, _)) = targets.iter_mut().find(|(_, existing_param)| {
            existing_param.name == param.name && existing_param.default == param.default
        })
    {
        if !channels.contains(&channel) {
            channels.push(channel);
        }
        return;
    }

    targets.push((vec![channel], param));
}

#[derive(Clone, Debug)]
pub struct PointSelection {
    selection_id: String,
    tool_id: String,
    dimensions: Vec<PointSelectionDimension>,
    clause_id: Option<Expr>,
    facet_scope: CoordinationScope,
    facet_context_fields: Vec<(String, Expr)>,
    empty: EmptySelectionBehavior,
    shift_toggle: bool,
    double_click_clear: bool,
    enabled_by_default: bool,
}

#[derive(Clone, Debug)]
struct PointSelectionDimension {
    field_expr: Expr,
    datum_field: String,
}

impl PointSelection {
    pub fn new(selection_id: impl Into<String>) -> Self {
        let selection_id = selection_id.into();
        Self {
            tool_id: selection_id.clone(),
            selection_id,
            dimensions: Vec::new(),
            clause_id: None,
            facet_scope: CoordinationScope::Free,
            facet_context_fields: Vec::new(),
            empty: EmptySelectionBehavior::SelectNothing,
            shift_toggle: true,
            double_click_clear: true,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, tool_id: impl Into<String>) -> Self {
        self.tool_id = tool_id.into();
        self
    }

    pub fn field(self, field: impl Into<String>) -> Self {
        let field = field.into();
        self.dimension(col(&field), field)
    }

    pub fn dimension(mut self, field_expr: impl IntoExpr, datum_field: impl Into<String>) -> Self {
        self.dimensions.push(PointSelectionDimension {
            field_expr: field_expr.into_expr(),
            datum_field: datum_field.into(),
        });
        self
    }

    pub fn clause_id(mut self, id: impl IntoExpr) -> Self {
        self.clause_id = Some(id.into_expr());
        self
    }

    pub fn facet_scope(mut self, scope: CoordinationScope) -> Self {
        self.facet_scope = scope;
        self
    }

    pub fn facet_context_field(mut self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.facet_context_fields
            .push((id.into(), expr.into_expr()));
        self
    }

    pub fn empty_selects_nothing(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectNothing;
        self
    }

    pub fn empty_selects_all(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectAll;
        self
    }

    pub fn shift_toggle(mut self, enabled: bool) -> Self {
        self.shift_toggle = enabled;
        self
    }

    pub fn double_click_clear(mut self, enabled: bool) -> Self {
        self.double_click_clear = enabled;
        self
    }

    pub fn enabled_by_default(mut self, enabled: bool) -> Self {
        self.enabled_by_default = enabled;
        self
    }

    pub fn predicate(&self) -> Expr {
        Selection::new(&self.selection_id).predicate()
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.tool_id, "enabled")
    }

    fn selection(&self) -> Selection {
        let mut selection =
            Selection::new(&self.selection_id).combine(avenger_chart_core::SelectionCombine::Union);
        selection = match self.empty {
            EmptySelectionBehavior::SelectAll => selection.empty_selects_all(),
            EmptySelectionBehavior::SelectNothing => selection.empty_selects_nothing(),
        };
        for (id, expr) in &self.facet_context_fields {
            selection = selection.facet_context_field(id.clone(), expr.clone());
        }
        selection
    }

    fn clause(&self) -> Result<SelectionClauseUpdate, AvengerChartError> {
        match self.dimensions.as_slice() {
            [] => Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires at least one point selection dimension",
                self.tool_id
            ))),
            [dimension] if self.clause_id.is_none() => Ok(SelectionClauseUpdate::equality_value(
                dimension.field_expr.clone(),
                ev::datum(&dimension.datum_field),
            )
            .facet_scope(self.facet_scope)),
            dimensions => {
                let Some(clause_id) = &self.clause_id else {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "tool '{}' has multiple point selection dimensions and requires \
                         .clause_id(...)",
                        self.tool_id
                    )));
                };
                let mut builder = SelectionClauseUpdate::equality(clause_id.clone())
                    .facet_scope(self.facet_scope);
                for dimension in dimensions {
                    builder = builder.dimension(
                        dimension.field_expr.clone(),
                        ev::datum(&dimension.datum_field),
                    );
                }
                Ok(builder.build())
            }
        }
    }
}

impl<C: CoordinateSystemCore> ChartTool<C> for PointSelection {
    fn id(&self) -> &str {
        &self.tool_id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<C>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let clause = self.clause()?;
        let mut expansion = ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .selection(self.selection())
            .event_binding(point_selection_replace_binding(
                &enabled.name,
                &self.selection_id,
                &self.dimensions,
                clause.clone(),
            ))
            .metadata(
                ToolMetadata::new(self.tool_id.clone(), "Point Selection")
                    .enabled_param(enabled.name.clone()),
            );

        if self.shift_toggle {
            expansion = expansion.event_binding(point_selection_toggle_binding(
                &enabled.name,
                &self.selection_id,
                &self.dimensions,
                clause,
            ));
        }
        if self.double_click_clear {
            expansion = expansion.event_binding(point_selection_clear_binding(
                &enabled.name,
                &self.selection_id,
            ));
        }

        Ok(expansion)
    }
}

fn point_selection_replace_binding(
    enabled_param: &str,
    selection_id: &str,
    dimensions: &[PointSelectionDimension],
    clause: SelectionClauseUpdate,
) -> ChartEventBinding {
    point_selection_click_binding(enabled_param, dimensions, false)
        .set_selection(selection_id, SelectionUpdate::replace_all_clauses([clause]))
}

fn point_selection_toggle_binding(
    enabled_param: &str,
    selection_id: &str,
    dimensions: &[PointSelectionDimension],
    clause: SelectionClauseUpdate,
) -> ChartEventBinding {
    point_selection_click_binding(enabled_param, dimensions, true)
        .set_selection(selection_id, SelectionUpdate::toggle_clause(clause))
}

fn point_selection_click_binding(
    enabled_param: &str,
    dimensions: &[PointSelectionDimension],
    shift: bool,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::button().eq(lit("left")))
        .filter(ev::shift().eq(lit(shift)))
        .exact();
    for dimension in dimensions {
        binding = binding.filter(ev::datum(&dimension.datum_field).is_not_null());
    }
    binding
}

fn point_selection_clear_binding(enabled_param: &str, selection_id: &str) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .clear_selection(selection_id)
        .exact()
}

#[derive(Clone, Debug)]
pub struct LassoSelection {
    selection_id: String,
    tool_id: String,
    fields: Vec<LassoSelectionField>,
    mark_ids: Vec<String>,
    facet_scope: CoordinationScope,
    facet_context_fields: Vec<(String, Expr)>,
    empty: EmptySelectionBehavior,
    drag_button: String,
    event_path_min_distance_px: f32,
    double_click_clear: bool,
    enabled_by_default: bool,
}

#[derive(Clone, Debug)]
struct LassoSelectionField {
    id: String,
    datum_field: String,
    field_expr: Expr,
}

impl LassoSelection {
    pub fn new(selection_id: impl Into<String>) -> Self {
        let selection_id = selection_id.into();
        Self {
            tool_id: selection_id.clone(),
            selection_id,
            fields: Vec::new(),
            mark_ids: Vec::new(),
            facet_scope: CoordinationScope::Free,
            facet_context_fields: Vec::new(),
            empty: EmptySelectionBehavior::SelectNothing,
            drag_button: "left".to_string(),
            event_path_min_distance_px:
                avenger_chart_core::event::DEFAULT_EVENT_PATH_MIN_DISTANCE_PX,
            double_click_clear: true,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, tool_id: impl Into<String>) -> Self {
        self.tool_id = tool_id.into();
        self
    }

    pub fn field(self, field: impl Into<String>) -> Self {
        let field = field.into();
        self.dimension(col(&field), field)
    }

    pub fn mark(mut self, id: impl Into<String>) -> Self {
        self.mark_ids = vec![id.into()];
        self
    }

    pub fn marks<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.mark_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn dimension(self, field_expr: impl IntoExpr, datum_field: impl Into<String>) -> Self {
        let datum_field = datum_field.into();
        self.dimension_named(datum_field.clone(), field_expr, datum_field)
    }

    pub fn dimension_named(
        mut self,
        id: impl Into<String>,
        field_expr: impl IntoExpr,
        datum_field: impl Into<String>,
    ) -> Self {
        self.fields.push(LassoSelectionField {
            id: id.into(),
            datum_field: datum_field.into(),
            field_expr: field_expr.into_expr(),
        });
        self
    }

    pub fn facet_scope(mut self, scope: CoordinationScope) -> Self {
        self.facet_scope = scope;
        self
    }

    pub fn facet_context_field(mut self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.facet_context_fields
            .push((id.into(), expr.into_expr()));
        self
    }

    pub fn empty_selects_nothing(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectNothing;
        self
    }

    pub fn empty_selects_all(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectAll;
        self
    }

    pub fn drag_button(mut self, button: impl Into<String>) -> Self {
        self.drag_button = button.into();
        self
    }

    pub fn event_path_min_distance_px(mut self, distance: f32) -> Self {
        self.event_path_min_distance_px = distance;
        self
    }

    pub fn double_click_clear(mut self, enabled: bool) -> Self {
        self.double_click_clear = enabled;
        self
    }

    pub fn enabled_by_default(mut self, enabled: bool) -> Self {
        self.enabled_by_default = enabled;
        self
    }

    pub fn predicate(&self) -> Expr {
        Selection::new(&self.selection_id).predicate()
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.tool_id, "enabled")
    }

    fn selection(&self) -> Selection {
        let mut selection =
            Selection::new(&self.selection_id).combine(avenger_chart_core::SelectionCombine::Union);
        selection = match self.empty {
            EmptySelectionBehavior::SelectAll => selection.empty_selects_all(),
            EmptySelectionBehavior::SelectNothing => selection.empty_selects_nothing(),
        };
        for (id, expr) in &self.facet_context_fields {
            selection = selection.facet_context_field(id.clone(), expr.clone());
        }
        selection
    }

    fn scene_query_update(&self) -> Result<SelectionSceneQuery, AvengerChartError> {
        if self.fields.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires at least one lasso selection field",
                self.tool_id
            )));
        }
        if !self.event_path_min_distance_px.is_finite() || self.event_path_min_distance_px < 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires a finite non-negative event path minimum distance",
                self.tool_id
            )));
        }

        let mut query = SceneGeometryQuery::polygon(ev::event_path())
            .hit_policy(SceneGeometryHitPolicy::AnchorInside);
        if !self.mark_ids.is_empty() {
            query = query.marks(self.mark_ids.clone());
        }
        for field in &self.fields {
            query = query.datum_field(
                SceneQueryDatumField::new(&field.id)
                    .datum(&field.datum_field)
                    .field_expr(field.field_expr.clone()),
            );
        }
        let unique_fields = self
            .fields
            .iter()
            .map(|field| field.id.clone())
            .collect::<Vec<_>>();
        Ok(SelectionSceneQuery::new(query.unique_by(unique_fields)).sharing(self.facet_scope))
    }
}

impl<C: CoordinateSystemCore> ChartTool<C> for LassoSelection {
    fn id(&self) -> &str {
        &self.tool_id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<C>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let mut expansion = ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .selection(self.selection())
            .event_binding(lasso_selection_drag_binding(
                &enabled.name,
                &self.selection_id,
                &self.drag_button,
                self.event_path_min_distance_px,
                self.scene_query_update()?,
            ))
            .metadata(
                ToolMetadata::new(self.tool_id.clone(), "Lasso Selection")
                    .enabled_param(enabled.name.clone()),
            );

        if self.double_click_clear {
            expansion = expansion.event_binding(lasso_selection_clear_binding(
                &enabled.name,
                &self.selection_id,
            ));
        }

        Ok(expansion)
    }
}

fn lasso_selection_drag_binding(
    enabled_param: &str,
    selection_id: &str,
    drag_button: &str,
    event_path_min_distance_px: f32,
    update: SelectionSceneQuery,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .between(
            ChartEventStream::on(ChartEventType::MouseDown)
                .filter(ev::button().eq(lit(drag_button.to_string()))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .set_selection_at_start_scope(
            selection_id,
            SelectionUpdate::replace_all_from_scene_query(update),
        )
        .event_path_min_distance_px(event_path_min_distance_px)
        .preview()
        .settle_exact()
}

fn lasso_selection_clear_binding(enabled_param: &str, selection_id: &str) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .clear_selection(selection_id)
        .exact()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSelectionResolve {
    Global,
    Union,
    Intersect,
}

#[derive(Clone, Debug)]
pub struct BoxSelection {
    selection_id: String,
    tool_id: String,
    x_channel: String,
    y_channel: String,
    x_dimension: Expr,
    y_dimension: Expr,
    resolve: BoxSelectionResolve,
    facet_scope: CoordinationScope,
    facet_context_fields: Vec<(String, Expr)>,
    empty: EmptySelectionBehavior,
    drag_button: String,
    repeat_cell_chrome: bool,
    double_click_clear: bool,
    enabled_by_default: bool,
}

impl BoxSelection {
    pub fn cartesian(selection_id: impl Into<String>) -> Self {
        let selection_id = selection_id.into();
        Self {
            tool_id: selection_id.clone(),
            selection_id,
            x_channel: "x".to_string(),
            y_channel: "y".to_string(),
            x_dimension: col("x"),
            y_dimension: col("y"),
            resolve: BoxSelectionResolve::Global,
            facet_scope: CoordinationScope::Free,
            facet_context_fields: Vec::new(),
            empty: EmptySelectionBehavior::SelectNothing,
            drag_button: "left".to_string(),
            repeat_cell_chrome: false,
            double_click_clear: true,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, tool_id: impl Into<String>) -> Self {
        self.tool_id = tool_id.into();
        self
    }

    pub fn x_channel(mut self, channel: impl Into<String>) -> Self {
        self.x_channel = channel.into();
        self
    }

    pub fn y_channel(mut self, channel: impl Into<String>) -> Self {
        self.y_channel = channel.into();
        self
    }

    pub fn channels(mut self, x: impl Into<String>, y: impl Into<String>) -> Self {
        self.x_channel = x.into();
        self.y_channel = y.into();
        self
    }

    pub fn x_dimension(mut self, expr: impl IntoExpr) -> Self {
        self.x_dimension = expr.into_expr();
        self
    }

    pub fn y_dimension(mut self, expr: impl IntoExpr) -> Self {
        self.y_dimension = expr.into_expr();
        self
    }

    pub fn dimensions(mut self, x: impl IntoExpr, y: impl IntoExpr) -> Self {
        self.x_dimension = x.into_expr();
        self.y_dimension = y.into_expr();
        self
    }

    pub fn resolve(mut self, resolve: BoxSelectionResolve) -> Self {
        self.resolve = resolve;
        if matches!(resolve, BoxSelectionResolve::Intersect) {
            self.empty = EmptySelectionBehavior::SelectAll;
        }
        self
    }

    pub fn facet_scope(mut self, scope: CoordinationScope) -> Self {
        self.facet_scope = scope;
        self
    }

    pub fn facet_context_field(mut self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.facet_context_fields
            .push((id.into(), expr.into_expr()));
        self
    }

    pub fn empty_selects_nothing(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectNothing;
        self
    }

    pub fn empty_selects_all(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectAll;
        self
    }

    pub fn drag_button(mut self, button: impl Into<String>) -> Self {
        self.drag_button = button.into();
        self
    }

    pub fn repeat_cell_chrome(mut self) -> Self {
        self.repeat_cell_chrome = true;
        self
    }

    pub fn double_click_clear(mut self, enabled: bool) -> Self {
        self.double_click_clear = enabled;
        self
    }

    pub fn enabled_by_default(mut self, enabled: bool) -> Self {
        self.enabled_by_default = enabled;
        self
    }

    pub fn predicate(&self) -> Expr {
        Selection::new(&self.selection_id).predicate()
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.tool_id, "enabled")
    }

    fn store_name(&self) -> String {
        generated_tool_name(&self.tool_id, "boxes")
    }

    fn selection(&self) -> Selection {
        let combine = match self.resolve {
            BoxSelectionResolve::Intersect => SelectionCombine::Intersect,
            BoxSelectionResolve::Global | BoxSelectionResolve::Union => SelectionCombine::Union,
        };
        let mut selection = Selection::new(&self.selection_id).combine(combine);
        selection = match self.empty {
            EmptySelectionBehavior::SelectAll => selection.empty_selects_all(),
            EmptySelectionBehavior::SelectNothing => selection.empty_selects_nothing(),
        };
        for (id, expr) in &self.facet_context_fields {
            selection = selection.facet_context_field(id.clone(), expr.clone());
        }
        selection
    }

    fn store(&self) -> Store {
        Store::empty(self.store_name())
            .field("id", DataType::Utf8, false)
            .field("cell_id", DataType::Utf8, false)
            .field("row_id", DataType::Utf8, false)
            .field("column_id", DataType::Utf8, false)
            .field("x_min", DataType::Float64, false)
            .field("x_max", DataType::Float64, false)
            .field("y_min", DataType::Float64, false)
            .field("y_max", DataType::Float64, false)
            .primary_key(["id"])
            .sharing(self.facet_scope)
    }

    fn overlay_mark(&self) -> Rect<Cartesian> {
        let store_name = self.store_name();
        let mark = Rect::<Cartesian>::new()
            .data_store(StoreData::new(store_name))
            .exclude_from_scale_domains()
            .x_with(col("x_min"), |c| c.with_scale_name(&self.x_channel))
            .x2_with(col("x_max"), |c| c.with_scale_name(&self.x_channel))
            .y_with(col("y_min"), |c| c.with_scale_name(&self.y_channel))
            .y2_with(col("y_max"), |c| c.with_scale_name(&self.y_channel))
            .fill("rgba(37, 99, 235, 0.10)")
            .stroke("#2563eb")
            .stroke_width(1.5)
            .zindex(10_000);

        if self.repeat_cell_chrome {
            mark.opacity_with(lit(0.0), |c| {
                c.no_scale()
                    .when_value(repeat::current_cell_predicate(), lit(1.0))
            })
        } else {
            mark
        }
    }

    fn row_id_expr(&self) -> Expr {
        match self.resolve {
            BoxSelectionResolve::Global => lit("active"),
            BoxSelectionResolve::Union | BoxSelectionResolve::Intersect => {
                if self.repeat_cell_chrome {
                    repeat::cell_id()
                } else {
                    lit("active")
                }
            }
        }
    }

    fn cell_id_expr(&self) -> Expr {
        if self.repeat_cell_chrome {
            repeat::cell_id()
        } else {
            lit("active")
        }
    }

    fn row_id_field_expr(&self) -> Expr {
        if self.repeat_cell_chrome {
            repeat::row_id()
        } else {
            lit("active")
        }
    }

    fn column_id_field_expr(&self) -> Expr {
        if self.repeat_cell_chrome {
            repeat::column_id()
        } else {
            lit("active")
        }
    }

    fn selection_clause(&self) -> SelectionClauseUpdate {
        let x_interval = drag_domain_interval(&self.x_channel);
        let y_interval = drag_domain_interval(&self.y_channel);
        SelectionClauseUpdate::interval(self.row_id_expr())
            .facet_scope(self.facet_scope)
            .dimension(self.x_dimension.clone())
            .endpoints(
                ev::interval_start(x_interval.clone()),
                ev::interval_end(x_interval),
            )
            .dimension(self.y_dimension.clone())
            .endpoints(
                ev::interval_start(y_interval.clone()),
                ev::interval_end(y_interval),
            )
            .build()
    }

    fn selection_update(&self) -> SelectionUpdate {
        match self.resolve {
            BoxSelectionResolve::Global => {
                SelectionUpdate::replace_all_clauses([self.selection_clause()])
            }
            BoxSelectionResolve::Union | BoxSelectionResolve::Intersect => {
                SelectionUpdate::upsert_clause(self.selection_clause())
            }
        }
    }

    fn store_row(&self) -> StoreRow {
        let x_interval = drag_domain_interval(&self.x_channel);
        let y_interval = drag_domain_interval(&self.y_channel);
        StoreRow::new()
            .field("id", self.row_id_expr())
            .field("cell_id", self.cell_id_expr())
            .field("row_id", self.row_id_field_expr())
            .field("column_id", self.column_id_field_expr())
            .field("x_min", ev::interval_start(x_interval.clone()))
            .field("x_max", ev::interval_end(x_interval))
            .field("y_min", ev::interval_start(y_interval.clone()))
            .field("y_max", ev::interval_end(y_interval))
    }

    fn store_update(&self) -> StoreUpdate {
        match self.resolve {
            BoxSelectionResolve::Global => StoreUpdate::replace_rows([self.store_row()]),
            BoxSelectionResolve::Union | BoxSelectionResolve::Intersect => {
                StoreUpdate::upsert_rows([self.store_row()])
            }
        }
    }
}

impl ChartTool<Cartesian> for BoxSelection {
    fn id(&self) -> &str {
        &self.tool_id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
        if self.x_channel.is_empty() || self.y_channel.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires non-empty x and y channels",
                self.tool_id
            )));
        }

        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let store = self.store();
        let mut expansion = ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .store(store.clone())
            .selection(self.selection())
            .event_binding(box_selection_drag_binding(
                &enabled.name,
                &self.selection_id,
                &store.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                self.selection_update(),
                self.store_update(),
            ))
            .event_binding(box_selection_release_binding(
                &enabled.name,
                &self.selection_id,
                &store.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                self.selection_update(),
                self.store_update(),
            ))
            .mark(self.overlay_mark())
            .metadata(
                ToolMetadata::new(self.tool_id.clone(), "Box Selection")
                    .enabled_param(enabled.name.clone()),
            );

        if self.double_click_clear {
            expansion = expansion.event_binding(box_selection_clear_binding(
                &enabled.name,
                &self.selection_id,
                &store.name,
            ));
        }

        Ok(expansion)
    }
}

#[allow(clippy::too_many_arguments)]
fn box_selection_drag_binding(
    enabled_param: &str,
    selection_id: &str,
    store_name: &str,
    drag_button: &str,
    x_channel: &str,
    y_channel: &str,
    selection_update: SelectionUpdate,
    store_update: StoreUpdate,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown)
                .filter(ev::button().eq(lit(drag_button.to_string()))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::start_coord(x_channel).is_not_null())
        .filter(ev::start_coord(y_channel).is_not_null())
        .filter(ev::event_at_start_clipped_coord(x_channel).is_not_null())
        .filter(ev::event_at_start_clipped_coord(y_channel).is_not_null())
        .set_selection_at_start_scope(selection_id, selection_update)
        .set_store_at_start_scope(store_name, store_update)
        .preview()
}

#[allow(clippy::too_many_arguments)]
fn box_selection_release_binding(
    enabled_param: &str,
    selection_id: &str,
    store_name: &str,
    drag_button: &str,
    x_channel: &str,
    y_channel: &str,
    selection_update: SelectionUpdate,
    store_update: StoreUpdate,
) -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown)
            .filter(ev::button().eq(lit(drag_button.to_string()))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(ev::start_coord(x_channel).is_not_null())
    .filter(ev::start_coord(y_channel).is_not_null())
    .filter(ev::event_at_start_clipped_coord(x_channel).is_not_null())
    .filter(ev::event_at_start_clipped_coord(y_channel).is_not_null())
    .set_selection_at_start_scope(selection_id, selection_update)
    .set_store_at_start_scope(store_name, store_update)
    .exact()
}

fn box_selection_clear_binding(
    enabled_param: &str,
    selection_id: &str,
    store_name: &str,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .clear_selection(selection_id)
        .set_store_replacing_scopes(store_name, StoreUpdate::clear())
        .exact()
}

#[derive(Clone, Debug)]
pub struct BoxZoom {
    id: String,
    x_channel: String,
    y_channel: String,
    x_domain_param: Option<Param>,
    y_domain_param: Option<Param>,
    x_sharing: Option<CoordinationScope>,
    y_sharing: Option<CoordinationScope>,
    drag_button: String,
    min_size_px: f64,
    enabled_by_default: bool,
}

impl BoxZoom {
    pub fn cartesian() -> Self {
        Self {
            id: "box_zoom".to_string(),
            x_channel: "x".to_string(),
            y_channel: "y".to_string(),
            x_domain_param: None,
            y_domain_param: None,
            x_sharing: None,
            y_sharing: None,
            drag_button: "left".to_string(),
            min_size_px: 4.0,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn x_channel(mut self, channel: impl Into<String>) -> Self {
        self.x_channel = channel.into();
        self
    }

    pub fn y_channel(mut self, channel: impl Into<String>) -> Self {
        self.y_channel = channel.into();
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

    pub fn x_sharing(mut self, sharing: CoordinationScope) -> Self {
        self.x_sharing = Some(sharing);
        self
    }

    pub fn y_sharing(mut self, sharing: CoordinationScope) -> Self {
        self.y_sharing = Some(sharing);
        self
    }

    pub fn drag_button(mut self, button: impl Into<String>) -> Self {
        self.drag_button = button.into();
        self
    }

    pub fn min_size_px(mut self, size: f64) -> Self {
        self.min_size_px = size;
        self
    }

    pub fn enabled_by_default(mut self, enabled: bool) -> Self {
        self.enabled_by_default = enabled;
        self
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.id, "enabled")
    }

    fn active_param_name(&self) -> String {
        generated_tool_name(&self.id, "active")
    }

    fn default_domain_param(&self, channel: &str) -> Param {
        Param::raw_domain(generated_tool_name(&self.id, &format!("{channel}_domain")))
    }

    fn overlay_param(&self, suffix: &str) -> Param {
        Param::new(
            generated_tool_name(&self.id, suffix),
            ScalarValue::Float64(Some(0.0)),
        )
    }
}

impl ChartTool<Cartesian> for BoxZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
        if self.x_channel.is_empty() || self.y_channel.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires non-empty x and y channels",
                self.id
            )));
        }
        if self.min_size_px < 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires a non-negative minimum drag size",
                self.id
            )));
        }

        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let active = Param::new(self.active_param_name(), ScalarValue::Boolean(Some(false)));
        let box_x0 = self.overlay_param("x0");
        let box_y0 = self.overlay_param("y0");
        let box_x1 = self.overlay_param("x1");
        let box_y1 = self.overlay_param("y1");
        let x_domain = self
            .x_domain_param
            .clone()
            .unwrap_or_else(|| self.default_domain_param(&self.x_channel));
        let y_domain = self
            .y_domain_param
            .clone()
            .unwrap_or_else(|| self.default_domain_param(&self.y_channel));

        let x_sharing = self
            .x_sharing
            .map(ToolParamSharing::Explicit)
            .unwrap_or_else(|| ToolParamSharing::mirror_scale(&self.x_channel));
        let y_sharing = self
            .y_sharing
            .map(ToolParamSharing::Explicit)
            .unwrap_or_else(|| ToolParamSharing::mirror_scale(&self.y_channel));

        let overlay = Rect::<Cartesian>::new()
            .unit_data()
            .exclude_from_scale_domains()
            .visible(ev::param(&active))
            .x(box_x0.expr())
            .x2_with(box_x1.expr(), |c| c.with_scale_name(&self.x_channel))
            .y(box_y0.expr())
            .y2_with(box_y1.expr(), |c| c.with_scale_name(&self.y_channel))
            .fill("rgba(66, 133, 244, 0.08)")
            .stroke("#4285f4")
            .stroke_width(1.5)
            .zindex(10_000);

        let channels = [
            (self.x_channel.clone(), x_domain.clone()),
            (self.y_channel.clone(), y_domain.clone()),
        ];

        Ok(ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .param(
                active.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param(
                box_x0.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param(
                box_y0.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param(
                box_x1.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param(
                box_y1.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param(x_domain.clone(), x_sharing)
            .param(y_domain.clone(), y_sharing)
            .scale_edit(ToolScaleEdit::raw_domain(
                self.x_channel.clone(),
                x_domain.name.clone(),
            ))
            .scale_edit(ToolScaleEdit::raw_domain(
                self.y_channel.clone(),
                y_domain.name.clone(),
            ))
            .event_binding(box_zoom_start_binding(
                &enabled.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                &active,
                &box_x0,
                &box_y0,
                &box_x1,
                &box_y1,
            ))
            .event_binding(box_zoom_drag_binding(
                &enabled.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                &active,
                &box_x0,
                &box_y0,
                &box_x1,
                &box_y1,
            ))
            .event_binding(box_zoom_cancel_binding(
                &enabled.name,
                &self.drag_button,
                self.min_size_px,
                &active,
            ))
            .event_binding(box_zoom_release_binding(
                &enabled.name,
                &self.drag_button,
                self.min_size_px,
                &active,
                &channels,
            ))
            .event_binding(box_zoom_reset_binding(&enabled.name, &active, &channels))
            .mark(overlay)
            .metadata(
                ToolMetadata::new(self.id.clone(), "Box Zoom").enabled_param(enabled.name.clone()),
            ))
    }
}

#[allow(clippy::too_many_arguments)]
fn box_zoom_start_binding(
    enabled_param: &str,
    drag_button: &str,
    x_channel: &str,
    y_channel: &str,
    active: &Param,
    box_x0: &Param,
    box_y0: &Param,
    box_x1: &Param,
    box_y1: &Param,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::MouseDown)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::button().eq(lit(drag_button.to_string())))
        .filter(ev::event_coord(x_channel).is_not_null())
        .filter(ev::event_coord(y_channel).is_not_null())
        .set_param(active, lit(true))
        .set_param(box_x0, ev::event_coord(x_channel))
        .set_param(box_y0, ev::event_coord(y_channel))
        .set_param(box_x1, ev::event_coord(x_channel))
        .set_param(box_y1, ev::event_coord(y_channel))
        .preview()
}

#[allow(clippy::too_many_arguments)]
fn box_zoom_drag_binding(
    enabled_param: &str,
    drag_button: &str,
    x_channel: &str,
    y_channel: &str,
    active: &Param,
    box_x0: &Param,
    box_y0: &Param,
    box_x1: &Param,
    box_y1: &Param,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::start_coord(x_channel).is_not_null())
        .filter(ev::start_coord(y_channel).is_not_null())
        .filter(ev::event_at_start_clipped_coord(x_channel).is_not_null())
        .filter(ev::event_at_start_clipped_coord(y_channel).is_not_null())
        .between(
            box_zoom_drag_start_stream(drag_button),
            box_zoom_drag_end_stream(drag_button),
        )
        .set_param_at_start_scope(active, lit(true))
        .set_param_at_start_scope(box_x0, ev::start_coord(x_channel))
        .set_param_at_start_scope(box_y0, ev::start_coord(y_channel))
        .set_param_at_start_scope(box_x1, ev::event_at_start_clipped_coord(x_channel))
        .set_param_at_start_scope(box_y1, ev::event_at_start_clipped_coord(y_channel))
        .preview()
}

fn box_zoom_cancel_binding(
    enabled_param: &str,
    drag_button: &str,
    min_size_px: f64,
    active: &Param,
) -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(drag_distance_squared().lt(lit(min_size_px * min_size_px)))
    .set_param_at_start_scope(active, lit(false))
    .preview()
}

fn box_zoom_release_binding(
    enabled_param: &str,
    drag_button: &str,
    min_size_px: f64,
    active: &Param,
    channels: &[(String, Param); 2],
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(drag_distance_squared().gt_eq(lit(min_size_px * min_size_px)))
    .set_param_at_start_scope(active, lit(false))
    .exact();

    for (channel, param) in channels {
        binding = binding
            .filter(ev::start_coord(channel).is_not_null())
            .filter(ev::event_at_start_clipped_coord(channel).is_not_null())
            .set_param_at_start_scope(param, drag_domain_interval(channel));
    }

    binding
}

fn box_zoom_reset_binding(
    enabled_param: &str,
    active: &Param,
    channels: &[(String, Param); 2],
) -> ChartEventBinding {
    let targets = channels
        .iter()
        .map(|(channel, param)| (vec![channel.clone()], param.clone()))
        .collect::<Vec<_>>();
    reset_view_binding(enabled_param, &targets).set_param(active, lit(false))
}

fn box_zoom_drag_start_stream(drag_button: &str) -> ChartEventStream {
    ChartEventStream::on(ChartEventType::MouseDown)
        .filter(ev::button().eq(lit(drag_button.to_string())))
}

fn box_zoom_drag_end_stream(drag_button: &str) -> ChartEventStream {
    ChartEventStream::on(ChartEventType::MouseUp)
        .filter(ev::button().eq(lit(drag_button.to_string())))
}

fn drag_distance_squared() -> Expr {
    let dx = ev::x() - ev::start_x();
    let dy = ev::y() - ev::start_y();
    dx.clone() * dx + dy.clone() * dy
}

fn drag_domain_interval(channel: &str) -> Expr {
    ev::interval_ordered(
        ev::start_coord(channel),
        ev::event_at_start_clipped_coord(channel),
    )
}

fn drag_pan_binding(
    enabled_param: &str,
    drag_button: &str,
    targets: &[(Vec<String>, Param)],
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

    for (channels, param) in targets {
        let channel = primary_event_channel(channels);
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
    targets: &[(Vec<String>, Param)],
    zoom_base: f64,
    consume_wheel: bool,
) -> ChartEventBinding {
    let factor = power(lit(zoom_base), lit(-1.0_f64) * ev::wheel_delta_y());
    let mut binding = ChartEventBinding::on(ChartEventType::MouseWheel)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::wheel_delta_y().not_eq(lit(0.0_f64)))
        .preview()
        .consume(consume_wheel);

    for (channels, param) in targets {
        let channel = primary_event_channel(channels);
        binding = binding
            .filter(ev::event_coord(channel).is_not_null())
            .set_param(param, zoom_interval(channel, factor.clone()));
    }

    binding
}

fn reset_view_binding(enabled_param: &str, targets: &[(Vec<String>, Param)]) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .exact();

    for (channels, param) in targets {
        let channel = primary_event_channel(channels);
        binding = binding.filter(ev::event_coord(channel).is_not_null());
        binding = binding.set_param(param, lit(param.default.clone()));
    }

    binding
}

fn primary_event_channel(channels: &[String]) -> &str {
    channels
        .iter()
        .find(|channel| channel.as_str() == "x")
        .or_else(|| channels.first())
        .map(String::as_str)
        .expect("pan/zoom event target must have at least one channel")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_scroll_zoom_expands_to_params_bindings_edits_and_metadata() {
        let tool = PanScrollZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.params.len(), 3);
        assert_eq!(expansion.event_bindings.len(), 3);
        assert_eq!(expansion.scale_edits.len(), 2);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_pan_scroll_zoom__enabled")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_pan_scroll_zoom__y_domain")
        );
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 3);
        assert_eq!(reset.assignments.len(), 2);
        assert_eq!(
            reset.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Exact
        );
    }

    #[test]
    fn pan_scroll_zoom_x_only_expands_single_domain_target() {
        let tool = PanScrollZoom::cartesian().id("nav").x_only();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.params.len(), 2);
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 2);
        assert_eq!(reset.assignments.len(), 1);
        assert_eq!(expansion.scale_edits.len(), 1);
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_nav__x_domain")
        );
        assert!(
            !expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_nav__y_domain")
        );
    }

    #[test]
    fn pan_scroll_zoom_groups_generated_params_by_domain_target() {
        let tool = PanScrollZoom::cartesian();
        let coordination =
            DomainCoordination::named(CoordinationScope::Shared, "measurement").unwrap();
        let targets = vec![
            avenger_chart_core::ToolScaleTarget {
                coord_channel: "x".to_string(),
                scale_name: "x".to_string(),
                domain_coordination: coordination.clone(),
            },
            avenger_chart_core::ToolScaleTarget {
                coord_channel: "y".to_string(),
                scale_name: "y".to_string(),
                domain_coordination: coordination,
            },
        ];
        let expansion = tool
            .expand(ToolExpansionContext::new(ChartTool::id(&tool), &targets))
            .expect("expand");

        assert_eq!(expansion.params.len(), 2);
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_pan_scroll_zoom__domain__measurement")
        );
        assert_eq!(expansion.scale_edits.len(), 2);

        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert_eq!(drag.assignments.len(), 1);
        assert_eq!(
            drag.assignments[0].param_name,
            "__tool_pan_scroll_zoom__domain__measurement"
        );
    }

    #[test]
    fn point_selection_expands_to_selection_param_bindings_and_metadata() {
        let tool = PointSelection::new("picked").field("category");
        let expansion = <PointSelection as ChartTool<Cartesian>>::expand(
            &tool,
            ToolExpansionContext::empty(<PointSelection as ChartTool<Cartesian>>::id(&tool)),
        )
        .expect("expand");

        assert_eq!(expansion.params.len(), 1);
        assert_eq!(expansion.selections.len(), 1);
        assert_eq!(expansion.event_bindings.len(), 3);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(expansion.stores.is_empty());
        assert!(expansion.marks.is_empty());
        assert!(expansion.scale_edits.is_empty());
        assert_eq!(expansion.params[0].param.name, "__tool_picked__enabled");
        assert_eq!(expansion.selections[0].id, "picked");
        assert_eq!(expansion.metadata[0].id, "picked");
        assert_eq!(
            expansion
                .event_bindings
                .iter()
                .filter(|binding| binding.event_type == ChartEventType::Click)
                .count(),
            2
        );
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding.event_type == ChartEventType::DoubleClick
                && binding.selection_assignments.len() == 1
        }));
    }

    #[test]
    fn point_selection_multiple_dimensions_require_clause_id() {
        let tool = PointSelection::new("picked")
            .dimension(col("category"), "category")
            .dimension(col("region"), "region");
        let err = match <PointSelection as ChartTool<Cartesian>>::expand(
            &tool,
            ToolExpansionContext::empty(<PointSelection as ChartTool<Cartesian>>::id(&tool)),
        ) {
            Ok(_) => panic!("missing clause id should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("requires .clause_id(...)"));
    }

    #[test]
    fn point_selection_disabled_default_sets_enabled_param_false() {
        let tool = PointSelection::new("picked")
            .field("category")
            .enabled_by_default(false);
        let expansion = <PointSelection as ChartTool<Cartesian>>::expand(
            &tool,
            ToolExpansionContext::empty(<PointSelection as ChartTool<Cartesian>>::id(&tool)),
        )
        .expect("expand");

        assert_eq!(
            expansion.params[0].param.default,
            ScalarValue::Boolean(Some(false))
        );
        assert!(
            expansion
                .event_bindings
                .iter()
                .all(|binding| binding.filters.len() >= 1),
            "every binding should include the enabled-param filter"
        );
    }

    #[test]
    fn lasso_selection_expands_to_selection_query_binding_and_metadata() {
        let tool = LassoSelection::new("picked")
            .field("point_id")
            .mark("points")
            .event_path_min_distance_px(7.0)
            .facet_scope(CoordinationScope::Shared);
        let expansion = <LassoSelection as ChartTool<Cartesian>>::expand(
            &tool,
            ToolExpansionContext::empty(<LassoSelection as ChartTool<Cartesian>>::id(&tool)),
        )
        .expect("expand");

        assert_eq!(expansion.params.len(), 1);
        assert_eq!(expansion.selections.len(), 1);
        assert_eq!(expansion.event_bindings.len(), 2);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(expansion.stores.is_empty());
        assert!(expansion.marks.is_empty());
        assert!(expansion.scale_edits.is_empty());
        assert_eq!(expansion.params[0].param.name, "__tool_picked__enabled");
        assert_eq!(expansion.selections[0].id, "picked");
        assert_eq!(expansion.metadata[0].id, "picked");

        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert!(drag.between.is_some());
        assert_eq!(drag.event_path_min_distance_px, Some(7.0));
        assert_eq!(drag.selection_assignments.len(), 1);
        assert_eq!(
            drag.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Preview
        );
        assert!(drag.settle_exact);

        let assignment = &drag.selection_assignments[0];
        assert_eq!(assignment.selection_id, "picked");
        let SelectionUpdate::ReplaceAllFromSceneQuery { query } = &assignment.update else {
            panic!("lasso drag should use a scene-query selection update");
        };
        assert_eq!(query.sharing, CoordinationScope::Shared);
        assert_eq!(query.query.datum_fields.len(), 1);
        assert_eq!(query.query.datum_fields[0].id, "point_id");
        assert_eq!(query.query.target.mark_ids(), &["points".to_string()]);
        assert_eq!(query.query.hit_policy, SceneGeometryHitPolicy::AnchorInside);
        assert!(
            matches!(
                query.query.geometry,
                avenger_chart_core::SceneGeometryQueryGeometry::Polygon { .. }
            ),
            "lasso selection should use the event-path polygon query"
        );

        assert!(expansion.event_bindings.iter().any(|binding| {
            binding.event_type == ChartEventType::DoubleClick
                && binding.selection_assignments.len() == 1
        }));
    }

    #[test]
    fn lasso_selection_requires_at_least_one_field() {
        let tool = LassoSelection::new("picked");
        let err = match <LassoSelection as ChartTool<Cartesian>>::expand(
            &tool,
            ToolExpansionContext::empty(<LassoSelection as ChartTool<Cartesian>>::id(&tool)),
        ) {
            Ok(_) => panic!("missing lasso field should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("requires at least one"));
    }

    #[test]
    fn box_selection_expands_to_store_selection_bindings_and_overlay_mark() {
        let tool = BoxSelection::cartesian("brush").dimensions(col("source_x"), col("source_y"));
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.params.len(), 1);
        assert_eq!(expansion.stores.len(), 1);
        assert_eq!(expansion.selections.len(), 1);
        assert_eq!(expansion.event_bindings.len(), 3);
        assert_eq!(expansion.marks.len(), 1);
        assert_eq!(expansion.metadata.len(), 1);
        assert_eq!(expansion.params[0].param.name, "__tool_brush__enabled");
        assert_eq!(expansion.stores[0].name, "__tool_brush__boxes");
        assert_eq!(expansion.stores[0].primary_key, ["id"]);
        assert_eq!(expansion.stores[0].sharing, CoordinationScope::Free);
        assert_eq!(expansion.selections[0].id, "brush");
        assert_eq!(expansion.selections[0].combine, SelectionCombine::Union);
        assert_eq!(
            expansion.selections[0].empty,
            EmptySelectionBehavior::SelectNothing
        );
        assert!(
            expansion
                .event_bindings
                .iter()
                .any(|binding| binding.event_type == ChartEventType::CursorMoved
                    && binding.selection_assignments.len() == 1
                    && binding.store_assignments.len() == 1)
        );
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding
                .between
                .as_ref()
                .is_some_and(|between| between.emit_end_event)
                && binding.selection_assignments.len() == 1
                && binding.store_assignments.len() == 1
        }));
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding.event_type == ChartEventType::DoubleClick
                && binding.selection_assignments.len() == 1
                && binding.store_assignments.len() == 1
        }));
    }

    #[test]
    fn box_selection_repeat_union_uses_upsert_and_repeat_cell_chrome() {
        let tool = BoxSelection::cartesian("brush")
            .dimensions(repeat::column(), repeat::row())
            .resolve(BoxSelectionResolve::Union)
            .repeat_cell_chrome();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.selections[0].combine, SelectionCombine::Union);
        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        let SelectionUpdate::UpsertClauses { clauses } = &drag.selection_assignments[0].update
        else {
            panic!("repeat union should upsert selection clauses");
        };
        assert_eq!(clauses.len(), 1);
        let StoreUpdate::UpsertRows { rows } = &drag.store_assignments[0].update else {
            panic!("repeat union should upsert store rows");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(expansion.marks.len(), 1);
    }

    #[test]
    fn box_selection_intersect_defaults_empty_to_all() {
        let tool = BoxSelection::cartesian("brush").resolve(BoxSelectionResolve::Intersect);
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.selections[0].combine, SelectionCombine::Intersect);
        assert_eq!(
            expansion.selections[0].empty,
            EmptySelectionBehavior::SelectAll
        );
    }

    #[test]
    fn box_zoom_expands_to_overlay_params_bindings_edits_and_mark() {
        let tool = BoxZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.params.len(), 8);
        assert_eq!(expansion.event_bindings.len(), 5);
        assert_eq!(expansion.scale_edits.len(), 2);
        assert_eq!(expansion.marks.len(), 1);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(
            expansion
                .params
                .iter()
                .any(|p| p.param.name == "__tool_box_zoom__active")
        );
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding
                .between
                .as_ref()
                .is_some_and(|between| between.emit_end_event)
        }));
        assert!(
            expansion
                .event_bindings
                .iter()
                .flat_map(|binding| binding.assignments.iter())
                .any(|assignment| assignment.scope
                    == avenger_chart_core::event::ChartEventAssignmentScope::Start)
        );
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 3);
        assert_eq!(reset.assignments.len(), 3);
        assert_eq!(
            reset.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Exact
        );
    }
}
