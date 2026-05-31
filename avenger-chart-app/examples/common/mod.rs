use avenger_chart::{
    event::{self as ev, ChartEventBinding, ChartEventStream, ChartEventType},
    prelude::{Expr, Param},
};
use datafusion::{functions::expr_fn::power, prelude::lit};

pub(crate) fn cartesian_drag_pan_binding(
    x_domain: &Param,
    y_domain: &Param,
    settle_exact: bool,
) -> ChartEventBinding {
    let dx = ev::event_at_start_coord("x") - ev::start_coord("x");
    let dy = ev::event_at_start_coord("y") - ev::start_coord("y");

    let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .set_param(
            x_domain,
            ev::interval(
                ev::interval_start(ev::start_domain("x")) - dx.clone(),
                ev::interval_end(ev::start_domain("x")) - dx,
            ),
        )
        .set_param(
            y_domain,
            ev::interval(
                ev::interval_start(ev::start_domain("y")) - dy.clone(),
                ev::interval_end(ev::start_domain("y")) - dy,
            ),
        )
        .preview();

    if settle_exact {
        binding.settle_exact()
    } else {
        binding
    }
}

pub(crate) fn cartesian_scroll_zoom_binding(
    x_domain: &Param,
    y_domain: &Param,
) -> ChartEventBinding {
    // Positive wheel delta zooms in around the pointer. A power curve keeps
    // trackpad deltas smooth while still making mouse-wheel notches visible.
    let factor = power(lit(1.02_f64), lit(-1.0_f64) * ev::wheel_delta_y());

    ChartEventBinding::on(ChartEventType::MouseWheel)
        .filter(ev::wheel_delta_y().not_eq(lit(0.0_f64)))
        .filter(ev::event_coord("x").is_not_null())
        .filter(ev::event_coord("y").is_not_null())
        .set_param(x_domain, zoom_interval("x", factor.clone()))
        .set_param(y_domain, zoom_interval("y", factor))
        .preview()
        .consume(true)
}

fn zoom_interval(channel: &str, factor: Expr) -> Expr {
    let domain = ev::event_domain(channel);
    let anchor = ev::event_coord(channel);
    ev::interval(
        anchor.clone() + (ev::interval_start(domain.clone()) - anchor.clone()) * factor.clone(),
        anchor.clone() + (ev::interval_end(domain) - anchor) * factor,
    )
}
