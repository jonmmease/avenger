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
                .with_title("avenger-chart canvas-width-height scatter")
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
    let height = Param::new("height", ScalarValue::Float64(Some(520.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (0.8, 1.6, 'A'), (1.6, 2.9, 'A'), (2.4, 2.1, 'A'),
                (3.2, 4.7, 'B'), (4.0, 3.8, 'B'), (4.8, 5.4, 'B'),
                (5.6, 5.9, 'C'), (6.4, 7.2, 'C'), (7.2, 6.4, 'C'),
                (8.0, 8.5, 'D'), (8.8, 7.7, 'D'), (9.6, 9.1, 'D')
            ) AS t(x, y, group_name)",
        )
        .await
        .expect("build data");

    let plot = Plot::<Cartesian>::new()
        .add_params([width.clone(), height.clone()])
        .canvas_size(width.expr(), height.expr())
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(140.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::width_height("width", "height"),
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
