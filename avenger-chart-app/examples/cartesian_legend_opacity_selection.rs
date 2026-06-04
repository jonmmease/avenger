//! Scatter plot with legend-driven opacity selection.
//!
//! Click legend items to toggle categories in or out of the selection. Selected
//! categories render at full opacity and unselected categories render at 0.4
//! opacity. Double-click a legend item to clear the selection and make all
//! categories opaque again.

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::{SessionContext, col, lit};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart legend opacity selection")
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
                ('Alpha', 0.7, 1.2), ('Alpha', 1.1, 1.9), ('Alpha', 1.6, 1.5), ('Alpha', 2.1, 2.4),
                ('Beta',  2.2, 3.2), ('Beta',  2.7, 3.7), ('Beta',  3.1, 2.9), ('Beta',  3.5, 4.2),
                ('Gamma', 1.4, 4.6), ('Gamma', 1.9, 5.1), ('Gamma', 2.4, 5.7), ('Gamma', 3.0, 5.3),
                ('Delta', 4.0, 1.7), ('Delta', 4.5, 2.3), ('Delta', 5.1, 1.9), ('Delta', 5.6, 2.8),
                ('Epsilon', 4.2, 4.8), ('Epsilon', 4.8, 5.4), ('Epsilon', 5.3, 4.9), ('Epsilon', 5.8, 5.8)
            ) AS t(category, x_value, y_value)",
        )
        .await
        .expect("build data");

    let picked = Selection::new("picked").empty_selects_all();
    let selected = picked.predicate();

    let legend_toggle = ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::button().eq(lit("left")))
        .filter(ev::datum("channel").eq(lit("fill")))
        .set_selection(
            "picked",
            SelectionUpdate::toggle_clause(
                SelectionClauseUpdate::equality_value(col("category"), ev::datum("value"))
                    .facet_scope(Sharing::Shared),
            ),
        )
        .exact();
    let legend_clear = ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(ev::datum("channel").eq(lit("fill")))
        .set_selection("picked", SelectionUpdate::clear())
        .exact();

    let plot = Plot::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .title("Legend opacity selection")
        .add_selection(picked)
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x_value"))
                .y(col("y_value"))
                .size(180.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| {
                        l.id("category_legend")
                            .title("Category")
                            .event_binding(legend_toggle)
                            .event_binding(legend_clear)
                    })
                })
                .opacity_with(lit(0.4), |c| {
                    c.no_scale().when_value(selected, lit(1.0)).no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.75),
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
