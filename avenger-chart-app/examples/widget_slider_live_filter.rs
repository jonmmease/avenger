//! A Slider widget live-filters a scatter plot during pointer drags.
//!
//! The grey layer retains the full dataset for context. The blue layer keeps
//! rows whose score is at least the Slider value, so track clicks and drags
//! visibly rebuild a parameter-driven transform at interactive cadence.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example widget_slider_live_filter --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_widgets::Slider;
use datafusion::prelude::{SessionContext, col};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [760.0, 460.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart slider live filter")
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
                (8.0,  1.2), (14.0, 2.0), (21.0, 1.6), (27.0, 3.5),
                (33.0, 2.8), (39.0, 4.7), (46.0, 4.0), (52.0, 5.8),
                (58.0, 5.1), (64.0, 6.9), (71.0, 6.2), (77.0, 7.8),
                (83.0, 7.1), (89.0, 8.7), (96.0, 8.1)
            ) AS t(score, response)",
        )
        .await
        .expect("build scatter data");
    let threshold = Slider::new("minimum_score", 0.0, 100.0)
        .step(1.0)
        .default(40.0)
        .title("Minimum score")
        .format(".0f")
        .throttle_ms(16);
    let selected = col("score").gt_eq(threshold.value());

    let chart = Chart::<Cartesian>::new()
        .title("Slider live filter")
        .canvas_size(SIZE[0], SIZE[1])
        .plot_size(500.0, 320.0)
        .data(data)
        .mark(
            Symbol::new()
                .x_with(col("score"), |x| x.axis(|axis| axis.title("Score")))
                .y_with(col("response"), |y| {
                    y.axis(|axis| axis.title("Response").grid(true))
                })
                .size(150.0)
                .fill("#C8CDD2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::new()
                .transform_no_output(Filter::new(selected), |mark| mark)
                .x(col("score"))
                .y(col("response"))
                .size(150.0)
                .fill("#0072B2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .widget(threshold.position(ChromePosition::Left));

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
