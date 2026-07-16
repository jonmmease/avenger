// Low-level parallel-coordinate header drag reordering.
//
// Drag a dimension title to preview a displaced axis. On release, explicit
// bindings write a new order-list parameter. This intentionally stays manual
// rather than introducing a reusable reorder tool.
//
// Run with:
// ```bash
// cargo run --release -p avenger-chart-app --example parallel_coordinates_header_drag_reorder --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::{event as ev, parallel::event as parallel_ev, prelude::*};
use avenger_chart_app::chart_avenger_app;
use datafusion::{
    logical_expr::when,
    prelude::{SessionContext, col, lit},
    scalar::ScalarValue,
};

mod parallel_common;

const ORDER_PARAM: &str = "axis_order";
const DRAG_DIMENSION_PARAM: &str = "drag_dimension";
const DRAG_START_X_PARAM: &str = "drag_start_x";
const DRAG_DISPLAY_X_PARAM: &str = "drag_display_x";

fn main() {
    parallel_common::run_fixed_window_app(
        "avenger-chart parallel header drag reorder",
        build_app(),
    );
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let order = Param::new(
        ORDER_PARAM,
        parallel_common::string_list_scalar(&dimension_ids()),
    );
    let drag_dimension = Param::new(DRAG_DIMENSION_PARAM, ScalarValue::Utf8(None));
    let drag_start_x = Param::new(DRAG_START_X_PARAM, ScalarValue::Float64(None));
    let drag_display_x = Param::new(DRAG_DISPLAY_X_PARAM, ScalarValue::Float64(None));
    let coord = parallel_common::demo_parallel()
        .order_param(order.name.clone())
        .active_axis_display_params(drag_dimension.name.clone(), drag_display_x.name.clone());

    let mut plot = Chart::with_coord(coord)
        .params([
            order.clone(),
            drag_dimension.clone(),
            drag_start_x.clone(),
            drag_display_x.clone(),
        ])
        .canvas_size(
            parallel_common::CANVAS_SIZE[0],
            parallel_common::CANVAS_SIZE[1],
        )
        .plot_size(parallel_common::PLOT_SIZE[0], parallel_common::PLOT_SIZE[1])
        .title("Drag a dimension title to reorder axes")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            parallel_common::demo_parallel_line()
                .details(["sample_id"])
                .stroke("#94a3b8")
                .stroke_width(1.0)
                .opacity(0.42),
        )
        .mark(
            parallel_common::demo_parallel_symbol()
                .fill(col("segment"))
                .stroke("#ffffff")
                .stroke_width(0.8)
                .size(18.0)
                .opacity(0.72),
        )
        .event_binding(cursor_binding())
        .event_binding(start_drag_binding())
        .event_binding(preview_drag_binding());

    for source_id in dimension_ids() {
        for target_index in 0..parallel_common::NUMERIC_DIMENSIONS.len() {
            plot = plot.event_binding(commit_order_binding(source_id, target_index));
        }
    }
    plot = plot.event_binding(clear_drag_binding());

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(compiled, ctx, parallel_common::app_options())
        .await
        .expect("build chart app")
}

fn cursor_binding() -> ChartEventBinding {
    let over_header = parallel_ev::parallel_dimension_id().is_not_null();
    let cursor_expr = when(over_header, ev::cursor(CursorStyle::Grab))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("cursor conditional");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_cursor(cursor_expr)
        .preview()
}

fn start_drag_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::MouseDown)
        .filter(ev::button().eq(lit("left")))
        .filter(parallel_ev::parallel_dimension_id().is_not_null())
        .set_param(DRAG_DIMENSION_PARAM, parallel_ev::parallel_dimension_id())
        .set_param(DRAG_START_X_PARAM, parallel_ev::parallel_display_x())
        .set_param(DRAG_DISPLAY_X_PARAM, parallel_ev::parallel_display_x())
        .set_cursor(ev::cursor(CursorStyle::Grabbing))
        .preview()
}

fn preview_drag_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::param(DRAG_DIMENSION_PARAM).is_not_null())
        .set_param(
            DRAG_DISPLAY_X_PARAM,
            ev::param(DRAG_START_X_PARAM) + ev::dx(),
        )
        .set_cursor(ev::cursor(CursorStyle::Grabbing))
        .preview()
}

fn commit_order_binding(source_id: &'static str, target_index: usize) -> ChartEventBinding {
    let (lower, upper) = target_slot_bounds(target_index);
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::param(DRAG_DIMENSION_PARAM).eq(lit(source_id)))
    .filter(ev::param(DRAG_DISPLAY_X_PARAM).gt_eq(lit(lower)))
    .filter(ev::param(DRAG_DISPLAY_X_PARAM).lt(lit(upper)))
    .set_param(
        ORDER_PARAM,
        parallel_common::order_literal(&order_after_move(source_id, target_index)),
    )
    .set_param(DRAG_DIMENSION_PARAM, lit(ScalarValue::Utf8(None)))
    .set_param(DRAG_START_X_PARAM, lit(ScalarValue::Float64(None)))
    .set_param(DRAG_DISPLAY_X_PARAM, lit(ScalarValue::Float64(None)))
    .exact()
}

fn clear_drag_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::MouseUp)
        .set_param(DRAG_DIMENSION_PARAM, lit(ScalarValue::Utf8(None)))
        .set_param(DRAG_START_X_PARAM, lit(ScalarValue::Float64(None)))
        .set_param(DRAG_DISPLAY_X_PARAM, lit(ScalarValue::Float64(None)))
        .set_cursor(ev::cursor(CursorStyle::Default))
        .exact()
}

fn target_slot_bounds(target_index: usize) -> (f64, f64) {
    let slot_width = f64::from(parallel_common::PLOT_SIZE[0])
        / (parallel_common::NUMERIC_DIMENSIONS.len() - 1) as f64;
    let center = target_index as f64 * slot_width;
    let lower = if target_index == 0 {
        -10_000.0
    } else {
        center - slot_width / 2.0
    };
    let upper = if target_index + 1 == parallel_common::NUMERIC_DIMENSIONS.len() {
        10_000.0
    } else {
        center + slot_width / 2.0
    };
    (lower, upper)
}

fn order_after_move(source_id: &'static str, target_index: usize) -> Vec<&'static str> {
    let mut ids = dimension_ids()
        .into_iter()
        .filter(|id| *id != source_id)
        .collect::<Vec<_>>();
    ids.insert(target_index.min(ids.len()), source_id);
    ids
}

fn dimension_ids() -> Vec<&'static str> {
    parallel_common::NUMERIC_DIMENSIONS
        .iter()
        .map(|dimension| dimension.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_coordinates_header_drag_reorder_app_builds() {
        let _ = build_app().await;
    }

    #[test]
    fn manual_order_update_moves_source_to_target_slot() {
        assert_eq!(
            order_after_move("stability", 0),
            vec!["stability", "speed", "efficiency", "cost", "quality"]
        );
        assert_eq!(
            order_after_move("speed", 3),
            vec!["efficiency", "stability", "cost", "speed", "quality"]
        );
    }
}
