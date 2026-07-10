//! Shared pieces of the bake server/client example: file locations and the
//! chart definition the server compiles and bakes.

use std::path::PathBuf;

use avenger_chart::prelude::*;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::col, prelude::lit};

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

pub fn parquet_path() -> PathBuf {
    data_dir().join("sales.parquet")
}

pub fn artifact_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifact")
        .join("baked-chart.bin")
}

/// The chart the server bakes: per-region totals over sales rows above a
/// live `$min` threshold. The `$min` filter sits BELOW the aggregate, so the
/// bake embeds the raw rows and the client re-aggregates them per param
/// value — interactively, with zero access to the parquet source.
///
/// `$min` is bound to the cursor's horizontal position, so the baked chart
/// stays interactive through machinery that serializes with the plot: move
/// the cursor across the chart to sweep the threshold.
pub fn sales_threshold_chart(data: DataFrame) -> Plot<Cartesian> {
    let min = Param::new("min", ScalarValue::Float64(Some(0.0)));
    Plot::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .title("Baked sales — move the cursor to sweep the $min threshold")
        .add_params([min])
        .data(data)
        .mark(
            Symbol::new()
                .x(col("region"))
                .y(col("total"))
                .fill(col("region"))
                .size(160.0),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::CursorMoved)
                .set_param(
                    "min",
                    avenger_chart::event::x() / avenger_chart::event::canvas_width() * lit(100.0),
                )
                .throttle_ms(16)
                .exact(),
        )
}

/// SQL evaluated over the `sales` table registered by whichever side owns
/// the data. The scan below the `$min` filter is param-free and folds into
/// the baked artifact; the filter and aggregate stay live above it.
pub const SALES_QUERY: &str = "SELECT region, SUM(value) AS total, COUNT(*) AS orders \
     FROM sales WHERE value >= $min GROUP BY region ORDER BY region";

pub fn init_logging() {
    let _ = env_logger::try_init();
}
