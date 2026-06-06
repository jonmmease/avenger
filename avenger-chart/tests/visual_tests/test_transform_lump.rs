use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::functions_window::expr_fn::{ntile, percent_rank, rank};
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn lump_category_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let categories = vec![
        "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta", "Iota",
    ];
    let values = vec![90.0, 75.0, 60.0, 45.0, 25.0, 18.0, 14.0, 9.0, 6.0];
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(categories)) as _,
            Arc::new(Float64Array::from(values.clone())) as _,
            Arc::new(Float64Array::from(
                (0..values.len()).map(|idx| idx as f64).collect::<Vec<_>>(),
            )) as _,
            Arc::new(Float64Array::from(
                values.iter().map(|value| value / 15.0).collect::<Vec<_>>(),
            )) as _,
        ],
    )
    .expect("lump category batch");
    ctx.read_batch(batch).expect("lump category dataframe")
}

fn lump_tie_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta",
            ])) as _,
            Arc::new(Float64Array::from(vec![90.0, 75.0, 60.0, 60.0, 20.0, 8.0])) as _,
        ],
    )
    .expect("lump tie batch");
    ctx.read_batch(batch).expect("lump tie dataframe")
}

fn lump_facet_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("facet", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "West", "West", "West", "East", "East", "East",
            ])) as _,
            Arc::new(StringArray::from(vec![
                "Alpha", "Beta", "Gamma", "Alpha", "Beta", "Gamma",
            ])) as _,
            Arc::new(Float64Array::from(vec![90.0, 5.0, 3.0, 4.0, 80.0, 6.0])) as _,
        ],
    )
    .expect("lump facet batch");
    ctx.read_batch(batch).expect("lump facet dataframe")
}

fn lumped_bar_mark(lump: Lump, fill: &str) -> Rect<Cartesian> {
    Rect::new().transform(lump, move |mark, lump| {
        mark.x_with(lump.value(), |c| c.axis(|a| a.title("Category")))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2_with(sum(col("value")), |c| c.axis(|a| a.title("Total value")))
            .fill(fill)
            .stroke("#ffffff")
            .stroke_width(1.0)
    })
}

fn lumped_bar_plot(df: DataFrame, title: &str, lump: Lump, fill: &str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .title(title)
        .canvas_size(680.0, 420.0)
        .data(df)
        .mark(lumped_bar_mark(lump, fill))
}

fn faceted_lump_plot(
    df: DataFrame,
    scope: CoordinationScope,
    title: &str,
    fill: &str,
) -> Plot<FacetColumn> {
    let leaf = Plot::<Cartesian>::new().mark(
        Rect::new().transform_with_scope(
            scope,
            Lump::top_n(col("category"), 1)
                .order_by(sum(col("value")))
                .name("category_lump"),
            move |mark, lump| {
                mark.x_with(lump.value(), |c| c.axis(|a| a.title("Lumped category")))
                    .x2_with(col(":x"), |c| c.band(1.0))
                    .y(lit(0.0))
                    .y2(sum(col("value")))
                    .fill(fill)
                    .stroke("#ffffff")
                    .stroke_width(1.0)
            },
        ),
    );

    Plot::<FacetColumn>::new()
        .data(df)
        .title(title)
        .mark(Subplot::new(leaf).col_with(col("facet"), |c| c.guide(|g| g.title("Facet"))))
}

#[tokio::test]
async fn lump_bar_top_n_other() {
    let ctx = SessionContext::new();
    let plot = lumped_bar_plot(
        lump_category_data(&ctx),
        "Top 4 Categories Plus Other",
        Lump::top_n(col("category"), 4)
            .order_by(sum(col("value")))
            .name("category_lump"),
        "#2f80ed",
    );

    let compiled = plot.compile(&ctx).await.expect("compile lump bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_bar_top_n_other",
    )
    .await;
}

#[tokio::test]
async fn lump_color_top_n_other() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Top Categories In Fill Legend")
        .canvas_size(700.0, 420.0)
        .data(lump_category_data(&ctx))
        .mark(
            Symbol::new().transform(
                Lump::top_n(col("category"), 4)
                    .order_by(sum(col("value")))
                    .name("category_lump"),
                |mark, lump| {
                    mark.x_with(col("x"), |c| {
                        c.axis(|a| a.title("x"))
                            .scale_with::<Linear>(|s| s.domain_interval(lit(-1.0), lit(9.0)))
                    })
                    .y_with(col("y"), |c| {
                        c.axis(|a| a.title("value / 15"))
                            .scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(7.0)))
                    })
                    .fill_with(lump.value(), |c| c.legend(|l| l.title("Category")))
                    .size(120.0)
                    .stroke("#ffffff")
                    .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile lump color");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_color_top_n_other",
    )
    .await;
}

#[tokio::test]
async fn lump_bar_drop_other() {
    let ctx = SessionContext::new();
    let plot = lumped_bar_plot(
        lump_category_data(&ctx),
        "Top 4 Categories, Other Dropped",
        Lump::top_n(col("category"), 4)
            .order_by(sum(col("value")))
            .drop_other()
            .name("category_lump"),
        "#1f9d6a",
    );

    let compiled = plot.compile(&ctx).await.expect("compile drop-other bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_bar_drop_other",
    )
    .await;
}

#[tokio::test]
async fn lump_color_drop_other() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Dropped Other Categories")
        .canvas_size(700.0, 420.0)
        .data(lump_category_data(&ctx))
        .mark(
            Symbol::new().transform(
                Lump::top_n(col("category"), 4)
                    .order_by(sum(col("value")))
                    .drop_other()
                    .name("category_lump"),
                |mark, lump| {
                    mark.x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.domain_interval(lit(-1.0), lit(9.0)))
                    })
                    .y_with(col("y"), |c| {
                        c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(7.0)))
                    })
                    .fill_with(lump.value(), |c| c.legend(|l| l.title("Category")))
                    .size(120.0)
                    .stroke("#ffffff")
                    .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile drop-other color");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_color_drop_other",
    )
    .await;
}

#[tokio::test]
async fn lump_rank_includes_ties() {
    let ctx = SessionContext::new();
    let plot = lumped_bar_plot(
        lump_tie_data(&ctx),
        "Rank Keeps Ties At Cutoff",
        Lump::top_n(col("category"), 3)
            .order_by(sum(col("value")))
            .window(rank())
            .keep(lump::window_value().lt_eq(lit(3)))
            .name("category_lump"),
        "#8b5cf6",
    );

    let compiled = plot.compile(&ctx).await.expect("compile rank lump");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_rank_includes_ties",
    )
    .await;
}

#[tokio::test]
async fn lump_percent_rank() {
    let ctx = SessionContext::new();
    let plot = lumped_bar_plot(
        lump_category_data(&ctx),
        "Percent Rank Predicate",
        Lump::top_n(col("category"), 5)
            .order_by(sum(col("value")))
            .window(percent_rank())
            .keep(lump::window_value().lt_eq(lit(0.5)))
            .name("category_lump"),
        "#d946ef",
    );

    let compiled = plot.compile(&ctx).await.expect("compile percent rank lump");
    assert_visual_match_default(&compiled, &ctx, None, "transform_lump", "lump_percent_rank").await;
}

#[tokio::test]
async fn lump_ntile() {
    let ctx = SessionContext::new();
    let plot = lumped_bar_plot(
        lump_category_data(&ctx),
        "Ntile Predicate",
        Lump::top_n(col("category"), 9)
            .order_by(sum(col("value")))
            .window(ntile(lit(4)))
            .keep(lump::window_value().lt_eq(lit(2)))
            .name("category_lump"),
        "#f97316",
    );

    let compiled = plot.compile(&ctx).await.expect("compile ntile lump");
    assert_visual_match_default(&compiled, &ctx, None, "transform_lump", "lump_ntile").await;
}

#[tokio::test]
async fn lump_faceted_free_vs_shared() {
    let ctx = SessionContext::new();
    let df = lump_facet_data(&ctx);
    let plot = Plot::<HConcat>::new()
        .canvas_size(980.0, 380.0)
        .title("Free vs Shared Lump Scope")
        .mark(
            Subplot::new(faceted_lump_plot(
                df.clone(),
                CoordinationScope::Free,
                "Free: local top category",
                "#2563eb",
            ))
            .key("free"),
        )
        .mark(
            Subplot::new(faceted_lump_plot(
                df,
                CoordinationScope::Shared,
                "Shared: global top category",
                "#ea580c",
            ))
            .key("shared"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile faceted lump");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_lump",
        "lump_faceted_free_vs_shared",
    )
    .await;
}
