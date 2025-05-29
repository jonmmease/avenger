use arrow::array::{ArrayRef, Float32Builder, StringArray, StringBuilder};
use avenger_app::app::{AvengerApp, SceneGraphBuilder};
use avenger_common::types::{ColorOrGradient, SymbolShape};
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, EventStreamFilter, UpdateStatus};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_guides::axis::numeric::make_numeric_axis_marks;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{make_symbol_legend, SymbolLegendConfig};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::EventStreamHandler;
use csv::Reader;
use rand_distr::Distribution;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct PanAnchor {
    pub range_position: [f32; 2],
    pub x_domain: (f32, f32),
    pub y_domain: (f32, f32),
}

#[derive(Clone)]
pub struct ChartState {
    #[allow(dead_code)]
    pub hover_index: Option<usize>,
    pub width: f32,
    pub height: f32,
    pub domain_sepal_length: (f32, f32),
    pub domain_sepal_width: (f32, f32),
    pub sepal_length: ArrayRef,
    #[allow(dead_code)]
    pub sepal_width: ArrayRef,
    #[allow(dead_code)]
    pub species: ArrayRef,
    pub plot_group_name: String,

    // Scales for the base plot, which are used to scale the original x/y
    pub base_x_scale: ConfiguredScale,
    pub base_y_scale: ConfiguredScale,

    // Prescaled
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub symbol_legend: SceneMark,

    // For panning
    pub pan_anchor: Option<PanAnchor>,
}

impl ChartState {
    pub fn new() -> Self {
        // Load data
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let data_path = format!("{}/data/Iris.csv", manifest_dir);
        let file = File::open(data_path).expect("Failed to open Iris.csv");
        let reader = BufReader::new(file);
        let mut csv_reader = Reader::from_reader(reader);

        // Initialize vectors to store the data
        let mut sepal_length: Vec<f32> = Vec::new();
        let mut sepal_width: Vec<f32> = Vec::new();
        let mut petal_length: Vec<f32> = Vec::new();
        let mut petal_width: Vec<f32> = Vec::new();
        let mut species: Vec<String> = Vec::new();

        // Read the CSV records
        for result in csv_reader.records() {
            let record = result.expect("Failed to read CSV record");

            // Skip header row by checking if the first column is numeric
            if record[1].parse::<f32>().is_ok() {
                sepal_length.push(record[1].parse::<f32>().unwrap());
                sepal_width.push(record[2].parse::<f32>().unwrap());
                petal_length.push(record[3].parse::<f32>().unwrap());
                petal_width.push(record[4].parse::<f32>().unwrap());
                species.push(record[5].to_string());
            }
        }

        // Duplicate data N times with random jitter
        let n = 100;
        let jitter = 0.1;
        // let mut large_sepal_length = Vec::with_capacity(n * species.len());
        // let mut large_sepal_width = Vec::with_capacity(n * species.len());
        // let mut large_species = Vec::with_capacity(n * species.len());

        let mut sepal_langth_builder = Float32Builder::new();
        let mut sepal_width_builder = Float32Builder::new();
        let mut species_builder = StringBuilder::new();

        let normal = rand_distr::Normal::new(0.0, jitter).unwrap();
        let mut rng = rand::thread_rng();

        for _ in 0..n {
            sepal_length.iter().for_each(|x| {
                sepal_langth_builder.append_value(x + normal.sample(&mut rng));
            });
            sepal_width.iter().for_each(|x| {
                sepal_width_builder.append_value(x + normal.sample(&mut rng));
            });
            species.iter().for_each(|x| {
                species_builder.append_value(x.clone());
            });
        }

        // Build arrays
        let sepal_length_array = Arc::new(sepal_langth_builder.finish()) as ArrayRef;
        let sepal_width_array = Arc::new(sepal_width_builder.finish()) as ArrayRef;
        let species_array = Arc::new(species_builder.finish()) as ArrayRef;

        // println!("data length: {:?}", sepal_length_array.len());

        let fill_opacity = 0.1;
        let color_scale = OrdinalScale::new(Arc::new(StringArray::from(vec![
            "Iris-setosa",
            "Iris-versicolor",
            "Iris-virginica",
        ])))
        .with_range_colors(vec![
            [0.9, 0.0, 0.0, fill_opacity],
            [0.0, 0.9, 0.0, fill_opacity],
            [0.0, 0.0, 0.9, fill_opacity],
        ])
        .unwrap();

        // println!("color_scale {:?}", color_scale);

        // Make fill, taking the hover index into account
        let fill = color_scale.scale_to_color(&species_array).unwrap();

        // Dimensions
        let width = 200.0;
        let height = 200.0;

        let domain_sepal_length = (4.0, 8.5);
        let domain_sepal_width = (1.5, 5.0);

        let base_x_scale = LinearScale::new(domain_sepal_length, (0.0, width));
        let base_y_scale = LinearScale::new(domain_sepal_width, (height, 0.0));

        let x = base_x_scale.scale_to_numeric(&sepal_length_array).unwrap();
        let y = base_y_scale.scale_to_numeric(&sepal_width_array).unwrap();

        // Make symbol legend
        let symbol_legend = SceneMark::Group(
            make_symbol_legend(&SymbolLegendConfig {
                text: color_scale.format(color_scale.domain()).unwrap(),
                title: None,
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                stroke_width: Some(1.0),
                fill: color_scale.scale_to_color(color_scale.domain()).unwrap(),
                angle: 0.0.into(),
                inner_width: width,
                inner_height: height,
                ..Default::default()
            })
            .unwrap(),
        );

        Self {
            hover_index: None,
            width,
            height,
            base_x_scale,
            base_y_scale,
            domain_sepal_length,
            domain_sepal_width,
            x,
            y,
            sepal_length: sepal_length_array,
            sepal_width: sepal_width_array,
            species: species_array,
            plot_group_name: "plot".to_string(),
            fill,
            symbol_legend,
            pan_anchor: None,
        }
    }

    /// Scale for current x domain
    pub fn x_scale(&self) -> ConfiguredScale {
        LinearScale::new(self.domain_sepal_length, (0.0, self.width))
    }

    /// Scale for current y domain
    pub fn y_scale(&self) -> ConfiguredScale {
        LinearScale::new(self.domain_sepal_width, (self.height, 0.0))
    }
}

#[derive(Clone, Debug)]
struct IrisSceneGraphBuilder;

#[async_trait::async_trait]
impl SceneGraphBuilder<ChartState> for IrisSceneGraphBuilder {
    async fn build(&self, state: &mut ChartState) -> Result<SceneGraph, AvengerAppError> {
        Ok(make_scene_graph(state))
    }
}

fn make_scene_graph(chart_state: &ChartState) -> SceneGraph {
    let start_time = Instant::now(); // Start timing

    // Build scales and compute adjustments
    let x_scale = chart_state.x_scale();
    let x_adjustment = chart_state.base_x_scale.adjust(&x_scale).unwrap();
    let y_scale = chart_state.y_scale();
    let y_adjustment = chart_state.base_y_scale.adjust(&y_scale).unwrap();

    // // lift hover point to the top with indices
    // let indices = if let Some(hover) = chart_state.hover_index {
    //     (0..chart_state.species.len())
    //         .map(|i| {
    //             if i < hover {
    //                 i // before hover: keep same
    //             } else if i < chart_state.species.len() - 1 {
    //                 i + 1 // after hover: shift up by 1
    //             } else {
    //                 hover // last position: put hover index
    //             }
    //         })
    //         .collect()
    // } else {
    //     Vec::from_iter(0..chart_state.species.len())
    // };

    // // Build size array, taking the hover index into account
    // let mut size = vec![20.0; chart_state.species.len()];
    // if let Some(index) = chart_state.hover_index {
    //     size[index] = 60.0;
    // }

    // Make symbol mark
    // let shape = SymbolShape::from_vega_str("square").unwrap();
    let shape = SymbolShape::from_vega_str("circle").unwrap();

    let points = SceneSymbolMark {
        len: chart_state.sepal_length.len() as u32,
        x: chart_state.x.clone(),
        y: chart_state.y.clone(),
        x_adjustment: Some(x_adjustment),
        y_adjustment: Some(y_adjustment),
        shapes: vec![shape.clone()],
        shape_index: 0.into(),
        fill: chart_state.fill.clone(),
        size: 20.0.into(),
        // size: ScalarOrArray::new_array(size),
        // indices: Some(Arc::new(indices)),
        // stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 0.0f32.into(),
        ..Default::default()
    };

    // make mark group with clipping
    let mark_group = SceneGroup {
        name: chart_state.plot_group_name.clone(),
        origin: [0.0, 0.0],
        marks: vec![points.into()],
        // Clip to not overlap with axis
        clip: Clip::Rect {
            x: 0.5,
            y: 0.5,
            width: chart_state.width - 1.0,
            height: chart_state.height - 1.0,
        },
        ..Default::default()
    };

    // Make y-axis
    let y_axis = make_numeric_axis_marks(
        &y_scale,
        "Sepal Width",
        [0.0, 0.0],
        &AxisConfig {
            dimensions: [chart_state.width, chart_state.height],
            orientation: AxisOrientation::Left,
            grid: true,
        },
    )
    .unwrap();

    // Make x-axis
    let x_axis = make_numeric_axis_marks(
        &x_scale,
        "Sepal Length",
        [0.0, 0.0],
        &AxisConfig {
            dimensions: [chart_state.width, chart_state.height],
            orientation: AxisOrientation::Bottom,
            grid: true,
        },
    )
    .unwrap();

    // Wrap axis and rect in group
    let group = SceneMark::Group(SceneGroup {
        origin: [60.0, 60.0],
        marks: vec![
            y_axis.into(),
            x_axis.into(),
            mark_group.into(),
            chart_state.symbol_legend.clone().into(),
        ],
        ..Default::default()
    });

    let scene_graph = SceneGraph {
        marks: vec![group],
        width: 340.0,
        height: 300.0,
        origin: [0.0; 2],
    };

    let duration = start_time.elapsed(); // Calculate elapsed time
    println!("Scene construction time: {:?}", duration); // Print the duration

    scene_graph
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
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            let SceneGraphEvent::MouseDown(mouse_down) = event else {
                return false;
            };
            mouse_down.button == MouseButton::Left
        }))]),
        ..Default::default()
    };
    let left_mouse_up_config = EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            let SceneGraphEvent::MouseUp(mouse_up) = event else {
                return false;
            };
            mouse_up.button == MouseButton::Left
        }))]),
        ..Default::default()
    };

    let avenger_app = AvengerApp::try_new(
        ChartState::new(),
        Arc::new(IrisSceneGraphBuilder),
        vec![
            // Panning (record click anchor)
            (left_mouse_down_config.clone(), Arc::new(PanningClick)),
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
                Arc::new(PanningDrag),
            ),
            // Panning (release)
            (left_mouse_up_config, Arc::new(PanningRelease)),
            // wheel zoom
            (
                EventStreamConfig {
                    types: vec![SceneGraphEventType::MouseWheel],
                    throttle: Some(8), // Don't update faster than 60fps
                    ..Default::default()
                },
                Arc::new(WheelZoom),
            ),
        ],
    )
    .await
    .expect("Failed to create AvengerApp");

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

// Panning (record click anchor)
struct PanningClick;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningClick {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
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
        if normalized_x < 0.0 || normalized_x > 1.0 || normalized_y < 0.0 || normalized_y > 1.0 {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        }

        state.pan_anchor = Some(PanAnchor {
            range_position: [plot_x, plot_y],
            x_domain: state.domain_sepal_length,
            y_domain: state.domain_sepal_width,
        });

        UpdateStatus {
            rerender: false,
            rebuild_geometry: false,
        }
    }
}

// Panning (dragging)
struct PanningDrag;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningDrag {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let Some(pan_anchor) = &state.pan_anchor else {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        };

        // Get the cursor position in range space
        let event_position = event.position().unwrap();
        let plot_origin = rtree.named_group_origin(&state.plot_group_name).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        let x_scale = state.x_scale().with_domain_interval(pan_anchor.x_domain);
        let y_scale = state.y_scale().with_domain_interval(pan_anchor.y_domain);

        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let x_delta = (plot_x - pan_anchor.range_position[0]) / (range_end - range_start);

        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let y_delta = (plot_y - pan_anchor.range_position[1]) / (range_end - range_start);

        // Update domains
        state.domain_sepal_length = x_scale
            .pan(x_delta)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();
        state.domain_sepal_width = y_scale
            .pan(y_delta)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();

        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}

// Panning (release)
struct PanningRelease;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for PanningRelease {
    async fn handle(
        &self,
        _event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        state.pan_anchor = None;
        UpdateStatus {
            rerender: true,
            rebuild_geometry: true,
        }
    }
}

// wheel zoom
struct WheelZoom;

#[async_trait::async_trait]
impl EventStreamHandler<ChartState> for WheelZoom {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseWheel(event) = event else {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        };

        // Get scales
        let x_scale = state.x_scale();
        let y_scale = state.y_scale();

        // Get cursor position
        let event_position = event.position;
        let plot_origin = rtree.named_group_origin(&state.plot_group_name).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let normalized_x = (plot_x - range_start) / (range_end - range_start);
        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let normalized_y = (plot_y - range_start) / (range_end - range_start);

        // Check if cursor is over the plot area
        if normalized_x < 0.0 || normalized_x > 1.0 || normalized_y < 0.0 || normalized_y > 1.0 {
            return UpdateStatus {
                rerender: false,
                rebuild_geometry: false,
            };
        }

        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let factor = match event.delta {
            MouseScrollDelta::LineDelta(x_line_delta, y_line_delta) => {
                -(x_line_delta + y_line_delta) * 0.005 + 1.0
            }
            MouseScrollDelta::PixelDelta(x_pixel_delta, y_pixel_delta) => {
                -((x_pixel_delta + y_pixel_delta) as f32 / (range_end - range_start)) * 0.01 + 1.0
            }
        };

        state.domain_sepal_length = x_scale
            .zoom(normalized_x, factor)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();
        state.domain_sepal_width = y_scale
            .zoom(normalized_y, factor)
            .unwrap()
            .numeric_interval_domain()
            .unwrap();

        UpdateStatus {
            rerender: true,
            rebuild_geometry: false,
        }
    }
}
