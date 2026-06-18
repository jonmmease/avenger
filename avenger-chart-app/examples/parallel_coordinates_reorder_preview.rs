// Parallel coordinates with a parameterized axis-drag preview state.
//
// This is a non-interactive preview of the same low-level state used by the
// header-drag example: one dimension is displayed away from its equilibrium
// slot while lines, axes, titles, symbols, and overlays all follow `display_x`.
//
// Run with:
// ```bash
// cargo run -p avenger-chart-app --example parallel_coordinates_reorder_preview --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::chart_avenger_app;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};

mod parallel_common;

fn main() {
    parallel_common::run_fixed_window_app("avenger-chart parallel reorder preview", build_app());
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let active_dimension = Param::new(
        "drag_dimension",
        ScalarValue::Utf8(Some("stability".into())),
    );
    let active_x = Param::new("drag_display_x", ScalarValue::Float64(Some(580.0)));
    let coord = parallel_common::demo_parallel()
        .active_axis_display_params(active_dimension.name.clone(), active_x.name.clone());

    let plot = Plot::with_coord(coord)
        .add_params([active_dimension, active_x])
        .canvas_size(
            parallel_common::CANVAS_SIZE[0],
            parallel_common::CANVAS_SIZE[1],
        )
        .plot_size(parallel_common::PLOT_SIZE[0], parallel_common::PLOT_SIZE[1])
        .title("Parameterized axis reorder preview")
        .data(parallel_common::demo_dataframe(&ctx))
        .mark(
            ParallelLine::new()
                .details(["sample_id"])
                .stroke("#94a3b8")
                .stroke_width(1.0)
                .opacity(0.42),
        )
        .mark(
            ParallelSymbol::new()
                .fill("#2563eb")
                .stroke("#ffffff")
                .stroke_width(0.8)
                .size(20.0)
                .opacity(0.78),
        )
        .mark(
            ParallelAxisOverlay::new(
                "stability",
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .data(parallel_common::interval_dataframe(&ctx, 67.0, 84.0))
                        .exclude_from_scale_domains()
                        .x(datafusion::prelude::col("x_min"))
                        .x2(datafusion::prelude::col("x_max"))
                        .y(datafusion::prelude::col("value_min"))
                        .y2(datafusion::prelude::col("value_max"))
                        .fill("rgba(37, 99, 235, 0.16)")
                        .stroke("#2563eb")
                        .stroke_width(1.0),
                ),
            )
            .width_px(28.0)
            .zindex(10),
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
    async fn parallel_coordinates_reorder_preview_app_builds() {
        let _ = build_app().await;
    }
}
