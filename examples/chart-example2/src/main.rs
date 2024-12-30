// mod util;

use avenger_app::app::AvengerApp;
use avenger_chart::error::AvengerChartError;
use avenger_chart::param::Param;
use avenger_chart::runtime::app::AvengerChartState;
use avenger_chart::runtime::controller::pan_zoom::PanZoomController;
use avenger_chart::runtime::scale::scale_expr;
use avenger_chart::runtime::AvengerRuntime;
use avenger_chart::types::group::Group;
use avenger_chart::types::mark::Mark;
use avenger_chart::types::scales::{Scale, ScaleRange};
use avenger_eventstream::scene::SceneGraphEventType;
use avenger_eventstream::stream::EventStreamConfig;
use avenger_scales::scales::linear::LinearScale;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::logical_expr::{ident, lit};
use datafusion::prelude::{array_element, CsvReadOptions, SessionContext};
use datafusion::scalar::ScalarValue;
use palette::Srgba;
use std::sync::Arc;
use arrow::array::{Float32Array, ListArray};
use arrow::datatypes::DataType;
use datafusion::common::utils::array_into_list_array;
use winit::event_loop::EventLoop;

pub async fn make_app() -> Result<AvengerApp<AvengerChartState>, AvengerChartError> {
    // Build Avenger runtime
    let runtime = Arc::new(AvengerRuntime::new(SessionContext::new()));

    // Load data
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_path = format!("{}/../iris/data/Iris.csv", manifest_dir);
    let df = runtime
        .ctx()
        .read_csv(data_path, CsvReadOptions::default())
        .await?;

    // Create x/y scales with the pan/zoom controller
    let pan_zoom_controller = PanZoomController::with_auto_range(
        df.clone(),
        "SepalLengthCm",
        "SepalWidthCm",
        400.0,
        400.0,
    );
    let x_scale = pan_zoom_controller.x_scale();
    let y_scale = pan_zoom_controller.y_scale();

    // Custom color scale
    let color_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "PetalWidthCm")
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    // Params
    let stroke_color = Param::new("stroke_color", "cyan");
    let size = Param::new("size", 60);

    // Params with event stream sources
    let cursor_config = EventStreamConfig {
        types: vec![SceneGraphEventType::CursorMoved],
        throttle: Some(8), // Don't update faster than 60fps
        ..Default::default()
    };
    // param to track the x cursor position
    let cursor_x = Param::new("cursor_x", 0.0f32).with_stream(
        cursor_config.clone(),
        &[],
        &[],
        Arc::new(|event, _params, _scales, origin| {
            let Some(position) = event.position() else {
                return None;
            };
            let group_position = Vec::from([position[0] - origin[0], position[1] - origin[1]]);
            Some(group_position[0].into())
        }),
    );
    // param to track the y cursor position
    let cursor_y = Param::new("cursor_y", 0.0f32).with_stream(
        cursor_config.clone(),
        &[],
        &[],
        Arc::new(|event, _params, _scales, origin| {
            let Some(position) = event.position() else {
                return None;
            };
            let group_position = Vec::from([position[0] - origin[0], position[1] - origin[1]]);
            Some(group_position[1].into())
        }),
    );

    // Build chart group
    let chart = Group::new()
        .x(0.0)
        .y(0.0)
        .mark(
            Mark::symbol()
                .from(df)
                .x(scale_expr(&x_scale, ident("SepalLengthCm"))?)
                .y(scale_expr(&y_scale, ident("SepalWidthCm"))?)
                .fill(scale_expr(&color_scale, ident("PetalWidthCm"))?)
                .size(&size)
                .stroke(&stroke_color)
                .stroke_width(lit(3.0)),
        )
        .mark(
            Mark::rule()
                .encode("x0", &cursor_x)
                .encode("x1", lit(100.0))
                .encode("y0", &cursor_y)
                .encode("y1", lit(100.0))
                .stroke(&stroke_color)
                .stroke_width(lit(3.0)),
        )
        .controller(Arc::new(pan_zoom_controller))
        .param(stroke_color)
        .param(cursor_x)
        .param(cursor_y)
        .param(size);

    Ok(runtime.build_app(chart).await?)
}

fn main() -> Result<(), AvengerChartError> {
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let avenger_app = tokio_runtime.block_on(make_app())?;
    let mut app = WinitWgpuAvengerApp::new(avenger_app, 2.0, tokio_runtime);
    let event_loop = EventLoop::new().expect("Failed to build event loop");
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
    Ok(())
}
