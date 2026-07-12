mod common;

use avenger_chart::{channel::LegendableChannel, plot::Chart};
use avenger_chart_treemap::{TreeLabel, TreeRect, Treemap, TreemapGuide};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let plot = Chart::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3),
    )
    .data(ctx.read_batch(common::deep_data())?)
    .plot_size(640.0, 360.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(false),
    )
    .mark(
        TreeRect::new()
            .fill_with(col("division"), |fill| {
                fill.legend(|legend| legend.title("Division"))
            })
            .stroke("#ffffff"),
    )
    .mark(TreeLabel::new().color("#ffffff"));

    common::evaluate_and_print(&ctx, plot, "strict area treemap with legend and labels").await
}
