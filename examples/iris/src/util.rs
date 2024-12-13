use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::ColorOrGradient;
use avenger_eventstream::stream::{EventStreamConfig, EventStreamManager};
use avenger_eventstream::window::WindowEvent as AvengerWindowEvent;
use avenger_eventstream::SceneGraphEventType;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_guides::axis::band::make_band_axis_marks;
use avenger_guides::axis::numeric::make_numeric_axis_marks;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{make_symbol_legend, SymbolLegendConfig};
use avenger_scales::band::BandScale;
use avenger_scales::color::continuous_color::ContinuousColorScale;
use avenger_scales::color::Srgba;
use avenger_scales::numeric::linear::{LinearNumericScale, LinearNumericScaleConfig};
use avenger_scales::numeric::log::{LogNumericScale, LogNumericScaleConfig};
use avenger_scales::numeric::pow::{PowNumericScale, PowNumericScaleConfig};
use avenger_scales::numeric::symlog::{SymlogNumericScale, SymlogNumericScaleConfig};
use avenger_scales::numeric::ContinuousNumericScale;
use avenger_scales::ordinal::OrdinalScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::{SceneSymbolMark, SymbolShape};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;

use csv::Reader;
use std::fs::File;
use std::io::BufReader;
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};

struct App<'a> {
    canvas: Option<WindowCanvas<'a>>,
    scene_graph: SceneGraph,
    scale: f32,
    rtree: SceneGraphRTree,
    event_stream_manager: EventStreamManager,
}

impl<'a> ApplicationHandler for App<'a> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(false))
            .expect("Failed to create window");

        #[cfg(target_arch = "wasm32")]
        {
            use winit::dpi::PhysicalSize;
            let _ = window.request_inner_size(PhysicalSize::new(450, 400));

            use winit::platform::web::WindowExtWebSys;
            web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| {
                    let dst = doc.get_element_by_id("wasm-example")?;
                    let canvas =
                        web_sys::Element::from(window.canvas().expect("Failed to get canvas"));
                    dst.append_child(&canvas).ok()?;
                    Some(())
                })
                .expect("Couldn't append canvas to document body.");
        }

        let dimensions = CanvasDimensions {
            size: [self.scene_graph.width, self.scene_graph.height],
            scale: self.scale,
        };

        let mut canvas =
            pollster::block_on(WindowCanvas::new(window, dimensions, Default::default()))
                .expect("Failed to create canvas");

        canvas.set_scene(&self.scene_graph).unwrap();

        // Request initial redraw
        canvas.window().request_redraw();

        self.canvas = Some(canvas);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let canvas = match &mut self.canvas {
            Some(canvas) => canvas,
            None => return,
        };

        if window_id == canvas.window().id() && !canvas.input(&event) {
            match event {
                WindowEvent::CloseRequested
                | WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            logical_key: keyboard::Key::Named(NamedKey::Escape),
                            state: ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    self.canvas.take();
                    _event_loop.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    canvas.resize(physical_size);
                }
                WindowEvent::RedrawRequested => {
                    canvas.update();

                    match canvas.render() {
                        Ok(_) => {}
                        Err(AvengerWgpuError::SurfaceError(err)) => match err {
                            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                canvas.resize(canvas.get_size());
                            }
                            wgpu::SurfaceError::OutOfMemory => {
                                _event_loop.exit();
                            }
                            wgpu::SurfaceError::Timeout => {
                                log::warn!("Surface timeout");
                            }
                        },
                        Err(err) => {
                            log::error!("{:?}", err);
                        }
                    }
                }
                event => {
                    if let Some(event) = AvengerWindowEvent::from_winit_event(event, self.scale) {
                        self.event_stream_manager.dispatch_event(
                            &event,
                            &self.rtree,
                            Instant::now(),
                        );
                    }
                }
            }
        }
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

    let scene_graph = make_scene_graph(
        200.0,
        200.0,
        (4.0, 8.5),
        (1.5, 5.0),
        sepal_length,
        sepal_width,
        species,
    );
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);
    let svg = rtree.to_svg();
    std::fs::write("geometry.svg", svg).expect("Failed to write SVG file");

    let scale = 2.0;

    // Build event stream manager and register handlers
    let mut event_stream_manager = EventStreamManager::new();
    // event_stream_manager.register_handler(
    //     EventStreamConfig {
    //         types: vec![SceneGraphEventType::MarkMouseEnter],
    //         mark_paths: Some(vec![vec![0, 2, 0]]),
    //         ..Default::default()
    //     },
    //     |event| {
    //         println!("cursor entered: {:?}", event.mark_instance().unwrap());
    //     },
    // );
    // event_stream_manager.register_handler(
    //     EventStreamConfig {
    //         types: vec![SceneGraphEventType::MarkMouseLeave],
    //         mark_paths: Some(vec![vec![0, 2, 0]]),
    //         ..Default::default()
    //     },
    //     |event| {
    //         println!("cursor left: {:?}", event.mark_instance().unwrap());
    //     },
    // );
    event_stream_manager.register_handler(
        EventStreamConfig {
            types: vec![SceneGraphEventType::Click],
            mark_paths: Some(vec![vec![0, 2, 0]]),
            ..Default::default()
        },
        |event| {
            println!("clicked: {:?}", event);
        },
    );
    // event_stream_manager.register_handler(
    //     EventStreamConfig {
    //         types: vec![SceneGraphEventType::DoubleClick],
    //         // mark_paths: Some(vec![vec![0, 2, 0]]),
    //         ..Default::default()
    //     },
    //     |event| {
    //         println!("double clicked: {:?}", event.mark_instance());
    //     },
    // );
    // event_stream_manager.register_handler(
    //     EventStreamConfig {
    //         types: vec![SceneGraphEventType::KeyPress],
    //         between: Some((
    //             Box::new(EventStreamConfig {
    //                 types: vec![SceneGraphEventType::MarkMouseEnter],
    //                 mark_paths: Some(vec![vec![0, 2, 0]]),
    //                 ..Default::default()
    //             }),
    //             Box::new(EventStreamConfig {
    //                 types: vec![SceneGraphEventType::MarkMouseLeave],
    //                 mark_paths: Some(vec![vec![0, 2, 0]]),
    //                 ..Default::default()
    //             }),
    //         )),
    //         ..Default::default()
    //     },
    //     |event| {
    //         println!("key pressed: {:?}", event);
    //     },
    // );

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let mut app = App {
        canvas: None,
        scene_graph,
        rtree,
        scale,
        event_stream_manager,
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

fn make_scene_graph(
    width: f32,
    height: f32,
    domain_sepal_length: (f32, f32),
    domain_sepal_width: (f32, f32),
    sepal_length: Vec<f32>,
    sepal_width: Vec<f32>,
    species: Vec<String>,
) -> SceneGraph {
    // Build scales
    let x_scale = LinearNumericScale::new(&Default::default())
        .with_domain(domain_sepal_length)
        .with_range((0.0, width))
        .with_round(true);

    let y_scale = LinearNumericScale::new(&Default::default())
        .with_domain(domain_sepal_width)
        .with_range((height, 0.0))
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

    // Make rect mark
    let points = SceneSymbolMark {
        len: sepal_length.len() as u32,
        x: x_scale.scale(&sepal_length),
        y: y_scale.scale(&sepal_width),
        fill: color_scale.scale(&species),
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
            width: width - 1.0,
            height: height - 1.0,
        },
        ..Default::default()
    };

    // Make y-axis
    let y_axis = make_numeric_axis_marks(
        &y_scale,
        "Sepal Width",
        [0.0, 0.0],
        &AxisConfig {
            dimensions: [width, height],
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
            dimensions: [width, height],
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
        inner_width: width,
        inner_height: height,
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
