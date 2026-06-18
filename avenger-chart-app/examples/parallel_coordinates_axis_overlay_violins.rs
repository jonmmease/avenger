// Parallel coordinates with semi-transparent violin overlays on each axis.
//
// Each violin is a compound Cartesian mark inside a `ParallelAxisOverlay`.
// The child plot inherits parent rows and uses the injected per-dimension y
// scale, while x is a single local band value.
//
// Run with:
// ```bash
// cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_violins --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::chart_avenger_app;
use datafusion::prelude::{SessionContext, col};

mod parallel_common;

fn main() {
    parallel_common::run_fixed_window_app(
        "avenger-chart parallel axis overlay violins",
        build_app(),
    );
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let mut plot = Plot::with_coord(parallel_common::demo_parallel())
        .canvas_size(
            parallel_common::CANVAS_SIZE[0],
            parallel_common::CANVAS_SIZE[1],
        )
        .plot_size(parallel_common::PLOT_SIZE[0], parallel_common::PLOT_SIZE[1])
        .title("Axis-local violin overlays")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            ParallelLine::new()
                .details(["sample_id"])
                .stroke("#cbd5e1")
                .stroke_width(0.8)
                .opacity(0.2),
        );

    for dimension in parallel_common::NUMERIC_DIMENSIONS {
        plot = plot.mark(violin_overlay(dimension));
    }

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(compiled, ctx, parallel_common::app_options())
        .await
        .expect("build chart app")
}

fn violin_overlay(
    dimension: parallel_common::ParallelDemoDimension,
) -> ParallelAxisOverlay<Parallel> {
    let violin = Violin::new()
        .vertical()
        .x_with(col(parallel_common::AXIS_GROUP_FIELD), |x| {
            x.axis(|axis| axis.visible(false))
        })
        .y_with(col(dimension.field), |y| y.axis(|axis| axis.visible(false)))
        .steps(48.0)
        .width(0.92)
        .width_normalization(ViolinWidthNormalization::Shared)
        .fill("rgba(37, 99, 235, 0.24)")
        .stroke("#1d4ed8")
        .stroke_width(1.0)
        .opacity(0.92);

    ParallelAxisOverlay::new(dimension.id, Plot::<Cartesian>::new().mark(violin))
        .width_px(54.0)
        .zindex(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_coordinates_axis_overlay_violins_app_builds() {
        let _ = build_app().await;
    }
}
