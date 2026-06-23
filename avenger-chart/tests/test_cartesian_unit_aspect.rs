use avenger_chart::prelude::*;
use avenger_chart::render::SvgRenderer;
use datafusion::prelude::{SessionContext, col};

async fn xy_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql("SELECT 0.0 AS x, 0.0 AS y UNION ALL SELECT 1.0 AS x, 1.0 AS y")
        .await
        .unwrap()
}

async fn compile_error(
    plot: Plot<Cartesian>,
    ctx: &SessionContext,
    label: &str,
) -> AvengerChartError {
    match plot.compile(ctx).await {
        Ok(_) => panic!("expected {label} to fail"),
        Err(err) => err,
    }
}

#[tokio::test]
async fn cartesian_unit_aspect_compiles_and_serializes() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .data(df)
        .mark(Symbol::new().x(col("x")).y(col("y")));

    let compiled = plot.compile(&ctx).await.expect("compile unit-aspect plot");
    let json = serde_json::to_string(&compiled).expect("serialize compiled plot");

    assert!(json.contains("unit_aspect"));
    assert!(json.contains("\"ratio\":1.0"));
}

#[tokio::test]
async fn cartesian_unit_aspect_resolves_named_position_scales() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().equal_units())
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| x.with_scale_name("x_metric"))
                .y_with(col("y"), |y| y.with_scale_name("y_metric")),
        );

    plot.compile(&ctx)
        .await
        .expect("named x/y scales should resolve");
}

#[tokio::test]
async fn cartesian_unit_aspect_rejects_invalid_ratio_at_compile() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(0.0))
        .data(df)
        .mark(Symbol::new().x(col("x")).y(col("y")));

    let err = compile_error(plot, &ctx, "invalid ratio").await;
    assert!(err.to_string().contains("positive and finite"));
}

#[tokio::test]
async fn cartesian_unit_aspect_rejects_multiple_x_scales() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .data(df)
        .mark(Symbol::new().x(col("x")).y(col("y")))
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| x.with_scale_name("x_alt"))
                .y(col("y")),
        );

    let err = compile_error(plot, &ctx, "ambiguous x scale").await;
    assert!(err.to_string().contains("resolves to multiple scales"));
}

#[tokio::test]
async fn cartesian_unit_aspect_rejects_same_scale_for_x_and_y() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| x.with_scale_name("position"))
                .y_with(col("y"), |y| y.with_scale_name("position")),
        );

    let err = compile_error(plot, &ctx, "same x/y scale").await;
    assert!(err.to_string().contains("both resolve to scale 'position'"));
}

#[tokio::test]
async fn authored_concat_rejects_unit_aspect_shared_child_domain() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let child = || {
        Plot::with_coord(Cartesian::new().unit_aspect(1.0))
            .data(df.clone())
            .mark(
                Symbol::new()
                    .x_with(col("x"), |x| x.with_domain_scope(CoordinationScope::Shared))
                    .y_with(col("y"), |y| y.with_domain_scope(CoordinationScope::Shared)),
            )
    };
    let plot = Plot::<HConcat>::new()
        .plot_size(300.0, 120.0)
        .mark(Subplot::new(child()).key("left"))
        .mark(Subplot::new(child()).key("right"));
    let compiled = plot.compile(&ctx).await.expect("compile concat");

    let err = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect_err("authored concat sharing should be rejected");

    assert!(err.to_string().contains("HConcat does not support"));
    assert!(err.to_string().contains("unit_aspect child scale"));
}
