use avenger::marks::group::SceneGroup;
use avenger::marks::mark::SceneMark;
use avenger::marks::symbol::{SymbolMark, SymbolShape};
use avenger::marks::value::{ColorOrGradient, EncodingValue};
use avenger::scene_graph::SceneGraph;

use avenger_wgpu::canvas::{Canvas, CanvasDimensions, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use rand::Rng;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use winit::event::{ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoop;
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::WindowBuilder;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
        } else {
            // Initialize logging controlled by RUST_LOG environment variable
            tracing_subscriber::registry()
                .with(fmt::layer().with_span_events(FmtSpan::CLOSE))
                .with(EnvFilter::from_default_env())
                .init();
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

    // Extract dims and set window size
    let mut rng = rand::thread_rng();
    let inner_width = 400.0;
    let inner_height = 400.0;
    let margin = 20.0;
    let width = inner_width + 2.0 * margin;
    let height = inner_height + 2.0 * margin;
    let scale = 2.0;

    let shape = SymbolShape::from_vega_str("circle").unwrap();
    let mut x: Vec<f32> = Vec::new();
    let mut y: Vec<f32> = Vec::new();
    let mut fill: Vec<ColorOrGradient> = Vec::new();
    let mut size: Vec<f32> = Vec::new();

    // let n = 1000000;
    let n = 100000;
    // let n = 50000;
    // let n = 50;
    for _ in 0..n {
        x.push(rng.gen::<f32>() * inner_width + margin);
        y.push(rng.gen::<f32>() * inner_height + margin);
        size.push(rng.gen::<f32>() * 300.0 + 100.0);
        fill.push(ColorOrGradient::Color([0.5, 0.5, rng.gen::<f32>(), 0.4]));
    }

    let scene_graph = make_sg(width, height, &shape, &x, &y, &fill, &size, 0.0, 0.0);

    // Save to png
    let dimensions = CanvasDimensions {
        size: [width, height],
        scale,
    };
    let mut canvas = WindowCanvas::new(window, dimensions, Default::default())
        .await
        .unwrap();

    canvas.set_scene(&scene_graph).unwrap();

    event_loop
        .run(move |event, target| {
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
                                event:
                                    KeyEvent {
                                        logical_key: keyboard::Key::Named(NamedKey::Escape),
                                        state: ElementState::Pressed,
                                        ..
                                    },
                                ..
                            } => {
                                target.exit();
                            }
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
                                            wgpu::SurfaceError::Lost
                                            | wgpu::SurfaceError::Outdated => {
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
                            WindowEvent::CursorMoved { position, .. } => {
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
                                    (position.y / scale as f64) as f32 - 100.0,
                                );
                                canvas.set_scene(&scene_graph).unwrap();
                                canvas.window().request_redraw();
                            }
                            WindowEvent::MouseInput { .. } => {}
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        })
        .expect("Failed to start event loop");
}

fn make_sg(
    width: f32,
    height: f32,
    shape: &SymbolShape,
    x: &[f32],
    y: &[f32],
    fill: &[ColorOrGradient],
    size: &[f32],
    x_offset: f32,
    y_offset: f32,
) -> SceneGraph {
    let x: Vec<f32> = x.iter().map(|v| *v + x_offset).collect();
    let y: Vec<f32> = y.iter().map(|v| *v + y_offset).collect();
    let fill: Vec<ColorOrGradient> = Vec::from(fill);
    let size: Vec<f32> = Vec::from(size);

    SceneGraph {
        groups: vec![SceneGroup {
            name: "".to_string(),
            origin: [0.0, 0.0],
            clip: Default::default(),
            marks: vec![SceneMark::Symbol(SymbolMark {
                name: "scatter".to_string(),
                clip: false,
                shapes: vec![shape.clone()],
                stroke_width: None,
                len: x.len() as u32,
                x: EncodingValue::Array { values: x },
                y: EncodingValue::Array { values: y },
                fill: EncodingValue::Array { values: fill },
                size: EncodingValue::Array { values: size },
                stroke: EncodingValue::Scalar {
                    value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
                },
                angle: EncodingValue::Scalar { value: 0.0 },
                indices: None,
                gradients: vec![],
                shape_index: EncodingValue::Scalar { value: 0 },
                zindex: None,
            })],
            gradients: vec![],
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        }],
        width,
        height,
        origin: [0.0, 0.0],
    }
}
