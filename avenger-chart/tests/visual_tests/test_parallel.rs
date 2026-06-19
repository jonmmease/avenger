use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

fn parallel_drag_params(dimension_id: &str, display_x: f64) -> IndexMap<String, ScalarValue> {
    IndexMap::from([
        (
            "drag_dimension".to_string(),
            ScalarValue::Utf8(Some(dimension_id.to_string())),
        ),
        (
            "drag_display_x".to_string(),
            ScalarValue::Float64(Some(display_x)),
        ),
    ])
}

fn numeric_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("speed", DataType::Float64, false),
        Field::new("efficiency", DataType::Float64, false),
        Field::new("stability", DataType::Float64, false),
        Field::new("cost", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "a0", "a1", "a2", "a3", "b0", "b1", "b2", "b3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "alpha", "alpha", "alpha", "alpha", "beta", "beta", "beta", "beta",
            ])),
            Arc::new(Float64Array::from(vec![
                48.0, 53.0, 59.0, 64.0, 42.0, 47.0, 52.0, 57.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.62, 0.68, 0.71, 0.76, 0.57, 0.61, 0.66, 0.70,
            ])),
            Arc::new(Float64Array::from(vec![
                69.0, 73.0, 78.0, 81.0, 75.0, 77.0, 80.0, 83.0,
            ])),
            Arc::new(Float64Array::from(vec![
                118.0, 126.0, 134.0, 145.0, 99.0, 108.0, 116.0, 124.0,
            ])),
        ],
    )
    .expect("numeric parallel test data");
    ctx.read_batch(batch).expect("numeric parallel dataframe")
}

fn missing_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("speed", DataType::Float64, true),
        Field::new("efficiency", DataType::Float64, true),
        Field::new("stability", DataType::Float64, true),
        Field::new("cost", DataType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["a0", "a1", "b0", "b1"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["alpha", "alpha", "beta", "beta"])),
            Arc::new(Float64Array::from(vec![
                Some(48.0),
                Some(62.0),
                Some(44.0),
                Some(57.0),
            ])),
            Arc::new(Float64Array::from(vec![
                Some(0.62),
                None,
                Some(0.58),
                Some(0.70),
            ])),
            Arc::new(Float64Array::from(vec![
                Some(70.0),
                Some(82.0),
                None,
                Some(78.0),
            ])),
            Arc::new(Float64Array::from(vec![
                Some(115.0),
                Some(142.0),
                Some(100.0),
                None,
            ])),
        ],
    )
    .expect("missing-value parallel test data");
    ctx.read_batch(batch)
        .expect("missing-value parallel dataframe")
}

fn mixed_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("tier", DataType::Utf8, false),
        Field::new("latency", DataType::Float64, false),
        Field::new("quality", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "bronze", "silver", "gold", "bronze", "silver", "gold", "bronze", "silver", "gold",
            ])),
            Arc::new(Float64Array::from(vec![
                94.0, 82.0, 71.0, 88.0, 74.0, 62.0, 101.0, 86.0, 68.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.48, 0.61, 0.78, 0.55, 0.66, 0.84, 0.44, 0.58, 0.80,
            ])),
        ],
    )
    .expect("mixed parallel test data");
    ctx.read_batch(batch).expect("mixed parallel dataframe")
}

fn facet_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("panel", DataType::Utf8, false),
        Field::new("team", DataType::Utf8, false),
        Field::new("speed", DataType::Float64, false),
        Field::new("efficiency", DataType::Float64, false),
        Field::new("stability", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "low", "low", "low", "low", "high", "high", "high", "high",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "north", "south", "east", "west", "north", "south", "east", "west",
            ])),
            Arc::new(Float64Array::from(vec![
                22.0, 27.0, 31.0, 36.0, 76.0, 83.0, 91.0, 98.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.31, 0.38, 0.44, 0.50, 0.69, 0.76, 0.83, 0.90,
            ])),
            Arc::new(Float64Array::from(vec![
                42.0, 47.0, 51.0, 55.0, 68.0, 74.0, 81.0, 88.0,
            ])),
        ],
    )
    .expect("facet parallel test data");
    ctx.read_batch(batch).expect("facet parallel dataframe")
}

fn facet_parallel_child(free_domains: bool) -> Plot<Parallel> {
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| {
            let d = d.scale_with::<Linear>(|s| s).axis(|a| a.title("Speed"));
            if free_domains {
                d.free_domain()
            } else {
                d.share_domain()
            }
        })
        .dimension_with("efficiency", col("efficiency"), |d| {
            let d = d
                .scale_with::<Linear>(|s| s)
                .axis(|a| a.title("Efficiency"));
            if free_domains {
                d.free_domain()
            } else {
                d.share_domain()
            }
        })
        .dimension_with("stability", col("stability"), |d| {
            let d = d.scale_with::<Linear>(|s| s).axis(|a| a.title("Stability"));
            if free_domains {
                d.free_domain()
            } else {
                d.share_domain()
            }
        });

    Plot::with_coord(coord)
        .mark(
            ParallelLine::new()
                .stroke("#64748b")
                .stroke_width(1.4)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("team"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(70.0)
                .opacity(0.9),
        )
}

fn repeat_parallel_cell() -> Plot<Parallel> {
    let coord = Parallel::new()
        .dimension_with("metric", repeat::column(), |d| {
            d.axis(|a| a.title(repeat::column_title()))
        })
        .dimension_with("stability", col("stability"), |d| d.axis(|a| a.title("S")));

    Plot::with_coord(coord)
        .mark(
            ParallelLine::new()
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(1.5)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(70.0)
                .opacity(0.9),
        )
}

#[tokio::test]
async fn parallel_points_overlay() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")));

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(82.0)
                .opacity(0.92),
        );

    let compiled = plot.compile(&ctx).await.expect("compile parallel points");
    assert_visual_match_default(&compiled, &ctx, None, "parallel", "parallel_points_overlay").await;
}

#[tokio::test]
async fn parallel_missing_values() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")));

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(missing_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(2.0)
                .opacity(0.65),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(82.0)
                .opacity(0.95),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile parallel missing values");
    assert_visual_match_default(&compiled, &ctx, None, "parallel", "parallel_missing_values").await;
}

#[tokio::test]
async fn parallel_axis_grid_enabled() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| {
            d.axis(|a| a.title("Speed").grid(true).tick_count(6.0))
        })
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")));

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke("#64748b")
                .stroke_width(1.4)
                .opacity(0.52),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(74.0)
                .opacity(0.92),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile parallel axis grid");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_grid_enabled",
    )
    .await;
}

#[tokio::test]
async fn parallel_points_categorical_axis() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("latency", col("latency"), |d| {
            d.axis(|a| a.title("Latency"))
        })
        .dimension_with("tier", col("tier"), |d| {
            d.scale_with::<Point>(|s| s).axis(|a| a.title("Tier"))
        })
        .dimension_with("quality", col("quality"), |d| {
            d.axis(|a| a.title("Quality"))
        });

    let plot = Plot::with_coord(coord)
        .canvas_size(560.0, 340.0)
        .plot_size(430.0, 205.0)
        .data(mixed_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke_with(col("tier"), |stroke| stroke.no_legend())
                .stroke_width(1.5)
                .opacity(0.38),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("tier"), |fill| fill.no_legend())
                .stroke("#1f2937")
                .stroke_width(0.75)
                .size(74.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile parallel categorical points");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_points_categorical_axis",
    )
    .await;
}

#[tokio::test]
async fn parallel_facet_shared_domains() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .plot_size(260.0, 180.0)
        .data(facet_parallel_data(&ctx))
        .mark(Subplot::new(facet_parallel_child(false)).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-domain parallel facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_facet_shared_domains",
    )
    .await;
}

#[tokio::test]
async fn parallel_facet_free_domains() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .plot_size(260.0, 180.0)
        .data(facet_parallel_data(&ctx))
        .mark(Subplot::new(facet_parallel_child(true)).column(col("panel")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free-domain parallel facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_facet_free_domains",
    )
    .await;
}

#[tokio::test]
async fn parallel_repeat_small_multiples() {
    let ctx = SessionContext::new();
    let plot = Plot::<RepeatColumns>::new()
        .canvas_size(680.0, 330.0)
        .plot_size(215.0, 180.0)
        .data(numeric_parallel_data(&ctx))
        .columns([
            RepeatVariable::new("speed", col("speed")).title("Speed"),
            RepeatVariable::new("cost", col("cost")).title("Cost"),
        ])
        .cell(repeat_parallel_cell());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeated parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_repeat_small_multiples",
    )
    .await;
}

#[tokio::test]
async fn parallel_static_reordered_axes() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")))
        .order(["cost", "speed", "stability", "efficiency"]);

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke("#64748b")
                .stroke_width(1.35)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(78.0)
                .opacity(0.9),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile static reordered parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_static_reordered_axes",
    )
    .await;
}

#[tokio::test]
async fn parallel_displaced_axis_preview() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")))
        .active_axis_display_params("drag_dimension", "drag_display_x");

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .add_param(Param::new("drag_dimension", ScalarValue::Utf8(None)))
        .add_param(Param::new("drag_display_x", ScalarValue::Float64(None)))
        .data(numeric_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(82.0)
                .opacity(0.92),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile displaced parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(parallel_drag_params("efficiency", 245.0)),
        "parallel",
        "parallel_displaced_axis_preview",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_displaced_axis() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")))
        .active_axis_display_params("drag_dimension", "drag_display_x");

    let stability_overlay = ParallelAxisOverlay::new(
        "stability",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(74.0))
                .y2(lit(82.0))
                .fill("rgba(37, 99, 235, 0.16)")
                .stroke("#2563eb")
                .stroke_width(1.4),
        ),
    )
    .width_px(38.0)
    .zindex(10);

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .add_param(Param::new("drag_dimension", ScalarValue::Utf8(None)))
        .add_param(Param::new("drag_display_x", ScalarValue::Float64(None)))
        .data(numeric_parallel_data(&ctx))
        .mark(stability_overlay)
        .mark(
            ParallelLine::new()
                .stroke("#94a3b8")
                .stroke_width(1.25)
                .opacity(0.38),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(76.0)
                .opacity(0.9),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile displaced overlay parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(parallel_drag_params("stability", 260.0)),
        "parallel",
        "parallel_axis_overlay_displaced_axis",
    )
    .await;
}
