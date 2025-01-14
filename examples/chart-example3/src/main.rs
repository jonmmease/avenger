// mod util;

use arrow::array::{Float32Array, ListArray};
use arrow::datatypes::DataType;
use avenger_app::app::AvengerApp;
use avenger_chart::error::AvengerChartError;
use avenger_chart::param::Param;
use avenger_chart::runtime::app::AvengerChartState;
use avenger_chart::runtime::controller::box_select::BoxSelectController;
use avenger_chart::runtime::controller::pan_zoom::PanZoomController;
use avenger_chart::runtime::controller::tooltip::TooltipController;
use avenger_chart::runtime::scale::scale_expr;
use avenger_chart::runtime::AvengerRuntime;
use avenger_chart::types::group::Group;
use avenger_chart::types::mark::Mark;
use avenger_chart::types::scales::{Scale, ScaleRange};
use avenger_eventstream::scene::SceneGraphEventType;
use avenger_eventstream::stream::EventStreamConfig;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::utils::ScalarValueUtils;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::common::utils::array_into_list_array;
use datafusion::logical_expr::{ident, lit};
use datafusion::prelude::{array_element, when, CsvReadOptions, SessionContext};
use datafusion::scalar::ScalarValue;
use palette::Srgba;
use std::sync::Arc;
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

    // Build scales
    let x_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "SepalLengthCm")
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));
    let y_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "SepalWidthCm")
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));

    // Build controller
    let tooltip = TooltipController::new();

    // Custom color scale
    let color_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(df.clone()), "PetalWidthCm")
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    // Params
    let size = Param::new("size", 60);

    // Build chart group
    let chart = Group::new()
        .x(0.0)
        .y(0.0)
        .mark(
            Mark::symbol()
                .from(df)
                .details(vec!["Species"])
                .x(scale_expr(&x_scale, ident("SepalLengthCm"))?)
                .y(scale_expr(&y_scale, ident("SepalWidthCm"))?)
                .fill(scale_expr(&color_scale, ident("PetalWidthCm"))?)
                .size(&size),
        )
        .controller(Arc::new(tooltip))
        .param(size);

    Ok(runtime.build_app(chart).await?)
}

fn main() -> Result<(), AvengerChartError> {
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
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
