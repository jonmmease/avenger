//! Repeated grouped bars with cross-filtered aggregate overlays.
//!
//! Each subplot groups count-aggregated source rows by quarter plus the repeated
//! field name. The grey bars show the full counts; the blue overlay filters the
//! source data by the active click selection before aggregating. Click a bar in
//! one subplot to see the corresponding filtered counts in the other subplot.
//! Double-click to clear.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example repeated_grouped_bar_cross_filter --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit, when},
};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [920.0, 450.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart repeated grouped-bar cross-filter")
            .with_inner_size(LogicalSize::new(f64::from(SIZE[0]), f64::from(SIZE[1])))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let picked = Selection::new("picked").empty_selects_nothing();
    let selected_predicate = picked.predicate();
    let cursor = Param::cursor("pick_cursor", CursorStyle::Default);

    let background = Rect::new().transform(grouped_count(), |mark, count| {
        mark.id("full_counts")
            .x_with(grouped_x(), configure_grouped_x)
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(lit(0.0), |y| y.axis(|a| a.title("Rows").grid(true)))
            .y2(count.output("count"))
            .fill("#d1d5db")
            .stroke("#ffffff")
            .stroke_width(1.0)
    });

    let foreground = Rect::new()
        .transform_no_output(Filter::new(selected_predicate), |mark| mark)
        .transform(grouped_count(), |mark, count| {
            mark.id("filtered_counts")
                .x_with(grouped_x(), configure_grouped_x)
                .x2_with(col(":x"), |x| x.band(1.0))
                .y(lit(0.0))
                .y2(count.output("count"))
                .fill("#2563eb")
                .opacity(0.86)
                .stroke("#ffffff")
                .stroke_width(1.0)
        });

    let cell = Plot::<Cartesian>::new()
        .mark(background)
        .mark(foreground)
        .event_binding(cursor_binding(&cursor))
        .event_binding(select_bar_binding())
        .event_binding(clear_selection_binding());

    let plot = Plot::<RepeatColumns>::new()
        .title("Click a grouped bar to cross-filter")
        .canvas_size(SIZE[0], SIZE[1])
        .data(ctx.read_batch(source_batch()).expect("read data"))
        .plot_size(380.0, 250.0)
        .add_selection(picked)
        .add_param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .columns(vec![
            RepeatVariable::field("team").title("Team"),
            RepeatVariable::field("channel").title("Channel"),
        ])
        .cell(cell);

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

fn grouped_count() -> Aggregate {
    Aggregate::new()
        .group_by([col("quarter"), col(repeat::column_name())])
        .count("count")
}

fn grouped_x() -> ChannelExpr {
    nested(["quarter".to_string(), repeat::column_name()])
}

fn configure_grouped_x(x: CartesianPositionConfig) -> CartesianPositionConfig {
    x.axis(|axis| axis.title("Quarter").grid(false))
        .level(0, |level| level.padding_inner(0.38).padding_outer(0.12))
        .level(1, |level| {
            level
                .nest_scope(NestScope::Shared)
                .padding_inner(0.06)
                .axis(|axis| axis.title(repeat::column_title()))
        })
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_bar = ev::datum("quarter").is_not_null();
    let cursor_expr = when(over_bar, ev::cursor(CursorStyle::Grab))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn select_bar_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::button().eq(lit("left")))
        .filter(ev::datum("quarter").is_not_null())
        .set_selection(
            "picked",
            SelectionUpdate::replace_clause(selection_clause()),
        )
        .exact()
}

fn clear_selection_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("picked")
        .exact()
}

fn selection_clause() -> SelectionClauseUpdate {
    SelectionClauseUpdate::equality(lit("active"))
        .facet_scope(CoordinationScope::Shared)
        .dimension_datum_named("quarter", "quarter")
        .dimension_datum_named("selected_repeat_value", repeat::column_name())
        .build()
}

fn source_batch() -> RecordBatch {
    let mut quarter = Vec::new();
    let mut team = Vec::new();
    let mut channel = Vec::new();

    for (q, t, c, n) in [
        ("Q1", "Alpha", "Retail", 18),
        ("Q1", "Alpha", "Partner", 8),
        ("Q1", "Beta", "Retail", 7),
        ("Q1", "Beta", "Online", 12),
        ("Q1", "Gamma", "Retail", 5),
        ("Q1", "Gamma", "Partner", 10),
        ("Q2", "Alpha", "Retail", 13),
        ("Q2", "Alpha", "Online", 11),
        ("Q2", "Beta", "Retail", 9),
        ("Q2", "Beta", "Partner", 14),
        ("Q2", "Gamma", "Online", 8),
        ("Q2", "Gamma", "Partner", 7),
        ("Q3", "Alpha", "Retail", 9),
        ("Q3", "Alpha", "Partner", 13),
        ("Q3", "Beta", "Online", 17),
        ("Q3", "Beta", "Partner", 6),
        ("Q3", "Gamma", "Retail", 12),
        ("Q3", "Gamma", "Online", 8),
        ("Q4", "Alpha", "Retail", 10),
        ("Q4", "Alpha", "Online", 15),
        ("Q4", "Beta", "Retail", 6),
        ("Q4", "Beta", "Partner", 11),
        ("Q4", "Gamma", "Online", 10),
        ("Q4", "Gamma", "Partner", 12),
    ] {
        for _ in 0..n {
            quarter.push(q);
            team.push(t);
            channel.push(c);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("quarter", DataType::Utf8, false),
        Field::new("team", DataType::Utf8, false),
        Field::new("channel", DataType::Utf8, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(quarter)) as ArrayRef,
            Arc::new(StringArray::from(team)) as ArrayRef,
            Arc::new(StringArray::from(channel)) as ArrayRef,
        ],
    )
    .expect("cross-filter example data")
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
