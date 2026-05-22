use super::helpers::{assert_visual_match_default, assert_visual_match_default_with_options};
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn positioned_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS x,
            column2 AS y,
            column3 AS kind
         FROM (VALUES
            (1.0, 1.0, 'cartesian'),
            (2.4, 2.0, 'cartesian'),
            (3.8, 1.4, 'cartesian')
         )",
    )
    .await
    .expect("create positioned subplot data")
}

async fn child_cartesian_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS u,
            column2 AS v
         FROM (VALUES
            (0.0, 0.0),
            (0.5, 0.8),
            (1.0, 0.2)
         )",
    )
    .await
    .expect("create child Cartesian data")
}

async fn child_polar_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS radius,
            column2 AS theta
         FROM (VALUES
            (0.35, 0.0),
            (0.65, 1.7),
            (0.95, 3.4)
         )",
    )
    .await
    .expect("create child polar data")
}

async fn cartesian_child(ctx: &SessionContext) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .data(child_cartesian_data(ctx).await)
        .mark(
            Line::<Cartesian>::new()
                .x_with(col("u"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                })
                .y_with(col("v"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                })
                .stroke("#4c78a8")
                .stroke_width(2.0),
        )
        .mark(
            Symbol::<Cartesian>::new()
                .x_with(col("u"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                })
                .y_with(col("v"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                })
                .fill("#f58518")
                .size(45.0),
        )
}

async fn polar_child(ctx: &SessionContext) -> Plot<Polar> {
    Plot::<Polar>::new().data(child_polar_data(ctx).await).mark(
        Symbol::new()
            .r_with(col("radius"), |c| {
                c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(1.0))))
            })
            .theta(col("theta"))
            .fill("#54a24b")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(55.0),
    )
}

#[tokio::test]
async fn cartesian_positioned_cartesian_subplots() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .plot_size(430.0, 300.0)
        .data(positioned_data(&ctx).await)
        .title("Cartesian-positioned subplots")
        .mark(
            Subplot::new(cartesian_child(&ctx).await)
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(4.1))))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(2.3))))
                })
                .plot_size(86.0, 64.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile Cartesian-positioned Cartesian subplots");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_subplot",
        "cartesian_positioned_cartesian_subplots",
    )
    .await;
}

#[tokio::test]
async fn cartesian_positioned_mixed_subplots() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .plot_size(430.0, 300.0)
        .data(positioned_data(&ctx).await)
        .title("Mixed positioned subplots")
        .mark(
            Subplot::new(cartesian_child(&ctx).await)
                .x_with(lit(1.2), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.8), lit(3.9))))
                })
                .y_with(lit(1.2), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.8), lit(2.1))))
                })
                .plot_size(82.0, 62.0),
        )
        .mark(
            Subplot::new(polar_child(&ctx).await)
                .x(lit(3.5))
                .y(lit(1.8))
                .plot_size(82.0, 82.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile mixed coordinate positioned subplots");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_subplot",
        "cartesian_positioned_mixed_subplots",
    )
    .await;
}

#[tokio::test]
async fn cartesian_positioned_components_debug() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .plot_size(430.0, 300.0)
        .data(positioned_data(&ctx).await)
        .title("Positioned subplot debug")
        .mark(
            Subplot::new(cartesian_child(&ctx).await)
                .key("mini")
                .label("Mini")
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(4.1))))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(2.3))))
                })
                .plot_size(86.0, 64.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile positioned subplot debug chart");
    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            layout_snapshot: LayoutSnapshot::Final,
            ..EvaluationOptions::default()
        },
        "cartesian_subplot",
        "cartesian_positioned_components_debug",
    )
    .await;
}
