use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::CartesianGuide;
use avenger_chart::coords::CoordinateSystem;
use avenger_chart::legend::LegendPosition;
use avenger_chart::polar::{PolarGuide, PolarSubplotPositionChannels};
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::average::avg;
use datafusion::prelude::*;

const BASELINE_CATEGORY: &str = "positioned_subplot_legend_sharing";

async fn positioned_legend_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS slot,
            column2 AS parent_x,
            column3 AS parent_y,
            column4 AS parent_r,
            column5 AS parent_theta,
            column6 AS child_x,
            column7 AS child_y,
            column8 AS category
         FROM (VALUES
            ('near', 0.30, 0.54, 0.34, 0.70, 0.10, 0.20, 'Alpha'),
            ('near', 0.30, 0.54, 0.34, 0.70, 0.26, 0.48, 'Alpha'),
            ('near', 0.30, 0.54, 0.34, 0.70, 0.42, 0.78, 'Alpha'),
            ('far',  0.72, 0.46, 0.70, 3.80, 0.58, 0.28, 'Beta'),
            ('far',  0.72, 0.46, 0.70, 3.80, 0.76, 0.56, 'Beta'),
            ('far',  0.72, 0.46, 0.70, 3.80, 0.92, 0.84, 'Beta')
         )",
    )
    .await
    .expect("create positioned subplot legend sharing data")
}

fn shared_legend_child() -> Plot<Cartesian> {
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
                .fill_with(col("category"), |c| {
                    c.with_scale_sharing(CoordinationScope::Shared)
                        .legend(|l| l.title("Category").position(LegendPosition::Right))
                })
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(76.0),
        )
}

fn cartesian_parent_plot(df: DataFrame) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .data(df)
        .plot_size(620.0, 360.0)
        .title("Cartesian positioned shared legend")
        .configure_guide(CartesianGuide::new().plot_background_color("#fffdf6"))
        .mark(
            Subplot::<Cartesian>::new(shared_legend_child())
                .partition_by(col("slot"))
                .subplot_x_with(avg(col("parent_x")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.title("subplot_x"))
                })
                .subplot_y_with(avg(col("parent_y")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.title("subplot_y"))
                })
                .plot_size(132.0, 102.0),
        )
}

fn polar_parent_plot(df: DataFrame) -> Plot<Polar> {
    Plot::<Polar>::new()
        .data(df)
        .plot_size(520.0, 420.0)
        .title("Polar positioned shared legend")
        .configure_guide(PolarGuide::new().plot_background_color("#fffdf6"))
        .mark(
            Subplot::<Polar>::new(shared_legend_child())
                .partition_by(col("slot"))
                .r_with(avg(col("parent_r")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                })
                .theta_with(avg(col("parent_theta")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(std::f64::consts::TAU)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .plot_size(132.0, 102.0),
        )
}

async fn assert_positioned_subplot_legend_baseline<C>(
    name: &'static str,
    make_plot: impl FnOnce(DataFrame) -> Plot<C> + Send + 'static,
) where
    C: CoordinateSystem + 'static,
{
    let ctx = SessionContext::new();
    let plot = make_plot(positioned_legend_data(&ctx).await);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile positioned subplot legend sharing plot");
    assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
}

#[tokio::test]
async fn positioned_subplot_legend_sharing_cartesian_parent_shared_fill_legend() {
    assert_positioned_subplot_legend_baseline(
        "positioned_subplot_legend_sharing_cartesian_parent_shared_fill_legend",
        cartesian_parent_plot,
    )
    .await;
}

#[tokio::test]
async fn positioned_subplot_legend_sharing_polar_parent_shared_fill_legend() {
    assert_positioned_subplot_legend_baseline(
        "positioned_subplot_legend_sharing_polar_parent_shared_fill_legend",
        polar_parent_plot,
    )
    .await;
}
