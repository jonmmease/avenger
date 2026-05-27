use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    canvas_frame_options_for_resize_policy, chart_avenger_app,
    window_scene_sizing_for_resize_policy,
};
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let mut avenger_app = tokio_runtime.block_on(build_app());
    let resize_policy = tokio_runtime.block_on(avenger_app.app_state_mut().resize_policy());
    let options = WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart canvas-width scatter")
                .with_resizable(true),
        )
        .window_scene_sizing(window_scene_sizing_for_resize_policy(resize_policy))
        .canvas_frame(canvas_frame_options_for_resize_policy(resize_policy))
        .resize_settle_delay_ms(Some(160));
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let width = Param::new("width", ScalarValue::Float64(Some(760.0)));
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

    let plot = Plot::<Cartesian>::new()
        .add_param(width.clone())
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(360.0))
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(120.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::width("width"),
            resize_throttle_ms: Some(8),
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
