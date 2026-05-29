//! Single-panel Cartesian pan example.
//!
//! Drag with the left mouse button inside the plot area to pan. The pan binding
//! reads pointer positions in data coordinates, computes new raw x/y scale
//! domains with ordinary DataFusion expressions, and writes them through scoped
//! `set_param` assignments. Preview evaluation is used during the drag and an
//! exact evaluation settles on release.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_pan --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::event::{self as ev, ChartEventBinding, ChartEventStream, ChartEventType};
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::{SessionContext, lit};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart Cartesian pan (drag with left mouse button)")
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
                (1.0, 2.0, 'A'),
                (2.0, 3.6, 'A'),
                (3.0, 4.2, 'B'),
                (4.0, 3.1, 'B'),
                (5.0, 5.2, 'C'),
                (6.0, 4.8, 'C'),
                (7.0, 6.5, 'D'),
                (8.0, 5.8, 'D')
            ) AS t(x, y, group_name)",
        )
        .await
        .expect("build data");

    // Raw-domain params: null by default, so the scales fall back to their
    // inferred domains until a pan gesture writes a concrete domain.
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    // Pan delta in data space, measured through the frozen start scale so the
    // gesture does not feed back as the raw domains update during the drag.
    let dx = ev::event_at_start_data("x") - ev::start_data("x");
    let dy = ev::event_at_start_data("y") - ev::start_data("y");

    let pan = ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .set_param(
            &x_domain,
            ev::domain_interval(
                ev::domain_start(ev::start_domain("x")) - dx.clone(),
                ev::domain_end(ev::start_domain("x")) - dx,
            ),
        )
        .set_param(
            &y_domain,
            ev::domain_interval(
                ev::domain_start(ev::start_domain("y")) - dy.clone(),
                ev::domain_end(ev::start_domain("y")) - dy,
            ),
        )
        .preview()
        .settle_exact();

    let plot = Plot::<Cartesian>::new()
        .add_param(x_domain.clone())
        .add_param(y_domain.clone())
        .canvas_size(760.0, 520.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(x_raw.clone()).nice(false).zero(false)
                    })
                })
                .y_with(col("y"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(y_raw.clone()).nice(false).zero(false)
                    })
                })
                .fill(col("group_name"))
                .size(120.0),
        )
        .event_binding(pan);

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
