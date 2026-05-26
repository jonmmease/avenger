use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::CartesianGuide;
use avenger_chart::prelude::*;
use avenger_chart_scales::{Linear, Ordinal};
use datafusion::functions_aggregate::min_max::max;
use datafusion::prelude::*;

const BASELINE_CATEGORY: &str = "facet_wrap";

async fn facet_wrap_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            ('Alpha',  0.2, 0.1, 'low',  11.0),
            ('Alpha',  0.8, 0.3, 'mid',  11.0),
            ('Alpha',  1.4, 0.5, 'high', 11.0),
            ('Alpha',  2.0, 0.7, 'low',  11.0),
            ('Bravo', 10.2, 0.2, 'mid',  26.0),
            ('Bravo', 10.8, 0.4, 'high', 26.0),
            ('Bravo', 11.4, 0.6, 'low',  26.0),
            ('Bravo', 12.0, 0.8, 'mid',  26.0),
            ('Cedar', 20.2, 0.3, 'high', 37.0),
            ('Cedar', 20.8, 0.5, 'low',  37.0),
            ('Cedar', 21.4, 0.7, 'mid',  37.0),
            ('Cedar', 22.0, 0.9, 'high', 37.0),
            ('Delta', 30.2, 0.1, 'low',  44.0),
            ('Delta', 30.8, 0.4, 'mid',  44.0),
            ('Delta', 31.4, 0.7, 'high', 44.0),
            ('Delta', 32.0, 1.0, 'low',  44.0),
            ('Ember', 40.2, 0.2, 'mid',  59.0),
            ('Ember', 40.8, 0.5, 'high', 59.0),
            ('Ember', 41.4, 0.8, 'low',  59.0),
            ('Ember', 42.0, 1.1, 'mid',  59.0),
            ('Fjord', 50.2, 0.3, 'high', 63.0),
            ('Fjord', 50.8, 0.6, 'low',  63.0),
            ('Fjord', 51.4, 0.9, 'mid',  63.0),
            ('Fjord', 52.0, 1.2, 'high', 63.0),
            ('Grove', 60.2, 0.4, 'low',  79.0),
            ('Grove', 60.8, 0.7, 'mid',  79.0),
            ('Grove', 61.4, 1.0, 'high', 79.0),
            ('Grove', 62.0, 1.3, 'low',  79.0)
        ) AS t(facet, x, y, group_name, rank_score)",
    )
    .await
    .expect("facet wrap data")
}

fn wrap_leaf_plot(x_sharing: u8, y_sharing: u8, fill_sharing: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#fffefa"))
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.with_scale_sharing(ScaleSharing::Level(x_sharing))
                        .scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("x"))
                })
                .y_with(col("y"), move |c| {
                    c.with_scale_sharing(ScaleSharing::Level(y_sharing))
                        .scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("y"))
                })
                .fill_with(col("group_name"), move |c| {
                    c.with_scale_sharing(ScaleSharing::Level(fill_sharing))
                        .scale_with::<Ordinal>(|s| {
                            s.range_discrete(vec!["#5778a4", "#e49444", "#d1615d"])
                        })
                        .legend(|l| l.title("Group").position(LegendPosition::Right))
                })
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(70.0),
        )
}

fn facet_wrap_plot(
    df: DataFrame,
    columns: Option<usize>,
    order_desc: bool,
    x_sharing: u8,
    y_sharing: u8,
    fill_sharing: u8,
) -> Plot<FacetWrap> {
    Plot::<FacetWrap>::new()
        .data(df)
        .canvas_size(980.0, 760.0)
        .mark(
            Subplot::new(wrap_leaf_plot(x_sharing, y_sharing, fill_sharing)).wrap_with(
                col("facet"),
                move |c| {
                    let c = if let Some(columns) = columns {
                        c.columns(columns)
                    } else {
                        c
                    };
                    let c = if order_desc {
                        c.order_by(max(col("rank_score"))).order_desc()
                    } else {
                        c
                    };
                    c.guide(|g| g.title("Facet"))
                },
            ),
        )
}

async fn assert_facet_wrap_baseline(
    name: &str,
    columns: Option<usize>,
    order_desc: bool,
    x_sharing: u8,
    y_sharing: u8,
    fill_sharing: u8,
) {
    let ctx = SessionContext::new();
    let plot = facet_wrap_plot(
        facet_wrap_data(&ctx).await,
        columns,
        order_desc,
        x_sharing,
        y_sharing,
        fill_sharing,
    );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
}

#[tokio::test]
async fn facet_wrap_auto_columns() {
    assert_facet_wrap_baseline("facet_wrap_auto_columns", None, false, 1, 1, 1).await;
}

#[tokio::test]
async fn facet_wrap_columns_4_order_by_max_desc() {
    assert_facet_wrap_baseline(
        "facet_wrap_columns_4_order_by_max_desc",
        Some(4),
        true,
        1,
        1,
        1,
    )
    .await;
}

#[tokio::test]
async fn facet_wrap_xy_level_0_free() {
    assert_facet_wrap_baseline("facet_wrap_xy_level_0_free", Some(4), false, 0, 0, 1).await;
}

#[tokio::test]
async fn facet_wrap_xy_level_1_shared() {
    assert_facet_wrap_baseline("facet_wrap_xy_level_1_shared", Some(4), false, 1, 1, 1).await;
}

#[tokio::test]
async fn facet_wrap_fill_level_0_local_legends() {
    assert_facet_wrap_baseline(
        "facet_wrap_fill_level_0_local_legends",
        Some(4),
        false,
        1,
        1,
        0,
    )
    .await;
}

#[tokio::test]
async fn facet_wrap_fill_level_1_hoisted_legend() {
    assert_facet_wrap_baseline(
        "facet_wrap_fill_level_1_hoisted_legend",
        Some(4),
        false,
        1,
        1,
        1,
    )
    .await;
}
