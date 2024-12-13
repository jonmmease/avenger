use avenger_app::app::AvengerApp;
use avenger_common::types::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_guides::axis::numeric::make_numeric_axis_marks;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{make_symbol_legend, SymbolLegendConfig};
use avenger_scales::numeric::linear::LinearNumericScale;
use avenger_scales::numeric::ContinuousNumericScale;
use avenger_scales::ordinal::OrdinalScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::{SceneSymbolMark, SymbolShape};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

use csv::Reader;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use winit::event_loop::EventLoop;

#[derive(Clone)]
pub struct ChartState {
    pub hover_index: Option<usize>,
    pub width: f32,
    pub height: f32,
    pub domain_sepal_length: (f32, f32),
    pub domain_sepal_width: (f32, f32),
    pub sepal_length: Vec<f32>,
    pub sepal_width: Vec<f32>,
    pub species: Vec<String>,
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

        Self {
            hover_index: None,
            width: 200.0,
            height: 200.0,
            domain_sepal_length: (4.0, 8.5),
            domain_sepal_width: (1.5, 5.0),
            sepal_length,
            sepal_width,
            species,
        }
    }
}

fn make_scene_graph(chart_state: &ChartState) -> SceneGraph {
    // Build scales
    let x_scale = LinearNumericScale::new(&Default::default())
        .with_domain(chart_state.domain_sepal_length)
        .with_range((0.0, chart_state.width))
        .with_round(true);

    let y_scale = LinearNumericScale::new(&Default::default())
        .with_domain(chart_state.domain_sepal_width)
        .with_range((chart_state.height, 0.0))
        .with_round(true);

    let color_scale = OrdinalScale::new(
        &[
            "Iris-setosa".to_string(),
            "Iris-versicolor".to_string(),
            "Iris-virginica".to_string(),
        ],
        &[
            ColorOrGradient::Color([0.9, 0.0, 0.0, 1.0]),
            ColorOrGradient::Color([0.0, 0.9, 0.0, 1.0]),
            ColorOrGradient::Color([0.0, 0.0, 0.9, 1.0]),
        ],
        ColorOrGradient::Color([0.9, 0.9, 0.9, 1.0]),
    )
    .unwrap();

    // lift hover point to the top with indices
    let indices = if let Some(hover) = chart_state.hover_index {
        (0..chart_state.species.len())
            .map(|i| {
                if i < hover {
                    i // before hover: keep same
                } else if i < chart_state.species.len() - 1 {
                    i + 1 // after hover: shift up by 1
                } else {
                    hover // last position: put hover index
                }
            })
            .collect()
    } else {
        Vec::from_iter(0..chart_state.species.len())
    };

    // Make fill, taking the hover index into account
    let fill = match color_scale.scale(&chart_state.species) {
        ScalarOrArray::Array(arc) => {
            let mut colors = Arc::try_unwrap(arc).unwrap_or_else(|arc| arc.as_ref().clone());
            if let Some(index) = chart_state.hover_index {
                colors[index] = ColorOrGradient::Color([1.0, 1.0, 0.0, 1.0]);
            }
            ScalarOrArray::Array(Arc::new(colors))
        }
        s => s,
    };

    // Build size array, taking the hover index into account
    let mut size = vec![30.0; chart_state.species.len()];
    if let Some(index) = chart_state.hover_index {
        size[index] = 60.0;
    }

    // Make symbol mark
    let points = SceneSymbolMark {
        len: chart_state.sepal_length.len() as u32,
        x: x_scale.scale(&chart_state.sepal_length),
        y: y_scale.scale(&chart_state.sepal_width),
        fill,
        size: ScalarOrArray::Array(Arc::new(size)),
        indices: Some(Arc::new(indices)),
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 1.0f32.into(),
        ..Default::default()
    };

    // make mark group with clipping
    let mark_group = SceneGroup {
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
    );

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
    );

    // Make symbol legend
    let symbol_legend = make_symbol_legend(&SymbolLegendConfig {
        text: color_scale.domain().into(),
        shape: SymbolShape::Circle.into(),
        title: None,
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: Some(1.0),
        fill: color_scale.scale(&color_scale.domain()),
        angle: 0.0.into(),
        inner_width: chart_state.width,
        inner_height: chart_state.height,
        ..Default::default()
    })
    .unwrap();

    // Wrap axis and rect in group
    let group = SceneMark::Group(SceneGroup {
        origin: [60.0, 60.0],
        marks: vec![
            y_axis.into(),
            x_axis.into(),
            mark_group.into(),
            symbol_legend.into(),
        ],
        ..Default::default()
    });

    SceneGraph {
        marks: vec![group],
        width: 340.0,
        height: 300.0,
        origin: [0.0; 2],
    }
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

    let avenger_app = AvengerApp::new(
        ChartState::new(),
        Arc::new(make_scene_graph),
        vec![
            (
                EventStreamConfig {
                    types: vec![SceneGraphEventType::MarkMouseEnter],
                    mark_paths: Some(vec![vec![0, 2, 0]]),
                    ..Default::default()
                },
                Arc::new(
                    |event: &SceneGraphEvent, state: &mut ChartState| -> UpdateStatus {
                        state.hover_index = event.mark_instance().and_then(|i| i.instance_index);
                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                        }
                    },
                ),
            ),
            (
                EventStreamConfig {
                    types: vec![SceneGraphEventType::MarkMouseLeave],
                    mark_paths: Some(vec![vec![0, 2, 0]]),
                    ..Default::default()
                },
                Arc::new(
                    |_event: &SceneGraphEvent, state: &mut ChartState| -> UpdateStatus {
                        state.hover_index = None;
                        UpdateStatus {
                            rerender: true,
                            rebuild_geometry: true,
                        }
                    },
                ),
            ),
        ],
    );

    let mut app = WinitWgpuAvengerApp::new(avenger_app, 3.0);

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
