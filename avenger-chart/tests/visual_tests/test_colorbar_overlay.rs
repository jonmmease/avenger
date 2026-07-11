use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use datafusion::prelude::SessionContext;

use super::helpers::assert_visual_match_default;

fn colorbar_points_batch() -> RecordBatch {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut temps = Vec::new();
    for row in 0..7 {
        for col_index in 0..9 {
            let idx = row * 9 + col_index;
            xs.push(col_index as f64 + (row as f64 * 0.17).sin() * 0.16);
            ys.push(row as f64 + (col_index as f64 * 0.31).cos() * 0.18);
            temps.push(8.0 + row as f64 * 7.5 + col_index as f64 * 3.2 + (idx as f64).sin() * 2.5);
        }
    }
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("temperature", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
            Arc::new(Float64Array::from(temps)),
        ],
    )
    .expect("points batch")
}

fn interval_store_batch(lo: f64, hi: f64) -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("temperature_min", DataType::Float64, false),
            Field::new("temperature_max", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["active"])),
            Arc::new(Float64Array::from(vec![lo])),
            Arc::new(Float64Array::from(vec![hi])),
        ],
    )
    .expect("interval store batch")
}

fn interval_store(lo: f64, hi: f64) -> Store {
    Store::from_record_batch("temperature_interval", interval_store_batch(lo, hi))
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn vertical_colorbar_overlay() -> ColorbarOverlay {
    ColorbarOverlay::new().mark(
        Rect::<Cartesian>::new()
            .data_store(StoreData::new("temperature_interval"))
            .exclude_from_scale_domains()
            .x(lit(0.0))
            .x2(lit(1.0))
            .y(col("temperature_min"))
            .y2(col("temperature_max"))
            .fill("rgba(37, 99, 235, 0.18)")
            .stroke("#2563eb")
            .stroke_width(1.5)
            .zindex(10_000),
    )
}

fn horizontal_colorbar_overlay() -> ColorbarOverlay {
    ColorbarOverlay::new().mark(
        Rect::<Cartesian>::new()
            .data_store(StoreData::new("temperature_interval"))
            .exclude_from_scale_domains()
            .x(col("temperature_min"))
            .x2(col("temperature_max"))
            .y(lit(0.0))
            .y2(lit(1.0))
            .fill("rgba(37, 99, 235, 0.18)")
            .stroke("#2563eb")
            .stroke_width(1.5)
            .zindex(10_000),
    )
}

async fn base_plot(
    ctx: &SessionContext,
    position: LegendPosition,
    overlay: ColorbarOverlay,
    highlight_selected: bool,
) -> CompiledPlot {
    let df = ctx
        .read_batch(colorbar_points_batch())
        .expect("read colorbar points");
    let selected = col("temperature")
        .gt_eq(lit(28.0))
        .and(col("temperature").lt_eq(lit(48.0)));

    let highlight_mark = Symbol::new()
        .x(col("x"))
        .y(col("y"))
        .fill_with(col("temperature"), |c| {
            c.scale_with::<Linear>(|s| s.domain((0.0, 70.0)).nice(false).zero(false))
                .legend(|l| {
                    l.title("Temperature")
                        .position(position)
                        .colorbar_overlay(overlay)
                })
        })
        .stroke("#ffffff")
        .stroke_width(0.8)
        .size(96.0);
    let highlight_mark = if highlight_selected {
        highlight_mark.opacity_with(lit(0.0), |c| {
            c.no_scale().when_value(selected, lit(1.0)).no_legend()
        })
    } else {
        highlight_mark.opacity_with(lit(1.0), |c| c.no_scale().no_legend())
    };

    Chart::<Cartesian>::new()
        .canvas_size(760.0, 460.0)
        .data(df)
        .store(interval_store(28.0, 48.0))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill("#aeb6c4")
                .stroke("#ffffff")
                .stroke_width(0.6)
                .size(82.0),
        )
        .mark(highlight_mark)
        .compile(ctx)
        .await
        .expect("compile colorbar overlay plot")
}

#[tokio::test]
async fn colorbar_overlay_vertical_interval() {
    let ctx = SessionContext::new();
    let compiled = base_plot(
        &ctx,
        LegendPosition::Right,
        vertical_colorbar_overlay(),
        false,
    )
    .await;
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_overlay",
        "colorbar_overlay_vertical_interval",
    )
    .await;
}

#[tokio::test]
async fn colorbar_overlay_horizontal_interval() {
    let ctx = SessionContext::new();
    let compiled = base_plot(
        &ctx,
        LegendPosition::Bottom,
        horizontal_colorbar_overlay(),
        false,
    )
    .await;
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_overlay",
        "colorbar_overlay_horizontal_interval",
    )
    .await;
}

#[tokio::test]
async fn colorbar_interval_selection_final_state() {
    let ctx = SessionContext::new();
    let compiled = base_plot(
        &ctx,
        LegendPosition::Right,
        vertical_colorbar_overlay(),
        true,
    )
    .await;
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "colorbar_overlay",
        "colorbar_interval_selection_final_state",
    )
    .await;
}
