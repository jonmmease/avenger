use std::{sync::Arc, time::Duration};

use avenger_chart::{plot::CompiledPlot, prelude::*, render::EvaluatedPlot};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    prelude::{CsvReadOptions, SessionContext, col, lit},
};
use indexmap::IndexMap;

use super::helpers::assert_scene_graph_visual_match;

const VIEW_SCOPE_CATEGORY: &str = "view_scope";
const VIEW_RASTER_CATEGORY: &str = "view_uniform_raster_2d";

const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

const TAXI_ZOOM_X_MIN: f64 = -8_239_500.0;
const TAXI_ZOOM_X_MAX: f64 = -8_232_000.0;
const TAXI_ZOOM_Y_MIN: f64 = 4_971_000.0;
const TAXI_ZOOM_Y_MAX: f64 = 4_978_500.0;

fn exact_request(param_patch: Option<IndexMap<String, ScalarValue>>) -> EvaluationRequest {
    let request = EvaluationRequest::new().exact();
    if let Some(param_patch) = param_patch {
        request.param_patch(param_patch)
    } else {
        request
    }
}

fn preview_request(param_patch: IndexMap<String, ScalarValue>) -> EvaluationRequest {
    EvaluationRequest::new().preview().param_patch(param_patch)
}

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

fn pan_scroll_zoom_domain_patch(
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
) -> IndexMap<String, ScalarValue> {
    let mut patch = IndexMap::new();
    patch.insert(
        "__tool_pan_scroll_zoom__x_domain".to_string(),
        list_domain(x_min, x_max),
    );
    patch.insert(
        "__tool_pan_scroll_zoom__y_domain".to_string(),
        list_domain(y_min, y_max),
    );
    patch
}

fn count_image_marks(scene_graph: &SceneGraph) -> usize {
    scene_graph
        .marks
        .iter()
        .map(count_image_marks_in_mark)
        .sum()
}

fn count_image_marks_in_mark(mark: &SceneMark) -> usize {
    match mark {
        SceneMark::Image(_) => 1,
        SceneMark::Group(group) => group.marks.iter().map(count_image_marks_in_mark).sum(),
        _ => 0,
    }
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

async fn evaluate_async_ready_scene(
    compiled: Arc<CompiledPlot>,
    ctx: Arc<SessionContext>,
    param_patch: Option<IndexMap<String, ScalarValue>>,
) -> EvaluatedPlot {
    let mut session = compiled.instantiate(ctx);
    let initial_epoch = session.evaluation_invalidation_epoch();
    let (warmup, warmup_metrics) = session
        .evaluate_with_metrics(exact_request(param_patch.clone()))
        .await
        .expect("warm up view-local raster materialization");

    if warmup_metrics.pipeline.materialization_ready_used == 0 {
        assert!(
            warmup_metrics.pipeline.materialization_queued > 0
                || warmup_metrics.pipeline.materialization_running > 0,
            "initial view raster evaluation should enqueue materialization"
        );
        assert_eq!(
            count_image_marks(&warmup.scene_graph),
            0,
            "initial view raster fallback should not draw empty raster rows"
        );
        wait_for_materialization_invalidation(&session, initial_epoch).await;
    }

    let (ready, ready_metrics) = session
        .evaluate_with_metrics(exact_request(param_patch))
        .await
        .expect("evaluate ready view-local raster materialization");
    assert!(
        ready_metrics.pipeline.materialization_ready_used > 0,
        "ready evaluation should consume a materialized raster"
    );
    assert_eq!(
        ready_metrics.pipeline.materialization_stale_fallback_used, 0,
        "exact ready evaluation should not use stale fallback data"
    );
    assert!(
        count_image_marks(&ready.scene_graph) > 0,
        "ready raster evaluation should render image marks"
    );
    ready
}

async fn evaluate_preview_cached_scene(
    compiled: Arc<CompiledPlot>,
    ctx: Arc<SessionContext>,
    param_patch: IndexMap<String, ScalarValue>,
) -> EvaluatedPlot {
    let mut session = compiled.instantiate(ctx);
    let initial_epoch = session.evaluation_invalidation_epoch();
    let (warmup, warmup_metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .expect("warm up initial view-local raster materialization");
    assert!(
        warmup_metrics.pipeline.materialization_queued > 0
            || warmup_metrics.pipeline.materialization_ready_used > 0,
        "initial view raster evaluation should enqueue or consume materialization"
    );

    if warmup_metrics.pipeline.materialization_ready_used == 0 {
        assert_eq!(
            count_image_marks(&warmup.scene_graph),
            0,
            "initial async fallback should not draw raster rows"
        );
        wait_for_materialization_invalidation(&session, initial_epoch).await;
    }

    let (ready, ready_metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .expect("evaluate initial ready raster");
    assert!(
        ready_metrics.pipeline.materialization_ready_used > 0,
        "warm exact evaluation should render the initial materialized raster"
    );
    assert!(
        count_image_marks(&ready.scene_graph) > 0,
        "initial ready raster should render image marks"
    );

    let (preview, preview_metrics) = session
        .evaluate_with_metrics(preview_request(param_patch))
        .await
        .expect("evaluate preview-cached panned raster");
    assert_eq!(preview_metrics.mode, EvaluationMode::Preview);
    assert_eq!(
        preview_metrics.pipeline.materialization_stale_fallback_used, 1,
        "panned preview should reuse stale materialized raster data"
    );
    assert!(
        preview_metrics.pipeline.materialization_requests_emitted > 0,
        "panned preview should request a fresh raster for the updated domain"
    );
    assert!(
        count_image_marks(&preview.scene_graph) > 0,
        "panned preview should keep drawing the stale raster through current scales"
    );
    preview
}

async fn taxi_dataframe(ctx: &SessionContext) -> DataFrame {
    let taxi_path = format!(
        "{}/tests/data/nyc_taxi_2015/nyc_taxi.csv",
        env!("CARGO_MANIFEST_DIR")
    );
    ctx.read_csv(taxi_path, CsvReadOptions::new())
        .await
        .expect("load NYC taxi fixture")
        .filter(
            col("pickup_x")
                .gt_eq(lit(TAXI_X_MIN))
                .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
                .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
                .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
        )
        .expect("filter taxi fixture to valid projected pickup coordinates")
}

async fn compile_taxi_view_raster_plot(ctx: &SessionContext) -> CompiledPlot {
    let df = taxi_dataframe(ctx).await;
    Chart::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(320.0, 250.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .view(
                    View::cartesian()
                        .id("taxi_density")
                        .x_domain(col("pickup_x"))
                        .y_domain(col("pickup_y"))
                        .preview_cached(true),
                    |mark, view| {
                        mark.transform(
                            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                                .x(|x| {
                                    x.extent(view.x().domain_start(), view.x().domain_end())
                                        .bins(96)
                                })
                                .y(|y| {
                                    y.extent(view.y().domain_start(), view.y().domain_end())
                                        .bins(96)
                                })
                                .agg("count"),
                            |mark, hist| {
                                mark.raster_with(hist.raster(), |r| {
                                    r.x_with(hist.x_dim(), |x| {
                                        x.scale_with::<Linear>(|scale| {
                                            scale.nice(false).zero(false)
                                        })
                                        .axis(|axis| {
                                            axis.title("Pickup x").tick_count(4).format(".4~s")
                                        })
                                    })
                                    .y_with(hist.y_dim(), |y| {
                                        y.scale_with::<Linear>(|scale| {
                                            scale.nice(false).zero(false)
                                        })
                                        .axis(|axis| {
                                            axis.title("Pickup y").tick_count(4).format(".4~s")
                                        })
                                    })
                                    .fill(|fill| {
                                        fill.scale_with::<Sqrt>(|scale| {
                                            scale.domain((0.0, 120.0)).nice(false).zero(false)
                                        })
                                        .legend(|legend| legend.title("Trips"))
                                    })
                                })
                            },
                        )
                    },
                )
                .smooth(false),
        )
        .tool(PanScrollZoom::cartesian())
        .compile(ctx)
        .await
        .expect("compile taxi view raster plot")
}

#[tokio::test]
async fn view_rect_current_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT * FROM (VALUES (0.0, 0.0), (10.0, 8.0)) AS t(x, y)")
        .await
        .expect("build view domain fixture");
    let plot = Chart::<Cartesian>::new()
        .plot_size(260.0, 180.0)
        .data(df)
        .mark(
            Rect::new().view(
                View::cartesian()
                    .id("domain_box")
                    .x_domain(col("x"))
                    .y_domain(col("y")),
                |mark, view| {
                    mark.x_with(view.x().domain_start(), |x| {
                        x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("View x").tick_count(4))
                    })
                    .x2(view.x().domain_end())
                    .y_with(view.y().domain_start(), |y| {
                        y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("View y").tick_count(4))
                    })
                    .y2(view.y().domain_end())
                    .fill("#5ab4ac")
                    .stroke("#0f3b44")
                    .stroke_width(2.0)
                    .opacity(0.72)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile view rect plot");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate view rect plot");
    assert_scene_graph_visual_match(
        &evaluated.scene_graph,
        VIEW_SCOPE_CATEGORY,
        "view_rect_current_domain",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn taxi_pickup_count_async_ready() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_taxi_view_raster_plot(&ctx).await);
    let evaluated = evaluate_async_ready_scene(compiled, ctx, None).await;
    assert_scene_graph_visual_match(
        &evaluated.scene_graph,
        VIEW_RASTER_CATEGORY,
        "taxi_pickup_count_async_ready",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn taxi_pickup_count_async_zoomed_domain() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_taxi_view_raster_plot(&ctx).await);
    let evaluated = evaluate_async_ready_scene(
        compiled,
        ctx,
        Some(pan_scroll_zoom_domain_patch(
            TAXI_ZOOM_X_MIN,
            TAXI_ZOOM_X_MAX,
            TAXI_ZOOM_Y_MIN,
            TAXI_ZOOM_Y_MAX,
        )),
    )
    .await;
    assert_scene_graph_visual_match(
        &evaluated.scene_graph,
        VIEW_RASTER_CATEGORY,
        "taxi_pickup_count_async_zoomed_domain",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn taxi_pickup_count_async_preview_cached_pan_scroll_zoom() {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(compile_taxi_view_raster_plot(&ctx).await);
    let evaluated = evaluate_preview_cached_scene(
        compiled,
        ctx,
        pan_scroll_zoom_domain_patch(
            TAXI_ZOOM_X_MIN,
            TAXI_ZOOM_X_MAX,
            TAXI_ZOOM_Y_MIN,
            TAXI_ZOOM_Y_MAX,
        ),
    )
    .await;
    assert_scene_graph_visual_match(
        &evaluated.scene_graph,
        VIEW_RASTER_CATEGORY,
        "taxi_pickup_count_async_preview_cached_pan_scroll_zoom",
        0.9999,
    )
    .await;
}
