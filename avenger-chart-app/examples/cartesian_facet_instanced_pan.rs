//! 2×3 faceted scatter with **instanced** symbol marks + pan/zoom.
//!
//! Each of the 6 facet cells holds **200 scatter points**. In `avenger-wgpu` a
//! symbol mark renders through the GPU instanced path (`InstancedMarkRenderer`,
//! a dedicated draw) only when it has `>= 100` points, no gradients, and a
//! `None`/`Rect` clip — so every cell here crosses that threshold while the
//! existing small-data facet examples stay on the multi path. The frame
//! therefore interleaves **6 instanced draws** with the facet chrome (axes,
//! grid, labels) emitted by the shared multi-renderer, exercising the
//! render-pass merging in `MultiMarkRenderer::encode_multi_ranges` against the
//! instanced path (a combination not covered by the visual-regression suite).
//!
//! Drag with the left mouse button inside any cell to pan **all** cells together
//! and scroll inside any cell to zoom **all** cells around the pointer (x and y
//! scales are globally shared). The interaction bindings stay in Preview mode so
//! repeated drags/scrolls are not blocked by a full Exact settle pass. Per-frame
//! render metrics are logged (`surface_render_ms`, and `avenger_wgpu`
//! `command_count` at debug level).
//!
//! Run with:
//! ```bash
//! RUST_LOG=avenger_winit_wgpu=info,avenger_wgpu=debug \
//!   cargo run -p avenger-chart-app --example cartesian_facet_instanced_pan --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::{SessionContext, lit};
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
            .with_title("avenger-chart 2×3 instanced facet — 200 pts/cell — drag pan / scroll zoom")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

/// Build a deterministic 6-group × 200-point scatter dataset as a SQL `VALUES`
/// literal (1200 rows). Deterministic (seeded xorshift64*) so every launch
/// renders the same scene, keeping repeated render-time measurements comparable.
fn scatter_values_sql() -> String {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_unit = || -> f64 {
        // xorshift64* → uniform value in [0, 1).
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        let v = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut rows = String::new();
    for g in 0..6 {
        for i in 0..200 {
            if !(g == 0 && i == 0) {
                rows.push(',');
            }
            let x = next_unit() * 10.0;
            let y = next_unit() * 10.0;
            rows.push_str(&format!("('Group {g}', {x:.4}, {y:.4})"));
        }
    }
    format!("SELECT * FROM (VALUES {rows}) AS t(group_name, x, y)")
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx.sql(&scatter_values_sql()).await.expect("build data");

    // Globally shared raw-domain params: null until an interaction writes a
    // concrete domain, at which point every cell re-renders with that domain.
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    let pan = common::cartesian_drag_pan_binding(&x_domain, &y_domain, false);
    let zoom = common::cartesian_scroll_zoom_binding(&x_domain, &y_domain);

    // Leaf scatter: 200 pts/cell (>= 100 ⇒ instanced path). x and y scales are
    // globally shared and read the shared raw-domain params, so interacting in
    // any cell pans/zooms every cell.
    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()).nice(false).zero(false))
                    .share_scale()
            })
            .y_with(col("y"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()).nice(false).zero(false))
                    .share_scale()
            })
            .fill(col("group_name"))
            .size(40.0),
    );

    // Wrap the 6 groups into a 3-column layout ⇒ a 2-row × 3-column grid.
    let plot = Plot::<FacetWrap>::new()
        .add_param(x_domain.clone())
        .add_param(y_domain.clone())
        .canvas_size(960.0, 640.0)
        .data(df)
        .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| c.columns(lit(3))))
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
