mod common;

use avenger_chart::plot::Plot;
use avenger_chart_treemap::{TreeHeader, TreeLabel, TreeRect, Treemap, TreemapHeaderBars};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3)
            .header_bars(TreemapHeaderBars::enabled().height_px(24.0)),
    )
    .data(ctx.read_batch(common::deep_data())?)
    .plot_size(640.0, 360.0)
    .mark(TreeRect::new().fill(col("region")).stroke("#ffffff"))
    .mark(
        TreeHeader::new()
            .fill(col("division"))
            .stroke("#ffffff")
            .text_color("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    common::evaluate_and_print(&ctx, plot, "decorated treemap with header bars").await
}
