use arrow::array::{ArrayRef, Float32Array, StringArray};
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::ColorOrGradient;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_guides::axis::band::make_band_axis_marks;
use avenger_guides::axis::numeric::make_numeric_axis_marks;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::colorbar::{make_colorbar_marks, ColorbarConfig, ColorbarOrientation};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::{MarkInstance, SceneMark};
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use std::sync::Arc;

use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::linear::LinearScale;
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};

struct App {
    canvas_shared: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    scene_graph: SceneGraph,
    rtree: SceneGraphRTree,
    scale: f32,
    last_hover_mark: Option<MarkInstance>,
    window_id: Option<WindowId>,
}

impl App {
    #[cfg(target_arch = "wasm32")]
    fn setup_wasm_canvas(&self, window: &winit::window::Window) {
        use winit::dpi::PhysicalSize;
        use winit::platform::web::WindowExtWebSys;

        let _ = window.request_inner_size(PhysicalSize::new(450, 400));

        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| {
                let dst = doc.get_element_by_id("wasm-example")?;
                let canvas = web_sys::Element::from(window.canvas().expect("Failed to get canvas"));
                dst.append_child(&canvas).ok()?;
                Some(())
            })
            .expect("Couldn't append canvas to document body.");
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(false))
            .expect("Failed to create window");

        #[cfg(target_arch = "wasm32")]
        self.setup_wasm_canvas(&window);

        self.window_id = Some(window.id());
        let canvas_shared = self.canvas_shared.clone();
        let scene_graph = self.scene_graph.clone();

        let dimensions = CanvasDimensions {
            size: [self.scene_graph.width, self.scene_graph.height],
            scale: self.scale,
        };

        let canvas_future = WindowCanvas::new(window, dimensions, Default::default());

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                wasm_bindgen_futures::spawn_local(async move {
                    match canvas_future.await {
                        Ok(mut canvas) => {
                            canvas.set_scene(&scene_graph).unwrap();
                            canvas.window().request_redraw();
                            *canvas_shared.borrow_mut() = Some(canvas);
                        }
                        Err(e) => {
                            log::error!("Failed to create canvas: {e:?}");
                        }
                    }
                });
            } else {
                match pollster::block_on(canvas_future) {
                    Ok(mut canvas) => {
                        canvas.set_scene(&scene_graph).unwrap();
                        canvas.window().request_redraw();
                        *canvas_shared.borrow_mut() = Some(canvas);
                    }
                    Err(e) => {
                        log::error!("Failed to create canvas: {e:?}");
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Check if this is the correct window
        if Some(window_id) != self.window_id {
            return;
        }

        // Try to get canvas from shared reference
        let mut canvas_borrowed = self.canvas_shared.borrow_mut();
        let canvas = match canvas_borrowed.as_mut() {
            Some(canvas) => canvas,
            None => return, // Canvas not ready yet
        };

        if !canvas.input(&event) {
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
                    *canvas_borrowed = None;
                    _event_loop.exit();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let point = [
                        position.x as f32 / self.scale,
                        position.y as f32 / self.scale,
                    ];
                    let top_mark: Option<MarkInstance> =
                        self.rtree.pick_top_mark_at_point(&point).cloned();

                    if top_mark != self.last_hover_mark {
                        println!("hover: {top_mark:?}");
                    }
                    self.last_hover_mark = top_mark;
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
                            wgpu::SurfaceError::Other => {
                                log::error!("Other surface error");
                            }
                        },
                        Err(err) => {
                            log::error!("{err:?}");
                        }
                    }
                }
                _ => {}
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

    // Initialize data
    let x_values: Vec<_> = ["A", "B", "C", "D", "E", "F", "G", "H", "I"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let x_array = Arc::new(StringArray::from(x_values.clone())) as ArrayRef;

    let y_values = vec![28.0f32, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0];
    let y_array = Arc::new(Float32Array::from(y_values.clone())) as ArrayRef;

    // Build scales
    let width = 200.0;
    let height = 200.0;

    // Make x scales
    let x_scale = BandScale::configured(x_array.clone(), (0.0, width))
        .with_option("padding_inner", 0.2)
        .with_option("padding_outer", 0.2)
        .with_option("band", 0.0);
    let x2_scale = x_scale.clone().with_option("band", 1.0);

    let y_scale = LinearScale::configured((0.0, 100.0), (height, 0.0));

    let color_scale = LinearScale::configured_color((0.0, 100.0), vec!["white", "blue"])
        .with_option("nice", 10.0);

    // Make rect mark
    let rect = SceneRectMark {
        len: x_values.len() as u32,
        x: x_scale.scale_to_numeric(&x_array).unwrap(),
        x2: Some(x2_scale.scale_to_numeric(&x_array).unwrap()),
        y: y_scale.scale_scalar_to_numeric(&0.0.into()).unwrap(),
        y2: Some(y_scale.scale_to_numeric(&y_array).unwrap()),
        fill: color_scale.scale_to_color(&y_array).unwrap(),
        stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 1.0]).into(),
        stroke_width: 1.0f32.into(),
        ..Default::default()
    };

    // make mark group with clipping
    let mark_group = SceneGroup {
        origin: [0.0, 0.0],
        marks: vec![rect.into()],
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
        "My Long Y-Axis Label",
        [0.0, 0.0],
        &AxisConfig {
            dimensions: [width, height],
            orientation: AxisOrientation::Left,
            grid: true,
            format_number: None,
        },
    )
    .unwrap();

    // Make x-axis
    let x_axis = make_band_axis_marks(
        &x_scale,
        "My Long X-Axis Label",
        [0.0, 0.0],
        &AxisConfig {
            dimensions: [width, height],
            orientation: AxisOrientation::Bottom,
            grid: false,
            format_number: None,
        },
    )
    .unwrap();

    // // Make symbol legend
    // let symbol_legend = make_symbol_legend(&SymbolLegendConfig {
    //     text: vec!["First", "Second", "Third", "Fourth", "Fifth"].into(),
    //     // text: vec!["", "", "", "", ""].into(),
    //     // shape: vec![
    //     //     SymbolShape::Circle,
    //     //     SymbolShape::from_vega_str("triangle-up").unwrap(),
    //     //     SymbolShape::from_vega_str("diamond").unwrap(),
    //     // ]
    //     // .into(),
    //     shape: SymbolShape::Circle.into(),
    //     size: vec![10.0, 40.0, 80.0, 120.0, 240.0].into(),
    //     title: None,
    //     stroke: ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]).into(),
    //     stroke_width: Some(1.0),
    //     fill: ColorOrGradient::Color([1.0, 0.0, 1.0, 1.0]).into(),
    //     angle: 0.0.into(),
    //     inner_width: width,
    //     inner_height: height,
    //     ..Default::default()
    // })
    // .unwrap();

    // // Make line legend
    // let line_legend = make_line_legend(&LineLegendConfig {
    //     inner_width: width,
    //     inner_height: height,
    //     text: vec!["First", "Second", "Third"].into(),
    //     stroke: vec![
    //         ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]).into(),
    //         ColorOrGradient::Color([1.0, 0.0, 1.0, 1.0]).into(),
    //         ColorOrGradient::Color([0.0, 1.0, 1.0, 1.0]).into(),
    //     ]
    //     .into(),
    //     stroke_dash: vec![None, Some(vec![6.0, 2.0]), Some(vec![3.0])].into(),
    //     line_length: 20.0,
    //     ..Default::default()
    // })
    // .unwrap();

    // Make colorbar
    let colorbar = make_colorbar_marks(
        &color_scale,
        "My Colorbar",
        [0.0, 0.0],
        &ColorbarConfig {
            orientation: ColorbarOrientation::Right,
            dimensions: [width, height],
            colorbar_width: None,
            colorbar_height: None,
            colorbar_margin: Some(8.0),
            left_padding: None,
            format_number: None,
        },
    )
    .unwrap();

    // Wrap axis and rect in group
    let group = SceneGroup {
        origin: [60.0, 60.0],
        marks: vec![
            y_axis.into(),
            x_axis.into(),
            mark_group.into(),
            // symbol_legend.into(),
            // line_legend.into(),
            colorbar.into(),
        ],
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        marks: vec![SceneMark::Group(group)],
        width: 340.0,
        height: 300.0,
        origin: [0.0; 2],
    };

    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);
    let svg = rtree.to_svg();

    // Only write SVG file in native builds, not in WASM
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::write("geometry.svg", svg).expect("Failed to write SVG file");

    let scale = 2.0;
    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let mut app = App {
        canvas_shared: Rc::new(RefCell::new(None)),
        scene_graph,
        rtree,
        scale,
        last_hover_mark: None,
        window_id: None,
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
