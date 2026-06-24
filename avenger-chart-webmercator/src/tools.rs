use avenger_chart_core::{
    AvengerChartError, ChannelValue, ChartEventBinding, ChartEventStream, ChartEventType,
    ChartTool, CoordinationScope, Param, ToolExpansion, ToolExpansionContext, ToolMetadata,
    ToolParamSharing, event as ev,
};
use avenger_chart_marks::Rect;
use datafusion::{
    common::ScalarValue,
    functions::expr_fn::{abs, power},
    prelude::{Expr, lit, when},
};

use crate::WebMercator;

#[derive(Clone, Debug)]
pub struct WebMercatorPanZoom {
    id: String,
    viewport_id: String,
    sharing: CoordinationScope,
    drag_button: String,
    scroll_zoom: bool,
    zoom_base: f64,
    consume_wheel: bool,
    box_zoom: bool,
    box_zoom_requires_shift: bool,
    box_zoom_min_size_px: f64,
    settle_exact: bool,
    enabled_by_default: bool,
}

impl WebMercatorPanZoom {
    pub fn new() -> Self {
        Self {
            id: "webmercator_pan_zoom".to_string(),
            viewport_id: "default".to_string(),
            sharing: CoordinationScope::Shared,
            drag_button: "left".to_string(),
            scroll_zoom: true,
            zoom_base: 1.02,
            consume_wheel: true,
            box_zoom: true,
            box_zoom_requires_shift: true,
            box_zoom_min_size_px: 4.0,
            settle_exact: false,
            enabled_by_default: true,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn viewport_id(mut self, viewport_id: impl Into<String>) -> Self {
        self.viewport_id = viewport_id.into();
        self
    }

    pub fn sharing(mut self, sharing: CoordinationScope) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn shared(self) -> Self {
        self.sharing(CoordinationScope::Shared)
    }

    pub fn free(self) -> Self {
        self.sharing(CoordinationScope::Free)
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

    pub fn box_zoom(mut self, enabled: bool) -> Self {
        self.box_zoom = enabled;
        self
    }

    pub fn box_zoom_requires_shift(mut self, requires_shift: bool) -> Self {
        self.box_zoom_requires_shift = requires_shift;
        self
    }

    pub fn box_zoom_min_size_px(mut self, size: f64) -> Self {
        self.box_zoom_min_size_px = size;
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

    fn active_param_name(&self) -> String {
        generated_tool_name(&self.id, "box_active")
    }

    fn overlay_param(&self, suffix: &str) -> Param {
        Param::new(
            generated_tool_name(&self.id, suffix),
            ScalarValue::Float64(Some(0.0)),
        )
    }

    fn center_x_param_name(&self) -> String {
        webmercator_param_name(&self.viewport_id, "center_x")
    }

    fn center_y_param_name(&self) -> String {
        webmercator_param_name(&self.viewport_id, "center_y")
    }

    fn units_per_pixel_param_name(&self) -> String {
        webmercator_param_name(&self.viewport_id, "units_per_pixel")
    }
}

impl Default for WebMercatorPanZoom {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartTool<WebMercator> for WebMercatorPanZoom {
    fn id(&self) -> &str {
        &self.id
    }

    fn expand(
        &self,
        _ctx: ToolExpansionContext<'_>,
    ) -> Result<ToolExpansion<WebMercator>, AvengerChartError> {
        if self.viewport_id.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires a non-empty WebMercator viewport id",
                self.id
            )));
        }
        if !self.zoom_base.is_finite()
            || self.zoom_base <= 0.0
            || (self.zoom_base - 1.0).abs() < f64::EPSILON
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires a finite positive zoom_base other than 1",
                self.id
            )));
        }
        if self.box_zoom_min_size_px < 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "tool '{}' requires a non-negative box zoom minimum drag size",
                self.id
            )));
        }

        let enabled = Param::new(
            self.enabled_param_name(),
            ScalarValue::Boolean(Some(self.enabled_by_default)),
        );
        let center_x = viewport_param(self.center_x_param_name());
        let center_y = viewport_param(self.center_y_param_name());
        let units_per_pixel = viewport_param(self.units_per_pixel_param_name());
        let viewport = ViewportParams {
            center_x,
            center_y,
            units_per_pixel,
        };

        let mut expansion = ToolExpansion::new()
            .param(
                enabled.clone(),
                ToolParamSharing::Explicit(CoordinationScope::Shared),
            )
            .param(
                viewport.center_x.clone(),
                ToolParamSharing::Explicit(self.sharing),
            )
            .param(
                viewport.center_y.clone(),
                ToolParamSharing::Explicit(self.sharing),
            )
            .param(
                viewport.units_per_pixel.clone(),
                ToolParamSharing::Explicit(self.sharing),
            )
            .event_binding(drag_pan_binding(
                &enabled.name,
                &self.drag_button,
                &viewport,
                self.settle_exact,
                self.box_zoom && self.box_zoom_requires_shift,
            ))
            .metadata(
                ToolMetadata::new(self.id.clone(), "WebMercator Pan/Zoom")
                    .enabled_param(enabled.name.clone()),
            );

        if self.scroll_zoom {
            expansion = expansion.event_binding(scroll_zoom_binding(
                &enabled.name,
                &viewport,
                self.zoom_base,
                self.consume_wheel,
            ));
        }

        if self.box_zoom {
            let active = Param::new(self.active_param_name(), ScalarValue::Boolean(Some(false)));
            let box_x0 = self.overlay_param("box_x0");
            let box_y0 = self.overlay_param("box_y0");
            let box_x1 = self.overlay_param("box_x1");
            let box_y1 = self.overlay_param("box_y1");
            let overlay = Rect::<WebMercator>::new()
                .unit_data()
                .exclude_from_scale_domains()
                .visible(ev::param(active.name.as_str()))
                .with_channel_value("x", ChannelValue::from(box_x0.expr()).with_scale_name("x"))
                .with_channel_value("y", ChannelValue::from(box_y0.expr()).with_scale_name("y"))
                .with_channel_value("x2", ChannelValue::from(box_x1.expr()).with_scale_name("x"))
                .with_channel_value("y2", ChannelValue::from(box_y1.expr()).with_scale_name("y"))
                .fill("rgba(66, 133, 244, 0.08)")
                .stroke("#4285f4")
                .stroke_width(1.5)
                .zindex(10_000);

            expansion = expansion
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
                .event_binding(box_zoom_start_binding(
                    &enabled.name,
                    &self.drag_button,
                    &active,
                    &box_x0,
                    &box_y0,
                    &box_x1,
                    &box_y1,
                    self.box_zoom_requires_shift,
                ))
                .event_binding(box_zoom_drag_binding(
                    &enabled.name,
                    &self.drag_button,
                    &active,
                    &box_x0,
                    &box_y0,
                    &box_x1,
                    &box_y1,
                    self.box_zoom_requires_shift,
                ))
                .event_binding(box_zoom_cancel_binding(
                    &enabled.name,
                    &self.drag_button,
                    self.box_zoom_min_size_px,
                    &active,
                    self.box_zoom_requires_shift,
                ))
                .mark(overlay);

            expansion = expansion.event_binding(box_zoom_release_binding(
                &enabled.name,
                &self.drag_button,
                &viewport,
                self.box_zoom_min_size_px,
                self.box_zoom_requires_shift,
                &active,
            ));
        }

        let reset_active_param = self.box_zoom.then(|| self.active_param_name());
        expansion = expansion.event_binding(reset_view_binding(
            &enabled.name,
            &viewport,
            reset_active_param.as_deref(),
        ));

        Ok(expansion)
    }
}

#[derive(Clone, Debug)]
struct ViewportParams {
    center_x: Param,
    center_y: Param,
    units_per_pixel: Param,
}

fn viewport_param(name: impl Into<String>) -> Param {
    Param::new(name, ScalarValue::Float64(None))
}

fn drag_pan_binding(
    enabled_param: &str,
    drag_button: &str,
    viewport: &ViewportParams,
    settle_exact: bool,
    suppress_shift_drag: bool,
) -> ChartEventBinding {
    let mut start = ChartEventStream::on(ChartEventType::MouseDown)
        .filter(ev::button().eq(lit(drag_button.to_string())));
    if suppress_shift_drag {
        start = start.filter(ev::shift().eq(lit(false)));
    }

    let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_coord("x").is_not_null())
        .filter(ev::event_at_start_coord("y").is_not_null())
        .filter(ev::start_plot_width().gt(lit(0.0_f64)))
        .between(start, ChartEventStream::on(ChartEventType::MouseUp))
        .set_param_at_start_scope(
            &viewport.center_x,
            domain_center(ev::start_domain("x"))
                - (ev::event_at_start_coord("x") - ev::start_coord("x")),
        )
        .set_param_at_start_scope(
            &viewport.center_y,
            domain_center(ev::start_domain("y"))
                - (ev::event_at_start_coord("y") - ev::start_coord("y")),
        )
        .set_param_at_start_scope(
            &viewport.units_per_pixel,
            domain_span(ev::start_domain("x")) / ev::start_plot_width(),
        )
        .preview();

    if settle_exact {
        binding = binding.settle_exact();
    }
    binding
}

fn scroll_zoom_binding(
    enabled_param: &str,
    viewport: &ViewportParams,
    zoom_base: f64,
    consume_wheel: bool,
) -> ChartEventBinding {
    let factor = power(lit(zoom_base), lit(-1.0_f64) * ev::wheel_delta_y());
    ChartEventBinding::on(ChartEventType::MouseWheel)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::wheel_delta_y().not_eq(lit(0.0_f64)))
        .filter(ev::event_coord("x").is_not_null())
        .filter(ev::event_coord("y").is_not_null())
        .filter(ev::event_plot_width().gt(lit(0.0_f64)))
        .set_param(
            &viewport.center_x,
            anchored_zoom_center("x", factor.clone()),
        )
        .set_param(
            &viewport.center_y,
            anchored_zoom_center("y", factor.clone()),
        )
        .set_param(
            &viewport.units_per_pixel,
            domain_span(ev::event_domain("x")) / ev::event_plot_width() * factor,
        )
        .preview()
        .consume(consume_wheel)
}

fn reset_view_binding(
    enabled_param: &str,
    viewport: &ViewportParams,
    active_param: Option<&str>,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::event_coord("x").is_not_null())
        .filter(ev::event_coord("y").is_not_null())
        .set_param(&viewport.center_x, lit(viewport.center_x.default.clone()))
        .set_param(&viewport.center_y, lit(viewport.center_y.default.clone()))
        .set_param(
            &viewport.units_per_pixel,
            lit(viewport.units_per_pixel.default.clone()),
        )
        .exact();
    if let Some(active_param) = active_param {
        binding = binding.set_param(active_param, lit(false));
    }
    binding
}

#[allow(clippy::too_many_arguments)]
fn box_zoom_start_binding(
    enabled_param: &str,
    drag_button: &str,
    active: &Param,
    box_x0: &Param,
    box_y0: &Param,
    box_x1: &Param,
    box_y1: &Param,
    requires_shift: bool,
) -> ChartEventBinding {
    let mut binding = ChartEventBinding::on(ChartEventType::MouseDown)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::button().eq(lit(drag_button.to_string())))
        .filter(ev::event_coord("x").is_not_null())
        .filter(ev::event_coord("y").is_not_null())
        .set_param(active, lit(true))
        .set_param(box_x0, ev::event_coord("x"))
        .set_param(box_y0, ev::event_coord("y"))
        .set_param(box_x1, ev::event_coord("x"))
        .set_param(box_y1, ev::event_coord("y"))
        .preview();
    if requires_shift {
        binding = binding.filter(ev::shift().eq(lit(true)));
    }
    binding
}

#[allow(clippy::too_many_arguments)]
fn box_zoom_drag_binding(
    enabled_param: &str,
    drag_button: &str,
    active: &Param,
    box_x0: &Param,
    box_y0: &Param,
    box_x1: &Param,
    box_y1: &Param,
    requires_shift: bool,
) -> ChartEventBinding {
    let endpoints = webmercator_constrained_box_expressions();
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .filter(ev::param(enabled_param).eq(lit(true)))
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .filter(start_domain_span("x").gt(lit(0.0_f64)))
        .filter(start_domain_span("y").gt(lit(0.0_f64)))
        .filter(ev::start_plot_width().gt(lit(0.0_f64)))
        .filter(ev::start_plot_height().gt(lit(0.0_f64)))
        .between(
            box_zoom_drag_start_stream(drag_button, requires_shift),
            box_zoom_drag_end_stream(drag_button),
        )
        .set_param_at_start_scope(active, lit(true))
        .set_param_at_start_scope(box_x0, ev::start_coord("x"))
        .set_param_at_start_scope(box_y0, ev::start_coord("y"))
        .set_param_at_start_scope(box_x1, endpoints.x1)
        .set_param_at_start_scope(box_y1, endpoints.y1)
        .preview()
}

fn box_zoom_cancel_binding(
    enabled_param: &str,
    drag_button: &str,
    min_size_px: f64,
    active: &Param,
    requires_shift: bool,
) -> ChartEventBinding {
    let endpoints = webmercator_constrained_box_expressions();
    ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button, requires_shift),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(start_domain_span("x").gt(lit(0.0_f64)))
    .filter(start_domain_span("y").gt(lit(0.0_f64)))
    .filter(ev::start_plot_width().gt(lit(0.0_f64)))
    .filter(ev::start_plot_height().gt(lit(0.0_f64)))
    .filter(webmercator_box_distance_squared_px(&endpoints).lt(lit(min_size_px * min_size_px)))
    .set_param_at_start_scope(active, lit(false))
    .preview()
}

fn box_zoom_release_binding(
    enabled_param: &str,
    drag_button: &str,
    viewport: &ViewportParams,
    min_size_px: f64,
    requires_shift: bool,
    active: &Param,
) -> ChartEventBinding {
    let endpoints = webmercator_constrained_box_expressions();
    ChartEventBinding::on_between_end(
        box_zoom_drag_start_stream(drag_button, requires_shift),
        box_zoom_drag_end_stream(drag_button),
    )
    .filter(ev::param(enabled_param).eq(lit(true)))
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .filter(start_domain_span("x").gt(lit(0.0_f64)))
    .filter(start_domain_span("y").gt(lit(0.0_f64)))
    .filter(ev::start_plot_width().gt(lit(0.0_f64)))
    .filter(ev::start_plot_height().gt(lit(0.0_f64)))
    .filter(webmercator_box_distance_squared_px(&endpoints).gt_eq(lit(min_size_px * min_size_px)))
    .set_param_at_start_scope(
        &viewport.center_x,
        (ev::start_coord("x") + endpoints.x1.clone()) / lit(2.0_f64),
    )
    .set_param_at_start_scope(
        &viewport.center_y,
        (ev::start_coord("y") + endpoints.y1.clone()) / lit(2.0_f64),
    )
    .set_param_at_start_scope(
        &viewport.units_per_pixel,
        endpoints.abs_dx / ev::start_plot_width(),
    )
    .set_param_at_start_scope(active, lit(false))
    .exact()
}

fn box_zoom_drag_start_stream(drag_button: &str, requires_shift: bool) -> ChartEventStream {
    let mut start = ChartEventStream::on(ChartEventType::MouseDown)
        .filter(ev::button().eq(lit(drag_button.to_string())));
    if requires_shift {
        start = start.filter(ev::shift().eq(lit(true)));
    }
    start
}

fn box_zoom_drag_end_stream(drag_button: &str) -> ChartEventStream {
    ChartEventStream::on(ChartEventType::MouseUp)
        .filter(ev::button().eq(lit(drag_button.to_string())))
}

fn anchored_zoom_center(channel: &str, factor: Expr) -> Expr {
    let anchor = ev::event_coord(channel);
    anchor.clone() + (domain_center(ev::event_domain(channel)) - anchor) * factor
}

fn domain_center(domain: Expr) -> Expr {
    (ev::interval_start(domain.clone()) + ev::interval_end(domain)) / lit(2.0_f64)
}

fn domain_span(domain: Expr) -> Expr {
    ev::interval_end(domain.clone()) - ev::interval_start(domain)
}

struct WebMercatorConstrainedBoxExpressions {
    x1: Expr,
    y1: Expr,
    abs_dx: Expr,
    abs_dy: Expr,
}

fn webmercator_constrained_box_expressions() -> WebMercatorConstrainedBoxExpressions {
    let dx = ev::event_at_start_clipped_coord("x") - ev::start_coord("x");
    let dy = ev::event_at_start_clipped_coord("y") - ev::start_coord("y");
    let abs_dx = abs_expr(dx.clone());
    let abs_dy = abs_expr(dy.clone());
    let target_dy_per_dx = start_domain_span("y") / start_domain_span("x");
    let y_exceeds_target = abs_dy.clone().gt(abs_dx.clone() * target_dy_per_dx.clone());

    let constrained_abs_dx = when(y_exceeds_target.clone(), abs_dx.clone())
        .otherwise(abs_dy.clone() / target_dy_per_dx.clone())
        .expect("valid WebMercator box zoom x expression");
    let constrained_abs_dy = when(y_exceeds_target, abs_dx * target_dy_per_dx)
        .otherwise(abs_dy)
        .expect("valid WebMercator box zoom y expression");

    let x1 = ev::start_coord("x") + sign_expr(dx) * constrained_abs_dx.clone();
    let y1 = ev::start_coord("y") + sign_expr(dy) * constrained_abs_dy.clone();

    WebMercatorConstrainedBoxExpressions {
        x1,
        y1,
        abs_dx: constrained_abs_dx,
        abs_dy: constrained_abs_dy,
    }
}

fn webmercator_box_distance_squared_px(endpoints: &WebMercatorConstrainedBoxExpressions) -> Expr {
    let dx_px = endpoints.abs_dx.clone() * ev::start_plot_width() / start_domain_span("x");
    let dy_px = endpoints.abs_dy.clone() * ev::start_plot_height() / start_domain_span("y");
    dx_px.clone() * dx_px + dy_px.clone() * dy_px
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

fn webmercator_param_name(viewport_id: &str, suffix: &str) -> String {
    format!("__webmercator_{viewport_id}_{suffix}")
}

fn generated_tool_name(id: &str, suffix: &str) -> String {
    format!("__tool_{}__{}", sanitize_tool_name_part(id), suffix)
}

fn sanitize_tool_name_part(part: &str) -> String {
    part.chars()
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

    use super::*;
    use avenger_chart_core::{
        CompiledScalarExpressionProgram, PhysicalScalarExpressionSpec,
        PhysicalScalarProgramOptions, event::ChartEventEvaluationMode, one_row_batch_from_scalars,
    };
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::prelude::SessionContext;

    #[test]
    fn pan_zoom_expands_to_viewport_params_and_bindings() {
        let tool = WebMercatorPanZoom::new().viewport_id("map");
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.params.len(), 9);
        assert_eq!(expansion.event_bindings.len(), 7);
        assert!(expansion.scale_edits.is_empty());
        assert_eq!(expansion.marks.len(), 1);
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__tool_webmercator_pan_zoom__enabled")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__webmercator_map_center_x")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__webmercator_map_center_y")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__webmercator_map_units_per_pixel")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__tool_webmercator_pan_zoom__box_active")
        );
        assert!(
            expansion
                .params
                .iter()
                .any(|param| param.param.name == "__tool_webmercator_pan_zoom__box_x0")
        );

        let reset = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::DoubleClick)
            .expect("reset binding");
        assert_eq!(reset.assignments.len(), 4);
        assert_eq!(reset.evaluation_mode, ChartEventEvaluationMode::Exact);

        let wheel = expansion
            .event_bindings
            .iter()
            .find(|binding| binding.event_type == ChartEventType::MouseWheel)
            .expect("wheel binding");
        assert_eq!(wheel.assignments.len(), 3);
        assert!(wheel.consume);

        let box_zoom = expansion
            .event_bindings
            .iter()
            .find(|binding| {
                binding.event_type == ChartEventType::MouseUp
                    && binding.evaluation_mode == ChartEventEvaluationMode::Exact
            })
            .expect("box zoom binding");
        assert_eq!(box_zoom.assignments.len(), 4);
        assert_eq!(box_zoom.evaluation_mode, ChartEventEvaluationMode::Exact);

        let overlay_drag = expansion
            .event_bindings
            .iter()
            .find(|binding| {
                binding.event_type == ChartEventType::CursorMoved
                    && binding.assignments.len() == 5
                    && binding.evaluation_mode == ChartEventEvaluationMode::Preview
            })
            .expect("overlay drag binding");
        assert!(overlay_drag.between.is_some());
    }

    #[test]
    fn pan_zoom_can_disable_wheel_box_zoom_and_use_free_sharing() {
        let tool = WebMercatorPanZoom::new()
            .scroll_zoom(false)
            .box_zoom(false)
            .free();
        let expansion = tool
            .expand(ToolExpansionContext::empty(ChartTool::id(&tool)))
            .expect("expand");

        assert_eq!(expansion.event_bindings.len(), 2);
        assert!(
            expansion
                .params
                .iter()
                .filter(|param| param.param.name.starts_with("__webmercator_default_"))
                .all(|param| param.sharing == ToolParamSharing::Explicit(CoordinationScope::Free))
        );
    }

    #[test]
    fn box_zoom_rejects_negative_min_size() {
        let tool = WebMercatorPanZoom::new().box_zoom_min_size_px(-1.0);
        let err = match tool.expand(ToolExpansionContext::empty(ChartTool::id(&tool))) {
            Ok(_) => panic!("negative minimum size should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("non-negative"));
    }

    #[test]
    fn box_zoom_expression_uses_viewport_aspect_and_units_per_pixel() {
        let values = evaluate_box_zoom_expressions();
        assert_close(scalar_f64(&values[0]), 4.0);
        assert_close(scalar_f64(&values[1]), 2.0);
        assert_close(scalar_f64(&values[2]), 0.08);
        assert_close(scalar_f64(&values[3]), 80.0);
    }

    fn evaluate_box_zoom_expressions() -> Vec<ScalarValue> {
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
        let endpoints = webmercator_constrained_box_expressions();
        let program = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema.clone(),
            vec![
                PhysicalScalarExpressionSpec::new(
                    "center_x",
                    (ev::start_coord("x") + endpoints.x1.clone()) / lit(2.0_f64),
                ),
                PhysicalScalarExpressionSpec::new(
                    "center_y",
                    (ev::start_coord("y") + endpoints.y1.clone()) / lit(2.0_f64),
                ),
                PhysicalScalarExpressionSpec::new(
                    "units_per_pixel",
                    endpoints.abs_dx.clone() / ev::start_plot_width(),
                ),
                PhysicalScalarExpressionSpec::new(
                    "distance",
                    webmercator_box_distance_squared_px(&endpoints),
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
                (ev::start_domain_column_name("x"), domain_scalar(0.0, 100.0)),
                (ev::start_domain_column_name("y"), domain_scalar(0.0, 50.0)),
                (
                    ev::START_PLOT_WIDTH_FIELD.to_string(),
                    ScalarValue::Float64(Some(100.0)),
                ),
                (
                    ev::START_PLOT_HEIGHT_FIELD.to_string(),
                    ScalarValue::Float64(Some(50.0)),
                ),
            ]),
        )
        .expect("batch");
        program
            .evaluate(&batch)
            .expect("evaluate")
            .into_iter()
            .map(|(_, value)| value)
            .collect()
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
}
