use super::helpers::{assert_visual_match_default, assert_visual_match_default_with_options};
use avenger_chart::cartesian::guide::CartesianGuide;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::polar::PolarSubplotPositionChannels;
use avenger_chart::polar::guide::PolarGuide;
use avenger_chart::prelude::*;
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
use datafusion::functions_aggregate::average::avg;
use datafusion::prelude::*;
use std::future::Future;

fn run_async_test<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("positioned-subplot-visual-default-stack".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for positioned subplot visual test");
            rt.block_on(f());
        })
        .expect("spawn default-stack positioned subplot visual test thread")
        .join()
        .expect("default-stack positioned subplot visual test panicked");
}

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

async fn partitioned_subplot_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS species,
            column2 AS parent_x,
            column3 AS parent_y,
            column4 AS parent_radius,
            column5 AS parent_angle,
            column6 AS child_x,
            column7 AS child_y,
            column8 AS child_radius,
            column9 AS child_theta
         FROM (VALUES
            ('alpha', 1.0, 1.1, 0.35, 0.35, 0.10, 0.20, 0.15, 0.20),
            ('alpha', 1.2, 1.3, 0.40, 0.55, 0.24, 0.48, 0.30, 0.80),
            ('beta',  3.0, 2.6, 0.65, 2.40, 3.00, 1.10, 1.10, 1.80),
            ('beta',  3.2, 2.9, 0.70, 2.65, 4.00, 1.55, 1.55, 2.40),
            ('beta',  3.4, 2.7, 0.72, 2.85, 5.00, 1.90, 1.90, 3.00),
            ('gamma', 5.0, 1.5, 0.95, 4.65, 8.00, 3.00, 3.00, 4.20),
            ('gamma', 5.3, 1.7, 1.00, 4.90, 9.50, 3.70, 3.70, 4.80),
            ('gamma', 5.1, 1.9, 0.98, 5.10, 11.00, 4.80, 4.40, 5.40),
            ('gamma', 5.4, 1.6, 1.02, 5.30, 12.50, 6.00, 5.10, 6.00)
         )",
    )
    .await
    .expect("create partitioned positioned subplot data")
}

fn inherited_cartesian_scatter_child(
    x_sharing: ScaleSharing,
    y_sharing: ScaleSharing,
) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#f8fbff"))
        .mark(
            Symbol::<Cartesian>::new()
                .x_with(col("child_x"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .with_scale_sharing(x_sharing)
                        .axis(|a| a.tick_count(3).show_title(false))
                })
                .y_with(col("child_y"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .with_scale_sharing(y_sharing)
                        .axis(|a| a.tick_count(3).show_title(false))
                })
                .fill_with(col("species"), |c| c.no_legend())
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(48.0),
        )
}

fn inherited_polar_scatter_child(
    r_sharing: ScaleSharing,
    theta_sharing: ScaleSharing,
) -> Plot<Polar> {
    Plot::<Polar>::new()
        .configure_guide(PolarGuide::new().plot_background_color("#fbfaf7"))
        .mark(
            Symbol::<Polar>::new()
                .r_with(col("child_radius"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(true).zero(false))
                        .with_scale_sharing(r_sharing)
                        .axis(|a| a.tick_count(2).title(""))
                })
                .theta_with(col("child_theta"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .with_scale_sharing(theta_sharing)
                        .axis(|a| a.visible(false))
                })
                .fill_with(col("species"), |c| c.no_legend())
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(46.0),
        )
}

fn find_symbol_mark(mark: &SceneMark) -> Option<&SceneSymbolMark> {
    match mark {
        SceneMark::Symbol(symbol) => Some(symbol),
        SceneMark::Group(group) => group.marks.iter().find_map(find_symbol_mark),
        _ => None,
    }
}

async fn assert_partitioned_child_counts(
    compiled: &CompiledPlot,
    ctx: &SessionContext,
    prefix: &str,
) {
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("evaluate partitioned positioned subplot");
    let group_names = evaluated.scene_graph.group_names();
    for (child_index, partition, expected_len) in [
        (0usize, "alpha", 2u32),
        (1usize, "beta", 3u32),
        (2usize, "gamma", 4u32),
    ] {
        let group_name = format!("{prefix}_subplot_0_{child_index}_{partition}");
        let path = group_names
            .get(&group_name)
            .unwrap_or_else(|| panic!("missing child group {group_name}"));
        let group = evaluated
            .scene_graph
            .get_mark(path)
            .unwrap_or_else(|| panic!("missing mark path for {group_name}"));
        let symbol = find_symbol_mark(group)
            .unwrap_or_else(|| panic!("missing symbol mark inside {group_name}"));
        assert_eq!(
            symbol.len, expected_len,
            "partition {partition} should render {expected_len} inherited rows"
        );
    }
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

#[test]
fn cartesian_partitioned_subplot_scatter() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(620.0, 400.0)
            .data(partitioned_subplot_data(&ctx).await)
            .title("Cartesian partitioned subplots")
            .mark(
                Subplot::<Cartesian>::new(inherited_cartesian_scatter_child(
                    ScaleSharing::Shared,
                    ScaleSharing::Shared,
                ))
                .partition_by(col("species"))
                .subplot_x_with(avg(col("parent_x")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(6.2))).nice(false).zero(false)
                    })
                })
                .subplot_y_with(avg(col("parent_y")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(3.2))).nice(false).zero(false)
                    })
                })
                .plot_size(110.0, 82.0),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile Cartesian partitioned positioned subplots");
        assert_partitioned_child_counts(&compiled, &ctx, "cartesian").await;
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "positioned_subplot",
            "cartesian_partitioned_subplot_scatter",
        )
        .await;
    });
}

#[test]
fn cartesian_partitioned_subplot_polar_children() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(620.0, 400.0)
            .data(partitioned_subplot_data(&ctx).await)
            .title("Cartesian partitioned polar children")
            .mark(
                Subplot::<Cartesian>::new(inherited_polar_scatter_child(
                    ScaleSharing::Free,
                    ScaleSharing::Free,
                ))
                .partition_by(col("species"))
                .subplot_x_with(avg(col("parent_x")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(6.2))).nice(false).zero(false)
                    })
                })
                .subplot_y_with(avg(col("parent_y")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(3.2))).nice(false).zero(false)
                    })
                })
                .plot_size(104.0, 104.0),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile Cartesian partitioned positioned polar children");
        assert_partitioned_child_counts(&compiled, &ctx, "cartesian").await;
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "positioned_subplot",
            "cartesian_partitioned_subplot_polar_children",
        )
        .await;
    });
}

#[test]
fn polar_partitioned_subplot_cartesian_children() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Polar>::new()
            .plot_size(520.0, 440.0)
            .data(partitioned_subplot_data(&ctx).await)
            .title("Polar partitioned Cartesian children")
            .mark(
                Subplot::<Polar>::new(inherited_cartesian_scatter_child(
                    ScaleSharing::Free,
                    ScaleSharing::Free,
                ))
                .partition_by(col("species"))
                .r_with(avg(col("parent_radius")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.35))).nice(false).zero(false)
                    })
                })
                .theta_with(avg(col("parent_angle")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(6.283185307179586)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .plot_size(104.0, 78.0),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile Polar partitioned positioned Cartesian children");
        assert_partitioned_child_counts(&compiled, &ctx, "polar").await;
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "positioned_subplot",
            "polar_partitioned_subplot_cartesian_children",
        )
        .await;
    });
}

#[test]
fn polar_partitioned_subplot_polar_children() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Polar>::new()
            .plot_size(520.0, 440.0)
            .data(partitioned_subplot_data(&ctx).await)
            .title("Polar partitioned polar children")
            .mark(
                Subplot::<Polar>::new(inherited_polar_scatter_child(
                    ScaleSharing::Level(1),
                    ScaleSharing::Level(1),
                ))
                .partition_by(col("species"))
                .r_with(avg(col("parent_radius")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.35))).nice(false).zero(false)
                    })
                })
                .theta_with(avg(col("parent_angle")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(6.283185307179586)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .plot_size(96.0, 96.0),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile Polar partitioned positioned polar children");
        assert_partitioned_child_counts(&compiled, &ctx, "polar").await;
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "positioned_subplot",
            "polar_partitioned_subplot_polar_children",
        )
        .await;
    });
}

#[test]
fn cartesian_positioned_cartesian_subplots() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(430.0, 300.0)
            .data(positioned_data(&ctx).await)
            .title("Cartesian-positioned subplots")
            .mark(
                Subplot::new(cartesian_child(&ctx).await)
                    .subplot_x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(4.1))))
                    })
                    .subplot_y_with(col("y"), |c| {
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
    });
}

#[test]
fn cartesian_positioned_mixed_subplots() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(430.0, 300.0)
            .data(positioned_data(&ctx).await)
            .title("Mixed positioned subplots")
            .mark(
                Subplot::new(cartesian_child(&ctx).await)
                    .subplot_x_with(lit(1.2), |c| {
                        c.scale_with::<Linear>(|s| s.domain((lit(0.8), lit(3.9))))
                    })
                    .subplot_y_with(lit(1.2), |c| {
                        c.scale_with::<Linear>(|s| s.domain((lit(0.8), lit(2.1))))
                    })
                    .plot_size(82.0, 62.0),
            )
            .mark(
                Subplot::new(polar_child(&ctx).await)
                    .subplot_x(lit(3.5))
                    .subplot_y(lit(1.8))
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
    });
}

#[test]
fn cartesian_positioned_components_debug() {
    run_async_test(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(430.0, 300.0)
            .data(positioned_data(&ctx).await)
            .title("Positioned subplot debug")
            .mark(
                Subplot::new(cartesian_child(&ctx).await)
                    .key("mini")
                    .label("Mini")
                    .subplot_x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.domain((lit(0.7), lit(4.1))))
                    })
                    .subplot_y_with(col("y"), |c| {
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
    });
}
