//! The synchronous adaptive raster-to-scatter workflow: a group view scope
//! shares one in-view count between an async rasterized density child and a
//! synchronous scatter child, whose gates switch representation at a point
//! budget.

use std::{sync::Arc, time::Duration};

use avenger_chart::{plot::CompiledPlot, prelude::*, render::EvaluatedPlot};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

use super::helpers::assert_scene_graph_visual_match_default;

const CATEGORY: &str = "group_view";
const POINT_BUDGET: i64 = 10;

fn list_domain(min: f64, max: f64) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min)),
            ScalarValue::Float64(Some(max)),
        ],
        &DataType::Float64,
        true,
    ))
}

fn zoom_patch(x_min: f64, x_max: f64) -> IndexMap<String, ScalarValue> {
    let mut patch = IndexMap::new();
    patch.insert(
        "__tool_pan_scroll_zoom__x_domain".to_string(),
        list_domain(x_min, x_max),
    );
    patch.insert(
        "__tool_pan_scroll_zoom__y_domain".to_string(),
        list_domain(-1.0, 23.0),
    );
    patch
}

fn count_image_marks(scene: &SceneGraph) -> usize {
    fn walk(mark: &SceneMark) -> usize {
        match mark {
            SceneMark::Image(_) => 1,
            SceneMark::Group(group) => group.marks.iter().map(walk).sum(),
            _ => 0,
        }
    }
    scene.marks.iter().map(walk).sum()
}

fn count_symbol_marks(scene: &SceneGraph) -> usize {
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

/// 40 deterministic points: x = 1..=40, y = (7 * x) mod 23.
async fn adaptive_dataframe(ctx: &SessionContext) -> DataFrame {
    let values = (1..=40)
        .map(|index| format!("({}.0, {}.0)", index, (7 * index) % 23))
        .collect::<Vec<_>>()
        .join(", ");
    ctx.sql(&format!("SELECT * FROM (VALUES {values}) AS t(x, y)"))
        .await
        .expect("build adaptive fixture")
}

async fn compile_adaptive_plot(ctx: &SessionContext) -> CompiledPlot {
    let df = adaptive_dataframe(ctx).await;
    Chart::<Cartesian>::new()
        .plot_size(320.0, 250.0)
        .tool(PanScrollZoom::cartesian())
        .mark(
            MarkGroup::<Cartesian>::new().data(df).view(
                View::cartesian()
                    .id("adaptive")
                    .x_domain(col("x"))
                    .y_domain(col("y"))
                    .preview_cached(true),
                |group, v| {
                    let in_view = col("x")
                        .gt_eq(v.x().domain_start())
                        .and(col("x").lt_eq(v.x().domain_end()))
                        .and(col("y").gt_eq(v.y().domain_start()))
                        .and(col("y").lt_eq(v.y().domain_end()));
                    group
                        // Group view-local: shared in-view filter + count.
                        .transform(Filter::new(in_view), |group, _| group)
                        .transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group
                                .mark(
                                    UniformRaster2D::new()
                                        .transform(
                                            Rasterize2D::new(col("x"), col("y"))
                                                .x(|x| {
                                                    x.extent(
                                                        v.x().domain_start(),
                                                        v.x().domain_end(),
                                                    )
                                                    .bins(24)
                                                })
                                                .y(|y| {
                                                    y.extent(
                                                        v.y().domain_start(),
                                                        v.y().domain_end(),
                                                    )
                                                    .bins(24)
                                                })
                                                .agg("count"),
                                            |mark, hist| {
                                                mark.transform(
                                                    // Threshold gate AFTER
                                                    // Rasterize2D: drops the
                                                    // raster row in scatter
                                                    // mode.
                                                    Filter::new(
                                                        stats.scalar("n").gt_eq(lit(POINT_BUDGET)),
                                                    ),
                                                    |mark, _| mark,
                                                )
                                                .raster_with(hist.raster(), |r| {
                                                    r.x_with(hist.x_dim(), |x| {
                                                        x.scale_with::<Linear>(|scale| {
                                                            scale.nice(false).zero(false)
                                                        })
                                                        .axis(|axis| axis.tick_count(4))
                                                    })
                                                    .y_with(hist.y_dim(), |y| {
                                                        y.scale_with::<Linear>(|scale| {
                                                            scale.nice(false).zero(false)
                                                        })
                                                        .axis(|axis| axis.tick_count(4))
                                                    })
                                                    .fill(|fill| {
                                                        fill.scale_with::<Sqrt>(|scale| {
                                                            scale
                                                                .domain((0.0, 4.0))
                                                                .nice(false)
                                                                .zero(false)
                                                        })
                                                        .legend(|legend| legend.title("Points"))
                                                    })
                                                })
                                            },
                                        )
                                        .smooth(false),
                                )
                                .mark(
                                    Symbol::new()
                                        .transform(
                                            Filter::new(stats.scalar("n").lt(lit(POINT_BUDGET))),
                                            |mark, _| mark,
                                        )
                                        .x(col("x"))
                                        .y(col("y"))
                                        .size(40.0)
                                        .fill("#4682b4"),
                                )
                        })
                },
            ),
        )
        .compile(ctx)
        .await
        .expect("compile adaptive plot")
}

async fn wait_for_materialization_invalidation(session: &PlotSession, initial_epoch: u64) {
    for _ in 0..200 {
        if session.evaluation_invalidation_epoch() > initial_epoch {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for async view materialization");
}

/// Evaluate to the settled state: warm exact, wait for the async raster to
/// complete if it was queued, then evaluate exact again.
async fn evaluate_settled(
    compiled: Arc<CompiledPlot>,
    ctx: Arc<SessionContext>,
    param_patch: Option<IndexMap<String, ScalarValue>>,
) -> EvaluatedPlot {
    let mut session = compiled.instantiate(ctx);
    let initial_epoch = session.evaluation_invalidation_epoch();
    let request = || {
        let request = EvaluationRequest::new().exact();
        match param_patch.clone() {
            Some(patch) => request.param_patch(patch),
            None => request,
        }
    };
    let (_warmup, warmup_metrics) = session
        .evaluate_with_metrics(request())
        .await
        .expect("warm up adaptive evaluation");
    if warmup_metrics.pipeline.materialization_ready_used == 0
        && warmup_metrics.pipeline.materialization_queued > 0
    {
        wait_for_materialization_invalidation(&session, initial_epoch).await;
    }
    session
        .evaluate_with_metrics(request())
        .await
        .expect("evaluate settled adaptive scene")
        .0
}

/// Full domain: 40 points in view (>= budget), the raster renders and the
/// scatter child is gated off.
#[tokio::test]
async fn adaptive_group_view_raster_mode() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_adaptive_plot(&ctx).await);
    let evaluated = evaluate_settled(compiled, ctx, None).await;
    assert_eq!(count_image_marks(&evaluated.scene_graph), 1);
    assert_eq!(count_symbol_marks(&evaluated.scene_graph), 0);
    assert_scene_graph_visual_match_default(
        &evaluated.scene_graph,
        CATEGORY,
        "adaptive_raster_mode",
    )
    .await;
}

/// Zoomed to 6 in-view points (< budget): the scatter child renders the
/// exact points and the raster row is dropped.
#[tokio::test]
async fn adaptive_group_view_scatter_mode() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_adaptive_plot(&ctx).await);
    let evaluated = evaluate_settled(compiled, ctx, Some(zoom_patch(0.5, 6.5))).await;
    assert_eq!(count_image_marks(&evaluated.scene_graph), 0);
    assert_eq!(count_symbol_marks(&evaluated.scene_graph), 6);
    assert_scene_graph_visual_match_default(
        &evaluated.scene_graph,
        CATEGORY,
        "adaptive_scatter_mode",
    )
    .await;
}

/// Exactly at the budget (n = 10): raster mode.
#[tokio::test]
async fn adaptive_group_view_boundary_at_budget_is_raster() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_adaptive_plot(&ctx).await);
    let evaluated = evaluate_settled(compiled, ctx, Some(zoom_patch(0.5, 10.5))).await;
    assert_eq!(count_image_marks(&evaluated.scene_graph), 1);
    assert_eq!(count_symbol_marks(&evaluated.scene_graph), 0);
    assert_scene_graph_visual_match_default(
        &evaluated.scene_graph,
        CATEGORY,
        "adaptive_boundary_raster",
    )
    .await;
}

/// One under the budget (n = 9): scatter mode.
#[tokio::test]
async fn adaptive_group_view_boundary_under_budget_is_scatter() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_adaptive_plot(&ctx).await);
    let evaluated = evaluate_settled(compiled, ctx, Some(zoom_patch(0.5, 9.5))).await;
    assert_eq!(count_image_marks(&evaluated.scene_graph), 0);
    assert_eq!(count_symbol_marks(&evaluated.scene_graph), 9);
    assert_scene_graph_visual_match_default(
        &evaluated.scene_graph,
        CATEGORY,
        "adaptive_boundary_scatter",
    )
    .await;
}
