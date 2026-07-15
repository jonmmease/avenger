//! A scalar Checkbox toggles a fitted trend line without changing layout.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example widget_trend_toggle --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_widgets::Checkbox;
use datafusion::prelude::{SessionContext, col};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [720.0, 440.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart Checkbox trend toggle")
            .with_inner_size(LogicalSize::new(f64::from(SIZE[0]), f64::from(SIZE[1])))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let data = ctx
        .sql(
            "SELECT * FROM (VALUES
                (1.0, 1.3, 1.2), (2.0, 2.0, 2.1), (3.0, 3.4, 3.0),
                (4.0, 3.8, 3.9), (5.0, 5.1, 4.8), (6.0, 5.5, 5.7),
                (7.0, 6.9, 6.6)
            ) AS t(x_value, observed, trend)",
        )
        .await
        .expect("build trend data");
    let toggle = Checkbox::new("trend_toggle", "Show trend line", true);
    let trend_visible = toggle.checked();

    let chart = Chart::<Cartesian>::new()
        .title("Checkbox trend toggle")
        .canvas_size(SIZE[0], SIZE[1])
        .plot_size(540.0, 300.0)
        .data(data)
        .mark(
            Symbol::new()
                .x_with(col("x_value"), |x| x.axis(|axis| axis.title("X")))
                .y_with(col("observed"), |y| {
                    y.axis(|axis| axis.title("Observed").grid(true))
                })
                .size(150.0)
                .fill("#C8CDD2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .mark(
            Line::new()
                .x(col("x_value"))
                .y(col("trend"))
                .order(col("x_value"))
                .stroke("#0072B2")
                .stroke_width(3.0)
                .visible(trend_visible),
        )
        .widget(toggle.position(ChromePosition::Left));

    let compiled = chart.compile(&ctx).await.expect("compile plot");
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
