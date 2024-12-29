// use arrow::array::{ArrayRef, AsArray, Float32Array, ListArray, RecordBatch};
// use arrow::datatypes::{DataType, Field, Float32Type, Schema};
// use avenger_app::app::AvengerApp;
// use avenger_chart::error::AvengerChartError;
// use avenger_chart::param::Param;
// use avenger_chart::runtime::controller::pan_zoom::PanZoomController;
// use avenger_chart::runtime::scale::{compile_domain, eval_scale, scale_expr};
// use avenger_chart::runtime::AvengerRuntime;
// use avenger_chart::types::group::Group;
// use avenger_chart::types::mark::Mark;
// use avenger_chart::types::scales::{DataField, Scale, ScaleDomain, ScaleRange};
// use avenger_chart::utils::{DataFrameChartUtils, ExprHelpers};
// use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
// use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
// use avenger_eventstream::window::MouseButton;
// use avenger_geometry::rtree::SceneGraphRTree;
// use avenger_scales::scales::linear::LinearScale;
// use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
// use avenger_scenegraph::scene_graph::SceneGraph;
// use avenger_winit_wgpu::WinitWgpuAvengerApp;
// use datafusion::common::ParamValues;
// use datafusion::logical_expr::{col, lit};
// use datafusion::prelude::{ident, CsvReadOptions, DataFrame, Expr, SessionContext};
// use datafusion::scalar::ScalarValue;
// use palette::Srgba;
// use std::collections::HashMap;
// use std::sync::Arc;
// use winit::event_loop::EventLoop;
// use avenger_eventstream::manager::EventStreamHandler;
//
// // This is what the end user writes to build their interactive chart.
// async fn make_chart(runtime: &AvengerRuntime) -> Result<Group, AvengerChartError> {
//     // Load data
//     let manifest_dir = env!("CARGO_MANIFEST_DIR");
//     let data_path = format!("{}/../iris/data/Iris.csv", manifest_dir);
//     let df = runtime
//         .ctx()
//         .read_csv(data_path, CsvReadOptions::default())
//         .await?;
//
//     // Create x/y scales with the pan/zoom controller
//     let pan_zoom_controller = PanZoomController::with_auto_range(
//         df.clone(),
//         "SepalLengthCm",
//         "SepalWidthCm",
//         400.0,
//         400.0,
//     );
//     let x_scale = pan_zoom_controller.x_scale();
//     let y_scale = pan_zoom_controller.y_scale();
//
//     // Custom color scale
//     let color_scale = Scale::new(LinearScale)
//         .domain_data_field(Arc::new(df.clone()), "PetalWidthCm")
//         .range(ScaleRange::new_color(vec![
//             Srgba::new(1.0, 0.0, 0.0, 1.0),
//             Srgba::new(0.0, 1.0, 0.0, 1.0),
//         ]));
//
//     // Params
//     let stroke_color = Param::new("stroke_color", "cyan");
//     let size = Param::new("size", 60);
//
//     let chart = Group::new()
//         .x(0.0)
//         .y(0.0)
//         .mark(
//             Mark::symbol()
//                 .from(df)
//                 .x(scale_expr(&x_scale, ident("SepalLengthCm"))?)
//                 .y(scale_expr(&y_scale, ident("SepalWidthCm"))?)
//                 .size(&size)
//                 .fill(scale_expr(&color_scale, ident("PetalWidthCm"))?)
//                 .stroke(&stroke_color)
//                 .stroke_width(lit(3.0)),
//         )
//         .controller(Arc::new(pan_zoom_controller))
//         .param(stroke_color)
//         .param(size);
//
//     Ok(chart)
// }
//
//
// // #[derive(Clone)]
// // pub struct AvengerChartState {
// //     pub tokio_runtime: Arc<tokio::runtime::Runtime>,
// //     pub runtime: Arc<AvengerRuntime>,
// //     pub chart: Group,
// //     pub param_values: HashMap<String, ScalarValue>,
// // }
// //
// // impl AvengerChartState {
// //     pub fn new() -> Self {
// //         // runtime
// //         let tokio_runtime = Arc::new(
// //             tokio::runtime::Builder::new_current_thread()
// //                 .build()
// //                 .unwrap(),
// //         );
// //         let runtime = Arc::new(AvengerRuntime::new(SessionContext::new()));
// //         let chart = tokio_runtime
// //             .block_on(make_chart(&runtime))
// //             .unwrap();
// //
// //         // Initialize param values with initial values from controllers
// //         let mut param_values = chart.controllers.iter().flat_map(
// //             |c| c.params().iter().map(|p| (p.name.clone(), p.default.clone())).collect::<Vec<_>>()
// //         ).collect::<HashMap<_, _>>();
// //
// //         // Add explicit chart params
// //         for param in &chart.params {
// //             param_values.insert(param.name.clone(), param.default.clone());
// //         }
// //
// //         Self {
// //             runtime: Arc::new(AvengerRuntime::new(SessionContext::new())),
// //             tokio_runtime: Arc::new(
// //                 tokio::runtime::Builder::new_current_thread()
// //                     .build()
// //                     .unwrap(),
// //             ),
// //             chart,
// //             param_values,
// //         }
// //     }
// //
// //     pub fn param_values(&self) -> ParamValues {
// //         // Collect
// //         ParamValues::Map(self.param_values.clone())
// //     }
// //
// //     /// Scale for current x domain
// //     pub fn eval_scale(&self, scale: &Scale) -> ConfiguredScale {
// //         self.tokio_runtime
// //             .block_on(eval_scale(
// //                 &scale,
// //                 self.runtime.ctx(),
// //                 Some(&self.param_values()),
// //             ))
// //             .unwrap()
// //     }
// //
// //     pub fn compile_scene_graph(&self) -> Result<SceneGraph, AvengerChartError> {
// //         println!("compile_scene_graph");
// //         let scene_group = self
// //             .tokio_runtime
// //             .block_on(self.runtime.compile_group(&self.chart, self.param_values()))?;
// //
// //         let scene_graph = SceneGraph {
// //             marks: vec![scene_group.into()],
// //             width: 440.0,
// //             height: 440.0,
// //             origin: [20.0, 20.0],
// //         };
// //
// //         Ok(scene_graph)
// //     }
// // }
// //
// // fn make_scene_graph(chart_state: &AvengerChartState) -> SceneGraph {
// //     chart_state.compile_scene_graph().unwrap()
// // }
//
// #[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
// pub async fn run() {
//
//     // runtime
//     let tokio_runtime = Arc::new(
//         tokio::runtime::Builder::new_current_thread()
//             .build()
//             .unwrap(),
//     );
//     let runtime = Arc::new(AvengerRuntime::new(SessionContext::new()));
//     let chart = tokio_runtime
//         .block_on(make_chart(&runtime))
//         .unwrap();
//
//     let avenger_app = runtime.build_app(chart).unwrap();
//     let mut app = WinitWgpuAvengerApp::new(avenger_app, 2.0);
//
//     let event_loop = EventLoop::new().expect("Failed to build event loop");
//     event_loop
//         .run_app(&mut app)
//         .expect("Failed to run event loop");
//
//     //
//     // let chart_state = AvengerChartState::new();
//     //
//     // // Build event streams that wrap param streams (need to revisit naming these).
//     // let param_streams = chart_state.chart.controllers.iter().flat_map(|c| c.param_streams()).collect::<Vec<_>>();
//     // let mut stream_callbacks = Vec::new();
//     // for param_stream in param_streams {
//     //     let input_param_names = Vec::from(param_stream.input_params());
//     //     let input_scales = Vec::from(param_stream.input_scales());
//     //
//     //     let stream_config = param_stream.stream_config().clone();
//     //     let stream_callback: Arc<dyn EventStreamHandler<AvengerChartState>> = Arc::new(
//     //         move |event: &SceneGraphEvent,
//     //               state: &mut AvengerChartState,
//     //               rtree: &SceneGraphRTree|
//     //               -> UpdateStatus {
//     //
//     //             // Build param values to pass to param stream
//     //             let input_params = input_param_names.iter().map(
//     //                 |name| (name.clone(), state.param_values[name].clone())
//     //             ).collect::<HashMap<_, _>>();
//     //
//     //             // Evaluate scales to pass to param stream
//     //             let scales = input_scales.iter().map(
//     //                 |s| state.eval_scale(s)
//     //             ).collect::<Vec<_>>();
//     //
//     //             // Get group path (where should this come from?)
//     //             let group_path = vec![0 as usize];
//     //
//     //             let (new_params, update_status) = param_stream.update(
//     //                 event,
//     //                 &input_params,
//     //                 &scales,
//     //                 &group_path,
//     //                 rtree,
//     //             );
//     //
//     //             // Store params
//     //             for (name, value) in new_params {
//     //                 state.param_values.insert(name, value);
//     //             }
//     //
//     //             update_status
//     //         });
//     //
//     //     stream_callbacks.push((stream_config, stream_callback));
//     // }
//     //
//     // let avenger_app = AvengerApp::new(
//     //     AvengerChartState::new(),
//     //     Arc::new(make_scene_graph),
//     //     stream_callbacks,
//     // );
//
//     // let mut app = WinitWgpuAvengerApp::new(avenger_app, 2.0);
//     //
//     // let event_loop = EventLoop::new().expect("Failed to build event loop");
//     // event_loop
//     //     .run_app(&mut app)
//     //     .expect("Failed to run event loop");
// }
