//! Faceted Cartesian pan/zoom with **per-cell (`Free`) sharing**.
//!
//! A column-faceted scatter where the x and y raw-domain params are declared
//! `Sharing::Free` and each cell's scales are `free_scale()` (independent per
//! cell). Dragging with the left mouse button or scrolling inside ONE cell
//! pans/zooms ONLY that cell: the bindings route the pointer to the cell under
//! it, invert through that cell's scale, and write the param at the cell's own
//! owner path, so the other cells keep their domains.
//!
//! Contrast this with `cartesian_facet_pan` (Shared), where dragging any cell
//! pans every cell together.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_facet_pan_free --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

mod common;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart per-cell pan/zoom — drag or scroll one cell")
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
                ('A', 1.0, 2.0), ('A', 3.0, 4.5), ('A', 5.0, 3.2), ('A', 7.0, 6.1),
                ('B', 2.0, 5.0), ('B', 4.0, 3.0), ('B', 6.0, 7.0), ('B', 8.0, 4.4),
                ('C', 1.5, 3.5), ('C', 3.5, 6.0), ('C', 5.5, 2.5), ('C', 7.5, 5.5)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("build data");

    // Per-cell raw-domain params: `Free` means each facet cell keeps its own
    // copy. Null by default (scales fall back to their per-cell inferred domains)
    // until an interaction writes a concrete domain for that cell.
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    let pan = common::cartesian_drag_pan_binding(&x_domain, &y_domain, true);
    let zoom = common::cartesian_scroll_zoom_binding(&x_domain, &y_domain);

    // Leaf scatter: x and y scales are independent per cell (`free_scale`) and
    // read the per-cell raw-domain params.
    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()).nice(false).zero(false))
                    .free_scale()
            })
            .y_with(col("y"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()).nice(false).zero(false))
                    .free_scale()
            })
            .fill(col("group_name"))
            .size(80.0),
    );

    let plot = Plot::<FacetColumn>::new()
        .add_param_with_sharing(x_domain.clone(), Sharing::Free)
        .add_param_with_sharing(y_domain.clone(), Sharing::Free)
        .canvas_size(820.0, 360.0)
        .data(df)
        .mark(Subplot::new(leaf).column(col("group_name")))
        .event_bindings([pan, zoom]);

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
