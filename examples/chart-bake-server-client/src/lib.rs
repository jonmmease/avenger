//! Shared pieces of the bake server/client example: file locations and the
//! chart definition the server compiles and bakes.

use std::path::PathBuf;

use avenger_chart::prelude::*;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::col, prelude::lit};

pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

pub fn parquet_path() -> PathBuf {
    data_dir().join("trips.parquet")
}

pub fn artifact_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifact")
        .join("baked-chart.bin")
}

/// Upper end of the cursor-driven `$min` sweep; daily totals in the
/// generated data top out a little above this.
///
/// The pipeline the server bakes is daily `SUM(value)` per (region, day)
/// with the live `$min` threshold ABOVE the aggregate: the aggregate is
/// param-free, so partial evaluation executes it once at bake time and the
/// artifact embeds only the few thousand pre-aggregated rows instead of the
/// millions of raw events. The client's cost per interaction is a filter
/// over the small baked table.
pub const MIN_SWEEP_MAX: f64 = 100_000.0;

/// The chart: one point per (region, day) daily total, colored by region,
/// with `$min` bound to the cursor's horizontal position. The binding
/// serializes with the plot, so the baked chart stays interactive in a
/// session that has never seen the data: move the cursor across the chart
/// to sweep the threshold.
pub fn daily_totals_chart(data: DataFrame) -> Chart<Cartesian> {
    let min = {
        let __avenger_param_name = "min";
        let __avenger_param_default: datafusion::common::ScalarValue =
            (ScalarValue::Float64(Some(0.0))).into();
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };
    Chart::<Cartesian>::new()
        .canvas_size(860.0, 520.0)
        .title("Baked daily totals — move the cursor to sweep the $min threshold")
        .params([min])
        .data(data)
        .mark(
            Symbol::new()
                .x(col("day"))
                .y(col("total"))
                .fill(col("region"))
                .size(24.0),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::CursorMoved)
                .set_param(
                    "min",
                    avenger_chart::event::x() / avenger_chart::event::canvas_width()
                        * lit(MIN_SWEEP_MAX),
                )
                .throttle_ms(16)
                .exact(),
        )
}

pub fn init_logging() {
    let _ = env_logger::try_init();
}
