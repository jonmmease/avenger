//! Built-in chart tools.

pub mod language;

use avenger_chart_cartesian::{Cartesian, CartesianRectPositionChannels};
use avenger_chart_core::{
    AvengerChartError, ChannelConfig, ChartEventBinding, ChartEventStream, ChartEventType,
    ChartTool, CoordinateSystemCore, CoordinationScope, CursorStyle, DomainCoordination,
    DomainCoordinationGroup, EmptySelectionBehavior, IntoExpr, Param, ScaleChannelConfig,
    SceneGeometryHitPolicy, SceneGeometryQuery, SceneQueryDatumField, Selection,
    SelectionClauseUpdate, SelectionCombine, SelectionSceneQuery, SelectionUpdate, Store,
    StoreData, StoreRow, StoreUpdate, ToolBehaviorExpansion, ToolExpansionContext, ToolMetadata,
    ToolParamSharing, ToolScaleEdit, event as ev, repeat,
};
use avenger_chart_marks::Rect;
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    functions::expr_fn::{abs, power},
    prelude::{Expr, col, lit, when},
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
        repeat_cell_id: Option<&str>,
    ) -> Param {
        let Some(coordination) = coordination else {
            return self.default_domain_param(channel);
        };
        let mut param = if let DomainCoordinationGroup::Named(group) = &coordination.group {
            if group == channel {
                self.default_domain_param(channel)
            } else {
                Param::raw_domain(generated_tool_name(&self.id, &format!("domain__{group}")))
            }
        } else {
            self.default_domain_param(channel)
        };

        if coordination.scope.is_free()
            && let Some(cell_id) = repeat_cell_id
        {
            param.name = format!("{}__{}", param.name, sanitize_tool_name_part(cell_id));
        }
        param
    }
}

impl ChartTool<Cartesian> for PanScrollZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let mut expansion = ToolBehaviorExpansion::new(ctx.instance_id.clone())
            .param_as(
                "enabled",
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .metadata(
                ToolMetadata::new(self.id.clone(), "Pan/Zoom").enabled_param(enabled.name.clone()),
            );

        let mut scale_channels = Vec::new();
        let mut event_targets: Vec<(Vec<String>, Param)> = Vec::new();
        let repeat_cell_id = ctx.repeat_cell_id();
        if let Some(channel) = &self.x_channel {
            let target = ctx.single_domain_coordination_for_channel(channel);
            let param = self.x_domain_param.clone().unwrap_or_else(|| {
                self.default_domain_param_for_target(
                    channel,
                    target.as_ref(),
                    repeat_cell_id.as_deref(),
                )
            });
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
            let param = self.y_domain_param.clone().unwrap_or_else(|| {
                self.default_domain_param_for_target(
                    channel,
                    target.as_ref(),
                    repeat_cell_id.as_deref(),
                )
            });
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
                expansion = expansion.export_param_as(format!("{channel}_domain"), &param);
            } else {
                registered_domain_params.push((param.name.clone(), sharing.clone()));
                expansion = expansion.param_as(format!("{channel}_domain"), param.clone(), sharing);
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
                self.settle_exact,
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
    combine: SelectionCombine,
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
            combine: SelectionCombine::Union,
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

    pub fn combine(mut self, combine: SelectionCombine) -> Self {
        self.combine = combine;
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
        let mut selection = Selection::new(&self.selection_id).combine(self.combine);
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
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<C>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let clause = self.clause()?;
        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let mut expansion = ToolBehaviorExpansion::new(ctx.instance_id.clone())
            .param_as("enabled", enabled.clone(), shared.clone())
            .selection_as("selection", self.selection())
            .event_binding(point_selection_replace_binding(
                &enabled.name,
                &self.selection_id,
                &self.dimensions,
                clause.clone(),
            ))
            .event_binding(point_selection_cursor_binding(
                ChartEventType::MarkMouseEnter,
                &enabled.name,
                &self.dimensions,
                CursorStyle::Pointer,
            ))
            .event_binding(point_selection_cursor_binding(
                ChartEventType::MarkMouseLeave,
                &enabled.name,
                &self.dimensions,
                CursorStyle::Default,
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

fn point_selection_cursor_binding(
    event_type: ChartEventType,
    enabled_param: &str,
    dimensions: &[PointSelectionDimension],
    style: CursorStyle,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(event_type);
    if event_type == ChartEventType::MarkMouseEnter {
        binding = binding.filter(ev::param(enabled_param).eq(lit(true)));
    }
    binding = binding.set_cursor(ev::cursor(style));
    for dimension in dimensions {
        binding = binding.filter(ev::datum(&dimension.datum_field).is_not_null());
    }
    binding
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
    combine: SelectionCombine,
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
            combine: SelectionCombine::Union,
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

    pub fn combine(mut self, combine: SelectionCombine) -> Self {
        self.combine = combine;
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
        let mut selection = Selection::new(&self.selection_id).combine(self.combine);
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
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<C>, AvengerChartError> {
        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let mut expansion = ToolBehaviorExpansion::new(ctx.instance_id.clone())
            .param_as(
                "enabled",
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .selection_as("selection", self.selection())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitAspectBox {
    CoordinateMetric,
    Viewport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedUnitAspectBox {
    mode: UnitAspectBox,
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
    unit_aspect_box: Option<UnitAspectBox>,
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
            unit_aspect_box: None,
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

    pub fn unit_aspect(self) -> Self {
        self.unit_aspect_box(UnitAspectBox::CoordinateMetric)
    }

    pub fn unit_aspect_box(mut self, mode: UnitAspectBox) -> Self {
        self.unit_aspect_box = Some(mode);
        self
    }

    pub fn predicate(&self) -> Expr {
        Selection::new(&self.selection_id).predicate()
    }

    fn enabled_param_name(&self) -> String {
        generated_tool_name(&self.tool_id, "enabled")
    }

    fn store_name(&self) -> String {
        generated_tool_name(&self.tool_id, "store")
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
            .x_with(col("x_min"), |c| {
                c.with_scale_name(&self.x_channel)
                    .exclude_from_scale_domain()
            })
            .x2_with(col("x_max"), |c| {
                c.with_scale_name(&self.x_channel)
                    .exclude_from_scale_domain()
            })
            .y_with(col("y_min"), |c| {
                c.with_scale_name(&self.y_channel)
                    .exclude_from_scale_domain()
            })
            .y2_with(col("y_max"), |c| {
                c.with_scale_name(&self.y_channel)
                    .exclude_from_scale_domain()
            })
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

    fn selection_clause(
        &self,
        unit_aspect_box: Option<ResolvedUnitAspectBox>,
    ) -> SelectionClauseUpdate {
        let x_interval = constrained_drag_interval(
            &self.x_channel,
            &self.x_channel,
            &self.y_channel,
            unit_aspect_box,
        );
        let y_interval = constrained_drag_interval(
            &self.y_channel,
            &self.x_channel,
            &self.y_channel,
            unit_aspect_box,
        );
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

    fn selection_update(&self, unit_aspect_box: Option<ResolvedUnitAspectBox>) -> SelectionUpdate {
        match self.resolve {
            BoxSelectionResolve::Global => {
                SelectionUpdate::replace_all_clauses([self.selection_clause(unit_aspect_box)])
            }
            BoxSelectionResolve::Union | BoxSelectionResolve::Intersect => {
                SelectionUpdate::upsert_clause(self.selection_clause(unit_aspect_box))
            }
        }
    }

    fn store_row(&self, unit_aspect_box: Option<ResolvedUnitAspectBox>) -> StoreRow {
        let x_interval = constrained_drag_interval(
            &self.x_channel,
            &self.x_channel,
            &self.y_channel,
            unit_aspect_box,
        );
        let y_interval = constrained_drag_interval(
            &self.y_channel,
            &self.x_channel,
            &self.y_channel,
            unit_aspect_box,
        );
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

    fn store_update(&self, unit_aspect_box: Option<ResolvedUnitAspectBox>) -> StoreUpdate {
        match self.resolve {
            BoxSelectionResolve::Global => {
                StoreUpdate::replace_rows([self.store_row(unit_aspect_box)])
            }
            BoxSelectionResolve::Union | BoxSelectionResolve::Intersect => {
                StoreUpdate::upsert_rows([self.store_row(unit_aspect_box)])
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
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
        if self.x_channel.is_empty() || self.y_channel.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires non-empty x and y channels",
                self.tool_id
            )));
        }
        let unit_aspect_box = resolve_unit_aspect_box(
            &ctx,
            self.unit_aspect_box,
            &self.tool_id,
            &self.x_channel,
            &self.y_channel,
        )?;

        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let store = self.store();
        let mut expansion = ToolBehaviorExpansion::new(ctx.instance_id.clone())
            .param_as(
                "enabled",
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .store_as("store", store.clone())
            .selection_as("selection", self.selection())
            .event_binding(box_selection_drag_binding(
                &enabled.name,
                &self.selection_id,
                &store.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                self.selection_update(unit_aspect_box),
                self.store_update(unit_aspect_box),
                unit_aspect_box,
            ))
            .event_binding(box_selection_release_binding(
                &enabled.name,
                &self.selection_id,
                &store.name,
                &self.drag_button,
                &self.x_channel,
                &self.y_channel,
                self.selection_update(unit_aspect_box),
                self.store_update(unit_aspect_box),
                unit_aspect_box,
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
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
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
        .preview();
    for filter in unit_aspect_box_filters(unit_aspect_box, x_channel, y_channel) {
        binding = binding.filter(filter);
    }
    binding
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
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on_between_end(
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
    .exact();
    for filter in unit_aspect_box_filters(unit_aspect_box, x_channel, y_channel) {
        binding = binding.filter(filter);
    }
    binding
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
    unit_aspect_box: Option<UnitAspectBox>,
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
            unit_aspect_box: None,
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

    pub fn unit_aspect(self) -> Self {
        self.unit_aspect_box(UnitAspectBox::Viewport)
    }

    pub fn unit_aspect_box(mut self, mode: UnitAspectBox) -> Self {
        self.unit_aspect_box = Some(mode);
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
        ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolBehaviorExpansion<Cartesian>, AvengerChartError> {
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
        let unit_aspect_box = resolve_unit_aspect_box(
            &ctx,
            self.unit_aspect_box,
            &self.id,
            &self.x_channel,
            &self.y_channel,
        )?;

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
            .visible(ev::param(&active))
            .x_with(box_x0.expr(), |c| c.exclude_from_scale_domain())
            .x2_with(box_x1.expr(), |c| {
                c.with_scale_name(&self.x_channel)
                    .exclude_from_scale_domain()
            })
            .y_with(box_y0.expr(), |c| c.exclude_from_scale_domain())
            .y2_with(box_y1.expr(), |c| {
                c.with_scale_name(&self.y_channel)
                    .exclude_from_scale_domain()
            })
            .fill("rgba(66, 133, 244, 0.08)")
            .stroke("#4285f4")
            .stroke_width(1.5)
            .zindex(10_000);

        let channels = [
            (self.x_channel.clone(), x_domain.clone()),
            (self.y_channel.clone(), y_domain.clone()),
        ];

        Ok(ToolBehaviorExpansion::new(ctx.instance_id.clone())
            .param_as(
                "enabled",
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .param_as(
                "active",
                active.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param_as(
                "box_x0",
                box_x0.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param_as(
                "box_y0",
                box_y0.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param_as(
                "box_x1",
                box_x1.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param_as(
                "box_y1",
                box_y1.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Free),
            )
            .param_as("x_domain", x_domain.clone(), x_sharing)
            .param_as("y_domain", y_domain.clone(), y_sharing)
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
                unit_aspect_box,
            ))
            .event_binding(box_zoom_cancel_binding(
                &enabled.name,
                &self.drag_button,
                self.min_size_px,
                &active,
                unit_aspect_box,
                &self.x_channel,
                &self.y_channel,
            ))
            .event_binding(box_zoom_release_binding(
                &enabled.name,
                &self.drag_button,
                self.min_size_px,
                &active,
                &channels,
                unit_aspect_box,
            ))
            .event_binding(box_zoom_reset_binding(&enabled.name, &active, &channels))
            .mark_part("selection", overlay)
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
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
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
        .set_param_at_start_scope(
            box_x1,
            constrained_drag_endpoint(x_channel, x_channel, y_channel, unit_aspect_box),
        )
        .set_param_at_start_scope(
            box_y1,
            constrained_drag_endpoint(y_channel, x_channel, y_channel, unit_aspect_box),
        )
        .preview();
    for filter in unit_aspect_box_filters(unit_aspect_box, x_channel, y_channel) {
        binding = binding.filter(filter);
    }
    binding
}

fn box_zoom_cancel_binding(
    enabled_param: &str,
    drag_button: &str,
    min_size_px: f64,
    active: &Param,
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
    x_channel: &str,
    y_channel: &str,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(
        constrained_drag_distance_squared_px(unit_aspect_box, x_channel, y_channel)
            .lt(lit(min_size_px * min_size_px)),
    )
    .set_param_at_start_scope(active, lit(false))
    .preview();
    for filter in unit_aspect_box_filters(unit_aspect_box, x_channel, y_channel) {
        binding = binding.filter(filter);
    }
    binding
}

fn box_zoom_release_binding(
    enabled_param: &str,
    drag_button: &str,
    min_size_px: f64,
    active: &Param,
    channels: &[(String, Param); 2],
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> ChartEventBinding {
    let x_channel = channels[0].0.as_str();
    let y_channel = channels[1].0.as_str();
    let mut binding = ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(
        constrained_drag_distance_squared_px(unit_aspect_box, x_channel, y_channel)
            .gt_eq(lit(min_size_px * min_size_px)),
    )
    .set_param_at_start_scope(active, lit(false))
    .exact();
    for filter in unit_aspect_box_filters(unit_aspect_box, x_channel, y_channel) {
        binding = binding.filter(filter);
    }

    for (channel, param) in channels {
        binding = binding
            .filter(ev::start_coord(channel).is_not_null())
            .filter(ev::event_at_start_clipped_coord(channel).is_not_null())
            .set_param_at_start_scope(
                param,
                constrained_drag_interval(channel, x_channel, y_channel, unit_aspect_box),
            );
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

fn resolve_unit_aspect_box(
    ctx: &ToolExpansionContext<'_>,
    requested: Option<UnitAspectBox>,
    tool_id: &str,
    x_channel: &str,
    y_channel: &str,
) -> Result<Option<ResolvedUnitAspectBox>, AvengerChartError> {
    let Some(mode) = requested else {
        return Ok(None);
    };
    if ctx
        .coordinate_metric_for_channels(x_channel, y_channel)
        .is_none()
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "tool '{tool_id}' requested a unit-aspect box, but no active coordinate metric \
             targets channels '{x_channel}' and '{y_channel}'"
        )));
    }
    Ok(Some(ResolvedUnitAspectBox { mode }))
}

fn constrained_drag_interval(
    channel: &str,
    x_channel: &str,
    y_channel: &str,
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> Expr {
    ev::interval_ordered(
        ev::start_coord(channel),
        constrained_drag_endpoint(channel, x_channel, y_channel, unit_aspect_box),
    )
}

fn constrained_drag_endpoint(
    channel: &str,
    x_channel: &str,
    y_channel: &str,
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
) -> Expr {
    let Some(unit_aspect_box) = unit_aspect_box else {
        return ev::event_at_start_clipped_coord(channel);
    };
    let endpoints = constrained_box_expressions(x_channel, y_channel, unit_aspect_box);
    if channel == x_channel {
        endpoints.x1
    } else if channel == y_channel {
        endpoints.y1
    } else {
        ev::event_at_start_clipped_coord(channel)
    }
}

fn constrained_drag_distance_squared_px(
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
    x_channel: &str,
    y_channel: &str,
) -> Expr {
    let Some(unit_aspect_box) = unit_aspect_box else {
        return drag_distance_squared();
    };
    let endpoints = constrained_box_expressions(x_channel, y_channel, unit_aspect_box);
    let x_span = start_domain_span(x_channel);
    let y_span = start_domain_span(y_channel);
    let dx_px = endpoints.abs_dx * ev::start_plot_width() / x_span;
    let dy_px = endpoints.abs_dy * ev::start_plot_height() / y_span;
    dx_px.clone() * dx_px + dy_px.clone() * dy_px
}

fn unit_aspect_box_filters(
    unit_aspect_box: Option<ResolvedUnitAspectBox>,
    x_channel: &str,
    y_channel: &str,
) -> Vec<Expr> {
    if unit_aspect_box.is_none() {
        return Vec::new();
    }
    vec![
        start_domain_span(x_channel).gt(lit(0.0_f64)),
        start_domain_span(y_channel).gt(lit(0.0_f64)),
        ev::start_plot_width().gt(lit(0.0_f64)),
        ev::start_plot_height().gt(lit(0.0_f64)),
    ]
}

struct ConstrainedBoxExpressions {
    x1: Expr,
    y1: Expr,
    abs_dx: Expr,
    abs_dy: Expr,
}

fn constrained_box_expressions(
    x_channel: &str,
    y_channel: &str,
    unit_aspect_box: ResolvedUnitAspectBox,
) -> ConstrainedBoxExpressions {
    let dx = ev::event_at_start_clipped_coord(x_channel) - ev::start_coord(x_channel);
    let dy = ev::event_at_start_clipped_coord(y_channel) - ev::start_coord(y_channel);
    let abs_dx = abs_expr(dx.clone());
    let abs_dy = abs_expr(dy.clone());
    let target_dy_per_dx = target_dy_per_dx(x_channel, y_channel, unit_aspect_box.mode);
    let y_exceeds_target = abs_dy.clone().gt(abs_dx.clone() * target_dy_per_dx.clone());

    let constrained_abs_dx = when(y_exceeds_target.clone(), abs_dx.clone())
        .otherwise(abs_dy.clone() / target_dy_per_dx.clone())
        .expect("valid constrained x box expression");
    let constrained_abs_dy = when(y_exceeds_target, abs_dx * target_dy_per_dx)
        .otherwise(abs_dy)
        .expect("valid constrained y box expression");

    let x1 = ev::start_coord(x_channel) + sign_expr(dx) * constrained_abs_dx.clone();
    let y1 = ev::start_coord(y_channel) + sign_expr(dy) * constrained_abs_dy.clone();

    ConstrainedBoxExpressions {
        x1,
        y1,
        abs_dx: constrained_abs_dx,
        abs_dy: constrained_abs_dy,
    }
}

fn target_dy_per_dx(x_channel: &str, y_channel: &str, mode: UnitAspectBox) -> Expr {
    let x_span = start_domain_span(x_channel);
    let y_span = start_domain_span(y_channel);
    match mode {
        UnitAspectBox::CoordinateMetric => {
            ev::start_plot_width() * y_span / (ev::start_plot_height() * x_span)
        }
        UnitAspectBox::Viewport => y_span / x_span,
    }
}

fn start_domain_span(channel: &str) -> Expr {
    let domain = ev::start_domain(channel);
    abs_expr(ev::interval_end(domain.clone()) - ev::interval_start(domain))
}

fn abs_expr(expr: Expr) -> Expr {
    abs(expr)
}

fn sign_expr(expr: Expr) -> Expr {
    when(expr.clone().lt(lit(0.0_f64)), lit(-1.0_f64))
        .otherwise(lit(1.0_f64))
        .expect("valid sign expression")
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
    settle_exact: bool,
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

    if settle_exact {
        binding.settle_exact()
    } else {
        binding
    }
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

fn sanitize_tool_name_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_chart_core::{
        CompiledScalarExpressionProgram, CoordinateMetricDescriptor, PhysicalScalarExpressionSpec,
        PhysicalScalarProgramOptions, one_row_batch_from_scalars,
    };
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::prelude::SessionContext;

    use super::*;

    fn coordinate_metric_context<'a>(
        tool_id: &'a str,
        metrics: &'a [CoordinateMetricDescriptor],
    ) -> ToolExpansionContext<'a> {
        ToolExpansionContext::empty(tool_id).with_coordinate_metrics(metrics)
    }

    fn params<C: CoordinateSystemCore>(expansion: &ToolBehaviorExpansion<C>) -> Vec<&Param> {
        expansion
            .state
            .iter()
            .filter_map(|state| match state {
                avenger_chart_core::ResolvedStateDeclaration::Param { param, .. } => Some(param),
                _ => None,
            })
            .collect()
    }

    fn stores<C: CoordinateSystemCore>(expansion: &ToolBehaviorExpansion<C>) -> Vec<&Store> {
        expansion
            .state
            .iter()
            .filter_map(|state| match state {
                avenger_chart_core::ResolvedStateDeclaration::Store { store, .. } => Some(store),
                _ => None,
            })
            .collect()
    }

    fn selections<C: CoordinateSystemCore>(
        expansion: &ToolBehaviorExpansion<C>,
    ) -> Vec<&Selection> {
        expansion
            .state
            .iter()
            .filter_map(|state| match state {
                avenger_chart_core::ResolvedStateDeclaration::Selection { selection, .. } => {
                    Some(selection)
                }
                _ => None,
            })
            .collect()
    }

    fn xy_coordinate_metric() -> CoordinateMetricDescriptor {
        CoordinateMetricDescriptor::new("unit_aspect", "x", "y")
    }

    fn domain_scalar(min: f64, max: f64) -> ScalarValue {
        ScalarValue::List(ScalarValue::new_list(
            &[
                ScalarValue::Float64(Some(min)),
                ScalarValue::Float64(Some(max)),
            ],
            &DataType::Float64,
            true,
        ))
    }

    fn scalar_f64(value: &ScalarValue) -> f64 {
        match value {
            ScalarValue::Float64(Some(value)) => *value,
            other => panic!("expected Float64 scalar, got {other:?}"),
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn evaluate_unit_aspect_box(mode: UnitAspectBox) -> Vec<ScalarValue> {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new(ev::start_coord_column_name("x"), DataType::Float64, false),
            Field::new(ev::start_coord_column_name("y"), DataType::Float64, false),
            Field::new(
                ev::event_at_start_clipped_coord_column_name("x"),
                DataType::Float64,
                false,
            ),
            Field::new(
                ev::event_at_start_clipped_coord_column_name("y"),
                DataType::Float64,
                false,
            ),
            Field::new(
                ev::start_domain_column_name("x"),
                DataType::new_list(DataType::Float64, true),
                false,
            ),
            Field::new(
                ev::start_domain_column_name("y"),
                DataType::new_list(DataType::Float64, true),
                false,
            ),
            Field::new(ev::START_PLOT_WIDTH_FIELD, DataType::Float64, false),
            Field::new(ev::START_PLOT_HEIGHT_FIELD, DataType::Float64, false),
        ]));
        let unit_aspect_box = ResolvedUnitAspectBox { mode };
        let endpoints = constrained_box_expressions("x", "y", unit_aspect_box);
        let program = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema.clone(),
            vec![
                PhysicalScalarExpressionSpec::new("x1", endpoints.x1),
                PhysicalScalarExpressionSpec::new("y1", endpoints.y1),
                PhysicalScalarExpressionSpec::new(
                    "distance",
                    constrained_drag_distance_squared_px(Some(unit_aspect_box), "x", "y"),
                ),
            ],
            PhysicalScalarProgramOptions::default(),
        )
        .expect("compile expressions");
        let batch = one_row_batch_from_scalars(
            schema,
            &HashMap::from([
                (
                    ev::start_coord_column_name("x"),
                    ScalarValue::Float64(Some(0.0)),
                ),
                (
                    ev::start_coord_column_name("y"),
                    ScalarValue::Float64(Some(0.0)),
                ),
                (
                    ev::event_at_start_clipped_coord_column_name("x"),
                    ScalarValue::Float64(Some(8.0)),
                ),
                (
                    ev::event_at_start_clipped_coord_column_name("y"),
                    ScalarValue::Float64(Some(10.0)),
                ),
                (ev::start_domain_column_name("x"), domain_scalar(0.0, 10.0)),
                (ev::start_domain_column_name("y"), domain_scalar(0.0, 10.0)),
                (
                    ev::START_PLOT_WIDTH_FIELD.to_string(),
                    ScalarValue::Float64(Some(100.0)),
                ),
                (
                    ev::START_PLOT_HEIGHT_FIELD.to_string(),
                    ScalarValue::Float64(Some(200.0)),
                ),
            ]),
        )
        .expect("test batch");
        program.evaluate_values(&batch).expect("evaluate box")
    }

    #[test]
    fn pan_scroll_zoom_expands_to_params_bindings_edits_and_metadata() {
        let tool = PanScrollZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(params(&expansion).len(), 3);
        assert_eq!(expansion.event_bindings.len(), 3);
        assert_eq!(expansion.scale_edits.len(), 2);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_pan_scroll_zoom__enabled")
        );
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_pan_scroll_zoom__x_domain")
        );
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_pan_scroll_zoom__y_domain")
        );
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 3);
        assert_eq!(reset.action.param_steps().count(), 2);
        assert_eq!(
            reset.action.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Exact
        );
    }

    #[test]
    fn pan_scroll_zoom_x_only_expands_single_domain_target() {
        let tool = PanScrollZoom::cartesian().id("nav").x_only();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(params(&expansion).len(), 2);
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 2);
        assert_eq!(reset.action.param_steps().count(), 1);
        assert_eq!(expansion.scale_edits.len(), 1);
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_nav__x_domain")
        );
        assert!(
            !params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_nav__y_domain")
        );
    }

    #[test]
    fn pan_scroll_zoom_settle_exact_marks_drag_and_wheel_preview_bindings() {
        let tool = PanScrollZoom::cartesian().settle_exact(true);
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.event_bindings.len(), 3);
        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        let wheel = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::MouseWheel)
            .expect("wheel binding");
        assert!(drag.action.settle_exact);
        assert!(wheel.action.settle_exact);
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

        assert_eq!(params(&expansion).len(), 2);
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_pan_scroll_zoom__domain__measurement")
        );
        assert_eq!(expansion.scale_edits.len(), 2);

        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert_eq!(drag.action.param_steps().count(), 1);
        assert_eq!(
            drag.action.param_steps().next().unwrap().param_name,
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

        assert_eq!(params(&expansion).len(), 1);
        assert_eq!(selections(&expansion).len(), 1);
        assert_eq!(expansion.event_bindings.len(), 5);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(stores(&expansion).is_empty());
        assert!(expansion.marks.is_empty());
        assert!(expansion.scale_edits.is_empty());
        assert_eq!(params(&expansion)[0].name, "__tool_picked__enabled");
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding
                .action
                .steps
                .iter()
                .any(|step| matches!(step, avenger_chart_core::ChartActionStep::SetCursor(_)))
        }));
        assert_eq!(selections(&expansion)[0].id, "picked");
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
                && binding.action.selection_steps().count() == 1
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
            params(&expansion)[0].default,
            ScalarValue::Boolean(Some(false))
        );
        assert!(
            expansion
                .event_bindings
                .iter()
                .all(|binding| !binding.filters.is_empty()),
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

        assert_eq!(params(&expansion).len(), 1);
        assert_eq!(selections(&expansion).len(), 1);
        assert_eq!(expansion.event_bindings.len(), 2);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(stores(&expansion).is_empty());
        assert!(expansion.marks.is_empty());
        assert!(expansion.scale_edits.is_empty());
        assert_eq!(params(&expansion)[0].name, "__tool_picked__enabled");
        assert_eq!(selections(&expansion)[0].id, "picked");
        assert_eq!(expansion.metadata[0].id, "picked");

        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert!(drag.between.is_some());
        assert_eq!(drag.event_path_min_distance_px, Some(7.0));
        assert_eq!(drag.action.selection_steps().count(), 1);
        assert_eq!(
            drag.action.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Preview
        );
        assert!(drag.action.settle_exact);

        let assignment = drag.action.selection_steps().next().unwrap();
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
                && binding.action.selection_steps().count() == 1
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
    fn unit_aspect_box_coordinate_metric_uses_start_scale_geometry() {
        let values = evaluate_unit_aspect_box(UnitAspectBox::CoordinateMetric);

        assert_close(scalar_f64(&values[0]), 8.0);
        assert_close(scalar_f64(&values[1]), 4.0);
        assert_close(scalar_f64(&values[1]) / scalar_f64(&values[0]), 0.5);
        assert_close(scalar_f64(&values[2]), 12_800.0);
    }

    #[test]
    fn unit_aspect_box_viewport_uses_start_domain_ratio() {
        let values = evaluate_unit_aspect_box(UnitAspectBox::Viewport);

        assert_close(scalar_f64(&values[0]), 8.0);
        assert_close(scalar_f64(&values[1]), 8.0);
        assert_close(scalar_f64(&values[1]) / scalar_f64(&values[0]), 1.0);
    }

    #[test]
    fn unit_aspect_box_requires_matching_coordinate_constraint() {
        let tool = BoxSelection::cartesian("brush").unit_aspect();
        let err = match tool.expand(ToolExpansionContext::empty(ChartTool::id(&tool))) {
            Ok(_) => panic!("unit aspect box without coordinate constraint should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("no active coordinate metric"));

        let metrics = [xy_coordinate_metric()];
        tool.expand(coordinate_metric_context(ChartTool::id(&tool), &metrics))
            .expect("matching coordinate metric");
    }

    #[test]
    fn box_selection_expands_to_store_selection_bindings_and_overlay_mark() {
        let tool = BoxSelection::cartesian("brush").dimensions(col("source_x"), col("source_y"));
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(params(&expansion).len(), 1);
        assert_eq!(stores(&expansion).len(), 1);
        assert_eq!(selections(&expansion).len(), 1);
        assert_eq!(expansion.event_bindings.len(), 3);
        assert_eq!(expansion.marks.len(), 1);
        assert_eq!(expansion.metadata.len(), 1);
        assert_eq!(params(&expansion)[0].name, "__tool_brush__enabled");
        assert_eq!(stores(&expansion)[0].name, "__tool_brush__store");
        assert_eq!(stores(&expansion)[0].primary_key, ["id"]);
        assert_eq!(stores(&expansion)[0].sharing, CoordinationScope::Free);
        assert_eq!(selections(&expansion)[0].id, "brush");
        assert_eq!(selections(&expansion)[0].combine, SelectionCombine::Union);
        assert_eq!(
            selections(&expansion)[0].empty,
            EmptySelectionBehavior::SelectNothing
        );
        assert!(
            expansion
                .event_bindings
                .iter()
                .any(|binding| binding.event_type == ChartEventType::CursorMoved
                    && binding.action.selection_steps().count() == 1
                    && binding.action.store_steps().count() == 1)
        );
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding
                .between
                .as_ref()
                .is_some_and(|between| between.emit_end_event)
                && binding.action.selection_steps().count() == 1
                && binding.action.store_steps().count() == 1
        }));
        assert!(expansion.event_bindings.iter().any(|binding| {
            binding.event_type == ChartEventType::DoubleClick
                && binding.action.selection_steps().count() == 1
                && binding.action.store_steps().count() == 1
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

        assert_eq!(selections(&expansion)[0].combine, SelectionCombine::Union);
        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        let SelectionUpdate::UpsertClauses { clauses } =
            &drag.action.selection_steps().next().unwrap().update
        else {
            panic!("repeat union should upsert selection clauses");
        };
        assert_eq!(clauses.len(), 1);
        let StoreUpdate::UpsertRows { rows } = &drag.action.store_steps().next().unwrap().update
        else {
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

        assert_eq!(
            selections(&expansion)[0].combine,
            SelectionCombine::Intersect
        );
        assert_eq!(
            selections(&expansion)[0].empty,
            EmptySelectionBehavior::SelectAll
        );
    }

    #[test]
    fn box_zoom_expands_to_overlay_params_bindings_edits_and_mark() {
        let tool = BoxZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(params(&expansion).len(), 8);
        assert_eq!(expansion.event_bindings.len(), 5);
        assert_eq!(expansion.scale_edits.len(), 2);
        assert_eq!(expansion.marks.len(), 1);
        assert_eq!(expansion.metadata.len(), 1);
        assert!(
            params(&expansion)
                .iter()
                .any(|p| p.name == "__tool_box_zoom__active")
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
                .flat_map(|binding| binding.action.param_steps())
                .any(|assignment| assignment.scope
                    == avenger_chart_core::event::ChartEventAssignmentScope::Start)
        );
        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("double-click reset binding");
        assert_eq!(reset.filters.len(), 3);
        assert_eq!(reset.action.param_steps().count(), 3);
        assert_eq!(
            reset.action.evaluation_mode,
            avenger_chart_core::event::ChartEventEvaluationMode::Exact
        );
    }

    #[test]
    fn box_zoom_unit_aspect_defaults_to_viewport_mode() {
        let tool = BoxZoom::cartesian().unit_aspect();
        let metrics = [xy_coordinate_metric()];
        let expansion = tool
            .expand(coordinate_metric_context(ChartTool::id(&tool), &metrics))
            .expect("expand");

        let drag = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::CursorMoved)
            .expect("drag binding");
        assert_eq!(drag.filters.len(), 9);
        assert_eq!(drag.action.param_steps().count(), 5);
    }
}
