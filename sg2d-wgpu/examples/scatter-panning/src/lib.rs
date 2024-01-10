use rand::Rng;
use sg2d::scene_graph::SceneGraph;
use sg2d_vega::dims::VegaSceneGraphDims;
use sg2d_vega::scene_graph::VegaSceneGraph;
use winit::event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;
use sg2d::marks::group::{GroupBounds, SceneGroup};
use sg2d::marks::mark::SceneMark;
use sg2d::marks::symbol::{SymbolMark, SymbolShape};
use sg2d::value::EncodingValue;
use sg2d_vega::marks::symbol::shape_to_path;
use sg2d_wgpu::canvas::{Canvas, WindowCanvas};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

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

    // Extract dims and set window size
    let mut rng = rand::thread_rng();
    let origin = [20.0, 20.0];
    let inner_width = 400.0;
    let inner_height = 400.0;
    let margin = 20.0;
    let width = inner_width + 2.0 * margin;
    let height = inner_height + 2.0 * margin;
    let scale = 2.0;

    let shape = shape_to_path("circle").unwrap();

    let mut x: Vec<f32> = Vec::new();
    let mut y: Vec<f32> = Vec::new();
    let mut fill: Vec<[f32; 4]> = Vec::new();
    let mut size: Vec<f32> = Vec::new();

    let n = 100000;
    for _ in 0..n {
        x.push(rng.gen::<f32>() * inner_width + margin);
        y.push(rng.gen::<f32>() * inner_height + margin);
        size.push(rng.gen::<f32>() * 300.0 + 100.0);
        fill.push([0.5, 0.5, rng.gen::<f32>(), 0.4]);
    }

    let scene_graph = make_sg(width, height, &shape, &x, &y, &fill, &size, 0.0, 0.0);

    // Save to png
    let mut canvas = WindowCanvas::new(window, width, height, scale)
        .await
        .unwrap();

    canvas.set_scene(&scene_graph).unwrap();

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
                        WindowEvent::CursorMoved { device_id, position, modifiers } => {
                            // println!("position: {position:?}");
                            let scene_graph = make_sg(
                                width,
                                height,
                                &shape,
                                &x,
                                &y,
                                &fill,
                                &size,
                                (position.x / scale as f64) as f32 - 100.0,
                                (position.y / scale as f64) as f32 - 100.0
                            );
                            canvas.set_scene(&scene_graph).unwrap();
                            canvas.window().request_redraw();
                        }
                        WindowEvent::MouseInput { device_id, state, button, modifiers } => {
                            // println!("state: {state:?}, button: {button:?}");
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

fn make_sg(width: f32, height: f32, shape: &SymbolShape, x: &[f32], y: &[f32], fill: &[[f32; 4]], size: &[f32], x_offset: f32, y_offset: f32) -> SceneGraph {
    let x: Vec<f32> = x.iter().map(|v| *v + x_offset).collect();
    let y: Vec<f32> = y.iter().map(|v| *v + y_offset).collect();
    let fill: Vec<[f32; 4]> = Vec::from(fill);
    let size: Vec<f32> = Vec::from(size);

    SceneGraph {
        groups: vec![
            SceneGroup {
                bounds: GroupBounds {
                    x: 0.0,
                    y: 0.0,
                    width: None,
                    height: None,
                },
                marks: vec![
                    SceneMark::Symbol(SymbolMark {
                        name: "scatter".to_string(),
                        clip: false,
                        shape: shape.clone(),
                        stroke_width: None,
                        len: x.len() as u32,
                        x: EncodingValue::Array { values: x },
                        y: EncodingValue::Array { values: y },
                        fill: EncodingValue::Array { values: fill },
                        size: EncodingValue::Array { values: size },
                        stroke: EncodingValue::Scalar { value: [0.0, 0.0, 0.0, 0.0] },
                        angle: EncodingValue::Scalar { value: 0.0 },
                    })
                ],
            }
        ],
        width,
        height,
    }
}