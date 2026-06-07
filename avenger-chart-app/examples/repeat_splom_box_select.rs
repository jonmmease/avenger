//! Repeat-grid scatterplot matrix with low-level box selection.
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

use avenger_chart::event as ev;
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
    prelude::{Expr, SessionContext, col, lit, when},
};
use winit::window::WindowAttributes;

const BRUSH_STORE: &str = "repeat_brush_boxes";
const BRUSH_SELECTION: &str = "brush";

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

    let brush = Selection::new(BRUSH_SELECTION)
        .combine(SelectionCombine::Union)
        .empty_selects_nothing();
    let selected = brush.predicate();
    let cursor = Param::cursor("brush_cursor", CursorStyle::Default);

    let variables = vec![
        RepeatVariable::new("bill_length_mm", col("bill_length_mm")).title("bill length"),
        RepeatVariable::new("bill_depth_mm", col("bill_depth_mm")).title("bill depth"),
        RepeatVariable::new("flipper_length_mm", col("flipper_length_mm")).title("flipper length"),
    ];

    let cell = Plot::<Cartesian>::new()
        .mark(repeat_points(selected.clone(), 24.0))
        .mark(selection_overlay())
        .event_binding(cursor_binding(&cursor))
        .event_binding(selection_drag_binding(&cursor))
        .event_binding(selection_release_binding())
        .event_binding(selection_clear_binding());

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
        .add_store(brush_box_store())
        .add_selection(brush)
        .add_param(cursor.clone())
        .cursor_param(cursor.name.clone())
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

fn selection_overlay() -> Rect<Cartesian> {
    Rect::<Cartesian>::new()
        .data_store(StoreData::new(BRUSH_STORE))
        .transform_no_output(Filter::new(repeat::current_cell_predicate()), |mark| mark)
        .exclude_from_scale_domains()
        .x(col("x_min"))
        .x2(col("x_max"))
        .y(col("y_min"))
        .y2(col("y_max"))
        .fill("rgba(37, 99, 235, 0.10)")
        .stroke("#2563eb")
        .stroke_width(1.5)
        .zindex(10_000)
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_plot = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
    let cursor_expr = when(over_plot, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor case expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn selection_drag_binding(cursor: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_selection_at_start_scope(BRUSH_SELECTION, upsert_selection_update())
        .set_store_at_start_scope(BRUSH_STORE, upsert_store_update())
        .preview()
}

fn selection_release_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .set_selection_at_start_scope(BRUSH_SELECTION, upsert_selection_update())
    .set_store_at_start_scope(BRUSH_STORE, upsert_store_update())
    .exact()
}

fn selection_clear_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection(BRUSH_SELECTION)
        .set_store_replacing_scopes(BRUSH_STORE, StoreUpdate::clear())
        .exact()
}

fn brush_box_store() -> Store {
    Store::empty(BRUSH_STORE)
        .field("id", DataType::Utf8, false)
        .field("cell_id", DataType::Utf8, false)
        .field("row_id", DataType::Utf8, false)
        .field("column_id", DataType::Utf8, false)
        .field("x_min", DataType::Float64, false)
        .field("x_max", DataType::Float64, false)
        .field("y_min", DataType::Float64, false)
        .field("y_max", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn brush_selection_clause() -> SelectionClauseUpdate {
    SelectionClauseUpdate::interval(repeat::cell_id())
        .facet_scope(CoordinationScope::Shared)
        .dimension(repeat::column())
        .endpoints(
            ev::interval_start(ev::interval_ordered(
                ev::start_coord("x"),
                ev::event_at_start_clipped_coord("x"),
            )),
            ev::interval_end(ev::interval_ordered(
                ev::start_coord("x"),
                ev::event_at_start_clipped_coord("x"),
            )),
        )
        .dimension(repeat::row())
        .endpoints(
            ev::interval_start(ev::interval_ordered(
                ev::start_coord("y"),
                ev::event_at_start_clipped_coord("y"),
            )),
            ev::interval_end(ev::interval_ordered(
                ev::start_coord("y"),
                ev::event_at_start_clipped_coord("y"),
            )),
        )
        .build()
}

fn upsert_selection_update() -> SelectionUpdate {
    SelectionUpdate::upsert_clause(brush_selection_clause())
}

fn brush_box_row() -> StoreRow {
    let x_interval =
        ev::interval_ordered(ev::start_coord("x"), ev::event_at_start_clipped_coord("x"));
    let y_interval =
        ev::interval_ordered(ev::start_coord("y"), ev::event_at_start_clipped_coord("y"));
    StoreRow::new()
        .field("id", repeat::cell_id())
        .field("cell_id", repeat::cell_id())
        .field("row_id", repeat::row_id())
        .field("column_id", repeat::column_id())
        .field("x_min", ev::interval_start(x_interval.clone()))
        .field("x_max", ev::interval_end(x_interval))
        .field("y_min", ev::interval_start(y_interval.clone()))
        .field("y_max", ev::interval_end(y_interval))
}

fn upsert_store_update() -> StoreUpdate {
    StoreUpdate::upsert_rows([brush_box_row()])
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
