//! A checkbox-list widget cross-filters a scatter plot.
//!
//! The list and plot share one equality selection. Each list click toggles a
//! typed clause on the source `region` field; the blue foreground applies the
//! resulting selection predicate while the grey background retains context.
//! With no checked rows, `empty_selects_all` leaves the foreground unfiltered.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example widget_region_cross_filter --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_widgets::CheckboxList;
use datafusion::{
    common::ScalarValue,
    prelude::{SessionContext, col},
};
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
            .with_title("avenger-chart widget region cross-filter")
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
                ('North', 1.0, 6.0), ('North', 2.0, 7.5), ('North', 3.0, 6.8),
                ('South', 2.2, 2.0), ('South', 3.4, 3.1), ('South', 4.2, 2.4),
                ('West',  5.0, 5.8), ('West',  6.1, 4.9), ('West',  7.0, 6.4)
            ) AS t(region, x_value, y_value)",
        )
        .await
        .expect("build scatter data");
    let regions = Selection::new("regions").empty_selects_all();
    let selected = regions.predicate();

    let chart = Chart::<Cartesian>::new()
        .title("Region cross-filter")
        .canvas_size(SIZE[0], SIZE[1])
        .plot_size(480.0, 300.0)
        .data(data)
        .mark(
            Symbol::new()
                .x_with(col("x_value"), |x| x.axis(|axis| axis.title("X")))
                .y_with(col("y_value"), |y| {
                    y.axis(|axis| axis.title("Y").grid(true))
                })
                .size(150.0)
                .fill("#C8CDD2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::new()
                .transform_no_output(Filter::new(selected), |mark| mark)
                .x(col("x_value"))
                .y(col("y_value"))
                .size(150.0)
                .fill("#0072B2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .widget(
            CheckboxList::new("region_filter", region_items())
                .value(col("region"))
                .label(col("label"))
                .selection(&regions)
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

fn region_items() -> WidgetItems {
    WidgetItems::Static(
        ["North", "South", "West"]
            .into_iter()
            .map(|region| {
                WidgetItemRow::new([
                    (
                        "region".to_string(),
                        ScalarValue::Utf8(Some(region.to_string())),
                    ),
                    (
                        "label".to_string(),
                        ScalarValue::Utf8(Some(region.to_string())),
                    ),
                ])
            })
            .collect(),
    )
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
