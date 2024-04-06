use avenger::scene_graph::SceneGraph;
use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::keyboard;
use winit::keyboard::NamedKey;

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

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    #[cfg(target_arch = "wasm32")]
    {
        // Winit prevents sizing with CSS, so we have to set
        // the size manually when on web.
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

    // Load scene graph
    let scene_spec: VegaSceneGraph = serde_json::from_str(include_str!(
        "../../../avenger-vega-test-data/vega-scenegraphs/gradients/symbol_radial_gradient.sg.json"
    ))
    .unwrap();

    let scale = 2.0;

    let scene_graph: SceneGraph = scene_spec
        .to_scene_graph()
        .expect("Failed to parse scene graph");

    // Save to png
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale,
    };
    let mut canvas = WindowCanvas::new(window, dimensions, Default::default())
        .await
        .unwrap();

    canvas.set_scene(&scene_graph).unwrap();

    event_loop.run(move |event, target| {
        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == canvas.window().id() => {
                if !canvas.input(event) {
                    // UPDATED!
                    match event {
                        WindowEvent::CloseRequested
                        | WindowEvent::KeyboardInput {
                            event: KeyEvent {
                                logical_key: keyboard::Key::Named(NamedKey::Escape),
                                state: ElementState::Pressed,
                                ..
                            },
                            ..
                        } => {
                            target.exit();
                        },
                        WindowEvent::Resized(physical_size) => {
                            canvas.resize(*physical_size);
                        }
                        WindowEvent::RedrawRequested => {
                            canvas.update();

                            match canvas.render() {
                                Ok(_) => {}
                                // Reconfigure the surface if it's lost or outdated
                                Err(AvengerWgpuError::SurfaceError(err)) => {
                                    match err {
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                            canvas.resize(canvas.get_size());
                                        }
                                        wgpu::SurfaceError::OutOfMemory => {
                                            // The system is out of memory, we should probably quit
                                            target.exit();
                                        }
                                        wgpu::SurfaceError::Timeout => {
                                            log::warn!("Surface timeout");
                                        }
                                    }
                                }
                                Err(err) => {
                                    log::error!("{:?}", err);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }).expect("Failed to start event loop");
}
