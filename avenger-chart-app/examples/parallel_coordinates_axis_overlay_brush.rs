// Parallel coordinates with static axis-local brush bands.
//
// This uses `ParallelAxisOverlay` to put narrow Cartesian rect plots on top of
// selected axes. The child x scale is normalized to 0..1 while child y/y2 use
// the selected parallel dimension scale.
//
// Run with:
// ```bash
// cargo run -p avenger-chart-app --example parallel_coordinates_axis_overlay_brush --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::chart_avenger_app;
use datafusion::prelude::{SessionContext, col};

mod parallel_common;

fn main() {
    parallel_common::run_fixed_window_app("avenger-chart parallel axis overlay brush", build_app());
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let plot = Plot::with_coord(parallel_common::demo_parallel())
        .canvas_size(
            parallel_common::CANVAS_SIZE[0],
            parallel_common::CANVAS_SIZE[1],
        )
        .plot_size(parallel_common::PLOT_SIZE[0], parallel_common::PLOT_SIZE[1])
        .title("Axis-local brush overlays")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            ParallelLine::new()
                .details(["sample_id"])
                .stroke("#b8c1ce")
                .stroke_width(1.0)
                .opacity(0.36),
        )
        .mark(brush_overlay(
            &ctx,
            "speed",
            40.0,
            61.0,
            "rgba(37, 99, 235, 0.16)",
        ))
        .mark(brush_overlay(
            &ctx,
            "stability",
            67.0,
            84.0,
            "rgba(22, 163, 74, 0.16)",
        ))
        .mark(brush_overlay(
            &ctx,
            "quality",
            62.0,
            77.0,
            "rgba(217, 119, 6, 0.16)",
        ));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(compiled, ctx, parallel_common::app_options())
        .await
        .expect("build chart app")
}

fn brush_overlay(
    ctx: &SessionContext,
    dimension_id: &'static str,
    value_min: f64,
    value_max: f64,
    fill: &'static str,
) -> ParallelAxisOverlay<Parallel> {
    let rect = Rect::<Cartesian>::new()
        .data(parallel_common::interval_dataframe(
            ctx, value_min, value_max,
        ))
        .exclude_from_scale_domains()
        .x(col("x_min"))
        .x2(col("x_max"))
        .y(col("value_min"))
        .y2(col("value_max"))
        .fill(fill)
        .stroke("#1f2937")
        .stroke_width(1.0)
        .opacity(0.95);

    ParallelAxisOverlay::new(dimension_id, Plot::<Cartesian>::new().mark(rect))
        .width_px(22.0)
        .zindex(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_coordinates_axis_overlay_brush_app_builds() {
        let _ = build_app().await;
    }
}
