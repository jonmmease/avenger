pub mod renderers;
pub mod specs;
pub mod scene;
pub mod error;

use std::iter;
use wgpu::util::DeviceExt;

use winit::{
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};
use winit::dpi::{LogicalSize, PhysicalSize, Size};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use crate::renderers::canvas::{Canvas, PngCanvas};
use crate::renderers::symbol::SymbolMarkRenderer;
use crate::scene::rect::RectInstance;
use crate::scene::scene_graph::SceneGraph;
use crate::scene::symbol::SymbolInstance;
use crate::specs::group::GroupItemSpec;
use crate::specs::mark::{MarkContainerSpec, MarkSpec};
use crate::specs::SceneGraphSpec;


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

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    #[cfg(target_arch = "wasm32")]
    {
        // Winit prevents sizing with CSS, so we have to set
        // the size manually when on web.
        use winit::dpi::PhysicalSize;
        window.set_inner_size(PhysicalSize::new(450, 400));

        use winit::platform::web::WindowExtWebSys;
        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| {
                let dst = doc.get_element_by_id("wasm-example")?;
                let canvas = web_sys::Element::from(window.canvas());
                dst.append_child(&canvas).ok()?;
                Some(())
            })
            .expect("Couldn't append canvas to document body.");
    }

    // State::new uses async code, so we're going to wait for it to finish
    let origin = [20.0f32, 20.0];
    let width = 540.0f32;
    let height = 240.0f32;
    window.set_inner_size(Size::Physical(PhysicalSize::new(width as u32, height as u32)));

    let scene_spec: SceneGraphSpec = serde_json::from_str(include_str!("../tests/specs/circles.sg.json")).unwrap();
    // let scene_spec: SceneGraphSpec = serde_json::from_str(include_str!("../tests/specs/bar.sg.json")).unwrap();
    let scene_graph: SceneGraph = SceneGraph::from_spec(&scene_spec, origin, width, height).expect("Failed to parse scene graph");
    println!("{scene_graph:#?}");

    // Save to png
    let mut png_canvas = PngCanvas::new(width, 256.0, origin).await;
    let mut canvas = Canvas::new(window, origin).await;

    png_canvas.set_scene(&scene_graph);
    let img = png_canvas.render().await.expect("Failed to write PNG image");
    img.save("image2.png").unwrap();

    canvas.set_scene(&scene_graph);

    event_loop.run(move |event, _, control_flow| {
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
                            input:
                            KeyboardInput {
                                state: ElementState::Pressed,
                                virtual_keycode: Some(VirtualKeyCode::Escape),
                                ..
                            },
                            ..
                        } => *control_flow = ControlFlow::Exit,
                        WindowEvent::Resized(physical_size) => {
                            canvas.resize(*physical_size);
                        }
                        WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                            // new_inner_size is &&mut so w have to dereference it twice
                            canvas.resize(**new_inner_size);
                        }
                        _ => {}
                    }
                }
            }
            Event::RedrawRequested(window_id) if window_id == canvas.window().id() => {
                canvas.update();
                match canvas.render() {
                    Ok(_) => {}
                    // Reconfigure the surface if it's lost or outdated
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        canvas.resize(canvas.get_size())
                    }
                    // The system is out of memory, we should probably quit
                    Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,

                    Err(wgpu::SurfaceError::Timeout) => log::warn!("Surface timeout"),
                }
            }
            Event::RedrawEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                canvas.window().request_redraw();
            }
            _ => {}
        }
    });
}
