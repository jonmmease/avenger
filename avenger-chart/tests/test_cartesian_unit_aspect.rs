use avenger_chart::prelude::*;
#[cfg(feature = "svg")]
use avenger_chart::render::SvgRenderer;
use avenger_chart::render::{
    EvaluatedPlot, EvaluationOptions, FacetLayoutRefinement, InteractionScopeKind,
};
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

fn unit_aspect_ratio_from_domains(
    x_domain: (f32, f32),
    y_domain: (f32, f32),
    width: f32,
    height: f32,
) -> f32 {
    let px_per_x = width / (x_domain.1 - x_domain.0).abs();
    let px_per_y = height / (y_domain.1 - y_domain.0).abs();
    px_per_y / px_per_x
}

fn coordinate_scope_unit_aspect_ratio(evaluated: &EvaluatedPlot) -> f32 {
    let scope = evaluated
        .interaction
        .scopes
        .iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate interaction scope");
    let x_domain = scope
        .scales
        .get("x")
        .expect("x scale")
        .numeric_interval_domain()
        .expect("x numeric domain");
    let y_domain = scope
        .scales
        .get("y")
        .expect("y scale")
        .numeric_interval_domain()
        .expect("y numeric domain");
    let x_range = scope
        .scales
        .get("x")
        .expect("x scale")
        .numeric_interval_range()
        .expect("x numeric range");
    let y_range = scope
        .scales
        .get("y")
        .expect("y scale")
        .numeric_interval_range()
        .expect("y numeric range");

    unit_aspect_ratio_from_domains(
        x_domain,
        y_domain,
        (x_range.1 - x_range.0).abs(),
        (y_range.1 - y_range.0).abs(),
    )
}

fn coordinate_scope_domains(evaluated: &EvaluatedPlot) -> ((f32, f32), (f32, f32)) {
    let scope = evaluated
        .interaction
        .scopes
        .iter()
        .find(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .expect("coordinate interaction scope");
    let x_domain = scope
        .scales
        .get("x")
        .expect("x scale")
        .numeric_interval_domain()
        .expect("x numeric domain");
    let y_domain = scope
        .scales
        .get("y")
        .expect("y scale")
        .numeric_interval_domain()
        .expect("y numeric domain");
    (x_domain, y_domain)
}

fn coordinate_scope_unit_aspect_ratios(evaluated: &EvaluatedPlot) -> Vec<f32> {
    evaluated
        .interaction
        .scopes
        .iter()
        .filter(|scope| scope.kind == InteractionScopeKind::Coordinate)
        .filter_map(|scope| {
            let x_scale = scope.scales.get("x")?;
            let y_scale = scope.scales.get("y")?;
            let x_domain = x_scale.numeric_interval_domain().ok()?;
            let y_domain = y_scale.numeric_interval_domain().ok()?;
            let x_range = x_scale.numeric_interval_range().ok()?;
            let y_range = y_scale.numeric_interval_range().ok()?;
            Some(unit_aspect_ratio_from_domains(
                x_domain,
                y_domain,
                (x_range.1 - x_range.0).abs(),
                (y_range.1 - y_range.0).abs(),
            ))
        })
        .collect()
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
#[cfg(feature = "svg")]
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
        .mark(Subplot::new(child()).name("left"))
        .mark(Subplot::new(child()).name("right"));
    let compiled = plot.compile(&ctx).await.expect("compile concat");

    let err = SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect_err("authored concat sharing should be rejected");

    assert!(
        err.to_string()
            .contains("does not support authored concat/grid shared domains")
    );
}

#[tokio::test]
#[cfg(feature = "svg")]
async fn generated_repeat_allows_unit_aspect_shared_child_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 0.0 AS a, 0.0 AS b, 0.0 AS y \
             UNION ALL SELECT 10.0 AS a, 5.0 AS b, 10.0 AS y",
        )
        .await
        .unwrap();
    let cell = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .mark(Line::new().x(repeat::column()).y(col("y")));
    let plot = Plot::<RepeatColumns>::new()
        .plot_size(400.0, 100.0)
        .data(df)
        .configure_coord(|c| {
            c.columns([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .with_repeat_domain_coordination(RepeatDomainCoordination::by_variable(
                CoordinationScope::Shared,
            ))
            .cell(cell)
        });
    let compiled = plot.compile(&ctx).await.expect("compile repeat");

    SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("generated repeat should solve unit_aspect shared domains");
}

#[tokio::test]
#[cfg(feature = "svg")]
async fn facet_column_allows_unit_aspect_shared_child_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 'left' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'left' AS panel, 10.0 AS x, 10.0 AS y \
             UNION ALL SELECT 'right' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'right' AS panel, 5.0 AS x, 10.0 AS y",
        )
        .await
        .unwrap();
    let child = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Line::new()
            .x_with(col("x"), |x| x.with_domain_scope(CoordinationScope::Shared))
            .y_with(col("y"), |y| y.with_domain_scope(CoordinationScope::Shared)),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(400.0, 100.0)
        .data(df)
        .mark(Subplot::new(child).column(col("panel")));
    let compiled = plot.compile(&ctx).await.expect("compile facet");

    SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("facet should solve unit_aspect shared domains");
}

#[tokio::test]
#[cfg(feature = "svg")]
async fn generated_repeat_unit_aspect_shared_domain_handles_symbol_radius_padding() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 0.0 AS a, 0.0 AS b, 0.0 AS y \
             UNION ALL SELECT 10.0 AS a, 5.0 AS b, 10.0 AS y",
        )
        .await
        .unwrap();
    let cell = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .mark(Symbol::new().x(repeat::column()).y(col("y")).size(2500.0));
    let plot = Plot::<RepeatColumns>::new()
        .plot_size(500.0, 120.0)
        .data(df)
        .configure_coord(|c| {
            c.columns([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .with_repeat_domain_coordination(RepeatDomainCoordination::by_variable(
                CoordinationScope::Shared,
            ))
            .cell(cell)
        });
    let compiled = plot.compile(&ctx).await.expect("compile repeat");

    SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("generated repeat should solve radius-aware unit_aspect domains");
}

#[tokio::test]
#[cfg(feature = "svg")]
async fn facet_column_unit_aspect_shared_domain_handles_symbol_radius_padding() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 'left' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'left' AS panel, 10.0 AS x, 10.0 AS y \
             UNION ALL SELECT 'right' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'right' AS panel, 5.0 AS x, 10.0 AS y",
        )
        .await
        .unwrap();
    let child = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Symbol::new()
            .x_with(col("x"), |x| x.with_domain_scope(CoordinationScope::Shared))
            .y_with(col("y"), |y| y.with_domain_scope(CoordinationScope::Shared))
            .size(2500.0),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(500.0, 120.0)
        .data(df)
        .mark(Subplot::new(child).column(col("panel")));
    let compiled = plot.compile(&ctx).await.expect("compile facet");

    SvgRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("facet should solve radius-aware unit_aspect domains");
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
async fn cartesian_unit_aspect_final_scales_satisfy_fixed_plot_ratio() {
    let ctx = SessionContext::new();

    let (x_domain, y_domain) = unit_aspect_domains(&ctx, 1.0, 600.0, 300.0).await;
    assert_close(
        unit_aspect_ratio_from_domains(x_domain, y_domain, 600.0, 300.0),
        1.0,
    );

    let (x_domain, y_domain) = unit_aspect_domains(&ctx, 2.0, 600.0, 300.0).await;
    assert_close(
        unit_aspect_ratio_from_domains(x_domain, y_domain, 600.0, 300.0),
        2.0,
    );
}

#[tokio::test]
async fn cartesian_unit_aspect_canvas_refinement_final_scales_satisfy_realized_plot_ratio() {
    let ctx = SessionContext::new();
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .canvas_size(560.0, 340.0)
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |x| {
                    x.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                    .axis(|axis| axis.title("refined canvas x").grid(true))
                })
                .y_with(col("y"), |y| {
                    y.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                    .axis(|axis| axis.title("refined canvas y").grid(true))
                }),
        );
    let compiled = plot.compile(&ctx).await.expect("compile canvas plot");
    let (evaluated, metrics) = compiled
        .evaluate_with_options_and_metrics(
            &ctx,
            None,
            EvaluationOptions {
                facet_layout_refinement: FacetLayoutRefinement {
                    max_refinement_passes: 2,
                    overflow_growth_epsilon: 0.5,
                },
                ..EvaluationOptions::default()
            },
        )
        .await
        .expect("evaluate canvas plot");

    assert!(
        metrics.facet_layout.refinement_pass_count >= 1,
        "canvas layout should run refinement: {metrics:?}"
    );
    assert_close(coordinate_scope_unit_aspect_ratio(&evaluated), 1.0);
}

#[tokio::test]
async fn cartesian_unit_aspect_canvas_refinement_does_not_ratchet_domains() {
    let ctx = SessionContext::new();
    let width = Param::new("canvas_width", ScalarValue::Float64(Some(560.0)));
    let df = xy_data(&ctx).await;
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .add_param(width.clone())
        .canvas_size(width.expr(), 340.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |x| {
                    x.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                    .axis(|axis| axis.title("param x").grid(true))
                })
                .y_with(col("y"), |y| {
                    y.scale_with::<Linear>(|scale| {
                        scale.domain((0.0, 10.0)).nice(false).zero(false)
                    })
                    .axis(|axis| axis.title("param y").grid(true))
                })
                .size(80.0),
        );
    let compiled = plot.compile(&ctx).await.expect("compile param-width plot");
    let options = EvaluationOptions {
        facet_layout_refinement: FacetLayoutRefinement {
            max_refinement_passes: 2,
            overflow_growth_epsilon: 0.5,
        },
        ..EvaluationOptions::default()
    };
    let params = |value| {
        IndexMap::from([(
            "canvas_width".to_string(),
            ScalarValue::Float64(Some(value)),
        )])
    };

    let (first, _) = compiled
        .evaluate_with_options_and_metrics(&ctx, Some(params(560.0)), options.clone())
        .await
        .expect("evaluate first width");
    let (middle, _) = compiled
        .evaluate_with_options_and_metrics(&ctx, Some(params(460.0)), options.clone())
        .await
        .expect("evaluate middle width");
    let (second, _) = compiled
        .evaluate_with_options_and_metrics(&ctx, Some(params(560.0)), options)
        .await
        .expect("evaluate repeated width");

    let (first_x, first_y) = coordinate_scope_domains(&first);
    let (middle_x, middle_y) = coordinate_scope_domains(&middle);
    let (second_x, second_y) = coordinate_scope_domains(&second);

    assert_close(coordinate_scope_unit_aspect_ratio(&first), 1.0);
    assert_close(coordinate_scope_unit_aspect_ratio(&middle), 1.0);
    assert_close(coordinate_scope_unit_aspect_ratio(&second), 1.0);
    assert_close(first_x.0, second_x.0);
    assert_close(first_x.1, second_x.1);
    assert_close(first_y.0, second_y.0);
    assert_close(first_y.1, second_y.1);
    assert!(
        first_x != middle_x || first_y != middle_y,
        "different plot-area sizes should produce different constrained domains"
    );
}

#[tokio::test]
async fn facet_column_unit_aspect_canvas_refinement_reruns_shared_domain_solve() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 'left' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'left' AS panel, 10.0 AS x, 10.0 AS y \
             UNION ALL SELECT 'right' AS panel, 0.0 AS x, 0.0 AS y \
             UNION ALL SELECT 'right' AS panel, 5.0 AS x, 10.0 AS y",
        )
        .await
        .unwrap();
    let child = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Line::new()
            .x_with(col("x"), |x| {
                x.with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared canvas x").grid(true))
            })
            .y_with(col("y"), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("shared canvas y").grid(true))
            }),
    );
    let plot = Plot::<FacetColumn>::new()
        .canvas_size(660.0, 360.0)
        .data(df)
        .mark(Subplot::new(child).column(col("panel")));
    let compiled = plot.compile(&ctx).await.expect("compile canvas facet");
    let (evaluated, metrics) = compiled
        .evaluate_with_options_and_metrics(
            &ctx,
            None,
            EvaluationOptions {
                facet_layout_refinement: FacetLayoutRefinement {
                    max_refinement_passes: 2,
                    overflow_growth_epsilon: 0.5,
                },
                ..EvaluationOptions::default()
            },
        )
        .await
        .expect("evaluate canvas facet");

    assert!(
        metrics.facet_layout.refinement_pass_count >= 1,
        "facet canvas layout should run refinement: {metrics:?}"
    );
    let ratios = coordinate_scope_unit_aspect_ratios(&evaluated);
    assert!(
        ratios.len() >= 2,
        "expected child coordinate scopes with x/y scales"
    );
    for ratio in ratios {
        assert_close(ratio, 1.0);
    }
}

#[tokio::test]
async fn generated_repeat_unit_aspect_canvas_refinement_reruns_shared_domain_solve() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT 0.0 AS a, 0.0 AS b, 0.0 AS y \
             UNION ALL SELECT 10.0 AS a, 5.0 AS b, 10.0 AS y",
        )
        .await
        .unwrap();
    let cell = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        Line::new()
            .x_with(repeat::column(), |x| {
                x.axis(|axis| axis.title("repeat canvas x").grid(true))
            })
            .y_with(col("y"), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("repeat canvas y").grid(true))
            }),
    );
    let plot = Plot::<RepeatColumns>::new()
        .canvas_size(660.0, 320.0)
        .data(df)
        .configure_coord(|c| {
            c.columns([
                RepeatVariable::new("a", col("a")),
                RepeatVariable::new("b", col("b")),
            ])
            .with_repeat_domain_coordination(RepeatDomainCoordination::by_variable(
                CoordinationScope::Shared,
            ))
            .cell(cell)
        });
    let compiled = plot.compile(&ctx).await.expect("compile canvas repeat");
    let (evaluated, metrics) = compiled
        .evaluate_with_options_and_metrics(
            &ctx,
            None,
            EvaluationOptions {
                facet_layout_refinement: FacetLayoutRefinement {
                    max_refinement_passes: 2,
                    overflow_growth_epsilon: 0.5,
                },
                ..EvaluationOptions::default()
            },
        )
        .await
        .expect("evaluate canvas repeat");

    assert!(
        metrics.facet_layout.refinement_pass_count >= 1,
        "repeat canvas layout should run refinement: {metrics:?}"
    );
    let ratios = coordinate_scope_unit_aspect_ratios(&evaluated);
    assert!(
        ratios.len() >= 2,
        "expected repeat child coordinate scopes with x/y scales"
    );
    for ratio in ratios {
        assert_close(ratio, 1.0);
    }
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
