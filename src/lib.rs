pub mod mark_renderers;
pub mod canvas;
pub mod specs;

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
use crate::canvas::Canvas;
use crate::mark_renderers::rect::RectInstance;
use crate::mark_renderers::symbol::{SymbolInstance, SymbolMarkRenderer};
use crate::specs::group::GroupItemSpec;
use crate::specs::mark::MarkSpec;


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
    window.set_inner_size(Size::Physical(PhysicalSize::new(500 + 2 * origin[1] as u32, 200 + 2 * origin[1] as u32)));

    let mut state = Canvas::new(window, origin).await;

    // state.resize(PhysicalSize::new(500, 200));

    let scene: MarkSpec = serde_json::from_str(include_str!("../tests/specs/circles.sg.json")).unwrap();
    let MarkSpec::Group(group) = scene else { panic!() };
    let group_item = &group.items[0];
    state.add_rect_mark(&[
        RectInstance {
            position: [group_item.x, group_item.y],
            color: [1.0, 1.0, 1.0],
            width: group_item.width.unwrap(),
            height: group_item.height.unwrap(),
        },
    ]);
    match &group.items[0].items[0] {
        MarkSpec::Symbol(symbol_container) => {
            let symbol_instances = SymbolInstance::from_specs(symbol_container.items.as_slice());
            state.add_symbol_mark(symbol_instances.as_slice());
        }
        _ => {}
    }

    // state.add_symbol_mark(&[
    //     SymbolInstance {position: [0.0, 0.0], color: [0.5, 0.0, 0.5]},
    //     SymbolInstance {position: [30.0, 55.0], color: [0.5, 0.3, 0.5]},
    //     SymbolInstance {position: [-80.0, -46.0], color: [0.5, 0.6, 0.5]},
    // ]);
    //
    // state.add_symbol_mark(&[
    //     SymbolInstance {position: [-200.0, 0.0], color: [1.0, 0.0, 0.0]},
    //     SymbolInstance {position: [-200.0, 55.0], color: [0.0, 1.0, 0.0]},
    //     SymbolInstance {position: [-200.0, -46.0], color: [0.0, 0.0, 1.0]},
    // ]);

    state.add_rect_mark(&[
        RectInstance {
            position: [100.0, 0.0],
            color: [1.0, 0.0, 1.0],
            width: 20.0,
            height: 50.0,
        },
        RectInstance {
            position: [-100.0, 0.0],
            color: [0.0, 1.0, 1.0],
            width: 40.0,
            height: 100.0,
        }
    ]);

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == state.window().id() => {
                if !state.input(event) {
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
                            state.resize(*physical_size);
                        }
                        WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                            // new_inner_size is &&mut so w have to dereference it twice
                            state.resize(**new_inner_size);
                        }
                        _ => {}
                    }
                }
            }
            Event::RedrawRequested(window_id) if window_id == state.window().id() => {
                state.update();
                match state.render() {
                    Ok(_) => {}
                    // Reconfigure the surface if it's lost or outdated
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        state.resize(state.get_size())
                    }
                    // The system is out of memory, we should probably quit
                    Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,

                    Err(wgpu::SurfaceError::Timeout) => log::warn!("Surface timeout"),
                }
            }
            Event::RedrawEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                state.window().request_redraw();
            }
            _ => {}
        }
    });
}
