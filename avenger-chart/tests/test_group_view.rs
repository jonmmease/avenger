//! Group view scopes: authoring, compile-time lowering onto child marks,
//! validation, and serialization.

use avenger_chart::prelude::*;
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use datafusion::prelude::{SessionContext, col, lit};
use datafusion::scalar::ScalarValue;

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

/// A group view scope lowers onto both children: the plot compiles, view
/// domains come from the view declarations, and both marks render.
#[tokio::test]
async fn group_view_lowers_onto_children() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 5).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group
                    .mark(Symbol::new().x(col("x")).y(col("y")).size(20.0))
                    .mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y") + lit(0.5))
                            .size(10.0),
                    )
            },
        ),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(count_symbols(&evaluated.scene_graph), 10);
}

/// Group-view plots survive serialization: the compiled plot round-trips
/// through JSON and evaluates identically.
#[tokio::test]
async fn group_view_serialization_round_trip() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 4).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y")).size(20.0)),
        ),
    );
    let compiled = plot.compile(&ctx).await.unwrap();
    let direct = compiled.evaluate(&ctx, None).await.unwrap();

    let serialized = serde_json::to_string(&compiled).expect("serialize compiled plot");
    let deserialized: avenger_chart::plot::CompiledPlot =
        serde_json::from_str(&serialized).expect("deserialize compiled plot");
    let round_tripped = deserialized.evaluate(&ctx, None).await.unwrap();

    assert_eq!(
        count_symbols(&direct.scene_graph),
        count_symbols(&round_tripped.scene_graph)
    );
    assert_eq!(count_symbols(&round_tripped.scene_graph), 4);
}

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

/// The adaptive-switch shape: a group view-local chain (filter + count) is
/// shared by two children whose gates read the same scalar. Exactly one
/// child renders, and it renders the shared filtered rows.
///
/// Once-per-evaluation execution of the shared chain is structural (the
/// memo in `group_view_data_cache`); these assertions cover the observable
/// contract — both children agree on one scalar value and consume the same
/// shared dataframe.
#[tokio::test]
async fn group_view_shared_chain_feeds_both_children() {
    let ctx = SessionContext::new();
    let cutoff = Param::new("cutoff", ScalarValue::Float64(Some(10.0)));
    let df = xy_dataframe(&ctx, 5).await;
    let plot = Plot::<Cartesian>::new()
        .add_param(cutoff.clone())
        .mark(
            MarkGroup::<Cartesian>::new().data(df).view(
                View::cartesian()
                    .id("pts")
                    .x_domain(col("x"))
                    .y_domain(col("y")),
                |group, _v| {
                    group
                        // group view-local: shared filter + shared count
                        .transform(Filter::new(col("x").lt_eq(cutoff.expr())), |group, _| group)
                        .transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group
                                .mark(
                                    // "sparse" child: visible when n < 3
                                    Symbol::new()
                                        .transform(
                                            Filter::new(stats.scalar("n").lt(lit(3_i64))),
                                            |mark, _| mark,
                                        )
                                        .x(col("x"))
                                        .y(col("y"))
                                        .size(10.0),
                                )
                                .mark(
                                    // "dense" child: visible when n >= 3
                                    Symbol::new()
                                        .transform(
                                            Filter::new(stats.scalar("n").gt_eq(lit(3_i64))),
                                            |mark, _| mark,
                                        )
                                        .x(col("x"))
                                        .y(col("y"))
                                        .size(40.0),
                                )
                        })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.unwrap();

    // cutoff 10.0: all 5 rows pass the shared filter, n = 5 >= 3, only the
    // "dense" child renders, and it renders the shared 5 filtered rows.
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(
        collect_symbol_sizes(&evaluated.scene_graph),
        vec![40.0; 5],
        "dense child renders the shared filtered rows"
    );

    // cutoff 2.0: shared filter passes 2 rows, n = 2 < 3, only the "sparse"
    // child renders — the shared chain result is fresh per evaluation.
    let mut patch = indexmap::IndexMap::new();
    patch.insert("cutoff".to_string(), ScalarValue::Float64(Some(2.0)));
    let evaluated = compiled.evaluate(&ctx, Some(patch)).await.unwrap();
    assert_eq!(
        collect_symbol_sizes(&evaluated.scene_graph),
        vec![10.0; 2],
        "sparse child renders after the param change"
    );
}

fn list_domain(min: f64, max: f64) -> ScalarValue {
    use datafusion::arrow::datatypes::DataType;
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min)),
            ScalarValue::Float64(Some(max)),
        ],
        &DataType::Float64,
        true,
    ))
}

/// PanScrollZoom composition: tool-installed raw-domain edits flow into
/// group view resolution, so the shared in-view count (and the gates that
/// read it) track the zoomed domain. This is the miniature synchronous
/// adaptive raster-to-scatter workflow.
#[tokio::test]
async fn group_view_shared_count_tracks_pan_scroll_zoom_domain() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 5).await;
    let plot = Plot::<Cartesian>::new()
        .tool(PanScrollZoom::cartesian())
        .mark(
            MarkGroup::<Cartesian>::new().data(df).view(
                View::cartesian()
                    .id("pts")
                    .x_domain(col("x"))
                    .y_domain(col("y")),
                |group, v| {
                    let in_view = col("x")
                        .gt_eq(v.x().domain_start())
                        .and(col("x").lt_eq(v.x().domain_end()));
                    group
                        .transform(Filter::new(in_view), |group, _| group)
                        .transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group
                                .mark(
                                    Symbol::new()
                                        .transform(
                                            Filter::new(stats.scalar("n").lt(lit(3_i64))),
                                            |mark, _| mark,
                                        )
                                        .x(col("x"))
                                        .y(col("y"))
                                        .size(10.0),
                                )
                                .mark(
                                    Symbol::new()
                                        .transform(
                                            Filter::new(stats.scalar("n").gt_eq(lit(3_i64))),
                                            |mark, _| mark,
                                        )
                                        .x(col("x"))
                                        .y(col("y"))
                                        .size(40.0),
                                )
                        })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.unwrap();

    // Full domain: all 5 points in view, dense child renders.
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    assert_eq!(collect_symbol_sizes(&evaluated.scene_graph), vec![40.0; 5]);

    // Zoomed to x in [0.5, 2.5]: 2 points in view, sparse child renders.
    let mut patch = indexmap::IndexMap::new();
    patch.insert(
        "__tool_pan_scroll_zoom__x_domain".to_string(),
        list_domain(0.5, 2.5),
    );
    let evaluated = compiled.evaluate(&ctx, Some(patch)).await.unwrap();
    assert_eq!(collect_symbol_sizes(&evaluated.scene_graph), vec![10.0; 2]);
}

/// Group view scopes inside facet cells compute independent shared chains
/// per cell.
#[tokio::test]
async fn group_view_shared_chain_is_per_facet_cell() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('A', 1.0, 2.0), ('A', 2.0, 3.0),
                ('B', 1.0, 2.0), ('B', 2.0, 3.0), ('B', 3.0, 4.0), ('B', 4.0, 5.0)
            ) AS t(facet, x, y)",
        )
        .await
        .unwrap();
    let plot = Plot::<FacetWrap>::new()
        .plot_constraint(PlotConstraint::height(120.0))
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(MarkGroup::<Cartesian>::new().view(
                    View::cartesian()
                        .id("pts")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |group, _v| {
                        group.transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group.mark(
                                Symbol::new()
                                    .transform(
                                        Filter::new(stats.scalar("n").lt_eq(lit(2_i64))),
                                        |mark, _| mark,
                                    )
                                    .x(col("x"))
                                    .y(col("y"))
                                    .size(20.0),
                            )
                        })
                    },
                )),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );
    let compiled = plot.compile(&ctx).await.unwrap();
    let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
    // Facet A: n = 2, gate open (2 symbols). Facet B: n = 4, gate closed.
    assert_eq!(count_symbols(&evaluated.scene_graph), 2);
}

/// A mark-level view scope inside a viewed group is rejected.
#[tokio::test]
async fn mark_view_inside_group_view_errors() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("outer")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group.mark(Symbol::new().view(
                    View::cartesian()
                        .id("inner")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, _v| mark.x(col("x")).y(col("y")),
                ))
            },
        ),
    );
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("nested mark view should fail");
    assert!(
        err.to_string().contains("nested view scopes are not supported"),
        "unexpected error: {err}"
    );
}

/// A viewed group inside a viewed group is rejected.
#[tokio::test]
async fn nested_group_views_error() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let plot = Plot::<Cartesian>::new().mark(
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("outer")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| {
                group.mark(MarkGroup::<Cartesian>::new().view(
                    View::cartesian()
                        .id("inner")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y"))),
                ))
            },
        ),
    );
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("nested group views should fail");
    assert!(
        err.to_string().contains("nested view scopes are not supported"),
        "unexpected error: {err}"
    );
}

/// Duplicate view ids across the plot are rejected.
#[tokio::test]
async fn duplicate_view_ids_error() {
    let ctx = SessionContext::new();
    let df = xy_dataframe(&ctx, 3).await;
    let group = |df: datafusion::dataframe::DataFrame| {
        MarkGroup::<Cartesian>::new().data(df).view(
            View::cartesian()
                .id("pts")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |group, _v| group.mark(Symbol::new().x(col("x")).y(col("y"))),
        )
    };
    let plot = Plot::<Cartesian>::new()
        .mark(group(df.clone()))
        .mark(group(df));
    let err = plot
        .compile(&ctx)
        .await
        .err()
        .expect("duplicate view ids should fail");
    assert!(
        err.to_string().contains("Duplicate view id 'pts'"),
        "unexpected error: {err}"
    );
}
