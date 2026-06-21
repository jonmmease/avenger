mod common;

use avenger_chart::plot::Plot;
use avenger_chart_treemap::{
    TreeHeader, TreeLabel, TreeRect, Treemap, TreemapGuide, TreemapHeaderBars,
};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_id("division=Enterprise")
            .display_levels(2)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(ctx.read_batch(common::deep_data())?)
    .plot_size(640.0, 360.0)
    .configure_guide(TreemapGuide::new().breadcrumbs(true).separators(true))
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("region"))
            .stroke("#ffffff")
            .text_color("#ffffff")
            .font_weight("bold"),
    )
    .mark(
        TreeLabel::new()
            .font_size(13.0)
            .font_weight("bold")
            .color("#ffffff"),
    );

    common::evaluate_and_print(&ctx, plot, "zoomed decorated treemap with breadcrumbs").await
}
