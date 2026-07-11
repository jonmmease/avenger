//! Repeat-grid scatterplot matrix with pan/scroll zoom.
//!
//! Drag with the left mouse button inside any cell to pan. Scroll over any
//! cell to zoom around the pointer. Domains are coordinated by repeated
//! variable, so panning the x axis of one cell also updates y axes for cells
//! where the same variable appears on the row.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example repeat_splom_pan_zoom --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::SessionContext,
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
            .with_title("avenger-chart repeat SPLOM pan/zoom")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .read_batch(make_penguin_like_batch())
        .expect("read generated SPLOM data");

    let variables = vec![
        RepeatVariable::new("bill_length_mm", col("bill_length_mm")).title("bill length"),
        RepeatVariable::new("bill_depth_mm", col("bill_depth_mm")).title("bill depth"),
        RepeatVariable::new("flipper_length_mm", col("flipper_length_mm")).title("flipper length"),
        RepeatVariable::new("body_mass_g", col("body_mass_g")).title("body mass"),
    ];

    let cell = Plot::<Cartesian>::new()
        .mark(
            Symbol::new()
                .x(repeat::column())
                .y(repeat::row())
                .fill(col("species"))
                .size(28.0)
                .opacity(0.72),
        )
        .tool(PanScrollZoom::cartesian());

    let plot = Chart::<RepeatGrid>::new()
        .canvas_size(960.0, 820.0)
        .data(df)
        .configure_coord(|c| {
            c.rows(variables.clone())
                .columns(variables)
                .cell(cell)
                .matrix_domains()
                .matrix_axes()
        });

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

fn make_penguin_like_batch() -> RecordBatch {
    let mut species = Vec::with_capacity(180);
    let mut bill_length = Vec::with_capacity(180);
    let mut bill_depth = Vec::with_capacity(180);
    let mut flipper_length = Vec::with_capacity(180);
    let mut body_mass = Vec::with_capacity(180);

    let groups = [
        ("Adelie", 38.5, 18.4, 189.0, 3650.0),
        ("Chinstrap", 48.8, 18.2, 196.0, 3750.0),
        ("Gentoo", 47.5, 15.0, 216.0, 5100.0),
    ];
    for (group_index, (name, bill_base, depth_base, flipper_base, mass_base)) in
        groups.iter().enumerate()
    {
        for i in 0..60 {
            let t = i as f64;
            let wave = (t / 5.0 + group_index as f64).sin();
            let drift = (t - 30.0) / 30.0;
            species.push(*name);
            bill_length.push(bill_base + drift * 5.0 + wave * 1.8);
            bill_depth.push(depth_base + drift * 1.1 - wave * 0.7);
            flipper_length.push(flipper_base + drift * 12.0 + wave * 3.5);
            body_mass.push(mass_base + drift * 720.0 + wave * 180.0);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("species", DataType::Utf8, false),
        Field::new("bill_length_mm", DataType::Float64, false),
        Field::new("bill_depth_mm", DataType::Float64, false),
        Field::new("flipper_length_mm", DataType::Float64, false),
        Field::new("body_mass_g", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(species)),
            Arc::new(Float64Array::from(bill_length)),
            Arc::new(Float64Array::from(bill_depth)),
            Arc::new(Float64Array::from(flipper_length)),
            Arc::new(Float64Array::from(body_mass)),
        ],
    )
    .expect("build generated SPLOM data")
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
