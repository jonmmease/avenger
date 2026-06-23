use avenger_chart::prelude::*;
use avenger_chart::render::SvgRenderer;
use datafusion::common::ScalarValue;
use datafusion::prelude::{SessionContext, col};
use indexmap::IndexMap;

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

async fn explicit_domain_plot(ctx: &SessionContext, ratio: f64) -> (Plot<Cartesian>, DataFrame) {
    let df = xy_data(ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(ratio))
        .data(df.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| {
                    x.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                })
                .y_with(col("y"), |y| {
                    y.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                }),
        );
    (plot, df)
}

async fn unit_aspect_domains(
    ctx: &SessionContext,
    ratio: f64,
    width: f32,
    height: f32,
) -> ((f32, f32), (f32, f32)) {
    let (plot, df) = explicit_domain_plot(ctx, ratio).await;
    let compiled = plot.compile(ctx).await.expect("compile unit-aspect plot");
    let scales = compiled
        .build_scales_for_dataframe(
            &df,
            width,
            height,
            ctx,
            &IndexMap::<String, ScalarValue>::new(),
        )
        .await
        .expect("build unit-aspect scales");
    let x_domain = scales
        .get("x")
        .expect("x scale")
        .configured()
        .numeric_interval_domain()
        .expect("x numeric domain");
    let y_domain = scales
        .get("y")
        .expect("y scale")
        .configured()
        .numeric_interval_domain()
        .expect("y numeric domain");
    (x_domain, y_domain)
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
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

#[tokio::test]
async fn cartesian_unit_aspect_expands_x_for_wide_plot_area() {
    let ctx = SessionContext::new();
    let (x_domain, y_domain) = unit_aspect_domains(&ctx, 1.0, 200.0, 100.0).await;

    assert_close(x_domain.0, -5.0);
    assert_close(x_domain.1, 15.0);
    assert_close(y_domain.0, 0.0);
    assert_close(y_domain.1, 10.0);
}

#[tokio::test]
async fn cartesian_unit_aspect_expands_y_for_tall_plot_area() {
    let ctx = SessionContext::new();
    let (x_domain, y_domain) = unit_aspect_domains(&ctx, 1.0, 100.0, 200.0).await;

    assert_close(x_domain.0, 0.0);
    assert_close(x_domain.1, 10.0);
    assert_close(y_domain.0, -5.0);
    assert_close(y_domain.1, 15.0);
}

#[tokio::test]
async fn cartesian_unit_aspect_supports_non_equal_ratio() {
    let ctx = SessionContext::new();
    let (x_domain, y_domain) = unit_aspect_domains(&ctx, 2.0, 200.0, 100.0).await;

    assert_close(x_domain.0, -15.0);
    assert_close(x_domain.1, 25.0);
    assert_close(y_domain.0, 0.0);
    assert_close(y_domain.1, 10.0);
}

#[tokio::test]
async fn cartesian_unit_aspect_rejects_non_linear_scale_at_scale_build() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .data(df.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| {
                    x.scale_with::<Log>(|scale| scale.domain((1.0, 10.0)))
                })
                .y_with(col("y"), |y| {
                    y.scale_with::<Linear>(|scale| scale.domain((0.0, 10.0)))
                }),
        );
    let compiled = plot.compile(&ctx).await.expect("compile log-scale plot");

    let err = compiled
        .build_scales_for_dataframe(
            &df,
            200.0,
            100.0,
            &ctx,
            &IndexMap::<String, ScalarValue>::new(),
        )
        .await
        .expect_err("log scale should be rejected");

    assert!(err.to_string().contains("continuous linear numeric scale"));
}

#[tokio::test]
async fn cartesian_unit_aspect_rejects_shared_domain_in_local_scale_build() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .data(df.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| x.with_domain_scope(CoordinationScope::Shared))
                .y(col("y")),
        );
    let compiled = plot.compile(&ctx).await.expect("compile shared plot");

    let err = compiled
        .build_scales_for_dataframe(
            &df,
            200.0,
            100.0,
            &ctx,
            &IndexMap::<String, ScalarValue>::new(),
        )
        .await
        .expect_err("shared local unit-aspect domain should be rejected");

    assert!(
        err.to_string()
            .contains("shared unit_aspect domains require the sharing-aware domain solver")
    );
}
