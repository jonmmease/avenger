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
        Field::new("all", DataType::Utf8, false),
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
                "all", "all", "all", "all", "all", "all", "all", "all",
            ])),
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

fn axis_interval_data(
    ctx: &SessionContext,
    value_min: f64,
    value_max: f64,
) -> datafusion::dataframe::DataFrame {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("value_min", DataType::Float64, false),
            Field::new("value_max", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![value_min])) as ArrayRef,
            Arc::new(Float64Array::from(vec![value_max])),
        ],
    )
    .expect("axis interval data");
    ctx.read_batch(batch).expect("axis interval dataframe")
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

fn numeric_parallel_coord() -> Parallel {
    Parallel::new()
        .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", |d| d.axis(|a| a.title("Efficiency")))
        .dimension_with("stability", |d| d.axis(|a| a.title("Stability")))
        .dimension_with("cost", |d| d.axis(|a| a.title("Cost")))
}

fn numeric_parallel_line() -> ParallelLine {
    ParallelLine::new()
        .dimension("speed", col("speed"))
        .dimension("efficiency", col("efficiency"))
        .dimension("stability", col("stability"))
        .dimension("cost", col("cost"))
}

fn numeric_parallel_symbol() -> ParallelSymbol {
    ParallelSymbol::new()
        .dimension("speed", col("speed"))
        .dimension("efficiency", col("efficiency"))
        .dimension("stability", col("stability"))
        .dimension("cost", col("cost"))
}

fn mixed_parallel_coord() -> Parallel {
    Parallel::new()
        .dimension_with("latency", |d| d.axis(|a| a.title("Latency")))
        .dimension_with("tier", |d| d.axis(|a| a.title("Tier")))
        .dimension_with("quality", |d| d.axis(|a| a.title("Quality")))
}

fn mixed_parallel_line() -> ParallelLine {
    ParallelLine::new()
        .dimension("latency", col("latency"))
        .dimension_with("tier", col("tier"), |d| d.scale_with::<Point>(|s| s))
        .dimension("quality", col("quality"))
}

fn mixed_parallel_symbol() -> ParallelSymbol {
    ParallelSymbol::new()
        .dimension("latency", col("latency"))
        .dimension_with("tier", col("tier"), |d| d.scale_with::<Point>(|s| s))
        .dimension("quality", col("quality"))
}

fn facet_parallel_child(free_domains: bool) -> Plot<Parallel> {
    let coord = Parallel::new()
        .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", |d| d.axis(|a| a.title("Efficiency")))
        .dimension_with("stability", |d| d.axis(|a| a.title("Stability")));

    let configure_domain = |d: ParallelDimensionConfig| {
        let d = d.scale_with::<Linear>(|s| s);
        if free_domains {
            d.free_domain()
        } else {
            d.share_domain()
        }
    };

    Plot::with_coord(coord)
        .mark(
            ParallelLine::new()
                .dimension_with("speed", col("speed"), configure_domain)
                .dimension_with("efficiency", col("efficiency"), configure_domain)
                .dimension_with("stability", col("stability"), configure_domain)
                .stroke("#64748b")
                .stroke_width(1.4)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .dimension("speed", col("speed"))
                .dimension("efficiency", col("efficiency"))
                .dimension("stability", col("stability"))
                .fill_with(col("team"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(70.0)
                .opacity(0.9),
        )
}

fn repeat_parallel_cell() -> Plot<Parallel> {
    let coord = Parallel::new()
        .dimension_with("metric", |d| d.axis(|a| a.title(repeat::column_title())))
        .dimension_with("stability", |d| d.axis(|a| a.title("S")));

    Plot::with_coord(coord)
        .mark(
            ParallelLine::new()
                .dimension("metric", repeat::column())
                .dimension("stability", col("stability"))
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(1.5)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .dimension("metric", repeat::column())
                .dimension("stability", col("stability"))
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
    let coord = numeric_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            numeric_parallel_symbol()
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
    let coord = numeric_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(missing_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(2.0)
                .opacity(0.65),
        )
        .mark(
            numeric_parallel_symbol()
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
async fn parallel_numeric_basic() {
    let ctx = SessionContext::new();
    let coord = Parallel::new();

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#64748b")
                .stroke_width(1.45)
                .opacity(0.46),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile basic numeric parallel plot");
    assert_visual_match_default(&compiled, &ctx, None, "parallel", "parallel_numeric_basic").await;
}

#[tokio::test]
async fn parallel_color_by_category() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(720.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke_with(col("group"), |stroke| {
                    stroke.legend(|legend| legend.title("Group"))
                })
                .stroke_width(1.8)
                .opacity(0.64),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile category-colored parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_color_by_category",
    )
    .await;
}

#[tokio::test]
async fn parallel_numeric_axes() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", |d| d.axis(|a| a.title("Speed").tick_count(6.0)))
        .dimension_with("efficiency", |d| {
            d.axis(|a| a.title("Efficiency").tick_count(5.0))
        })
        .dimension_with("stability", |d| {
            d.axis(|a| a.title("Stability").tick_count(6.0))
        })
        .dimension_with("cost", |d| d.axis(|a| a.title("Cost").tick_count(6.0)));

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.25)
                .opacity(0.42),
        )
        .mark(
            numeric_parallel_symbol()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(62.0)
                .opacity(0.88),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile numeric-axis parallel plot");
    assert_visual_match_default(&compiled, &ctx, None, "parallel", "parallel_numeric_axes").await;
}

#[tokio::test]
async fn parallel_mixed_numeric_categorical() {
    let ctx = SessionContext::new();
    let coord = mixed_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(560.0, 340.0)
        .plot_size(430.0, 205.0)
        .data(mixed_parallel_data(&ctx))
        .mark(
            mixed_parallel_line()
                .stroke_with(col("tier"), |stroke| stroke.no_legend())
                .stroke_width(1.55)
                .opacity(0.52),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile mixed numeric/categorical parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_mixed_numeric_categorical",
    )
    .await;
}

#[tokio::test]
async fn parallel_long_axis_labels() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", |d| {
            d.axis(|a| a.title("Maximum observed operating speed"))
        })
        .dimension_with("efficiency", |d| {
            d.axis(|a| a.title("Energy conversion efficiency ratio"))
        })
        .dimension_with("stability", |d| {
            d.axis(|a| a.title("Long term stability score"))
        })
        .dimension_with("cost", |d| d.axis(|a| a.title("Estimated lifecycle cost")));

    let plot = Chart::with_coord(coord)
        .canvas_size(1120.0, 380.0)
        .plot_size(880.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#64748b")
                .stroke_width(1.35)
                .opacity(0.44),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile long-label parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_long_axis_labels",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_grid_enabled() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", |d| {
            d.axis(|a| a.title("Speed").grid(true).tick_count(6.0))
        })
        .dimension_with("efficiency", |d| d.axis(|a| a.title("Efficiency")))
        .dimension_with("stability", |d| d.axis(|a| a.title("Stability")))
        .dimension_with("cost", |d| d.axis(|a| a.title("Cost")));

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#64748b")
                .stroke_width(1.4)
                .opacity(0.52),
        )
        .mark(
            numeric_parallel_symbol()
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
    let coord = mixed_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(560.0, 340.0)
        .plot_size(430.0, 205.0)
        .data(mixed_parallel_data(&ctx))
        .mark(
            mixed_parallel_line()
                .stroke_with(col("tier"), |stroke| stroke.no_legend())
                .stroke_width(1.5)
                .opacity(0.38),
        )
        .mark(
            mixed_parallel_symbol()
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
    let plot = Chart::<FacetColumn>::new()
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
    let plot = Chart::<FacetColumn>::new()
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
    let plot = Chart::<RepeatColumns>::new()
        .canvas_size(680.0, 330.0)
        .plot_size(215.0, 180.0)
        .data(numeric_parallel_data(&ctx))
        .configure_coord(|c| {
            c.columns([
                RepeatVariable::new("speed", col("speed")).title("Speed"),
                RepeatVariable::new("cost", col("cost")).title("Cost"),
            ])
            .cell(repeat_parallel_cell())
        });

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
    let coord = numeric_parallel_coord().order(["cost", "speed", "stability", "efficiency"]);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#64748b")
                .stroke_width(1.35)
                .opacity(0.42),
        )
        .mark(
            numeric_parallel_symbol()
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
    let coord =
        numeric_parallel_coord().active_axis_display_params("drag_dimension", "drag_display_x");

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .param(Param::new("drag_dimension", ScalarValue::Utf8(None)))
        .param(Param::new("drag_display_x", ScalarValue::Float64(None)))
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            numeric_parallel_symbol()
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
    let coord =
        numeric_parallel_coord().active_axis_display_params("drag_dimension", "drag_display_x");

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

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .param(Param::new("drag_dimension", ScalarValue::Utf8(None)))
        .param(Param::new("drag_display_x", ScalarValue::Float64(None)))
        .data(numeric_parallel_data(&ctx))
        .mark(stability_overlay)
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.25)
                .opacity(0.38),
        )
        .mark(
            numeric_parallel_symbol()
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

#[tokio::test]
async fn parallel_selected_line_highlight() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#cbd5e1")
                .stroke_width(1.1)
                .opacity(0.48)
                .zindex(1),
        )
        .mark(
            numeric_parallel_line()
                .transform_no_output(Filter::new(col("id").eq(lit("a2"))), |mark| mark)
                .stroke("#2563eb")
                .stroke_width(3.0)
                .opacity(0.95)
                .zindex(20),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile selected-line parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_selected_line_highlight",
    )
    .await;
}

#[tokio::test]
async fn parallel_selected_axis_header() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", |d| d.axis(|a| a.title("Efficiency")))
        .dimension_with("stability", |d| {
            d.axis(|a| a.title("Stability").title_color("#2563eb"))
        })
        .dimension_with("cost", |d| d.axis(|a| a.title("Cost")));

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.25)
                .opacity(0.38),
        )
        .mark(
            numeric_parallel_symbol()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(70.0)
                .opacity(0.9),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile selected-axis-header parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_selected_axis_header",
    )
    .await;
}

#[tokio::test]
async fn parallel_reorder_drag_preview_state() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", |d| d.axis(|a| a.title("Efficiency")))
        .dimension_with("stability", |d| {
            d.axis(|a| a.title("Stability").title_color("#2563eb"))
        })
        .dimension_with("cost", |d| d.axis(|a| a.title("Cost")))
        .active_axis_display_params("drag_dimension", "drag_display_x");

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .param(Param::new("drag_dimension", ScalarValue::Utf8(None)))
        .param(Param::new("drag_display_x", ScalarValue::Float64(None)))
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            numeric_parallel_symbol()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(82.0)
                .opacity(0.92),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile reorder-preview parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        Some(parallel_drag_params("stability", 245.0)),
        "parallel",
        "parallel_reorder_drag_preview_state",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_brush_intersection_selected_lines() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let speed_brush = ParallelAxisOverlay::new(
        "speed",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .data(axis_interval_data(&ctx, 47.0, 59.0))
                .exclude_from_scale_domains()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(col("value_min"))
                .y2(col("value_max"))
                .fill("rgba(37, 99, 235, 0.14)")
                .stroke("#2563eb")
                .stroke_width(1.3),
        ),
    )
    .width_px(16.0)
    .zindex(10);
    let stability_brush = ParallelAxisOverlay::new(
        "stability",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .data(axis_interval_data(&ctx, 77.0, 81.0))
                .exclude_from_scale_domains()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(col("value_min"))
                .y2(col("value_max"))
                .fill("rgba(37, 99, 235, 0.14)")
                .stroke("#2563eb")
                .stroke_width(1.3),
        ),
    )
    .width_px(16.0)
    .zindex(10);

    let selected = col("speed")
        .gt_eq(lit(47.0))
        .and(col("speed").lt_eq(lit(59.0)))
        .and(col("stability").gt_eq(lit(77.0)))
        .and(col("stability").lt_eq(lit(81.0)));

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(speed_brush)
        .mark(stability_brush)
        .mark(
            numeric_parallel_line()
                .stroke("#cbd5e1")
                .stroke_width(1.05)
                .opacity(0.42)
                .zindex(1),
        )
        .mark(
            numeric_parallel_line()
                .transform_no_output(Filter::new(selected), |mark| mark)
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(2.8)
                .opacity(0.95)
                .zindex(20),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile axis-brush intersection parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_brush_intersection_selected_lines",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_rect_basic() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let stability_overlay = ParallelAxisOverlay::new(
        "stability",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(73.0))
                .y2(lit(81.0))
                .fill("rgba(37, 99, 235, 0.18)")
                .stroke("#2563eb")
                .stroke_width(1.5),
        ),
    )
    .width_px(44.0);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(numeric_parallel_line().visible(false))
        .mark(stability_overlay);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile basic overlay rect parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_rect_basic",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_rect_with_lines() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let speed_overlay = ParallelAxisOverlay::new(
        "speed",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(47.0))
                .y2(lit(57.0))
                .fill("rgba(14, 165, 233, 0.14)")
                .stroke("#0284c7")
                .stroke_width(1.3),
        ),
    )
    .width_px(42.0)
    .zindex(-2);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(speed_overlay)
        .mark(
            numeric_parallel_line()
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(1.55)
                .opacity(0.56)
                .zindex(2),
        )
        .mark(
            numeric_parallel_symbol()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(72.0)
                .opacity(0.93)
                .zindex(3),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overlay rect with lines parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_rect_with_lines",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_symbols() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let efficiency_symbols = ParallelAxisOverlay::new(
        "efficiency",
        Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(lit(0.5))
                .y(col("efficiency"))
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(72.0),
        ),
    )
    .width_px(46.0)
    .zindex(8);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.2)
                .opacity(0.34),
        )
        .mark(efficiency_symbols);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overlay symbol parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_symbols",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_reversed_scale() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let speed_overlay = ParallelAxisOverlay::new(
        "speed",
        Plot::<Cartesian>::new().mark(
            Rect::new()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(47.0))
                .y2(lit(57.0))
                .fill("rgba(14, 165, 233, 0.14)")
                .stroke("#0284c7")
                .stroke_width(1.3),
        ),
    )
    .width_px(42.0)
    .zindex(-2);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(speed_overlay)
        .mark(
            numeric_parallel_line()
                .dimension_with("speed", col("speed"), |d| {
                    d.scale_with::<Linear>(|scale| scale.domain_interval(lit(64.0), lit(42.0)))
                })
                .stroke_with(col("group"), |stroke| stroke.no_legend())
                .stroke_width(1.55)
                .opacity(0.56)
                .zindex(2),
        )
        .mark(
            numeric_parallel_symbol()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.75)
                .size(72.0)
                .opacity(0.93)
                .zindex(3),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile reversed-scale overlay parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_reversed_scale",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_boxplot() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let speed_box = ParallelAxisOverlay::new(
        "speed",
        Plot::<Cartesian>::new().mark(
            BoxPlot::new()
                .vertical()
                .x_with(col("all"), |x| x.scale_with::<Band>(|scale| scale))
                .y(col("speed"))
                .box_body(|body| {
                    body.fill("rgba(59, 130, 246, 0.28)")
                        .stroke("#1d4ed8")
                        .stroke_width(1.2)
                })
                .median(|rule| rule.stroke("#0f172a").stroke_width(1.4))
                .whiskers(|rule| rule.stroke("#1d4ed8").stroke_width(1.0))
                .caps(|rule| rule.stroke("#1d4ed8").stroke_width(1.0))
                .outliers(|outliers| {
                    outliers
                        .fill("#f97316")
                        .stroke("#7c2d12")
                        .stroke_width(0.8)
                        .size(44.0)
                }),
        ),
    )
    .width_px(58.0)
    .zindex(5);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.15)
                .opacity(0.30),
        )
        .mark(speed_box);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overlay boxplot parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_boxplot",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_violin() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let stability_violin = ParallelAxisOverlay::new(
        "stability",
        Plot::<Cartesian>::new().mark(
            Violin::new()
                .x_with(col("all"), |x| x.scale_with::<Band>(|scale| scale))
                .y(col("stability"))
                .bandwidth(1.45)
                .steps(96)
                .density_extent(69.0, 83.0)
                .width(0.78)
                .fill("rgba(99, 102, 241, 0.34)")
                .stroke("#4338ca")
                .stroke_width(1.1)
                .opacity(0.78),
        ),
    )
    .width_px(64.0)
    .zindex(4);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.1)
                .opacity(0.26),
        )
        .mark(stability_violin);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile overlay violin parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_violin",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_violins_all_axes() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let violin_overlay = |dimension: &'static str,
                          bandwidth: f64,
                          start: f64,
                          stop: f64,
                          fill: &'static str,
                          stroke: &'static str| {
        ParallelAxisOverlay::new(
            dimension,
            Plot::<Cartesian>::new().mark(
                Violin::new()
                    .x_with(col("all"), |x| x.scale_with::<Band>(|scale| scale))
                    .y(col(dimension))
                    .bandwidth(bandwidth)
                    .steps(80)
                    .density_extent(start, stop)
                    .width(0.72)
                    .fill(fill)
                    .stroke(stroke)
                    .stroke_width(0.95)
                    .opacity(0.64),
            ),
        )
        .width_px(50.0)
        .zindex(4)
    };

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#64748b")
                .stroke_width(1.05)
                .opacity(0.20),
        )
        .mark(violin_overlay(
            "speed",
            2.2,
            42.0,
            64.0,
            "rgba(59, 130, 246, 0.24)",
            "#1d4ed8",
        ))
        .mark(violin_overlay(
            "efficiency",
            0.018,
            0.56,
            0.76,
            "rgba(16, 185, 129, 0.24)",
            "#047857",
        ))
        .mark(violin_overlay(
            "stability",
            1.45,
            69.0,
            83.0,
            "rgba(99, 102, 241, 0.24)",
            "#4338ca",
        ))
        .mark(violin_overlay(
            "cost",
            4.5,
            95.0,
            145.0,
            "rgba(249, 115, 22, 0.22)",
            "#c2410c",
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile all-axis overlay violins parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_violins_all_axes",
    )
    .await;
}

#[tokio::test]
async fn parallel_axis_overlay_grouped_boxplots() {
    let ctx = SessionContext::new();
    let coord = numeric_parallel_coord();

    let cost_boxes = ParallelAxisOverlay::new(
        "cost",
        Plot::<Cartesian>::new().mark(
            BoxPlot::new()
                .vertical()
                .x_with(col("group"), |x| {
                    x.scale_with::<Band>(|scale| scale.padding_inner(0.16))
                })
                .y(col("cost"))
                .fill_with(col("group"), |fill| fill.no_legend())
                .box_body(|body| body.stroke("#111827").stroke_width(0.9).opacity(0.78))
                .median(|rule| rule.stroke("#111827").stroke_width(1.2))
                .whiskers(|rule| rule.stroke("#374151").stroke_width(0.9))
                .caps(|rule| rule.stroke("#374151").stroke_width(0.9))
                .outliers(|outliers| {
                    outliers
                        .fill("#f97316")
                        .stroke("#7c2d12")
                        .stroke_width(0.75)
                        .size(38.0)
                }),
        ),
    )
    .width_px(78.0)
    .zindex(5);

    let plot = Chart::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            numeric_parallel_line()
                .stroke("#94a3b8")
                .stroke_width(1.1)
                .opacity(0.28),
        )
        .mark(cost_boxes);

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grouped boxplot overlay parallel plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_axis_overlay_grouped_boxplots",
    )
    .await;
}
