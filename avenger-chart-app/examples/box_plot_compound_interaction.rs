//! Compound BoxPlot part targeting for selection and outlier tooltip state.
//!
//! Click a generated box part to select a category/segment group. Hover a
//! generated outlier symbol to populate a small tooltip label from the raw row
//! datum. Double-click clears the selected group and tooltip.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example box_plot_compound_interaction --features winit-wgpu
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
    functions::string::expr_fn::concat,
    logical_expr::{Expr, expr_fn::cast},
    prelude::{SessionContext, col, lit, when},
};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [920.0, 520.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart compound box plot interaction")
            .with_inner_size(LogicalSize::new(f64::from(SIZE[0]), f64::from(SIZE[1])))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let selected_group = Selection::new("selected_group").empty_selects_nothing();
    let selected = selected_group.predicate();
    let cursor = Param::cursor("box_plot_cursor", CursorStyle::Default);

    let plot = Plot::<Cartesian>::new()
        .title("Click boxes; hover outliers")
        .canvas_size(SIZE[0], SIZE[1])
        .data(ctx.read_batch(source_batch()).expect("read data"))
        .add_selection(selected_group)
        .add_store(outlier_tooltip_store())
        .add_param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .mark(base_box_plot())
        .mark(selected_box_plot(selected))
        .mark(
            Text::new()
                .id("outlier_tooltip")
                .data_store(StoreData::new("outlier_tooltip"))
                .exclude_from_scale_domains()
                .x(col("x"))
                .y_with(grouped_y(), configure_grouped_y)
                .text(col("label"))
                .align("right")
                .baseline("middle")
                .font_size(13.0)
                .font_weight("bold")
                .color("#111827")
                .opacity_with(col("opacity"), |opacity| opacity.no_scale())
                .zindex(20_000),
        )
        .event_binding(cursor_binding(&cursor))
        .event_binding(select_box_binding())
        .event_binding(show_outlier_tooltip_binding())
        .event_binding(clear_outlier_tooltip_binding())
        .event_binding(clear_all_binding());

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

fn grouped_y() -> ChannelExpr {
    nested(["category", "segment"])
}

fn configure_grouped_y(y: CartesianPositionConfig) -> CartesianPositionConfig {
    y.axis(|axis| axis.title("Segment grouped by category").grid(false))
        .level(0, |level| level.padding_inner(0.38).padding_outer(0.12))
        .level(1, |level| {
            level
                .nest_scope(NestScope::Shared)
                .padding_inner(0.12)
                .axis(|axis| axis.title("Segment"))
        })
}

fn base_box_plot() -> BoxPlot {
    BoxPlot::new()
        .id("my_box_plot")
        .x_with(col("value"), configure_value_x)
        .y_with(grouped_y(), configure_grouped_y)
        .fill_with(col("segment"), |fill| {
            fill.legend(|legend| legend.title("Segment"))
        })
        .box_body(|body| body.opacity(0.46))
        .outliers(|outliers| {
            outliers
                .size(110.0)
                .stroke("#ffffff")
                .stroke_width(1.5)
                .opacity(0.62)
        })
}

fn selected_box_plot(selected: Expr) -> BoxPlot {
    BoxPlot::new()
        .id("selected_box_plot")
        .transform_no_output(Filter::new(selected), |mark| mark)
        .x_with(col("value"), configure_value_x)
        .y_with(grouped_y(), configure_grouped_y)
        .fill("#2563eb")
        .box_body(|body| {
            body.fill("#2563eb")
                .stroke("#1e3a8a")
                .stroke_width(2.0)
                .opacity(0.92)
        })
        .median(|median| median.stroke("#111827").stroke_width(3.0))
        .whiskers(|whiskers| whiskers.stroke("#1e3a8a").stroke_width(2.0))
        .caps(|caps| caps.stroke("#1e3a8a").stroke_width(2.0))
        .outliers(|outliers| {
            outliers
                .size(135.0)
                .fill("#2563eb")
                .stroke("#ffffff")
                .stroke_width(2.0)
        })
}

fn configure_value_x(x: CartesianPositionConfig) -> CartesianPositionConfig {
    x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(40.0)))
        .axis(|axis| axis.title("Value").grid(true))
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_box_or_outlier = ev::datum("category")
        .is_not_null()
        .and(ev::datum("segment").is_not_null());
    let cursor_expr = when(over_box_or_outlier, ev::cursor(CursorStyle::Grab))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn select_box_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown)
            .mark("my_box_plot.box")
            .filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::datum("category").is_not_null())
    .filter(ev::datum("segment").is_not_null())
    .set_selection(
        "selected_group",
        SelectionUpdate::replace_clause(selected_group_clause()),
    )
    .exact()
}

fn show_outlier_tooltip_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MarkMouseEnter).mark("my_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MarkMouseLeave).mark("my_box_plot.outliers"),
        )
        .filter(ev::datum("value").is_not_null())
        .set_store_replacing_scopes("outlier_tooltip", outlier_tooltip_update())
        .preview()
}

fn clear_outlier_tooltip_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MarkMouseEnter).mark("my_box_plot.outliers"),
        ChartEventStream::on(ChartEventType::MarkMouseLeave).mark("my_box_plot.outliers"),
    )
    .set_store_replacing_scopes("outlier_tooltip", hidden_tooltip_update())
    .exact()
}

fn clear_all_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("selected_group")
        .set_store_replacing_scopes("outlier_tooltip", hidden_tooltip_update())
        .exact()
}

fn selected_group_clause() -> SelectionClauseUpdate {
    SelectionClauseUpdate::equality(lit("active"))
        .facet_scope(CoordinationScope::Shared)
        .dimension_datum_named("category", "category")
        .dimension_datum_named("segment", "segment")
        .build()
}

fn outlier_tooltip_store() -> Store {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("opacity", DataType::Float64, false),
    ]));
    let initial = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["hovered_outlier"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Platform"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["SMB"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
            Arc::new(StringArray::from(vec![""])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0])) as ArrayRef,
        ],
    )
    .expect("initial tooltip store row");
    Store::from_record_batch("outlier_tooltip", initial)
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn outlier_tooltip_update() -> StoreUpdate {
    StoreUpdate::replace_rows([StoreRow::new()
        .field("id", lit("hovered_outlier"))
        .field("category", ev::datum("category"))
        .field("segment", ev::datum("segment"))
        .field("x", ev::datum("value") - lit(0.75))
        .field("label", outlier_tooltip_label())
        .field("opacity", lit(1.0))])
}

fn hidden_tooltip_update() -> StoreUpdate {
    StoreUpdate::replace_rows([StoreRow::new()
        .field("id", lit("hovered_outlier"))
        .field("category", lit("Platform"))
        .field("segment", lit("SMB"))
        .field("x", lit(0.0))
        .field("label", lit(""))
        .field("opacity", lit(0.0))])
}

fn outlier_tooltip_label() -> Expr {
    concat(vec![
        ev::datum("category"),
        lit(" / "),
        ev::datum("segment"),
        lit(": "),
        cast(ev::datum("value"), DataType::Utf8),
    ])
}

fn source_batch() -> RecordBatch {
    let mut categories = Vec::new();
    let mut segments = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "Platform",
            "SMB",
            [12.0, 14.0, 15.0, 15.5, 16.0, 17.0, 18.5, 24.0].as_slice(),
        ),
        (
            "Platform",
            "Enterprise",
            [18.0, 19.0, 21.0, 22.0, 22.5, 23.0, 24.0, 31.0].as_slice(),
        ),
        (
            "Infrastructure",
            "SMB",
            [9.0, 10.0, 11.0, 12.0, 12.5, 13.0, 14.0, 19.0].as_slice(),
        ),
        (
            "Infrastructure",
            "Enterprise",
            [20.0, 21.0, 23.0, 24.0, 24.5, 25.0, 26.0, 34.0].as_slice(),
        ),
        (
            "Services",
            "SMB",
            [7.0, 8.0, 9.0, 9.5, 10.0, 11.0, 12.0, 18.0].as_slice(),
        ),
        (
            "Services",
            "Enterprise",
            [14.0, 15.0, 16.0, 17.0, 17.5, 18.0, 19.0, 27.0].as_slice(),
        ),
    ];

    for (category, segment, group_values) in observations {
        for value in group_values {
            categories.push(category);
            segments.push(segment);
            values.push(*value);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(categories)) as ArrayRef,
            Arc::new(StringArray::from(segments)) as ArrayRef,
            Arc::new(Float64Array::from(values)) as ArrayRef,
        ],
    )
    .expect("box plot interaction example data")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn box_plot_compound_interaction_app_builds() {
        let _ = build_app().await;
    }
}
