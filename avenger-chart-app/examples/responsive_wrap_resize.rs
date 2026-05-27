use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app, window_scene_sizing_for_resize_policy,
};
use datafusion::functions_aggregate::min_max::max;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use winit::window::WindowAttributes;

fn main() {
    let _ = env_logger::try_init();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let mut avenger_app = tokio_runtime.block_on(build_app());
    let resize_policy = tokio_runtime.block_on(avenger_app.app_state_mut().resize_policy());
    let options = WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart responsive facet wrap")
                .with_resizable(true),
        )
        .window_scene_sizing(window_scene_sizing_for_resize_policy(resize_policy))
        .resize_settle_delay_ms(Some(160));
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let width = Param::new("width", ScalarValue::Float64(Some(700.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Alpha', 0.0, 0.4), ('Alpha', 1.0, 1.1), ('Alpha', 2.0, 1.8),
                ('Beta', 0.0, 1.4), ('Beta', 1.0, 1.8), ('Beta', 2.0, 2.7),
                ('Gamma', 0.0, 0.8), ('Gamma', 1.0, 2.2), ('Gamma', 2.0, 3.0),
                ('Delta', 0.0, 1.7), ('Delta', 1.0, 2.5), ('Delta', 2.0, 3.2),
                ('Epsilon', 0.0, 2.0), ('Epsilon', 1.0, 2.7), ('Epsilon', 2.0, 4.1),
                ('Zeta', 0.0, 2.4), ('Zeta', 1.0, 3.5), ('Zeta', 2.0, 4.6),
                ('Eta', 0.0, 1.1), ('Eta', 1.0, 3.1), ('Eta', 2.0, 5.2),
                ('Theta', 0.0, 2.8), ('Theta', 1.0, 4.2), ('Theta', 2.0, 5.6),
                ('Iota', 0.0, 3.0), ('Iota', 1.0, 4.4), ('Iota', 2.0, 6.0)
            ) AS t(facet, x, y)",
        )
        .await
        .expect("build data");

    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("facet"))
            .size(90.0),
    );

    let plot = Plot::<FacetWrap>::new()
        .add_param(width.clone())
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(150.0))
        .data(df)
        .mark(Subplot::new(child).wrap_with(col("facet"), |c| {
            c.responsive_columns(190.0)
                .order_by(max(col("y")))
                .order_desc()
                .guide(|g| g.title("Facet"))
        }));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::width("width"),
            resize_throttle_ms: Some(8),
            exact_on_resize_settle: true,
            log_metrics: true,
        },
    )
    .await
    .expect("build chart app")
}
