//! Equality selection from clicked aggregate bars.
//!
//! Click a bar to replace the selected category. Shift-click bars to toggle
//! categories in or out of the selection. Double-click to clear. The sibling
//! scatter plot uses the same selection predicate for cross-highlighting.

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    functions_aggregate::expr_fn::sum,
    prelude::{SessionContext, col, lit},
};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart equality selection")
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
                ('Alpha',  7.0, 0.8, 1.2), ('Alpha',  5.0, 1.3, 2.0), ('Alpha',  6.0, 1.8, 1.6),
                ('Beta',   4.0, 2.0, 3.7), ('Beta',   8.0, 2.5, 4.8), ('Beta',   3.0, 3.0, 4.0),
                ('Gamma',  9.0, 3.2, 5.7), ('Gamma',  6.0, 3.7, 6.6), ('Gamma',  4.0, 4.2, 5.9),
                ('Delta',  3.0, 4.4, 2.3), ('Delta',  5.0, 4.9, 2.9), ('Delta',  7.0, 5.4, 3.4)
            ) AS t(category, amount, x_value, y_value)",
        )
        .await
        .expect("build data");

    let picked = PointSelection::new("picked").field("category");
    let selected = picked.predicate();

    let bars = Plot::<Cartesian>::new().data(df.clone()).tool(picked).mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.18))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("amount")))
            .fill_with(lit("#c8cdd7"), |c| {
                c.no_scale()
                    .when_value(selected.clone(), lit("#2563eb"))
                    .no_legend()
            })
            .stroke("#ffffff")
            .stroke_width(1.0),
    );

    let scatter = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x_value"))
            .y(col("y_value"))
            .fill_with(lit("#b8beca"), |c| {
                c.no_scale()
                    .when_value(selected, lit("#2563eb"))
                    .no_legend()
            })
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(150.0),
    );

    let plot = Chart::<HConcat>::new()
        .canvas_size(1040.0, 470.0)
        .mark(
            Subplot::new(bars)
                .caption("Click bars")
                .name("bars")
                .label("Bar selection"),
        )
        .mark(
            Subplot::new(scatter)
                .caption("Sibling scatter")
                .name("scatter")
                .label("Cross-highlight"),
        );

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
