use std::sync::Arc;

use arrow::{
    array::Float64Array,
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WindowSceneSizing, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_default_runtime_resources,
};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const POINT_COLUMNS: usize = 1000;
const POINT_ROWS: usize = 1000;
const POINT_COUNT: usize = POINT_COLUMNS * POINT_ROWS;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub async fn run() {
    init_diagnostics();

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let bundle = build_app().await;
            let options = app_options(bundle.runtime_resources.render_invalidation_hub);
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options);
            event_loop.run_app(&mut app).expect("run app");
        } else {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("build tokio runtime");
            let bundle = tokio_runtime.block_on(build_app());
            let options = app_options(bundle.runtime_resources.render_invalidation_hub);
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop_with_options(
                bundle.app,
                options,
                tokio_runtime,
            );
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}

fn app_options(
    render_invalidation_hub: avenger_resource::RenderInvalidationHub,
) -> WinitWgpuAvengerAppOptions {
    WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart 1M instanced scatter")
                .with_resizable(false),
        )
        .window_scene_sizing(WindowSceneSizing::MatchSceneGraph)
        .render_invalidation_hub(render_invalidation_hub)
}

async fn build_app() -> avenger_chart_app::ChartAppBundle {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .read_batch(make_points_batch())
        .expect("read generated 1M points");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(960.0, 640.0)
        .configure_title("Instanced scatter stress: $n = 1,000,000$", |title| {
            title.typst()
        })
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill("#1f77b4")
                .shape("circle")
                .size(9.0)
                .stroke_width(0.0)
                .opacity(0.45),
        )
        .tool(PanScrollZoom::cartesian());

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_default_runtime_resources(
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
    let mut xs = Vec::with_capacity(POINT_COUNT);
    let mut ys = Vec::with_capacity(POINT_COUNT);
    for row in 0..POINT_ROWS {
        for col in 0..POINT_COLUMNS {
            let idx = row * POINT_COLUMNS + col;
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
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("initialize logger");
        } else if #[cfg(not(target_arch = "wasm32"))] {
            if std::env::var_os("RUST_LOG").is_some() {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .try_init();
            } else {
                let _ = env_logger::try_init();
            }
        }
    }
}
