use super::helpers::{
    DEFAULT_SCALE, VisualTestConfig, assert_visual_match_default, compare_images, get_baseline_path,
};
use avenger_chart::cartesian::CartesianGuide;
use avenger_chart::plot::EvaluationRequest;
use avenger_chart::prelude::*;
use avenger_chart::render::EvaluatedPlot;
use avenger_chart_scales::{Linear, Ordinal};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::common::ScalarValue;
use datafusion::functions_aggregate::min_max::max;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

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

async fn nested_row_wrap_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            ('North', 'Alpha',  0.2, 0.10, 'low',  15.0),
            ('North', 'Alpha',  0.8, 0.35, 'mid',  15.0),
            ('North', 'Alpha',  1.4, 0.70, 'high', 15.0),
            ('North', 'Bravo',  8.2, 0.25, 'mid',  24.0),
            ('North', 'Bravo',  8.8, 0.55, 'high', 24.0),
            ('North', 'Bravo',  9.4, 0.95, 'low',  24.0),
            ('North', 'Cedar', 16.2, 0.20, 'high', 34.0),
            ('North', 'Cedar', 16.8, 0.50, 'low',  34.0),
            ('North', 'Cedar', 17.4, 0.85, 'mid',  34.0),
            ('North', 'Delta', 24.2, 0.15, 'low',  44.0),
            ('North', 'Delta', 24.8, 0.45, 'mid',  44.0),
            ('North', 'Delta', 25.4, 0.80, 'high', 44.0),
            ('North', 'Ember', 32.2, 0.30, 'mid',  54.0),
            ('North', 'Ember', 32.8, 0.60, 'high', 54.0),
            ('North', 'Ember', 33.4, 1.00, 'low',  54.0),
            ('South', 'Alpha', 100.2, 20.2, 'low',  18.0),
            ('South', 'Alpha', 100.8, 20.6, 'mid',  18.0),
            ('South', 'Alpha', 101.4, 21.1, 'high', 18.0),
            ('South', 'Bravo', 112.2, 20.4, 'mid',  28.0),
            ('South', 'Bravo', 112.8, 21.0, 'high', 28.0),
            ('South', 'Bravo', 113.4, 21.8, 'low',  28.0),
            ('South', 'Cedar', 124.2, 20.1, 'high', 38.0),
            ('South', 'Cedar', 124.8, 21.3, 'low',  38.0),
            ('South', 'Cedar', 125.4, 22.5, 'mid',  38.0),
            ('South', 'Delta', 136.2, 20.5, 'low',  48.0),
            ('South', 'Delta', 136.8, 22.1, 'mid',  48.0),
            ('South', 'Delta', 137.4, 23.0, 'high', 48.0),
            ('South', 'Ember', 148.2, 20.9, 'mid',  58.0),
            ('South', 'Ember', 148.8, 22.8, 'high', 58.0),
            ('South', 'Ember', 149.4, 24.2, 'low',  58.0)
        ) AS t(region, facet, x, y, group_name, rank_score)",
    )
    .await
    .expect("nested row wrap data")
}

async fn nested_row_wrap_independent_order_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT * FROM (VALUES
            ('North', 'Alpha',  0.2, 0.10, 'low',  15.0),
            ('North', 'Alpha',  0.8, 0.35, 'mid',  15.0),
            ('North', 'Alpha',  1.4, 0.70, 'high', 15.0),
            ('North', 'Bravo',  8.2, 0.25, 'mid',  24.0),
            ('North', 'Bravo',  8.8, 0.55, 'high', 24.0),
            ('North', 'Bravo',  9.4, 0.95, 'low',  24.0),
            ('North', 'Cedar', 16.2, 0.20, 'high', 34.0),
            ('North', 'Cedar', 16.8, 0.50, 'low',  34.0),
            ('North', 'Cedar', 17.4, 0.85, 'mid',  34.0),
            ('North', 'Delta', 24.2, 0.15, 'low',  44.0),
            ('North', 'Delta', 24.8, 0.45, 'mid',  44.0),
            ('North', 'Delta', 25.4, 0.80, 'high', 44.0),
            ('North', 'Ember', 32.2, 0.30, 'mid',  54.0),
            ('North', 'Ember', 32.8, 0.60, 'high', 54.0),
            ('North', 'Ember', 33.4, 1.00, 'low',  54.0),
            ('South', 'Alpha', 100.2, 20.2, 'low',  58.0),
            ('South', 'Alpha', 100.8, 20.6, 'mid',  58.0),
            ('South', 'Alpha', 101.4, 21.1, 'high', 58.0),
            ('South', 'Bravo', 112.2, 20.4, 'mid',  48.0),
            ('South', 'Bravo', 112.8, 21.0, 'high', 48.0),
            ('South', 'Bravo', 113.4, 21.8, 'low',  48.0),
            ('South', 'Cedar', 124.2, 20.1, 'high', 38.0),
            ('South', 'Cedar', 124.8, 21.3, 'low',  38.0),
            ('South', 'Cedar', 125.4, 22.5, 'mid',  38.0),
            ('South', 'Delta', 136.2, 20.5, 'low',  28.0),
            ('South', 'Delta', 136.8, 22.1, 'mid',  28.0),
            ('South', 'Delta', 137.4, 23.0, 'high', 28.0),
            ('South', 'Ember', 148.2, 20.9, 'mid',  18.0),
            ('South', 'Ember', 148.8, 22.8, 'high', 18.0),
            ('South', 'Ember', 149.4, 24.2, 'low',  18.0)
        ) AS t(region, facet, x, y, group_name, rank_score)",
    )
    .await
    .expect("nested row wrap independent order data")
}

fn wrap_leaf_plot(x_sharing: u8, y_sharing: u8, fill_sharing: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#fffefa"))
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.with_domain_scope(CoordinationScope::Level(x_sharing))
                        .scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("x"))
                })
                .y_with(col("y"), move |c| {
                    c.with_domain_scope(CoordinationScope::Level(y_sharing))
                        .scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("y"))
                })
                .fill_with(col("group_name"), move |c| {
                    c.with_domain_scope(CoordinationScope::Level(fill_sharing))
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

fn responsive_facet_wrap_plot(df: DataFrame, canvas_width: f32) -> Plot<FacetWrap> {
    Plot::<FacetWrap>::new()
        .data(df)
        .canvas_constraint(CanvasConstraint::width(canvas_width))
        .plot_constraint(PlotConstraint::height(135.0))
        .mark(
            Subplot::new(wrap_leaf_plot(1, 1, 1)).wrap_with(col("facet"), |c| {
                c.responsive_columns(220.0).guide(|g| g.title("Facet"))
            }),
        )
}

fn responsive_facet_wrap_session_plot(df: DataFrame) -> Plot<FacetWrap> {
    let width = Param::new("width", ScalarValue::Float64(Some(520.0)));
    Plot::<FacetWrap>::new()
        .add_param(width.clone())
        .data(df)
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(135.0))
        .mark(
            Subplot::new(wrap_leaf_plot(1, 1, 1)).wrap_with(col("facet"), |c| {
                c.responsive_columns(220.0).guide(|g| g.title("Facet"))
            }),
        )
}

async fn assert_evaluated_plot_visual_match(evaluated: &EvaluatedPlot, baseline_name: &str) {
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create visual test canvas");
    canvas
        .set_scene(&evaluated.scene_graph)
        .expect("set visual test scene");
    let image = canvas.render().await.expect("render visual test scene");
    let baseline_path = get_baseline_path(BASELINE_CATEGORY, baseline_name);
    if let Err(msg) = compare_images(&baseline_path, image, &VisualTestConfig::default()) {
        panic!(
            "Visual test '{}' failed (session rendering): {}",
            baseline_name, msg
        );
    }
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

fn nested_row_wrap_plot(df: DataFrame) -> Plot<FacetRow> {
    let wrap = Plot::<FacetWrap>::new().mark(Subplot::new(wrap_leaf_plot(1, 1, 1)).wrap_with(
        col("facet"),
        |c| {
            c.columns(3)
                .order_by(max(col("rank_score")))
                .order_desc()
                .guide(|g| g.title("Wrapped facet"))
        },
    ));

    Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(1120.0, 920.0)
        .mark(Subplot::new(wrap).row_with(col("region"), |c| c.guide(|g| g.title("Region"))))
}

fn nested_column_wrap_plot(df: DataFrame) -> Plot<FacetColumn> {
    let wrap = Plot::<FacetWrap>::new().mark(Subplot::new(wrap_leaf_plot(1, 1, 2)).wrap_with(
        col("facet"),
        |c| {
            c.columns(3)
                .order_by(max(col("rank_score")))
                .order_desc()
                .guide(|g| g.title("Wrapped facet"))
        },
    ));

    Plot::<FacetColumn>::new()
        .data(df)
        .canvas_size(1420.0, 760.0)
        .mark(Subplot::new(wrap).column_with(col("region"), |c| c.guide(|g| g.title("Region"))))
}

#[tokio::test]
async fn facet_row_wrap_nested_level_1_sharing() {
    let ctx = SessionContext::new();
    let plot = nested_row_wrap_plot(nested_row_wrap_data(&ctx).await);
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "facet_row_wrap_nested_level_1_sharing",
    )
    .await;
}

async fn assert_responsive_facet_wrap_baseline(name: &str, canvas_width: f32) {
    let ctx = SessionContext::new();
    let plot = responsive_facet_wrap_plot(facet_wrap_data(&ctx).await, canvas_width);
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
}

#[tokio::test]
async fn facet_column_wrap_nested_independent_ordering() {
    let ctx = SessionContext::new();
    let plot = nested_column_wrap_plot(nested_row_wrap_independent_order_data(&ctx).await);
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "facet_column_wrap_nested_independent_ordering",
    )
    .await;
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

#[tokio::test]
async fn facet_wrap_responsive_columns_narrow() {
    assert_responsive_facet_wrap_baseline("facet_wrap_responsive_columns_narrow", 640.0).await;
}

#[tokio::test]
async fn facet_wrap_responsive_columns_wide() {
    assert_responsive_facet_wrap_baseline("facet_wrap_responsive_columns_wide", 1180.0).await;
}

#[tokio::test]
async fn facet_wrap_preview_width_resize_flow() {
    let ctx = Arc::new(SessionContext::new());
    let plot = responsive_facet_wrap_session_plot(facet_wrap_data(ctx.as_ref()).await);
    let compiled = Arc::new(
        plot.compile(ctx.as_ref())
            .await
            .expect("compile preview flow"),
    );
    let mut session = compiled.instantiate(ctx);

    let (initial, exact) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .expect("initial exact responsive wrap evaluation");
    assert_eq!(exact.mode, EvaluationMode::Exact);
    assert_evaluated_plot_visual_match(&initial, "facet_wrap_preview_flow_width_520_cols_2").await;

    for (width, name) in [
        (700.0, "facet_wrap_preview_flow_width_700_cols_3"),
        (920.0, "facet_wrap_preview_flow_width_920_cols_4"),
        (1180.0, "facet_wrap_preview_flow_width_1180_cols_5"),
    ] {
        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(width)));
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await
            .expect("preview responsive wrap width evaluation");

        assert_eq!(metrics.mode, EvaluationMode::Preview);
        assert_eq!(metrics.pipeline.preview_profile_reuses, 1);
        assert_eq!(metrics.pipeline.preview_profile_misses, 0);
        assert_eq!(metrics.pipeline.preview_fallbacks, 0);
        assert_eq!(metrics.pipeline.preview_structure_reflow_reuses, 1);
        assert!(
            metrics.pipeline.facet_cell_measurement_profile_reuses > 0,
            "responsive wrap preview should reuse terminal cell profiles"
        );
        assert!(
            metrics.facet_layout.plot_component_measure_calls > 0,
            "responsive wrap structure changes should rebuild container layout"
        );
        assert!(
            (evaluated.scene_graph.width - width as f32).abs() <= 0.01,
            "preview width patch should update canvas width"
        );
        assert_evaluated_plot_visual_match(&evaluated, name).await;
    }
}
