use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_vega_scenegraph::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use std::cell::RefCell;
use std::rc::Rc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct App {
    canvas_shared: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    scene_graph: SceneGraph,
    scale: f32,
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
            .create_window(WindowAttributes::default())
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
                            log::error!("Failed to create canvas: {:?}", e);
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
                        log::error!("Failed to create canvas: {:?}", e);
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
        canvas_shared: Rc::new(RefCell::new(None)),
        scene_graph,
        scale,
        window_id: None,
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
