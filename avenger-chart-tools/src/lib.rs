//! Built-in chart tools.

use avenger_chart_cartesian::{Cartesian, CartesianRectPositionChannels};
use avenger_chart_core::{
    AvengerChartError, ChartEventBinding, ChartEventStream, ChartEventType, ChartTool, Param,
    Sharing, ToolExpansion, ToolExpansionContext, ToolMetadata, ToolParamSharing, ToolScaleEdit,
    event as ev,
};
use avenger_chart_marks::Rect;
use datafusion::{
    common::ScalarValue,
    functions::expr_fn::power,
    prelude::{Expr, lit, when},
};

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

impl ChartTool<Cartesian> for PanScrollZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<Cartesian>, AvengerChartError> {
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
        expansion = expansion.event_binding(reset_view_binding(&enabled.name, &channels));

        Ok(expansion)
    }
}

#[derive(Clone, Debug)]
pub struct BoxZoom {
    id: String,
    x_channel: String,
    y_channel: String,
    x_domain_param: Option<Param>,
    y_domain_param: Option<Param>,
    x_sharing: Option<Sharing>,
    y_sharing: Option<Sharing>,
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
            .param(enabled.clone(), ToolParamSharing::Explicit(Sharing::Shared))
            .param(active.clone(), ToolParamSharing::Explicit(Sharing::Free))
            .param(box_x0.clone(), ToolParamSharing::Explicit(Sharing::Free))
            .param(box_y0.clone(), ToolParamSharing::Explicit(Sharing::Free))
            .param(box_x1.clone(), ToolParamSharing::Explicit(Sharing::Free))
            .param(box_y1.clone(), ToolParamSharing::Explicit(Sharing::Free))
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
    ChartEventBinding::on(ChartEventType::MouseUp)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(drag_distance_squared().lt(lit(min_size_px * min_size_px)))
        .between(
            box_zoom_drag_start_stream(drag_button),
            box_zoom_drag_end_stream(drag_button),
        )
        .emit_between_end_event()
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
    let mut binding = ChartEventBinding::on(ChartEventType::MouseUp)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(drag_distance_squared().gt_eq(lit(min_size_px * min_size_px)))
        .between(
            box_zoom_drag_start_stream(drag_button),
            box_zoom_drag_end_stream(drag_button),
        )
        .emit_between_end_event()
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
    let start = ev::start_coord(channel);
    let end = ev::event_at_start_clipped_coord(channel);
    ev::interval(expr_min(start.clone(), end.clone()), expr_max(start, end))
}

fn expr_min(a: Expr, b: Expr) -> Expr {
    when(a.clone().lt_eq(b.clone()), a)
        .otherwise(b)
        .expect("valid min case expression")
}

fn expr_max(a: Expr, b: Expr) -> Expr {
    when(a.clone().gt_eq(b.clone()), a)
        .otherwise(b)
        .expect("valid max case expression")
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

fn reset_view_binding(enabled_param: &str, channels: &[(String, Param)]) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .exact();

    for (channel, param) in channels {
        binding = binding.filter(ev::event_coord(channel).is_not_null());
        binding = binding.set_param(param, lit(param.default.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_scroll_zoom_expands_to_params_bindings_edits_and_metadata() {
        let tool = PanScrollZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext {
                tool_id: ChartTool::id(&tool),
            })
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
            .expand(ToolExpansionContext {
                tool_id: ChartTool::id(&tool),
            })
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
    fn box_zoom_expands_to_overlay_params_bindings_edits_and_mark() {
        let tool = BoxZoom::cartesian();
        let expansion = tool
            .expand(ToolExpansionContext {
                tool_id: ChartTool::id(&tool),
            })
            .expect("expand");

        assert_eq!(expansion.params.len(), 8);
        assert_eq!(expansion.event_bindings.len(), 4);
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
    }
}
