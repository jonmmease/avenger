use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::guide::CartesianGuide;
use avenger_chart::prelude::*;
use datafusion::prelude::*;
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

fn positioned_child_plot(x_sharing_level: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#f8fbff"))
        .mark(
            Symbol::<Cartesian>::new()
                .x_with(col("child_x"), move |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .with_scale_sharing(ScaleSharing::Level(x_sharing_level))
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

fn positioned_parent_plot(x_sharing_level: u8) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .configure_guide(CartesianGuide::new().plot_background_color("#fffdf6"))
        .mark(
            Subplot::<Cartesian>::new(positioned_child_plot(x_sharing_level))
                .partition_by(col("parent_x"))
                .x_with(col("parent_x"), |c| {
                    c.with_scale_name("subplot_parent_x")
                        .scale_with::<Linear>(|s| {
                            s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                        })
                        .axis(|a| a.tick_count(3).show_title(false))
                })
                .y_with(lit(0.5), |c| {
                    c.with_scale_name("subplot_parent_y")
                        .scale_with::<Linear>(|s| {
                            s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                        })
                        .axis(|a| a.tick_count(3).show_title(false))
                })
                .plot_size(142.0, 104.0),
        )
}

fn row_col_positioned_subplot_plot(df: DataFrame, x_sharing_level: u8) -> Plot<FacetRow> {
    Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(1600.0, 1050.0)
        .title(format!(
            "Positioned subplot x sharing level {x_sharing_level}"
        ))
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(positioned_parent_plot(x_sharing_level))
                        .col_with(col("facet_col"), |c| c.facet(|f| f.title("Column"))),
                ),
            )
            .row_with(col("facet_row"), |c| c.facet(|f| f.title("Row"))),
        )
}

fn assert_x_sharing_baseline(name: &'static str, x_sharing_level: u8) {
    run_with_large_stack(move || async move {
        let ctx = SessionContext::new();
        let plot = row_col_positioned_subplot_plot(
            positioned_subplot_sharing_data(&ctx).await,
            x_sharing_level,
        );
        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile row/column positioned subplot sharing plot");
        assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
    });
}

#[test]
fn facet_row_col_cartesian_subplots_x_level_0_free() {
    assert_x_sharing_baseline("facet_row_col_cartesian_subplots_x_level_0_free", 0);
}

#[test]
fn facet_row_col_cartesian_subplots_x_level_1_cartesian() {
    assert_x_sharing_baseline("facet_row_col_cartesian_subplots_x_level_1_cartesian", 1);
}

#[test]
fn facet_row_col_cartesian_subplots_x_level_2_row() {
    assert_x_sharing_baseline("facet_row_col_cartesian_subplots_x_level_2_row", 2);
}

#[test]
fn facet_row_col_cartesian_subplots_x_level_3_global() {
    assert_x_sharing_baseline("facet_row_col_cartesian_subplots_x_level_3_global", 3);
}
