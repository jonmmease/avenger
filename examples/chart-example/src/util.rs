use arrow::array::{ArrayRef, AsArray, Float32Array, ListArray, RecordBatch};
use arrow::datatypes::{DataType, Field, Float32Type, Schema};
use avenger_app::app::AvengerApp;
use avenger_chart::error::AvengerChartError;
use avenger_chart::param::Param;
use avenger_chart::runtime::scale::{compile_domain, eval_scale, scale_expr};
use avenger_chart::runtime::AvengerRuntime;
use avenger_chart::types::group::Group;
use avenger_chart::types::mark::Mark;
use avenger_chart::types::scales::{DataField, Scale, ScaleDomain, ScaleRange};
use avenger_chart::utils::{DataFrameChartUtils, ExprHelpers};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::MouseButton;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use datafusion::common::ParamValues;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::{ident, CsvReadOptions, Expr, SessionContext};
use datafusion::scalar::ScalarValue;
use palette::Srgba;
use std::collections::HashMap;
use std::sync::Arc;
use winit::event_loop::EventLoop;

#[derive(Clone)]
pub struct ChartState {
    pub tokio_runtime: Arc<tokio::runtime::Runtime>,
    pub runtime: Arc<AvengerRuntime>,
    pub plot_group_name: String,
    pub controller_params: HashMap<String, ScalarValue>,
    pub chart: Group,
    pub x_scale: Scale,
    pub y_scale: Scale,
    pub color_scale: Scale,
}

impl ChartState {
    pub fn new() -> Self {
        // runtime
        let tokio_runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
        );
        let runtime = Arc::new(AvengerRuntime::new(SessionContext::new()));

        let plot_group_name = "plot".to_string();

        // params
        let stroke_color = Param::new("stroke_color", "cyan");
        let width = Param::new("width", 400.0);
        let height = Param::new("height", 400.0);
        let sepal_length_domain_raw = Param::new("sepal_length_domain_raw", ScalarValue::Null);
        let sepal_width_domain_raw = Param::new("sepal_width_domain_raw", ScalarValue::Null);

        let (chart, x_scale, y_scale, color_scale) = tokio_runtime
            .block_on(async {
                // Load data
                let manifest_dir = env!("CARGO_MANIFEST_DIR");
                let data_path = format!("{}/../iris/data/Iris.csv", manifest_dir);
                let df = runtime
                    .ctx()
                    .read_csv(data_path, CsvReadOptions::default())
                    .await?;

                // scales
                let x_scale = Scale::new(LinearScale)
                    .domain_data_field(Arc::new(df.clone()), "SepalLengthCm")
                    .raw_domain(&sepal_length_domain_raw)
                    .range(ScaleRange::new_interval(lit(0.0), &width));

                let y_scale = Scale::new(LinearScale)
                    .domain_data_field(Arc::new(df.clone()), "SepalWidthCm")
                    .raw_domain(&sepal_width_domain_raw)
                    .range(ScaleRange::new_interval(lit(0.0), &height));

                let color_scale = Scale::new(LinearScale)
                    .domain_data_field(Arc::new(df.clone()), "SepalWidthCm")
                    .range(ScaleRange::new_color(vec![
                        Srgba::new(1.0, 0.0, 0.0, 1.0),
                        Srgba::new(0.0, 1.0, 0.0, 1.0),
                    ]));

                let chart = Group::new()
                    .name(plot_group_name.clone())
                    .x(0.0)
                    .y(0.0)
                    .mark(
                        Mark::symbol()
                            .from(df)
                            .x(scale_expr(&x_scale, ident("SepalLengthCm"))?)
                            .y(scale_expr(&y_scale, ident("SepalWidthCm"))?)
                            .size(lit(60.0))
                            .fill(scale_expr(&color_scale, ident("SepalWidthCm"))?)
                            .stroke(&stroke_color)
                            .stroke_width(lit(3.0)),
                    );

                Ok::<(Group, Scale, Scale, Scale), AvengerChartError>((
                    chart,
                    x_scale,
                    y_scale,
                    color_scale,
                ))
            })
            .unwrap();

        let controller_params = vec![
            stroke_color,
            width,
            height,
            sepal_length_domain_raw,
            sepal_width_domain_raw,
        ]
        .into_iter()
        .map(|p| (p.name, p.default))
        .collect::<HashMap<_, _>>();

        Self {
            runtime: Arc::new(AvengerRuntime::new(SessionContext::new())),
            tokio_runtime: Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap(),
            ),
            plot_group_name: "plot".to_string(),
            controller_params,
            chart,
            x_scale,
            y_scale,
            color_scale,
        }
    }

    pub fn param_values(&self) -> ParamValues {
        ParamValues::Map(self.controller_params.clone())
    }

    /// Scale for current x domain
    pub fn x_scale(&self) -> ConfiguredScale {
        self.tokio_runtime
            .block_on(eval_scale(
                &self.x_scale,
                self.runtime.ctx(),
                Some(&self.param_values()),
            ))
            .unwrap()
    }

    pub fn y_scale(&self) -> ConfiguredScale {
        self.tokio_runtime
            .block_on(eval_scale(
                &self.y_scale,
                self.runtime.ctx(),
                Some(&self.param_values()),
            ))
            .unwrap()
    }

    pub fn compile_scene_graph(&self) -> Result<SceneGraph, AvengerChartError> {
        println!("compile_scene_graph");
        let scene_group = self
            .tokio_runtime
            .block_on(self.runtime.compile_group(&self.chart, self.param_values()))?;

        let scene_graph = SceneGraph {
            marks: vec![scene_group.into()],
            width: 440.0,
            height: 440.0,
            origin: [20.0, 20.0],
        };

        Ok(scene_graph)
    }

    // /// Scale for current y domain
    // pub fn y_scale(&self) -> ConfiguredScale {
    //     LinearScale::new(self.domain_sepal_width, (self.height, 0.0))
    // }
}

fn make_scene_graph(chart_state: &ChartState) -> SceneGraph {
    chart_state.compile_scene_graph().unwrap()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    // Predefine configs that are used in multiple handlers
    let left_mouse_down_config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        filter: Some(vec![Arc::new(|event| {
            let SceneGraphEvent::MouseDown(mouse_down) = event else {
                return false;
            };
            mouse_down.button == MouseButton::Left
        })]),
        ..Default::default()
    };
    let left_mouse_up_config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        filter: Some(vec![Arc::new(|event| {
            let SceneGraphEvent::MouseUp(mouse_up) = event else {
                return false;
            };
            mouse_up.button == MouseButton::Left
        })]),
        ..Default::default()
    };

    let avenger_app = AvengerApp::new(
        ChartState::new(),
        Arc::new(make_scene_graph),
        vec![
            // // Hover highlight
            // (
            //     EventStreamConfig {
            //         types: vec![SceneGraphEventType::MarkMouseEnter],
            //         mark_paths: Some(vec![vec![0, 2, 0]]),
            //         filter: Some(vec![Arc::new(|event| {
            //             let SceneGraphEvent::MouseEnter(mouse_enter) = event else {
            //                 return false;
            //             };
            //             mouse_enter.modifiers.meta
            //         })]),
            //         ..Default::default()
            //     },
            //     Arc::new(
            //         |event: &SceneGraphEvent,
            //          state: &mut ChartState,
            //          _rtree: &SceneGraphRTree|
            //          -> UpdateStatus {
            //             println!("MarkMouseEnter");
            //             state.hover_index = event.mark_instance().and_then(|i| i.instance_index);
            //             UpdateStatus {
            //                 rerender: true,
            //                 rebuild_geometry: false,
            //             }
            //         },
            //     ),
            // ),
            // (
            //     EventStreamConfig {
            //         types: vec![SceneGraphEventType::MarkMouseLeave],
            //         mark_paths: Some(vec![vec![0, 2, 0]]),
            //         ..Default::default()
            //     },
            //     Arc::new(
            //         |_event: &SceneGraphEvent,
            //          state: &mut ChartState,
            //          _rtree: &SceneGraphRTree|
            //          -> UpdateStatus {
            //             println!("MarkMouseLeave");
            //             state.hover_index = None;
            //             UpdateStatus {
            //                 rerender: true,
            //                 rebuild_geometry: false,
            //             }
            //         },
            //     ),
            // ),
            // Panning (record click anchor)
            (
                left_mouse_down_config.clone(),
                Arc::new(
                    |event: &SceneGraphEvent,
                     state: &mut ChartState,
                     rtree: &SceneGraphRTree|
                     -> UpdateStatus {
                        let event_position = event.position().unwrap();
                        let plot_origin = rtree.named_group_origin(&state.plot_group_name).unwrap();
                        let plot_x = event_position[0] - plot_origin[0];
                        let plot_y = event_position[1] - plot_origin[1];

                        // Get scales
                        let x_scale = state.x_scale();
                        let y_scale = state.y_scale();

                        // Check if cursor is over the plot area
                        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
                        let normalized_x = (plot_x - range_start) / (range_end - range_start);
                        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
                        let normalized_y = (plot_y - range_start) / (range_end - range_start);
                        if normalized_x < 0.0
                            || normalized_x > 1.0
                            || normalized_y < 0.0
                            || normalized_y > 1.0
                        {
                            return UpdateStatus {
                                rerender: false,
                                rebuild_geometry: false,
                            };
                        }

                        // Save state as controller params
                        let x_domain_scalar = x_scale.get_domain_scalar();
                        let y_domain_scalar = y_scale.get_domain_scalar();
                        let range_position =
                            ScalarValue::List(Arc::new(ListArray::from_iter_primitive::<
                                Float32Type,
                                _,
                                _,
                            >(vec![Some(
                                vec![Some(plot_x), Some(plot_y)],
                            )])));

                        state
                            .controller_params
                            .insert("anchor_range_position".to_string(), range_position.clone());
                        state
                            .controller_params
                            .insert("anchor_sepal_length_domain".to_string(), x_domain_scalar);
                        state
                            .controller_params
                            .insert("anchor_sepal_width_domain".to_string(), y_domain_scalar);

                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                        }
                    },
                ),
            ),
            // Panning (dragging)
            (
                EventStreamConfig {
                    types: vec![SceneGraphEventType::CursorMoved],
                    between: Some((
                        Box::new(left_mouse_down_config.clone()),
                        Box::new(left_mouse_up_config.clone()),
                    )),
                    throttle: Some(8), // Don't update faster than 60fps
                    ..Default::default()
                },
                Arc::new(
                    |event: &SceneGraphEvent,
                     state: &mut ChartState,
                     rtree: &SceneGraphRTree|
                     -> UpdateStatus {
                        // Extract stored anchor position
                        let Some(ScalarValue::List(range_position)) =
                            state.controller_params.get("anchor_range_position")
                        else {
                            return UpdateStatus {
                                rerender: false,
                                rebuild_geometry: false,
                            };
                        };
                        let range_position = range_position.value(0);
                        let range_position = range_position.as_primitive::<Float32Type>();
                        let anchor_x = range_position.value(0);
                        let anchor_y = range_position.value(1);

                        // Extract stored domains
                        let Some(ScalarValue::List(x_domain)) =
                            state.controller_params.get("anchor_sepal_length_domain")
                        else {
                            return UpdateStatus {
                                rerender: false,
                                rebuild_geometry: false,
                            };
                        };
                        let x_domain = x_domain.value(0);

                        let Some(ScalarValue::List(y_domain)) =
                            state.controller_params.get("anchor_sepal_width_domain")
                        else {
                            return UpdateStatus {
                                rerender: false,
                                rebuild_geometry: false,
                            };
                        };
                        let y_domain = y_domain.value(0);

                        // Get the cursor position in range space
                        // Get the cursor position in range space
                        let event_position = event.position().unwrap();
                        let plot_origin = rtree.named_group_origin(&state.plot_group_name).unwrap();
                        let plot_x = event_position[0] - plot_origin[0];
                        let plot_y = event_position[1] - plot_origin[1];

                        let x_scale = state.x_scale().with_domain(x_domain);
                        let y_scale = state.y_scale().with_domain(y_domain);

                        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
                        let x_delta = (plot_x - anchor_x) / (range_end - range_start);

                        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
                        let y_delta = (plot_y - anchor_y) / (range_end - range_start);

                        // Update domains
                        let x_domain_scalar = x_scale.pan(x_delta).unwrap().get_domain_scalar();
                        let y_domain_scalar = y_scale.pan(y_delta).unwrap().get_domain_scalar();

                        state
                            .controller_params
                            .insert("sepal_length_domain_raw".to_string(), x_domain_scalar);
                        state
                            .controller_params
                            .insert("sepal_width_domain_raw".to_string(), y_domain_scalar);

                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: false,
                        }
                    },
                ),
            ),
            // Panning (release)
            (
                left_mouse_up_config,
                Arc::new(
                    |_event: &SceneGraphEvent,
                     state: &mut ChartState,
                     _rtree: &SceneGraphRTree|
                     -> UpdateStatus {
                        state.controller_params.remove("anchor_range_position");
                        state.controller_params.remove("anchor_sepal_length_domain");
                        state.controller_params.remove("anchor_sepal_width_domain");
                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                        }
                    },
                ),
            ),
            (
                EventStreamConfig {
                    types: vec![SceneGraphEventType::DoubleClick],
                    ..Default::default()
                },
                Arc::new(
                    |_event: &SceneGraphEvent,
                     state: &mut ChartState,
                     _rtree: &SceneGraphRTree|
                     -> UpdateStatus {
                        println!("double click");

                        // Null raw domains so we fall back to default
                        state
                            .controller_params
                            .insert("sepal_length_domain_raw".to_string(), ScalarValue::Null);
                        state
                            .controller_params
                            .insert("sepal_width_domain_raw".to_string(), ScalarValue::Null);
                        state.controller_params.remove("anchor_range_position");
                        state.controller_params.remove("anchor_sepal_length_domain");
                        state.controller_params.remove("anchor_sepal_width_domain");

                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                        }
                    },
                ),
            ),
            // // wheel zoom
            // (
            //     EventStreamConfig {
            //         types: vec![SceneGraphEventType::MouseWheel],
            //         throttle: Some(8), // Don't update faster than 60fps
            //         ..Default::default()
            //     },
            //     Arc::new(
            //         |event: &SceneGraphEvent,
            //          state: &mut ChartState,
            //          rtree: &SceneGraphRTree|
            //          -> UpdateStatus {
            //             let SceneGraphEvent::MouseWheel(event) = event else {
            //                 return UpdateStatus {
            //                     rerender: false,
            //                     rebuild_geometry: false,
            //                 };
            //             };

            //             // Get scales
            //             let x_scale = state.x_scale();
            //             let y_scale = state.y_scale();

            //             // Get cursor position
            //             let event_position = event.position;
            //             let plot_origin = rtree.named_group_origin(&state.plot_group_name).unwrap();
            //             let plot_x = event_position[0] - plot_origin[0];
            //             let plot_y = event_position[1] - plot_origin[1];

            //             let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
            //             let normalized_x = (plot_x - range_start) / (range_end - range_start);
            //             let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
            //             let normalized_y = (plot_y - range_start) / (range_end - range_start);

            //             // Check if cursor is over the plot area
            //             if normalized_x < 0.0
            //                 || normalized_x > 1.0
            //                 || normalized_y < 0.0
            //                 || normalized_y > 1.0
            //             {
            //                 return UpdateStatus {
            //                     rerender: false,
            //                     rebuild_geometry: false,
            //                 };
            //             }

            //             let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
            //             let factor = match event.delta {
            //                 MouseScrollDelta::LineDelta(x_line_delta, y_line_delta) => {
            //                     -(x_line_delta + y_line_delta) * 0.005 + 1.0
            //                 }
            //                 MouseScrollDelta::PixelDelta(x_pixel_delta, y_pixel_delta) => {
            //                     -((x_pixel_delta + y_pixel_delta) as f32
            //                         / (range_end - range_start))
            //                         * 0.01
            //                         + 1.0
            //                 }
            //             };

            //             state.domain_sepal_length = x_scale
            //                 .zoom(normalized_x, factor)
            //                 .unwrap()
            //                 .numeric_interval_domain()
            //                 .unwrap();
            //             state.domain_sepal_width = y_scale
            //                 .zoom(normalized_y, factor)
            //                 .unwrap()
            //                 .numeric_interval_domain()
            //                 .unwrap();

            //             UpdateStatus {
            //                 rerender: true,
            //                 rebuild_geometry: false,
            //             }
            //         },
            //     ),
            // ),
        ],
    );

    let mut app = WinitWgpuAvengerApp::new(avenger_app, 2.0);

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
