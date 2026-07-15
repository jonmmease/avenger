//! A momentary Button clears a point selection through a parameter reaction.
//!
//! Click a point to select its category, then click **Clear selection**. The
//! Button increments its activation parameter; `Button::action` lowers that
//! change to the same serializable `ChartParamChangeBinding` available at the
//! plot level, and the selection clear commits atomically with the increment.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example button_clear_selection --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_widgets::{Button, ButtonVariant};
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
            .with_title("avenger-chart Button clear selection")
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
                ('Alpha', 1.0, 5.0), ('Alpha', 2.0, 6.5),
                ('Beta',  3.5, 2.0), ('Beta',  4.5, 3.2),
                ('Gamma', 6.0, 5.5), ('Gamma', 7.0, 6.8)
            ) AS t(category, x_value, y_value)",
        )
        .await
        .expect("build scatter data");
    let picked = PointSelection::new("picked")
        .field("category")
        .empty_selects_all();
    let selected = picked.predicate();

    let chart = Chart::<Cartesian>::new()
        .title("Button action: clear selection")
        .subtitle("Click a point, then clear the selection")
        .canvas_size(SIZE[0], SIZE[1])
        .plot_size(540.0, 300.0)
        .data(data)
        .tool(picked)
        .mark(
            Symbol::new()
                .x_with(col("x_value"), |x| x.axis(|axis| axis.title("X")))
                .y_with(col("y_value"), |y| {
                    y.axis(|axis| axis.title("Y").grid(true))
                })
                .size(170.0)
                .fill("#C8CDD2")
                .stroke("#FFFFFF")
                .stroke_width(1.0)
                .zindex(5),
        )
        .mark(
            Symbol::new()
                .transform_no_output(Filter::new(selected), |mark| mark)
                .x(col("x_value"))
                .y(col("y_value"))
                .size(170.0)
                .fill("#0072B2")
                .stroke("#FFFFFF")
                .stroke_width(1.0)
                .zindex(10),
        )
        .widget(
            Button::new("clear")
                .label("Clear selection")
                .variant(ButtonVariant::Accent)
                .action(ChartAction::new().clear_selection("picked"))
                .position(ChromePosition::Left),
        );

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
