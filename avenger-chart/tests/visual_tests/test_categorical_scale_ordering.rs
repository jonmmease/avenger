use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    functions_aggregate::min_max::max,
    prelude::*,
};

fn ordered_bar_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["A", "B", "C", "D"])),
            Arc::new(Float64Array::from(vec![70.0, 20.0, 90.0, 50.0])),
        ],
    )
    .expect("ordered bar batch");
    ctx.read_batch(batch).expect("ordered bar dataframe")
}

fn ordered_facet_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["G1", "G1", "G1", "G2", "G2", "G2"])),
            Arc::new(StringArray::from(vec!["A", "B", "C", "A", "B", "C"])),
            Arc::new(Float64Array::from(vec![20.0, 90.0, 60.0, 95.0, 10.0, 40.0])),
        ],
    )
    .expect("ordered facet batch");
    ctx.read_batch(batch).expect("ordered facet dataframe")
}

fn ordered_rect_mark(sharing: Option<ScaleSharing>) -> Rect<Cartesian> {
    Rect::new()
        .x_with(col("category"), move |c| {
            let c = c.scale_with::<Band>(|s| s.order_by(max(col("value"))).order_desc());
            let c = if let Some(sharing) = sharing.clone() {
                c.with_scale_sharing(sharing)
            } else {
                c
            };
            c.axis(|a| a.title("Category"))
        })
        .x2_with(col(":x"), |c| c.band(1.0))
        .y_with(lit(0.0), |c| {
            c.scale(|s| s.domain((0.0, 100.0)))
                .axis(|a| a.title("Value"))
        })
        .y2(col("value"))
        .fill("#4c78a8")
}

fn color_ordered_rect_mark(sharing: Option<ScaleSharing>) -> Rect<Cartesian> {
    Rect::new()
        .x_with(col("category"), |c| {
            c.scale_with::<Band>(|s| s).axis(|a| a.title("Category"))
        })
        .x2_with(col(":x"), |c| c.band(1.0))
        .y_with(lit(0.0), |c| {
            c.scale(|s| s.domain((0.0, 100.0)))
                .axis(|a| a.title("Value"))
        })
        .y2(col("value"))
        .fill_with(col("category"), move |c| {
            let c = c.scale_with::<Ordinal>(|s| s.order_by(max(col("value"))).order_desc());
            let c = if let Some(sharing) = sharing.clone() {
                c.with_scale_sharing(sharing)
            } else {
                c
            };
            c.legend(|l| l.title("Category").position(LegendPosition::Right))
        })
}

#[tokio::test]
async fn categorical_band_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .data(ordered_bar_data(&ctx))
        .canvas_size(620, 360)
        .mark(ordered_rect_mark(None));

    let compiled = plot.compile(&ctx).await.expect("compile ordered bar plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "categorical_band_order_by_max_desc",
    )
    .await;
}

#[tokio::test]
async fn categorical_ordinal_fill_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .data(ordered_bar_data(&ctx))
        .canvas_size(760, 360)
        .mark(color_ordered_rect_mark(None));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile color ordered bar plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "categorical_ordinal_fill_order_by_max_desc",
    )
    .await;
}

#[tokio::test]
async fn facet_free_categorical_band_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .data(ordered_facet_data(&ctx))
        .canvas_size(820, 360)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(ordered_rect_mark(Some(ScaleSharing::Free))),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free ordered facet");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "facet_free_categorical_band_order_by_max_desc",
    )
    .await;
}

#[tokio::test]
async fn facet_free_categorical_ordinal_fill_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .data(ordered_facet_data(&ctx))
        .canvas_size(1080, 390)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(color_ordered_rect_mark(Some(ScaleSharing::Free))),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free color ordered facet");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "facet_free_categorical_ordinal_fill_order_by_max_desc",
    )
    .await;
}

#[tokio::test]
async fn facet_shared_categorical_band_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .data(ordered_facet_data(&ctx))
        .canvas_size(820, 360)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(ordered_rect_mark(Some(ScaleSharing::Shared))),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared ordered facet");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "facet_shared_categorical_band_order_by_max_desc",
    )
    .await;
}

#[tokio::test]
async fn facet_shared_categorical_ordinal_fill_order_by_max_desc() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new()
        .data(ordered_facet_data(&ctx))
        .canvas_size(1080, 390)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(color_ordered_rect_mark(Some(ScaleSharing::Shared))),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared color ordered facet");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "categorical_scale_ordering",
        "facet_shared_categorical_ordinal_fill_order_by_max_desc",
    )
    .await;
}
