use avenger_chart::prelude::*;
use datafusion::functions_aggregate::expr_fn::count;
use datafusion::prelude::*;

use super::helpers::{assert_visual_match, assert_visual_match_default};

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

fn repeat_variables() -> Vec<RepeatVariable> {
    vec![
        RepeatVariable::new("a", col("a")).title("Alpha metric"),
        RepeatVariable::new("b", col("b")).title("Beta metric"),
        RepeatVariable::new("c", col("c")).title("Gamma metric"),
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
