//! Temporal nested bars cross-filter a non-nested monthly detail chart.
//!
//! The left chart uses `TimeLevels` as a nested categorical band position:
//! year -> quarter -> month. Click a nested month bar to select its integer
//! year/quarter/month keys. The right chart filters source rows by that
//! selection before aggregation, drawing the filtered month total in blue over a
//! grey full-data background.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example temporal_nested_cross_filter --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, TimestampMillisecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit},
};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [920.0, 460.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart temporal nested cross-filter")
            .with_inner_size(LogicalSize::new(f64::from(SIZE[0]), f64::from(SIZE[1])))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .read_batch(source_batch())
        .expect("read temporal nested data");
    let picked = Selection::new("picked").empty_selects_nothing();
    let selected = picked.predicate();

    let source_plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .title("Click a nested month")
        .plot_size(410.0, 270.0)
        .mark(nested_month_bars())
        .event_binding(select_period_binding())
        .event_binding(clear_selection_binding());

    let detail_plot = Plot::<Cartesian>::new()
        .data(df)
        .title("Filtered monthly detail")
        .plot_size(310.0, 270.0)
        .mark(month_detail_background())
        .mark(month_detail_overlay(selected));

    let plot = Plot::<HConcat>::new()
        .canvas_size(SIZE[0], SIZE[1])
        .add_selection(picked)
        .mark(Subplot::new(source_plot).key("nested"))
        .mark(Subplot::new(detail_plot).key("detail"));

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

fn nested_month_bars() -> Rect<Cartesian> {
    Rect::new().transform(
        TimeLevels::new(col("timestamp"))
            .year()
            .quarter()
            .month()
            .name("period"),
        |mark, period| {
            mark.transform(
                Aggregate::new()
                    .group_by(period.keys())
                    .sum("total", col("value")),
                |mark, aggregate| {
                    mark.x_with(period.nested(), |x| {
                        x.axis(|axis| axis.title("Month grouped by quarter"))
                            .level(1, |level| level.nest_scope(NestScope::Shared))
                            .level(2, |level| {
                                level
                                    .nest_scope(NestScope::Shared)
                                    .axis(|axis| axis.label_angle(-90.0))
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(lit(0.0), |y| y.axis(|axis| axis.title("Value").grid(true)))
                    .y2(aggregate.output("total"))
                    .fill("#c7ced8")
                    .stroke("#ffffff")
                    .stroke_width(1.0)
                },
            )
        },
    )
}

fn month_detail_background() -> Rect<Cartesian> {
    Rect::new()
        .transform(period_transform(), |mark, _period| mark)
        .transform(
            Aggregate::new()
                .group_by([col("period_month")])
                .sum("month_total", col("value")),
            |mark, aggregate| {
                mark.x_with(col("period_month"), |x| {
                    x.scale_with::<Band>(|scale| scale.padding_inner(0.22))
                        .axis(|axis| axis.title("Month key"))
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| y.axis(|axis| axis.title("Value").grid(true)))
                .y2(aggregate.output("month_total"))
                .fill("#d1d5db")
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        )
}

fn month_detail_overlay(selected: datafusion::logical_expr::Expr) -> Rect<Cartesian> {
    Rect::new()
        .transform(period_transform(), |mark, _period| mark)
        .transform_no_output(Filter::new(selected), |mark| mark)
        .transform(
            Aggregate::new()
                .group_by([col("period_month")])
                .sum("selected_total", col("value")),
            |mark, aggregate| {
                mark.x_with(col("period_month"), |x| {
                    x.scale_with::<Band>(|scale| scale.padding_inner(0.22))
                        .axis(|axis| axis.title("Month key"))
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y(lit(0.0))
                .y2(aggregate.output("selected_total"))
                .fill("#2563eb")
                .opacity(0.86)
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        )
}

fn period_transform() -> TimeLevels {
    TimeLevels::new(col("timestamp"))
        .year()
        .quarter()
        .month()
        .name("period")
}

fn select_period_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::button().eq(lit("left")))
        .filter(ev::datum("total").is_not_null())
        .set_selection("picked", SelectionUpdate::replace_clause(period_clause()))
        .exact()
}

fn clear_selection_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("picked")
        .exact()
}

fn period_clause() -> SelectionClauseUpdate {
    SelectionClauseUpdate::equality(lit("active"))
        .facet_scope(CoordinationScope::Shared)
        .dimension_named("year", col("period_year"), ev::datum("period_year"))
        .dimension_named(
            "quarter",
            col("period_quarter"),
            ev::datum("period_quarter"),
        )
        .dimension_named("month", col("period_month"), ev::datum("period_month"))
        .build()
}

fn source_batch() -> RecordBatch {
    let timestamp = vec![
        1_704_412_800_000_i64,
        1_705_708_800_000_i64,
        1_709_337_600_000_i64,
        1_712_880_000_000_i64,
        1_716_422_400_000_i64,
        1_719_014_400_000_i64,
        1_735_689_600_000_i64,
        1_738_368_000_000_i64,
        1_740_960_000_000_i64,
        1_744_243_200_000_i64,
    ];
    let value = vec![14.0, 5.0, 18.0, 22.0, 16.0, 24.0, 12.0, 20.0, 9.0, 27.0];
    let schema = Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        ),
        Field::new("value", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(TimestampMillisecondArray::from(timestamp)) as ArrayRef,
            Arc::new(Float64Array::from(value)) as ArrayRef,
        ],
    )
    .expect("temporal nested cross-filter data")
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
