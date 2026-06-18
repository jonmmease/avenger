// Basic wide-form parallel coordinates with line and point overlays.
//
// Run with:
// ```bash
// cargo run -p avenger-chart-app --example parallel_coordinates_basic --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::chart_avenger_app;
use datafusion::prelude::{SessionContext, col};

mod parallel_common;

fn main() {
    parallel_common::run_fixed_window_app("avenger-chart parallel coordinates basic", build_app());
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let plot = Plot::with_coord(parallel_common::demo_parallel_with_segment_axis())
        .canvas_size(
            parallel_common::CANVAS_SIZE[0],
            parallel_common::CANVAS_SIZE[1],
        )
        .plot_size(parallel_common::PLOT_SIZE[0], parallel_common::PLOT_SIZE[1])
        .title("Parallel coordinates")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            ParallelLine::new()
                .details(["sample_id"])
                .stroke(col("segment"))
                .stroke_width(1.15)
                .opacity(0.5),
        )
        .mark(
            ParallelSymbol::new()
                .fill(col("segment"))
                .stroke("#ffffff")
                .stroke_width(0.8)
                .size(23.0)
                .opacity(0.82),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(compiled, ctx, parallel_common::app_options())
        .await
        .expect("build chart app")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_coordinates_basic_app_builds() {
        let _ = build_app().await;
    }
}
