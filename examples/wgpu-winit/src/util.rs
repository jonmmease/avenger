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

    // Load scene graph
    let scene_spec: VegaSceneGraph = serde_json::from_str(include_str!(
        "../../../avenger-vega-test-data/vega-scenegraphs/gradients/symbol_radial_gradient.sg.json"
    ))
    .unwrap();

    let scale = 2.0;
    let scene_graph = scene_spec
        .to_scene_graph()
        .expect("Failed to parse scene graph");

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let mut app = App { 
        canvas: None,
        scene_graph,
        scale,
    };
    
    event_loop.run_app(&mut app).expect("Failed to run event loop");
}
