// Parallel coordinates with axis-local statistical summaries.
//
// Each overlay is an ordinary Cartesian child plot. It inherits the parent
// data, receives the selected dimension's y scale, and uses a one-value local
// band scale on x for the box plot group.
//
// Run with:
// ```bash
// cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_summary --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::chart_avenger_app;
use datafusion::prelude::{SessionContext, col};

mod parallel_common;

fn main() {
    parallel_common::run_fixed_window_app(
        "avenger-chart parallel axis overlay summary",
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
        .title("Axis-local box summaries")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            ParallelLine::new()
                .details(["sample_id"])
                .stroke("#c4cbd5")
                .stroke_width(0.9)
                .opacity(0.25),
        );

    for dimension in parallel_common::NUMERIC_DIMENSIONS {
        plot = plot.mark(summary_overlay(dimension));
    }

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(compiled, ctx, parallel_common::app_options())
        .await
        .expect("build chart app")
}

fn summary_overlay(
    dimension: parallel_common::ParallelDemoDimension,
) -> ParallelAxisOverlay<Parallel> {
    let box_plot = BoxPlot::new()
        .vertical()
        .x_with(col(parallel_common::AXIS_GROUP_FIELD), |x| {
            x.axis(|axis| axis.visible(false))
        })
        .y_with(col(dimension.field), |y| y.axis(|axis| axis.visible(false)))
        .fill("rgba(37, 99, 235, 0.30)")
        .box_stroke("#1d4ed8")
        .box_stroke_width(1.1)
        .box_opacity(0.82)
        .whiskers(|style| style.stroke("#1d4ed8").stroke_width(1.1))
        .caps(|style| style.stroke("#1d4ed8").stroke_width(1.1))
        .median(|style| style.stroke("#111827").stroke_width(1.4))
        .outliers(|style| style.fill("#1d4ed8").opacity(0.38).size(20.0));

    ParallelAxisOverlay::new(dimension.id, Plot::<Cartesian>::new().mark(box_plot))
        .width_px(42.0)
        .zindex(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_coordinates_axis_overlay_summary_app_builds() {
        let _ = build_app().await;
    }
}
