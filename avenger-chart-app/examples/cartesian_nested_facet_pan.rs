//! Nested row × column faceted Cartesian pan/zoom with **`Level(1)` sharing**.
//!
//! A two-level facet (rows wrap columns) where the x and y raw-domain params are
//! declared `Sharing::Level(1)` and the leaf scales are shared at `Level(1)`.
//! `Level(1)` means "share one facet level up", which for a leaf cell at depth 2
//! (`[row, column]`) is the ROW. So dragging with the left mouse button or
//! scrolling inside any cell pans/zooms EVERY cell in that cell's row together,
//! while the other rows stay put.
//!
//! This showcases the per-cell scoped-param resolution: the binding routes the
//! pointer to the cell under it, reads that cell's `Level(1)` owner path (the
//! row), and writes the shared-per-row domain — so the whole row re-renders with
//! the new domain and sibling rows are untouched.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_nested_facet_pan --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart nested pan/zoom — drag or scroll a row-owned cell")
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
                ('Top','Left', 1.0, 2.0),  ('Top','Left', 3.0, 4.5),  ('Top','Left', 5.0, 3.0),
                ('Top','Mid', 2.0, 5.0),   ('Top','Mid', 4.0, 3.0),   ('Top','Mid', 6.0, 7.0),
                ('Top','Right', 1.5, 3.5), ('Top','Right', 3.5, 6.0),  ('Top','Right', 5.5, 2.5),
                ('Bottom','Left', 2.0, 6.0),  ('Bottom','Left', 4.0, 4.0),  ('Bottom','Left', 6.0, 8.0),
                ('Bottom','Mid', 1.0, 4.5),   ('Bottom','Mid', 3.0, 2.5),   ('Bottom','Mid', 5.0, 6.5),
                ('Bottom','Right', 2.5, 3.0), ('Bottom','Right', 4.5, 5.5),  ('Bottom','Right', 6.5, 4.0)
            ) AS t(row_name, col_name, x, y)",
        )
        .await
        .expect("build data");

    // Leaf scatter: x and y scales are shared at Level(1) (per row). The tool
    // mirrors that sharing for the generated raw-domain params.
    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| c.with_scale_sharing(Sharing::Level(1)))
            .y_with(col("y"), |c| c.with_scale_sharing(Sharing::Level(1)))
            .fill(col("col_name"))
            .size(80.0),
    );

    // Inner: facet leaf by column. Outer: facet that by row.
    let columns = Plot::<FacetColumn>::new().mark(Subplot::new(leaf).column(col("col_name")));

    let plot = Plot::<FacetRow>::new()
        .canvas_size(820.0, 520.0)
        .data(df)
        .mark(Subplot::new(columns).row(col("row_name")))
        .tool(PanScrollZoom::cartesian().settle_exact(true));

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
