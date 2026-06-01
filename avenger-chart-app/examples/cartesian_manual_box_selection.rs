//! Manual Cartesian box selection prototype.
//!
//! This example intentionally uses the public low-level pieces directly:
//! `Selection`, `selection_predicate`, cursor params, ordinary event bindings,
//! and a unit `Rect` overlay mark. The bundled `BoxSelection` tool will be a
//! convenience wrapper over this shape.

use std::sync::Arc;

use avenger_chart::event as ev;
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::{Expr, SessionContext, lit, when};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart manual box selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (0.8, 1.2, 'A'), (1.6, 2.0, 'A'), (2.5, 1.8, 'A'),
                (3.4, 3.8, 'B'), (4.2, 4.5, 'B'), (5.1, 3.7, 'B'),
                (6.0, 6.4, 'C'), (6.8, 5.4, 'C'), (7.7, 6.1, 'C'),
                (8.5, 7.5, 'D'), (9.2, 8.4, 'D'), (9.8, 7.8, 'D')
            ) AS t(x, y, group_name)",
        )
        .await
        .expect("build data");

    let brush = Selection::single("brush")
        .empty(SelectionEmpty::None)
        .interval_xy("x", "y");
    let selected = selection_predicate("brush");

    let overlay_active = Param::new("brush_overlay_active", false);
    let brush_x0 = Param::new("brush_x0", 0.0);
    let brush_y0 = Param::new("brush_y0", 0.0);
    let brush_x1 = Param::new("brush_x1", 0.0);
    let brush_y1 = Param::new("brush_y1", 0.0);
    let cursor = Param::cursor("brush_cursor", CursorStyle::Default);

    let overlay = Rect::<Cartesian>::new()
        .unit_data()
        .exclude_from_scale_domains()
        .visible(overlay_active.expr())
        .x(brush_x0.expr())
        .x2_with(brush_x1.expr(), |c| c.with_scale_name("x"))
        .y(brush_y0.expr())
        .y2_with(brush_y1.expr(), |c| c.with_scale_name("y"))
        .fill("rgba(37, 99, 235, 0.08)")
        .stroke("#2563eb")
        .stroke_width(1.5)
        .zindex(10_000);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .data(df)
        .add_selection(brush)
        .add_params([
            overlay_active.clone(),
            brush_x0.clone(),
            brush_y0.clone(),
            brush_x1.clone(),
            brush_y1.clone(),
            cursor.clone(),
        ])
        .cursor_param(cursor.name.clone())
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.75)
                .size(180.0),
        )
        .mark(overlay)
        .event_binding(cursor_binding(&cursor))
        .event_binding(selection_start_binding(
            &overlay_active,
            &brush_x0,
            &brush_y0,
            &brush_x1,
            &brush_y1,
        ))
        .event_binding(selection_drag_binding(
            &cursor,
            &overlay_active,
            &brush_x0,
            &brush_y0,
            &brush_x1,
            &brush_y1,
        ))
        .event_binding(selection_release_binding(&overlay_active))
        .event_binding(selection_clear_binding(&overlay_active));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
    )
    .await
    .expect("build chart app")
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_plot = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
    let cursor_expr = when(over_plot, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor case expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

#[allow(clippy::too_many_arguments)]
fn selection_start_binding(
    active: &Param,
    x0: &Param,
    y0: &Param,
    x1: &Param,
    y1: &Param,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::MouseDown)
        .filter(ev::button().eq(lit("left")))
        .filter(ev::event_coord("x").is_not_null())
        .filter(ev::event_coord("y").is_not_null())
        .set_param(active, lit(true))
        .set_param(x0, ev::event_coord("x"))
        .set_param(y0, ev::event_coord("y"))
        .set_param(x1, ev::event_coord("x"))
        .set_param(y1, ev::event_coord("y"))
        .preview()
}

#[allow(clippy::too_many_arguments)]
fn selection_drag_binding(
    cursor: &Param,
    active: &Param,
    x0: &Param,
    y0: &Param,
    x1: &Param,
    y1: &Param,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_param_at_start_scope(active, lit(true))
        .set_param_at_start_scope(x0, ev::start_coord("x"))
        .set_param_at_start_scope(y0, ev::start_coord("y"))
        .set_param_at_start_scope(x1, ev::event_at_start_clipped_coord("x"))
        .set_param_at_start_scope(y1, ev::event_at_start_clipped_coord("y"))
        .set_selection_at_start_scope(
            "brush",
            SelectionUpdate::interval_xy()
                .x_range(selection_interval("x"))
                .y_range(selection_interval("y")),
        )
        .preview()
}

fn selection_release_binding(active: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::MouseUp)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .emit_between_end_event()
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .set_param_at_start_scope(active, lit(true))
        .set_selection_at_start_scope(
            "brush",
            SelectionUpdate::interval_xy()
                .x_range(selection_interval("x"))
                .y_range(selection_interval("y")),
        )
        .exact()
}

fn selection_clear_binding(active: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .set_param(active, lit(false))
        .set_selection("brush", SelectionUpdate::clear())
        .exact()
}

fn selection_interval(channel: &str) -> Expr {
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

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = env_logger::try_init();
    }
}
