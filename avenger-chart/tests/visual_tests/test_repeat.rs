use avenger_chart::plot::EvaluationRequest;
use avenger_chart::prelude::*;
use avenger_chart::render::{EvaluatedPlot, EvaluationMode};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::common::ScalarValue;
use datafusion::functions_aggregate::expr_fn::count;
use datafusion::prelude::*;
use image::RgbaImage;
use indexmap::IndexMap;
use std::sync::Arc;

use super::helpers::{
    DEFAULT_SCALE, VisualTestConfig, assert_visual_match, assert_visual_match_default,
    compare_images, get_baseline_path,
};

const BASELINE_CATEGORY: &str = "repeat";

async fn repeat_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS a,
            column2 AS b,
            column3 AS c,
            column4 AS score,
            column5 AS group_name
         FROM (VALUES
            (1.0, 3.2, 6.0, 1.0, 'Alpha'),
            (1.5, 2.8, 5.5, 1.6, 'Beta'),
            (2.1, 2.4, 5.0, 2.0, 'Alpha'),
            (2.6, 2.0, 4.6, 2.8, 'Beta'),
            (3.2, 1.7, 4.2, 3.1, 'Alpha'),
            (3.8, 1.3, 3.8, 3.8, 'Beta'),
            (4.3, 1.0, 3.3, 4.3, 'Alpha'),
            (4.9, 0.8, 2.9, 4.9, 'Beta')
         )",
    )
    .await
    .expect("repeat visual data")
}

async fn repeat_three_group_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS a,
            column2 AS b,
            column3 AS c,
            column4 AS score,
            column5 AS group_name
         FROM (VALUES
            (1.0, 3.2, 6.0, 1.0, 'Alpha'),
            (1.5, 2.8, 5.5, 1.6, 'Alpha'),
            (2.1, 2.4, 5.0, 2.0, 'Alpha'),
            (2.6, 2.0, 4.6, 2.8, 'Alpha'),
            (3.2, 1.7, 4.2, 3.1, 'Beta'),
            (3.8, 1.3, 3.8, 3.8, 'Beta'),
            (4.3, 1.0, 3.3, 4.3, 'Beta'),
            (4.9, 0.8, 2.9, 4.9, 'Beta'),
            (0.8, 4.4, 5.7, 1.4, 'Gamma'),
            (1.3, 4.0, 5.1, 2.2, 'Gamma'),
            (1.9, 3.6, 4.7, 3.0, 'Gamma'),
            (2.4, 3.0, 4.1, 3.7, 'Gamma')
         )",
    )
    .await
    .expect("repeat three-group visual data")
}

fn repeat_variables() -> Vec<RepeatVariable> {
    vec![
        RepeatVariable::new("a", col("a")).title("Alpha metric"),
        RepeatVariable::new("b", col("b")).title("Beta metric"),
        RepeatVariable::new("c", col("c")).title("Gamma metric"),
    ]
}

fn repeat_variables_short() -> Vec<RepeatVariable> {
    vec![
        RepeatVariable::new("a", col("a")).title("A"),
        RepeatVariable::new("b", col("b")).title("B"),
        RepeatVariable::new("c", col("c")).title("C"),
    ]
}

fn repeat_variables_four() -> Vec<RepeatVariable> {
    let mut variables = repeat_variables();
    variables.push(RepeatVariable::new("score", col("score")).title("Score"));
    variables
}

fn column_cell() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(repeat::column(), |c| {
                c.axis(|a| a.title(repeat::column_title()))
            })
            .y_with(col("score"), |c| c.axis(|a| a.title("score")))
            .fill_with(col("group_name"), |c| c.legend(|l| l.title("group")))
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(72.0),
    )
}

fn row_cell() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("score"), |c| c.axis(|a| a.title("score")))
            .y_with(repeat::row(), |c| c.axis(|a| a.title(repeat::row_title())))
            .fill_with(col("group_name"), |c| c.legend(|l| l.title("group")))
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(72.0),
    )
}

fn grid_cell() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(repeat::column(), |c| {
                c.axis(|a| a.title(repeat::column_title()))
            })
            .y_with(repeat::row(), |c| c.axis(|a| a.title(repeat::row_title())))
            .fill_with(col("group_name"), |c| c.legend(|l| l.title("group")))
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(52.0),
    )
}

fn grid_cell_no_legend() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(repeat::column(), |c| {
                c.axis(|a| a.title(repeat::column_title()))
            })
            .y_with(repeat::row(), |c| c.axis(|a| a.title(repeat::row_title())))
            .fill("#2f7ed8")
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(52.0),
    )
}

fn grid_cell_matrix_axes() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(repeat::column())
            .y(repeat::row())
            .fill("#2f7ed8")
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(52.0),
    )
}

fn grid_cell_outer_axes_short_titles() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(repeat::column(), |c| {
                c.axis(|a| a.title(repeat::column_title()).grid(true))
            })
            .y_with(repeat::row(), |c| {
                c.axis(|a| a.title(repeat::row_title()).grid(true))
            })
            .fill("#2f7ed8")
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(52.0),
    )
}

fn facet_column_cell_matrix_axes() -> Plot<FacetColumn> {
    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(repeat::column())
            .y(repeat::row())
            .fill("#2f7ed8")
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(46.0),
    );
    Plot::<FacetColumn>::new().mark(Subplot::new(child).column(col("group_name")))
}

fn facet_wrap_cell_matrix_axes() -> Plot<FacetWrap> {
    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(repeat::column())
            .y(repeat::row())
            .fill("#2f7ed8")
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(46.0),
    );
    Plot::<FacetWrap>::new().mark(
        Subplot::new(child).wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
    )
}

fn diagonal_histogram_cell() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(Rect::new().transform(
        Bin::new(repeat::column()).maxbins(5),
        |mark, bin| {
            mark.x_with(bin.start(), |c| c.axis(|a| a.title(repeat::column_title())))
                .x2(bin.end())
                .y_with(lit(0.0), |c| c.axis(|a| a.title("count")))
                .y2(count(lit(1)))
                .fill("#2f7ed8")
                .stroke("#ffffff")
                .stroke_width(1.0)
                .opacity(0.7)
        },
    ))
}

fn diagonal_histogram_cell_matrix_axes() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(Rect::new().transform(
        Bin::new(repeat::column()).maxbins(5),
        |mark, bin| {
            mark.x_with(bin.start(), |c| c.axis(|a| a.title(repeat::column_title())))
                .x2(bin.end())
                .y_with(lit(0.0), |c| c.axis(|a| a.title("count")))
                .y2(count(lit(1)))
                .fill("#2f7ed8")
                .stroke("#ffffff")
                .stroke_width(1.0)
                .opacity(0.7)
        },
    ))
}

fn diagonal_histogram_cell_outer_axes_short_titles() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(Rect::new().transform(
        Bin::new(repeat::column()).maxbins(5),
        |mark, bin| {
            mark.x_with(bin.start(), |c| {
                c.axis(|a| a.title(repeat::column_title()).grid(true))
            })
            .x2(bin.end())
            .y_with(lit(0.0), |c| c.axis(|a| a.title("count").grid(true)))
            .y2(count(lit(1)))
            .fill("#2f7ed8")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .opacity(0.4)
        },
    ))
}

fn wrap_cell() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("score"), |c| c.axis(|a| a.title("score")))
            .y_with(repeat::item(), |c| {
                c.axis(|a| a.title(repeat::item_title()))
            })
            .fill("#2f7ed8")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(72.0),
    )
}

fn wrap_cell_preview_flow() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("score"), |c| {
                c.axis(|a| a.title("score").ticks_start_step(1.0, 2.0))
            })
            .y_with(repeat::item(), |c| {
                c.axis(|a| a.title(repeat::item_title()).ticks_start_step(1.0, 2.0))
            })
            .fill("#2f7ed8")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(72.0),
    )
}

fn responsive_repeat_wrap_inside_facet_column_plot(df: DataFrame) -> Plot<FacetColumn> {
    let width = Param::new("width", ScalarValue::Float64(Some(780.0)));
    let repeat = Plot::<RepeatWrap>::new()
        .items(repeat_variables_four())
        .responsive_columns(230.0)
        .cell(wrap_cell_preview_flow())
        .item_domains();
    Plot::<FacetColumn>::new()
        .add_param(width.clone())
        .data(df)
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(125.0))
        .mark(Subplot::new(repeat).column(col("group_name")))
}

async fn render_evaluated_plot(evaluated: &EvaluatedPlot) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create repeat visual test canvas");
    canvas
        .set_scene(&evaluated.scene_graph)
        .expect("set repeat visual test scene");
    canvas
        .render()
        .await
        .expect("render repeat visual test scene")
}

async fn assert_evaluated_plot_visual_match(evaluated: &EvaluatedPlot, baseline_name: &str) {
    let image = render_evaluated_plot(evaluated).await;
    let baseline_path = get_baseline_path(BASELINE_CATEGORY, baseline_name);
    if let Err(msg) = compare_images(&baseline_path, image, &VisualTestConfig::default()) {
        panic!(
            "Visual test '{}' failed (session rendering): {}",
            baseline_name, msg
        );
    }
}

async fn assert_evaluated_plots_match(
    actual: &EvaluatedPlot,
    expected: &EvaluatedPlot,
    label: &str,
) {
    let actual_image = render_evaluated_plot(actual).await;
    let expected_image = render_evaluated_plot(expected).await;
    assert_eq!(
        actual_image.dimensions(),
        expected_image.dimensions(),
        "{label} dimensions should match"
    );
    let comparison = image_compare::rgba_hybrid_compare(&actual_image, &expected_image)
        .expect("compare repeat evaluated plots");
    assert!(
        comparison.score >= 0.9999,
        "{label} should visually match one-shot Exact, similarity={}",
        comparison.score
    );
}

#[tokio::test]
async fn repeat_columns_three_scatter() {
    let ctx = SessionContext::new();
    let plot = Plot::<RepeatColumns>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(150.0, 140.0)
        .columns(repeat_variables())
        .cell(column_cell());
    let compiled = plot.compile(&ctx).await.expect("compile repeat columns");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_columns_three_scatter",
    )
    .await;
}

#[tokio::test]
async fn repeat_rows_three_scatter() {
    let ctx = SessionContext::new();
    let plot = Plot::<RepeatRows>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(170.0, 115.0)
        .rows(repeat_variables())
        .cell(row_cell());
    let compiled = plot.compile(&ctx).await.expect("compile repeat rows");
    assert_visual_match_default(&compiled, &ctx, None, "repeat", "repeat_rows_three_scatter").await;
}

#[tokio::test]
async fn repeat_grid_scatter_matrix_independent() {
    let ctx = SessionContext::new();
    let variables = repeat_variables();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(120.0, 105.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell());
    let compiled = plot.compile(&ctx).await.expect("compile repeat grid");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_scatter_matrix_independent",
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_scatter_matrix_domains() {
    let ctx = SessionContext::new();
    let variables = repeat_variables();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(120.0, 105.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell())
        .matrix_domains();
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with matrix domains");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_scatter_matrix_domains",
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_matrix_axes_scatter() {
    let ctx = SessionContext::new();
    let variables = repeat_variables();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(120.0, 105.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with matrix axes");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_matrix_axes_scatter",
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_scatter_with_diagonal_histograms() {
    let ctx = SessionContext::new();
    let variables = repeat_variables();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(120.0, 105.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_no_legend())
        .cell_when(
            repeat::row_index().eq(repeat::column_index()),
            diagonal_histogram_cell(),
        );
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with conditional cells");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_scatter_with_diagonal_histograms",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_matrix_axes_diagonal_histograms() {
    let ctx = SessionContext::new();
    let variables = repeat_variables();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(120.0, 105.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_matrix_axes())
        .cell_when(
            repeat::row_index().eq(repeat::column_index()),
            diagonal_histogram_cell_matrix_axes(),
        )
        .matrix_domains()
        .matrix_axes();
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with matrix axes and diagonal histograms");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_matrix_axes_diagonal_histograms",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_matrix_axes_diagonal_histograms_large_short_titles() {
    let ctx = SessionContext::new();
    let variables = repeat_variables_short();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(270.0, 232.5)
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_outer_axes_short_titles())
        .cell_when(
            repeat::row_index().eq(repeat::column_index()),
            diagonal_histogram_cell_outer_axes_short_titles(),
        )
        .matrix_domains()
        .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile large repeat grid with matrix axes and diagonal histograms");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_matrix_axes_diagonal_histograms_large_short_titles",
        0.997,
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_inside_facet_matrix_domains() {
    let ctx = SessionContext::new();
    let variables = repeat_variables()[0..2].to_vec();
    let repeat = Plot::<RepeatGrid>::new()
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let plot = Plot::<FacetColumn>::new()
        .data(repeat_data(&ctx).await)
        .canvas_size(1320.0, 380.0)
        .mark(Subplot::new(repeat).column(col("group_name")));
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted repeat grid");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_inside_facet_matrix_domains",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_inside_facet_row_matrix_domains() {
    let ctx = SessionContext::new();
    let variables = repeat_variables()[0..2].to_vec();
    let repeat = Plot::<RepeatGrid>::new()
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let plot = Plot::<FacetRow>::new()
        .data(repeat_data(&ctx).await)
        .canvas_size(760.0, 820.0)
        .mark(Subplot::new(repeat).row(col("group_name")));
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile facet-row repeat grid");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "repeat_grid_inside_facet_row_matrix_domains",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_grid_inside_facet_wrap_aligned() {
    let ctx = SessionContext::new();
    let variables = repeat_variables()[0..2].to_vec();
    let repeat = Plot::<RepeatGrid>::new()
        .rows(variables.clone())
        .columns(variables)
        .cell(grid_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let plot = Plot::<FacetWrap>::new()
        .data(repeat_three_group_data(&ctx).await)
        .canvas_size(1320.0, 760.0)
        .mark(
            Subplot::new(repeat)
                .wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
        );
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile facet wrap containing repeat grid");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_grid_inside_facet_wrap_aligned",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_wrap_inside_facet_wrap_aligned() {
    let ctx = SessionContext::new();
    let repeat = Plot::<RepeatWrap>::new()
        .items(repeat_variables())
        .columns(2)
        .cell(wrap_cell())
        .item_domains();
    let plot = Plot::<FacetWrap>::new()
        .data(repeat_three_group_data(&ctx).await)
        .canvas_size(1320.0, 920.0)
        .mark(
            Subplot::new(repeat)
                .wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
        );
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile facet wrap containing repeat wrap");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_wrap_inside_facet_wrap_aligned",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn facet_column_inside_repeat_grid_aligned() {
    let ctx = SessionContext::new();
    let variables = repeat_variables()[0..2].to_vec();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_data(&ctx).await)
        .canvas_size(1320.0, 520.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(facet_column_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with faceted cells");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "repeat",
        "facet_column_inside_repeat_grid_aligned",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn facet_wrap_inside_repeat_grid_aligned() {
    let ctx = SessionContext::new();
    let variables = repeat_variables()[0..2].to_vec();
    let plot = Plot::<RepeatGrid>::new()
        .data(repeat_three_group_data(&ctx).await)
        .canvas_size(1320.0, 780.0)
        .rows(variables.clone())
        .columns(variables)
        .cell(facet_wrap_cell_matrix_axes())
        .matrix_domains()
        .matrix_axes();
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeat grid with facet-wrap cells");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "facet_wrap_inside_repeat_grid_aligned",
        0.999,
    )
    .await;
}

#[tokio::test]
async fn repeat_wrap_fixed_columns() {
    let ctx = SessionContext::new();
    let plot = Plot::<RepeatWrap>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(170.0, 130.0)
        .items(repeat_variables())
        .columns(2)
        .cell(wrap_cell());
    let compiled = plot.compile(&ctx).await.expect("compile repeat wrap");
    assert_visual_match_default(&compiled, &ctx, None, "repeat", "repeat_wrap_fixed_columns").await;
}

#[tokio::test]
async fn repeat_wrap_responsive_columns() {
    let ctx = SessionContext::new();
    let plot = Plot::<RepeatWrap>::new()
        .data(repeat_data(&ctx).await)
        .plot_size(500.0, 300.0)
        .items(repeat_variables_four())
        .responsive_columns(210.0)
        .cell(wrap_cell());
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile responsive repeat wrap");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_wrap_responsive_columns",
    )
    .await;
}

#[tokio::test]
async fn repeat_wrap_inside_facet_column_fixed_columns() {
    let ctx = SessionContext::new();
    let repeat = Plot::<RepeatWrap>::new()
        .items(repeat_variables())
        .columns(2)
        .cell(wrap_cell());
    let plot = Plot::<FacetColumn>::new()
        .data(repeat_data(&ctx).await)
        .canvas_size(1320.0, 380.0)
        .mark(Subplot::new(repeat).column(col("group_name")));
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted repeat wrap");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "repeat",
        "repeat_wrap_inside_facet_column_fixed_columns",
    )
    .await;
}

#[tokio::test]
async fn repeat_wrap_inside_facet_column_preview_width_resize_flow() {
    let ctx = Arc::new(SessionContext::new());
    let plot = responsive_repeat_wrap_inside_facet_column_plot(repeat_data(ctx.as_ref()).await);
    let compiled = Arc::new(
        plot.compile(ctx.as_ref())
            .await
            .expect("compile faceted responsive repeat wrap"),
    );
    let mut session = compiled.clone().instantiate(ctx.clone());

    let (initial, exact) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .expect("initial exact faceted responsive repeat wrap");
    assert_eq!(exact.mode, EvaluationMode::Exact);
    assert_evaluated_plot_visual_match(
        &initial,
        "repeat_wrap_inside_facet_column_preview_width_780",
    )
    .await;

    let widths = [
        (1080.0, "repeat_wrap_inside_facet_column_preview_width_1080"),
        (1380.0, "repeat_wrap_inside_facet_column_preview_width_1380"),
    ];
    let mut final_patch = IndexMap::new();

    for (width, name) in widths {
        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(width)));
        final_patch = patch.clone();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await
            .expect("preview faceted responsive repeat wrap width evaluation");

        assert_eq!(metrics.mode, EvaluationMode::Preview);
        assert!(
            metrics.pipeline.preview_structure_reflow_reuses > 0
                || metrics.pipeline.preview_fallbacks > 0,
            "responsive repeat wrap preview should either reflow or conservatively fall back"
        );
        assert!(
            (evaluated.scene_graph.width - width as f32).abs() <= 0.01,
            "preview width patch should update canvas width"
        );
        assert_evaluated_plot_visual_match(&evaluated, name).await;
    }

    let (settled, settled_metrics) = session
        .evaluate_with_metrics(
            EvaluationRequest::new()
                .exact()
                .param_patch(final_patch.clone()),
        )
        .await
        .expect("exact settle after faceted responsive repeat wrap previews");
    assert_eq!(settled_metrics.mode, EvaluationMode::Exact);

    let mut one_shot = compiled.instantiate(ctx);
    let (one_shot_exact, one_shot_metrics) = one_shot
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(final_patch))
        .await
        .expect("one-shot exact faceted responsive repeat wrap");
    assert_eq!(one_shot_metrics.mode, EvaluationMode::Exact);
    assert_evaluated_plots_match(
        &settled,
        &one_shot_exact,
        "faceted responsive repeat wrap exact settle",
    )
    .await;
}
