//! Grouped bars with a nested categorical x scale and clickable selection.
//!
//! Click a bar to select its quarter and team source-column values. The leaf
//! nested axis level is hidden, so the team value is shown by the legend while
//! selection remains cross-filter friendly. Double-click to clear.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example nested_grouped_bar_click_selection --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit, when},
};
use winit::window::WindowAttributes;

const SIZE: [f32; 2] = [760.0, 460.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart nested grouped-bar click selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let picked = Selection::new("picked").empty_selects_nothing();
    let selected = picked.predicate();
    let cursor = Param::cursor("pick_cursor", CursorStyle::Default);

    let plot = Chart::<Cartesian>::new()
        .title("Click a grouped bar")
        .canvas_size(SIZE[0], SIZE[1])
        .data(ctx.read_batch(grouped_bar_batch()).expect("read data"))
        .selection(picked)
        .param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .id("bars")
                .x_with(nested(["quarter", "team"]), |x| {
                    x.axis(|a| a.title("Quarter").grid(false))
                        .level(0, |level| level.padding_inner(0.45).padding_outer(0.15))
                        .level(1, |level| {
                            level
                                .nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                                .axis(|axis| axis.visible(false))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill(col("team"))
                .stroke_with(lit("#ffffff"), |stroke| {
                    stroke
                        .no_scale()
                        .when_value(selected.clone(), lit("#111827"))
                        .no_legend()
                })
                .stroke_width_with(lit(1.0), |stroke_width| {
                    stroke_width
                        .no_scale()
                        .when_value(selected.clone(), lit(3.0))
                        .no_legend()
                }),
        )
        .event_binding(cursor_binding(&cursor))
        .event_binding(select_bar_binding())
        .event_binding(clear_selection_binding());

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

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_bar = ev::datum("value")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
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
        .filter(ev::datum("value").is_not_null())
        .set_selection(
            "picked",
            SelectionUpdate::replace_clause(source_column_clause()),
        )
        .exact()
}

fn clear_selection_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("picked")
        .exact()
}

fn source_column_clause() -> SelectionClauseUpdate {
    SelectionClauseUpdate::equality(lit("active"))
        .facet_scope(CoordinationScope::Shared)
        .dimension_named("quarter", col("quarter"), ev::datum("quarter"))
        .dimension_named("team", col("team"), ev::datum("team"))
        .build()
}

fn grouped_bar_batch() -> RecordBatch {
    let quarter = ["Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3"];
    let team = [
        "North", "South", "East", "North", "East", "North", "South", "East",
    ];
    let value = [42.0, 30.0, 34.0, 47.0, 38.0, 51.0, 39.0, 44.0];

    let schema = Arc::new(Schema::new(vec![
        Field::new("quarter", DataType::Utf8, false),
        Field::new("team", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(quarter.to_vec())) as ArrayRef,
            Arc::new(StringArray::from(team.to_vec())) as ArrayRef,
            Arc::new(Float64Array::from(value.to_vec())) as ArrayRef,
        ],
    )
    .expect("grouped bar example data")
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
