use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::guide::CartesianGuide;
use avenger_chart::prelude::*;
use avenger_chart::scales::ScaleRange;
use datafusion::prelude::*;
use palette::rgb::Srgba;
use std::future::Future;

const BASELINE_CATEGORY: &str = "positioned_subplot_sharing";

fn run_with_large_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("positioned-subplot-sharing-visual-large-stack".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for positioned subplot sharing visual test");
            rt.block_on(f());
        })
        .expect("spawn large-stack positioned subplot sharing visual test thread")
        .join()
        .expect("large-stack positioned subplot sharing visual test panicked");
}

async fn positioned_subplot_sharing_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS facet_row,
            column2 AS facet_col,
            column3 AS subplot_slot,
            column4 AS parent_x,
            column5 AS parent_y,
            column6 AS child_x,
            column7 AS child_y
         FROM (VALUES
            ('North', 'West', 'near', 0.33, 0.50,   0.2, 0.25),
            ('North', 'West', 'near', 0.33, 0.50,   1.0, 0.55),
            ('North', 'West', 'near', 0.33, 0.50,   1.8, 0.85),
            ('North', 'West', 'far',  0.67, 0.50,  10.2, 0.25),
            ('North', 'West', 'far',  0.67, 0.50,  11.0, 0.55),
            ('North', 'West', 'far',  0.67, 0.50,  11.8, 0.85),
            ('North', 'East', 'near', 0.33, 0.50,  30.2, 0.25),
            ('North', 'East', 'near', 0.33, 0.50,  31.0, 0.55),
            ('North', 'East', 'near', 0.33, 0.50,  31.8, 0.85),
            ('North', 'East', 'far',  0.67, 0.50,  40.2, 0.25),
            ('North', 'East', 'far',  0.67, 0.50,  41.0, 0.55),
            ('North', 'East', 'far',  0.67, 0.50,  41.8, 0.85),
            ('South', 'West', 'near', 0.33, 0.50,  80.2, 0.25),
            ('South', 'West', 'near', 0.33, 0.50,  81.0, 0.55),
            ('South', 'West', 'near', 0.33, 0.50,  81.8, 0.85),
            ('South', 'West', 'far',  0.67, 0.50,  90.2, 0.25),
            ('South', 'West', 'far',  0.67, 0.50,  91.0, 0.55),
            ('South', 'West', 'far',  0.67, 0.50,  91.8, 0.85),
            ('South', 'East', 'near', 0.33, 0.50, 110.2, 0.25),
            ('South', 'East', 'near', 0.33, 0.50, 111.0, 0.55),
            ('South', 'East', 'near', 0.33, 0.50, 111.8, 0.85),
            ('South', 'East', 'far',  0.67, 0.50, 120.2, 0.25),
            ('South', 'East', 'far',  0.67, 0.50, 121.0, 0.55),
            ('South', 'East', 'far',  0.67, 0.50, 121.8, 0.85)
         )",
    )
    .await
    .expect("create positioned subplot sharing data")
}

async fn positioned_subplot_fill_sharing_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS facet_row,
            column2 AS facet_col,
            column3 AS parent_x,
            column4 AS child_x,
            column5 AS child_y,
            column6 AS fill_category
         FROM (VALUES
            ('North', 'West', 0.33, 0.20, 0.24, 'N-W-near'),
            ('North', 'West', 0.33, 0.50, 0.52, 'N-W-near'),
            ('North', 'West', 0.33, 0.80, 0.80, 'N-W-near'),
            ('North', 'West', 0.67, 0.20, 0.24, 'N-W-far'),
            ('North', 'West', 0.67, 0.50, 0.52, 'N-W-far'),
            ('North', 'West', 0.67, 0.80, 0.80, 'N-W-far'),
            ('North', 'East', 0.33, 0.20, 0.24, 'N-E-near'),
            ('North', 'East', 0.33, 0.50, 0.52, 'N-E-near'),
            ('North', 'East', 0.33, 0.80, 0.80, 'N-E-near'),
            ('North', 'East', 0.67, 0.20, 0.24, 'N-E-far'),
            ('North', 'East', 0.67, 0.50, 0.52, 'N-E-far'),
            ('North', 'East', 0.67, 0.80, 0.80, 'N-E-far'),
            ('South', 'West', 0.33, 0.20, 0.24, 'S-W-near'),
            ('South', 'West', 0.33, 0.50, 0.52, 'S-W-near'),
            ('South', 'West', 0.33, 0.80, 0.80, 'S-W-near'),
            ('South', 'West', 0.67, 0.20, 0.24, 'S-W-far'),
            ('South', 'West', 0.67, 0.50, 0.52, 'S-W-far'),
            ('South', 'West', 0.67, 0.80, 0.80, 'S-W-far'),
            ('South', 'East', 0.33, 0.20, 0.24, 'S-E-near'),
            ('South', 'East', 0.33, 0.50, 0.52, 'S-E-near'),
            ('South', 'East', 0.33, 0.80, 0.80, 'S-E-near'),
            ('South', 'East', 0.67, 0.20, 0.24, 'S-E-far'),
            ('South', 'East', 0.67, 0.50, 0.52, 'S-E-far'),
            ('South', 'East', 0.67, 0.80, 0.80, 'S-E-far')
         )",
    )
    .await
    .expect("create positioned subplot fill sharing data")
}

fn fill_sharing_palette() -> ScaleRange {
    ScaleRange::new_color(vec![
        Srgba::new(0.121, 0.466, 0.705, 1.0),
        Srgba::new(1.000, 0.498, 0.054, 1.0),
        Srgba::new(0.172, 0.627, 0.172, 1.0),
        Srgba::new(0.839, 0.153, 0.157, 1.0),
        Srgba::new(0.580, 0.404, 0.741, 1.0),
        Srgba::new(0.549, 0.337, 0.294, 1.0),
        Srgba::new(0.890, 0.467, 0.761, 1.0),
        Srgba::new(0.498, 0.498, 0.498, 1.0),
    ])
}

fn positioned_child_plot() -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#f8fbff"))
        .mark(
            Symbol::<Cartesian>::new()
                .x_with(col("child_x"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.tick_count(3).show_title(false))
                })
                .y_with(col("child_y"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.tick_count(3).show_title(false))
                })
                .fill("#0072b2")
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(54.0),
        )
}

fn positioned_fill_child_plot(fill_sharing_level: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#f8fbff"))
        .mark(
            Symbol::<Cartesian>::new()
                .x_with(col("child_x"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.tick_count(3).show_title(false))
                })
                .y_with(col("child_y"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.tick_count(3).show_title(false))
                })
                .fill_with(col("fill_category"), move |c| {
                    c.with_scale_sharing(ScaleSharing::Level(fill_sharing_level))
                        .scale_with::<Ordinal>(|s| s.range(fill_sharing_palette()))
                        .no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(78.0),
        )
}

fn positioned_parent_plot(
    subplot_x_sharing_level: u8,
    subplot_y_sharing_level: u8,
) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#fffdf6"))
        .mark(
            Subplot::<Cartesian>::new(positioned_child_plot())
                .partition_by(col("parent_x"))
                .subplot_x_with(col("parent_x"), move |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .with_scale_sharing(ScaleSharing::Level(subplot_x_sharing_level))
                    .axis(|a| a.title("subplot_x"))
                })
                .subplot_y_with(lit(0.5), move |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .with_scale_sharing(ScaleSharing::Level(subplot_y_sharing_level))
                    .axis(|a| a.title("subplot_y"))
                })
                .plot_size(142.0, 104.0),
        )
}

fn positioned_fill_parent_plot(fill_sharing_level: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#fffdf6"))
        .mark(
            Subplot::<Cartesian>::new(positioned_fill_child_plot(fill_sharing_level))
                .partition_by(col("parent_x"))
                .subplot_x_with(col("parent_x"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                })
                .subplot_y_with(lit(0.5), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                })
                .plot_size(142.0, 104.0),
        )
}

fn row_col_positioned_subplot_plot(
    df: DataFrame,
    subplot_x_sharing_level: u8,
    subplot_y_sharing_level: u8,
) -> Plot<FacetRow> {
    Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(1600.0, 1050.0)
        .title(format!(
            "Positioned subplot sharing levels x{subplot_x_sharing_level} y{subplot_y_sharing_level}"
        ))
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(positioned_parent_plot(
                        subplot_x_sharing_level,
                        subplot_y_sharing_level,
                    ))
                        .col_with(col("facet_col"), |c| c.facet(|f| f.title("Column"))),
                ),
            )
            .row_with(col("facet_row"), |c| c.facet(|f| f.title("Row"))),
        )
}

fn row_col_positioned_subplot_fill_plot(df: DataFrame, fill_sharing_level: u8) -> Plot<FacetRow> {
    Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(1600.0, 1050.0)
        .title(format!(
            "Positioned subplot fill sharing level {fill_sharing_level}"
        ))
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(positioned_fill_parent_plot(fill_sharing_level))
                        .col_with(col("facet_col"), |c| c.facet(|f| f.title("Column"))),
                ),
            )
            .row_with(col("facet_row"), |c| c.facet(|f| f.title("Row"))),
        )
}

fn assert_xy_sharing_baseline(
    name: &'static str,
    subplot_x_sharing_level: u8,
    subplot_y_sharing_level: u8,
) {
    run_with_large_stack(move || async move {
        let ctx = SessionContext::new();
        let plot = row_col_positioned_subplot_plot(
            positioned_subplot_sharing_data(&ctx).await,
            subplot_x_sharing_level,
            subplot_y_sharing_level,
        );
        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile row/column positioned subplot sharing plot");
        assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
    });
}

fn assert_fill_sharing_baseline(name: &'static str, fill_sharing_level: u8) {
    run_with_large_stack(move || async move {
        let ctx = SessionContext::new();
        let plot = row_col_positioned_subplot_fill_plot(
            positioned_subplot_fill_sharing_data(&ctx).await,
            fill_sharing_level,
        );
        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile row/column positioned subplot fill sharing plot");
        assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
    });
}

#[test]
fn facet_row_col_cartesian_subplots_xy_levels_x0_y3() {
    assert_xy_sharing_baseline("facet_row_col_cartesian_subplots_xy_levels_x0_y3", 0, 3);
}

#[test]
fn facet_row_col_cartesian_subplots_xy_levels_x1_y2() {
    assert_xy_sharing_baseline("facet_row_col_cartesian_subplots_xy_levels_x1_y2", 1, 2);
}

#[test]
fn facet_row_col_cartesian_subplots_xy_levels_x2_y1() {
    assert_xy_sharing_baseline("facet_row_col_cartesian_subplots_xy_levels_x2_y1", 2, 1);
}

#[test]
fn facet_row_col_cartesian_subplots_xy_levels_x3_y0() {
    assert_xy_sharing_baseline("facet_row_col_cartesian_subplots_xy_levels_x3_y0", 3, 0);
}

#[test]
fn facet_row_col_cartesian_subplots_fill_level_0_free() {
    assert_fill_sharing_baseline("facet_row_col_cartesian_subplots_fill_level_0_free", 0);
}

#[test]
fn facet_row_col_cartesian_subplots_fill_level_1_cartesian() {
    assert_fill_sharing_baseline("facet_row_col_cartesian_subplots_fill_level_1_cartesian", 1);
}

#[test]
fn facet_row_col_cartesian_subplots_fill_level_2_row() {
    assert_fill_sharing_baseline("facet_row_col_cartesian_subplots_fill_level_2_row", 2);
}

#[test]
fn facet_row_col_cartesian_subplots_fill_level_3_global() {
    assert_fill_sharing_baseline("facet_row_col_cartesian_subplots_fill_level_3_global", 3);
}
