//! Single-panel Cartesian pan/zoom stress example with 1M large translucent circle symbols.
//!
//! Drag with the left mouse button inside the plot area to pan. Scroll over the
//! plot area to zoom around the pointer. The pan binding stays in Preview mode
//! after mouse-up so repeated drags are not blocked by a full Exact settle pass.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_instanced_pan_stress --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::SessionContext,
};
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
            .with_title("avenger-chart 1M large translucent circles - drag pan / scroll zoom")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .read_batch(make_points_batch())
        .expect("read generated 1M points");

    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    let pan = common::cartesian_drag_pan_binding(&x_domain, &y_domain, false);
    let zoom = common::cartesian_scroll_zoom_binding(&x_domain, &y_domain);

    let plot = Plot::<Cartesian>::new()
        .add_param(x_domain.clone())
        .add_param(y_domain.clone())
        .canvas_size(960.0, 640.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(x_raw.clone()).nice(false).zero(false)
                    })
                })
                .y_with(col("y"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(y_raw.clone()).nice(false).zero(false)
                    })
                })
                .fill("#1f77b4")
                .shape("circle")
                .size(144.0)
                .stroke_width(0.0)
                .opacity(0.45),
        )
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

fn make_points_batch() -> RecordBatch {
    let columns = 1000;
    let rows = 1000;
    let point_count = columns * rows;
    let mut xs = Vec::with_capacity(point_count);
    let mut ys = Vec::with_capacity(point_count);
    for row in 0..rows {
        for col in 0..columns {
            let idx = row * columns + col;
            let jitter_x = (((idx * 37 + 11) % 100) as f64 - 50.0) / 120.0;
            let jitter_y = (((idx * 53 + 7) % 100) as f64 - 50.0) / 120.0;
            let x = col as f64 + jitter_x;
            let wave = (col as f64 / 16.0).sin() * 12.0 + (col as f64 / 37.0).cos() * 6.0;
            let y = row as f64 + wave + jitter_y;
            xs.push(x);
            ys.push(y);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("build generated 1M point batch")
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
