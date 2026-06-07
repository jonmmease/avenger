//! Repeat-grid scatterplot matrix with the built-in box selection tool.
//!
//! Drag inside a repeated cell to draw or update that cell's brush. Brushes are
//! OR'ed together and the same selection predicate highlights the sibling view.
//! Double-click inside any repeated cell to clear all brushes.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example repeat_splom_box_select --features winit-wgpu --release
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
    prelude::{Expr, SessionContext, col, lit},
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
            .with_title("avenger-chart repeat SPLOM box selection")
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

    let brush = BoxSelection::cartesian("brush")
        .dimensions(repeat::column(), repeat::row())
        .resolve(BoxSelectionResolve::Union)
        .repeat_cell_chrome();
    let selected = brush.predicate();

    let variables = vec![
        RepeatVariable::new("bill_length_mm", col("bill_length_mm")).title("bill length"),
        RepeatVariable::new("bill_depth_mm", col("bill_depth_mm")).title("bill depth"),
        RepeatVariable::new("flipper_length_mm", col("flipper_length_mm")).title("flipper length"),
    ];

    let cell = Plot::<Cartesian>::new()
        .mark(repeat_points(selected.clone(), 24.0))
        .tool(brush);

    let splom = Plot::<RepeatGrid>::new()
        .data(df.clone())
        .rows(variables.clone())
        .columns(variables)
        .cell(cell)
        .matrix_domains()
        .matrix_axes();

    let sibling = Plot::<Cartesian>::new()
        .data(df)
        .title("Selected rows")
        .mark(
            Symbol::new()
                .x(col("bill_length_mm"))
                .y(col("body_mass_g"))
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.6)
                .size(42.0),
        );

    let plot = Plot::<HConcat>::new()
        .canvas_size(1180.0, 720.0)
        .mark(Subplot::new(splom).id("splom").key("splom"))
        .mark(Subplot::new(sibling).id("sibling").key("sibling"));

    chart_avenger_app(
        plot.compile(&ctx).await.expect("compile plot"),
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

fn repeat_points(selected: Expr, size: f64) -> Symbol<Cartesian> {
    Symbol::new()
        .x(repeat::column())
        .y(repeat::row())
        .fill_with(lit("#b8beca"), |c| {
            c.no_scale()
                .when_value(selected, lit("#2563eb"))
                .no_legend()
        })
        .stroke("#ffffff")
        .stroke_width(0.5)
        .size(size)
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
