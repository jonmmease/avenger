//! Chain-level integration tests for `ScalarAggregate` derived scalars:
//! downstream stage resolution, inherited-scalar seeding, and facet scoping.

use avenger_chart::prelude::*;
use avenger_chart_core::derived_scalar;
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use datafusion::prelude::{SessionContext, col, lit};
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;

fn collect_symbol_sizes(scene: &SceneGraph) -> Vec<f32> {
    fn walk(mark: &SceneMark, sizes: &mut Vec<f32>) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    walk(child, sizes);
                }
            }
            SceneMark::Symbol(symbol) => {
                sizes.extend(symbol.size_vec());
            }
            _ => {}
        }
    }
    let mut sizes = Vec::new();
    for mark in &scene.marks {
        walk(mark, &mut sizes);
    }
    sizes.sort_by(|left, right| left.total_cmp(right));
    sizes
}

fn count_rules(scene: &SceneGraph) -> usize {
    fn walk(mark: &SceneMark, total: &mut usize) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    walk(child, total);
                }
            }
            SceneMark::Rule(rule) => {
                *total += rule.len as usize;
            }
            _ => {}
        }
    }
    let mut total = 0;
    for mark in &scene.marks {
        walk(mark, &mut total);
    }
    total
}

fn count_symbols(scene: &SceneGraph) -> usize {
    fn walk(mark: &SceneMark, total: &mut usize) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    walk(child, total);
                }
            }
            SceneMark::Symbol(symbol) => {
                *total += symbol.len as usize;
            }
            _ => {}
        }
    }
    let mut total = 0;
    for mark in &scene.marks {
        walk(mark, &mut total);
    }
    total
}

async fn xy_dataframe(ctx: &SessionContext, rows: usize) -> datafusion::dataframe::DataFrame {
    let values = (0..rows)
        .map(|index| format!("({}.0, {}.0)", index + 1, (index + 1) * 2))
        .collect::<Vec<_>>()
        .join(", ");
    ctx.sql(&format!("SELECT * FROM (VALUES {values}) AS t(x, y)"))
        .await
        .unwrap()
}

fn gated_symbol(gate: datafusion::logical_expr::Expr) -> Symbol<Cartesian> {
    Symbol::new()
        .transform(Filter::new(gate), |mark, _| mark)
        .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
        .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
        .size(20.0)
}

/// Downstream `Filter` gate (eager mode): all rows below the threshold, zero
/// rows past it.
#[tokio::test]
async fn scalar_gate_filters_downstream_rows() {
    for (threshold, expected) in [(10_i64, 5_usize), (3_i64, 0_usize)] {
        let ctx = SessionContext::new();
        let df = xy_dataframe(&ctx, 5).await;
        let plot = Chart::<Cartesian>::new().data(df).mark(
            Symbol::new()
                .transform(ScalarAggregate::new().count("n"), |mark, stats| {
                    mark.transform(
                        Filter::new(stats.scalar("n").lt(lit(threshold))),
                        |mark, _| mark,
                    )
                })
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
                .size(20.0),
        );
        let compiled = plot.compile(&ctx).await.unwrap();
        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        assert_eq!(
            count_symbols(&evaluated.scene_graph),
            expected,
            "threshold={threshold}"
        );
    }
}

/// Lazy scalars referenced by later transform stages resolve into scalar
/// subqueries inside protobuf-backed compiled expressions. DataFusion 48
/// could not serialize `ScalarSubquery` (the reference failed with
/// actionable guidance); DataFusion >= 54 serializes it, so the chain
/// evaluates inline. Both threshold branches prove the subquery really
/// computes (n = 5): below-threshold keeps every row, above drops them all.
#[tokio::test]
async fn lazy_scalar_stage_reference_resolves_inline() {
    for (threshold, expected) in [(10_i64, 5_usize), (3_i64, 0_usize)] {
        let ctx = SessionContext::new();
        let df = xy_dataframe(&ctx, 5).await;
        let plot = Chart::<Cartesian>::new().data(df).mark(
            Symbol::new()
                .transform(ScalarAggregate::new().count("n").lazy(), |mark, stats| {
                    mark.transform(
                        Filter::new(stats.scalar("n").lt(lit(threshold))),
                        |mark, _| mark,
                    )
                })
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
                .size(20.0),
        );
        let compiled = plot.compile(&ctx).await.unwrap();
        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        assert_eq!(
            count_symbols(&evaluated.scene_graph),
            expected,
            "threshold={threshold}"
        );
    }
}

/// A scalar referenced by a later `Calculate` expression.
#[tokio::test]
async fn scalar_in_later_calculate_expression() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 4).await; // y in 2,4,6,8; max 8
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .transform(ScalarAggregate::new().max("hi", col("y")), |mark, stats| {
                mark.transform(
                    Calculate::new().expr("frac", col("y") / stats.scalar("hi")),
                    |mark, _| mark,
                )
            })
            .transform(Filter::new(col("frac").lt_eq(lit(0.5))), |mark, _| mark)
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
            .size(20.0),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    // y/hi <= 0.5 keeps y in {2, 4} of {2, 4, 6, 8}.
    assert_eq!(count_symbols(&evaluated.scene_graph), 2);
}

/// A stage referencing a scalar produced by a later stage errors with the
/// canonical message.
#[tokio::test]
async fn scalar_reference_before_production_errors() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            // References "n" before any stage produces it.
            .transform(
                Filter::new(derived_scalar("n", None).lt(lit(10))),
                |mark, _| mark,
            )
            .transform(ScalarAggregate::new().count("n"), |mark, _| mark)
            .x(col("x"))
            .y(col("y"))
            .size(20.0),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let err = compiled
        .evaluate(&ctx, None)
        .await
        .err()
        .expect("reference before production should fail");
    assert!(
        err.to_string()
            .contains("Derived scalar 'n' was referenced but not produced"),
        "unexpected error: {err}"
    );
}

/// A scalar produced by a `MarkGroup` base chain seeds child mark stages,
/// and the post-chain inherited merge does not double-report it.
#[tokio::test]
async fn mark_group_scalar_seeds_child_stage() {
    for (threshold, visible, hidden) in [(10_i64, 5_usize, 0_usize), (3_i64, 0_usize, 5_usize)] {
        let ctx = SessionContext::new();
        let df = xy_dataframe(&ctx, 5).await;
        let plot =
            Chart::<Cartesian>::new().mark(MarkGroup::<Cartesian>::new().data(df).transform(
                ScalarAggregate::new().count("n"),
                |group, stats| {
                    group
                        .mark(gated_symbol(stats.scalar("n").lt(lit(threshold))))
                        .mark(gated_symbol(stats.scalar("n").gt_eq(lit(threshold))))
                },
            ));
        let compiled = plot.compile(&ctx).await.unwrap();
        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        assert_eq!(
            count_symbols(&evaluated.scene_graph),
            visible + hidden,
            "threshold={threshold}: exactly one child renders its rows"
        );
    }
}

/// A scalar produced by the pre-view chain seeds view-local stages.
#[tokio::test]
async fn pre_view_scalar_seeds_view_local_stage() {
    for (threshold, expected) in [(10_i64, 5_usize), (3_i64, 0_usize)] {
        let ctx = SessionContext::new();
        let df = xy_dataframe(&ctx, 5).await;
        let plot = Chart::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new().transform(ScalarAggregate::new().count("n"), |mark, stats| {
                    let gate = stats.scalar("n").lt(lit(threshold));
                    mark.view(
                        View::cartesian()
                            .id("pts")
                            .x_domain(col("x"))
                            .y_domain(col("y")),
                        move |mark, _v| {
                            mark.transform(Filter::new(gate), |mark, _| mark)
                                .x(col("x"))
                                .y(col("y"))
                                .size(20.0)
                        },
                    )
                }),
            );
        let compiled = plot.compile(&ctx).await.unwrap();
        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        assert_eq!(
            count_symbols(&evaluated.scene_graph),
            expected,
            "threshold={threshold}"
        );
    }
}

/// Params referenced upstream of the scalar's aggregation are re-bound on
/// every evaluation.
#[tokio::test]
async fn param_change_updates_scalar_gate() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 5).await;
    let cutoff = Param::new("cutoff", ScalarValue::Float64(Some(5.0)));
    let plot = Chart::<Cartesian>::new()
        .param(cutoff.clone())
        .data(df)
        .mark(
            Symbol::new()
                .transform(Filter::new(col("x").lt_eq(cutoff.expr())), |mark, _| mark)
                .transform(ScalarAggregate::new().count("n"), |mark, stats| {
                    mark.transform(
                        Filter::new(stats.scalar("n").gt_eq(lit(3_i64))),
                        |mark, _| mark,
                    )
                })
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
                .size(20.0),
        );
    let compiled = plot.compile(&ctx).await.unwrap();

    // cutoff 5.0: all five rows pass, n = 5 >= 3, gate open.
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(count_symbols(&evaluated.scene_graph), 5);

    // cutoff 2.0: n = 2 < 3, gate closed.
    let mut patch = IndexMap::new();
    patch.insert("cutoff".to_string(), ScalarValue::Float64(Some(2.0)));
    let evaluated = compiled.evaluate(&ctx, Some(patch)).await.unwrap();
    assert_eq!(count_symbols(&evaluated.scene_graph), 0);
}

fn scalar_sized_plot(
    df: datafusion::dataframe::DataFrame,
    stats: ScalarAggregate,
) -> Chart<Cartesian> {
    Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .transform(stats, |mark, stats| {
                mark.size_with(col("y") / stats.scalar("hi") * lit(100.0), |c| c.no_scale())
            })
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0)))),
    )
}

/// Eager scalars referenced by channel expressions: the size channel is
/// scalar-normalized, so the rendered sizes are exact fractions of the max.
#[tokio::test]
async fn scalar_in_size_channel_eager() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 4).await; // y in 2,4,6,8
    let plot = scalar_sized_plot(df, ScalarAggregate::new().max("hi", col("y")));
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(
        collect_symbol_sizes(&evaluated.scene_graph),
        vec![25.0, 50.0, 75.0, 100.0],
    );
}

/// Lazy scalars in channel expressions ride the same protobuf path as stage
/// references: unserializable at DataFusion 48, inline scalar subqueries at
/// DataFusion >= 54. The rendered sizes must match the eager variant
/// exactly (`scalar_in_size_channel_eager`).
#[tokio::test]
async fn lazy_scalar_channel_reference_resolves_inline() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 4).await;
    let plot = scalar_sized_plot(df, ScalarAggregate::new().max("hi", col("y")).lazy());
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(
        collect_symbol_sizes(&evaluated.scene_graph),
        vec![25.0, 50.0, 75.0, 100.0],
    );
}

/// A summary rule mark positioned by a scalar (vertical rule at the mean x).
/// Axis ticks and grid lines are also Rule scene marks, so compare against
/// the same plot without the summary rule.
#[tokio::test]
async fn scalar_positions_summary_rule() {
    let ctx = SessionContext::new();
    let scatter = |df| {
        Chart::<Cartesian>::new().data(df).mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 20.0))))
                .size(20.0),
        )
    };

    let baseline = scatter(xy_dataframe(&ctx, 5).await)
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let with_rule = scatter(xy_dataframe(&ctx, 5).await)
        .mark(Rule::new().transform(
            ScalarAggregate::new().mean("mean_x", col("x")),
            |mark, s| {
                mark.x(s.scalar("mean_x"))
                    .y(lit(0.0))
                    .y2(lit(20.0))
                    .stroke("#d62728")
            },
        ))
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    assert_eq!(
        count_rules(&with_rule.scene_graph),
        count_rules(&baseline.scene_graph) + 1,
        "exactly one summary rule beyond axis rules"
    );
    assert_eq!(count_symbols(&with_rule.scene_graph), 5);
}

/// Facet cells compute independent scalars over their narrowed data.
#[tokio::test]
async fn facet_scalars_are_per_cell() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('A', 1.0, 2.0), ('A', 2.0, 3.0),
                ('B', 1.0, 2.0), ('B', 2.0, 3.0), ('B', 3.0, 4.0), ('B', 4.0, 5.0), ('B', 5.0, 6.0)
            ) AS t(facet, x, y)",
        )
        .await
        .unwrap();
    let plot = Chart::<FacetWrap>::new()
        .plot_constraint(PlotConstraint::height(120.0))
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .transform(ScalarAggregate::new().count("n"), |mark, stats| {
                            mark.transform(
                                Filter::new(stats.scalar("n").lt_eq(lit(2_i64))),
                                |mark, _| mark,
                            )
                        })
                        .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                        .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                        .size(20.0),
                ),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    // Facet A has 2 rows (n = 2, gate open); facet B has 5 (n = 5, gate
    // closed). Only A's symbols render.
    assert_eq!(count_symbols(&evaluated.scene_graph), 2);
}
