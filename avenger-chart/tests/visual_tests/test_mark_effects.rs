use avenger_chart::prelude::*;
use avenger_chart_marks_statistical::BoxPlot;
use datafusion::{
    arrow::{
        array::{Float32Array, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit},
};
use std::sync::Arc;

use crate::mark_effects_support::FixedLabelPlacement;

use super::helpers::assert_visual_match_default;

fn compound_box_plot_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let groups = [
        "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Alpha", "Beta", "Beta", "Beta",
        "Beta", "Beta", "Beta", "Beta",
    ];
    let values = [
        4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 24.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 31.0,
    ];
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(groups.to_vec())),
            Arc::new(Float64Array::from(values.to_vec())),
        ],
    )
    .expect("compound mark effect visual data");
    ctx.read_batch(batch)
        .expect("compound mark effect visual dataframe")
}

fn compound_box_plot_faceted_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let mut facets = Vec::new();
    let mut groups = Vec::new();
    let mut values = Vec::new();
    for (facet, group, group_values) in [
        (
            "North",
            "Alpha",
            [4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 24.0].as_slice(),
        ),
        (
            "South",
            "Beta",
            [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 31.0].as_slice(),
        ),
    ] {
        for value in group_values {
            facets.push(facet);
            groups.push(group);
            values.push(*value);
        }
    }
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("facet", DataType::Utf8, false),
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(facets)),
            Arc::new(StringArray::from(groups)),
            Arc::new(Float64Array::from(values)),
        ],
    )
    .expect("faceted compound mark effect visual data");
    ctx.read_batch(batch)
        .expect("faceted compound mark effect visual dataframe")
}

#[tokio::test]
async fn adjust_symbol_expression_nudge() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![15.0, 35.0, 55.0, 75.0])),
            Arc::new(Float32Array::from(vec![20.0, 55.0, 35.0, 80.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(180.0)
            .fill("#2f6fed")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust(|point| {
                point
                    .x(point.channel("x") + lit(18.0))
                    .y(point.channel("y") - lit(12.0))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "adjust_symbol_expression_nudge",
    )
    .await;
}

#[tokio::test]
async fn adjust_symbol_transform_nudge() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![15.0, 35.0, 55.0, 75.0])),
            Arc::new(Float32Array::from(vec![20.0, 55.0, 35.0, 80.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(180.0)
            .fill("#2f6fed")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust_transform(Nudge::new(18.0, -12.0), |mark, nudge| {
                mark.x(nudge.x()).y(nudge.y())
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "adjust_symbol_transform_nudge",
    )
    .await;
}

#[tokio::test]
async fn adjust_symbol_jitter_seeded() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![46.0, 48.0, 50.0, 52.0, 54.0, 56.0])),
            Arc::new(Float32Array::from(vec![28.0, 40.0, 52.0, 64.0, 76.0, 88.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(140.0)
            .fill("#d64550")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust_transform(Jitter::x().width_px(18.0).seed(7), |mark, jitter| {
                mark.x(jitter.x())
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "adjust_symbol_jitter_seeded",
    )
    .await;
}

#[tokio::test]
async fn adjust_symbol_dodge_grouped() {
    let ctx = SessionContext::new();
    let batch = grouped_dodge_batch();
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x(col("category"))
            .y_with(col("value"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .fill(col("series"))
            .size(160.0)
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust_transform(Dodge::x().by("series").step_px(18.0), |mark, dodge| {
                mark.x(dodge.x())
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "adjust_symbol_dodge_grouped",
    )
    .await;
}

#[tokio::test]
async fn adjust_symbol_chained() {
    let ctx = SessionContext::new();
    let batch = grouped_dodge_batch();
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x(col("category"))
            .y_with(col("value"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .fill(col("series"))
            .size(160.0)
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust_transform(Dodge::x().by("series").step_px(18.0), |mark, dodge| {
                mark.x(dodge.x())
            })
            .adjust(|point| point.y(point.channel("y") - lit(10.0))),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "adjust_symbol_chained",
    )
    .await;
}

#[tokio::test]
async fn derive_symbol_halo() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("size", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![15.0, 35.0, 55.0, 75.0])),
            Arc::new(Float32Array::from(vec![20.0, 55.0, 35.0, 80.0])),
            Arc::new(Float32Array::from(vec![80.0, 120.0, 160.0, 220.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(col("size"))
            .fill("#2f6fed")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .derive(|point| {
                Symbol::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .size(point.channel("size") + lit(320.0))
                    .fill("#2f6fed")
                    .stroke("#2f6fed")
                    .opacity(0.18)
                    .zindex(-1)
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(&compiled, &ctx, None, "mark_effects", "derive_symbol_halo").await;
}

#[tokio::test]
async fn derive_rule_from_adjusted_symbol() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![15.0, 35.0, 55.0, 75.0])),
            Arc::new(Float32Array::from(vec![20.0, 55.0, 35.0, 80.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(140.0)
            .fill("#2f6fed")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .adjust(|point| point.x(point.channel("x") + lit(12.0)))
            .derive(|point| {
                Rule::new()
                    .x(point.channel("x"))
                    .x2(point.channel("x"))
                    .y(point.channel("y"))
                    .y2(point.channel("y") + lit(18.0))
                    .stroke("#334155")
                    .stroke_width(2.0)
                    .opacity(0.7)
                    .zindex(-1)
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "derive_rule_from_adjusted_symbol",
    )
    .await;
}

#[tokio::test]
async fn derive_rect_outline() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("x2", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("y2", DataType::Float32, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![12.0, 38.0, 64.0])),
            Arc::new(Float32Array::from(vec![28.0, 54.0, 82.0])),
            Arc::new(Float32Array::from(vec![30.0, 52.0, 24.0])),
            Arc::new(Float32Array::from(vec![78.0, 94.0, 66.0])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Rect::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .x2_with(col("x2"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y2_with(col("y2"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .fill("rgba(47, 111, 237, 0.55)")
            .stroke("transparent")
            .derive(|rect| {
                Rect::new()
                    .x(rect.bbox().left() - lit(3.0))
                    .x2(rect.bbox().right() + lit(3.0))
                    .y(rect.bbox().top() - lit(3.0))
                    .y2(rect.bbox().bottom() + lit(3.0))
                    .fill("transparent")
                    .stroke("#111827")
                    .stroke_width(2.0)
                    .zindex(1)
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(&compiled, &ctx, None, "mark_effects", "derive_rect_outline").await;
}

#[tokio::test]
async fn derive_text_fixed_label_overlap() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float32Array::from(vec![20.0, 50.0, 75.0])),
            Arc::new(Float32Array::from(vec![45.0, 45.0, 70.0])),
            Arc::new(StringArray::from(vec!["Hidden label", "Shown", "Also"])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let plot = Chart::<Cartesian>::new().plot_size(260.0, 180.0).mark(
        Symbol::new()
            .data(df)
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(96.0)
            .fill("#2563eb")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .derive(|point| {
                Text::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .text(point.data("label"))
                    .align("center")
                    .baseline("middle")
                    .font_size(11.0)
                    .color("#111827")
                    .adjust_transform(FixedLabelPlacement::new(44.0, 0.0), |text, placed| {
                        text.x(placed.x()).y(placed.y()).defined(placed.defined())
                    })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "derive_text_fixed_label_overlap",
    )
    .await;
}

#[tokio::test]
async fn derive_text_fixed_label_faceted() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("facet", DataType::Utf8, false),
            Field::new("x", DataType::Float32, false),
            Field::new("y", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["A", "B"])),
            Arc::new(Float32Array::from(vec![20.0, 40.0])),
            Arc::new(Float32Array::from(vec![55.0, 55.0])),
            Arc::new(StringArray::from(vec!["A visible", "B visible"])),
        ],
    )
    .expect("mark effect visual data");
    let df = ctx.read_batch(batch).expect("mark effect visual dataframe");

    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|scale| scale.domain((0.0, 100.0)).nice(false))
            })
            .size(72.0)
            .fill("#2563eb")
            .stroke("#ffffff")
            .stroke_width(2.0)
            .derive(|point| {
                Text::new()
                    .x(point.channel("x"))
                    .y(point.channel("y"))
                    .text(point.data("label"))
                    .align("center")
                    .baseline("middle")
                    .font_size(11.0)
                    .color("#111827")
                    .adjust_transform(FixedLabelPlacement::new(34.0, 0.0), |text, placed| {
                        text.x(placed.x()).y(placed.y()).defined(placed.defined())
                    })
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .data(df)
        .plot_size(220.0, 150.0)
        .mark(Subplot::new(child).column(col("facet")));

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "derive_text_fixed_label_faceted",
    )
    .await;
}

#[tokio::test]
async fn compound_box_plot_outlier_adjust_nudge() {
    let ctx = SessionContext::new();
    let df = compound_box_plot_data(&ctx);
    let plot = Chart::<Cartesian>::new().plot_size(340.0, 220.0).mark(
        BoxPlot::new()
            .data(df)
            .x_with(col("value"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 36.0)).nice(false))
            })
            .y(col("group"))
            .outliers(|outliers| {
                outliers
                    .fill("#dc2626")
                    .stroke("#ffffff")
                    .adjust(|point| point.x(point.channel("x") + lit(16.0)))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "compound_box_plot_outlier_adjust_nudge",
    )
    .await;
}

#[tokio::test]
async fn compound_box_plot_outlier_halo() {
    let ctx = SessionContext::new();
    let df = compound_box_plot_data(&ctx);
    let plot = Chart::<Cartesian>::new().plot_size(340.0, 220.0).mark(
        BoxPlot::new()
            .data(df)
            .x_with(col("value"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 36.0)).nice(false))
            })
            .y(col("group"))
            .outliers(|outliers| {
                outliers.fill("#2563eb").stroke("#ffffff").derive(|point| {
                    Symbol::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .size(point.channel("size") * lit(4.0))
                        .fill("#facc15")
                        .stroke("#f59e0b")
                        .stroke_width(1.5)
                        .opacity(0.45)
                        .zindex(4)
                })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "compound_box_plot_outlier_halo",
    )
    .await;
}

#[tokio::test]
async fn compound_box_plot_outlier_fixed_label() {
    let ctx = SessionContext::new();
    let df = compound_box_plot_data(&ctx);
    let plot = Chart::<Cartesian>::new().plot_size(340.0, 220.0).mark(
        BoxPlot::new()
            .data(df)
            .x_with(col("value"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 36.0)).nice(false))
            })
            .y(col("group"))
            .outliers(|outliers| {
                outliers.fill("#2563eb").stroke("#ffffff").derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text(point.data("group"))
                        .align("left")
                        .baseline("middle")
                        .font_size(11.0)
                        .color("#111827")
                        .adjust_transform(FixedLabelPlacement::new(18.0, 0.0), |text, placed| {
                            text.x(placed.x()).y(placed.y()).defined(placed.defined())
                        })
                })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "compound_box_plot_outlier_fixed_label",
    )
    .await;
}

#[tokio::test]
async fn compound_box_plot_outlier_fixed_label_faceted() {
    let ctx = SessionContext::new();
    let df = compound_box_plot_faceted_data(&ctx);
    let child = Plot::<Cartesian>::new().mark(
        BoxPlot::new()
            .x_with(col("value"), |x| {
                x.scale_with::<Linear>(|scale| scale.domain((0.0, 44.0)).nice(false))
            })
            .y(col("group"))
            .outliers(|outliers| {
                outliers.fill("#2563eb").stroke("#ffffff").derive(|point| {
                    Text::new()
                        .x(point.channel("x"))
                        .y(point.channel("y"))
                        .text(point.data("facet"))
                        .align("left")
                        .baseline("middle")
                        .font_size(11.0)
                        .color("#111827")
                        .adjust_transform(FixedLabelPlacement::new(18.0, 0.0), |text, placed| {
                            text.x(placed.x()).y(placed.y()).defined(placed.defined())
                        })
                })
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(560.0, 260.0)
        .mark(Subplot::new(child).column(col("facet")));

    let compiled = plot.compile(&ctx).await.expect("compile mark effect plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_effects",
        "compound_box_plot_outlier_fixed_label_faceted",
    )
    .await;
}

fn grouped_dodge_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("series", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["A", "A", "B", "B"])),
            Arc::new(StringArray::from(vec!["North", "South", "North", "South"])),
            Arc::new(Float32Array::from(vec![35.0, 52.0, 48.0, 72.0])),
        ],
    )
    .expect("grouped dodge visual data")
}
