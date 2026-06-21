mod common;

use avenger_chart::{channel::LegendableChannel, plot::Plot};
use avenger_chart_treemap::{TreeLabel, TreeRect, Treemap};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .display_levels(3),
    )
    .data(ctx.read_batch(common::deep_data())?)
    .plot_size(640.0, 360.0)
    .mark(
        TreeRect::new()
            .fill_with(col("division"), |fill| {
                fill.legend(|legend| legend.title("Division"))
            })
            .stroke("#ffffff"),
    )
    .mark(
        TreeLabel::new()
            .font_size(13.0)
            .font_weight("bold")
            .color("#ffffff"),
    );

    common::evaluate_and_print(&ctx, plot, "color-by-parent treemap with leaf labels").await
}
