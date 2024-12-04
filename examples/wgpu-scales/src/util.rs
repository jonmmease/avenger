use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};
use avenger_common::value::{ColorOrGradient, ScalarOrArray};
use avenger_scales::band::BandScale;
use avenger_scales::band::opts::BandScaleOptions;
use avenger_scales::color::numeric_color::NumericColorScale;
use avenger_scales::color::Srgba;
use avenger_scales::identity::IdentityScale;
use avenger_scales::numeric::linear::LinearNumericScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct App<'a> {
    canvas: Option<WindowCanvas<'a>>,
    scene_graph: SceneGraph,
    scale: f32,
}

impl<'a> ApplicationHandler for App<'a> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default())
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
                    let canvas = web_sys::Element::from(window.canvas().expect("Failed to get canvas"));
                    dst.append_child(&canvas).ok()?;
                    Some(())
                })
                .expect("Couldn't append canvas to document body.");
        }

        let dimensions = CanvasDimensions {
            size: [self.scene_graph.width, self.scene_graph.height],
            scale: self.scale,
        };
        
        let mut canvas = pollster::block_on(WindowCanvas::new(
            window, 
            dimensions,
            Default::default()
        )).expect("Failed to create canvas");

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
    let x_values: Vec<_> = ["A", "B", "C", "D", "E", "F", "G", "H", "I"].iter().map(
        |s| s.to_string()
    ).collect();

    let y_values = vec![28.0f32, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0];

    // Build scales
    let x_scale = BandScale::try_new(x_values.clone()).unwrap()
        .range((0.0, 200.0)).unwrap()
        .padding(0.2).unwrap();

    let y_scale = LinearNumericScale::new().domain((0.0, 100.0)).range((200.0, 0.0));
    let color_scale = NumericColorScale::new(y_scale.clone(), vec![
        Srgba::new(0.9, 0.9, 0.9, 1.0),
        Srgba::new(0.1, 0.1, 0.9, 1.0),
    ]);
    let id_scale = IdentityScale::new();

    // Make rect mark
    let rect = SceneRectMark {
        len: x_values.len() as u32,
        x: x_scale.scale(&x_values, &BandScaleOptions{ band: Some(0.0), ..Default::default() }),
        x2: Some(x_scale.scale(&x_values, &BandScaleOptions{ band: Some(1.0), ..Default::default() })),
        y: y_scale.scale(0.0, &Default::default()),
        y2: Some(y_scale.scale(&y_values, &Default::default())),
        fill: color_scale.scale(&y_values),
        stroke: ColorOrGradient::Color([1.0, 0.0, 1.0, 1.0]).into(),
        stroke_width: 1.0f32.into(),
        ..Default::default()
    };

    // Wrap in group
    let group = SceneGroup {
        origin: [20.0, 20.0],
        marks: vec![SceneMark::Rect(rect)],
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        groups: vec![group],
        width: 240.0,
        height: 240.0,
        origin: [0.0; 2],
    };

    println!("{:#?}", scene_graph);

    let scale = 2.0;
    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let mut app = App { 
        canvas: None,
        scene_graph,
        scale,
    };
    
    event_loop.run_app(&mut app).expect("Failed to run event loop");
}