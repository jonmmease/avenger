use avenger_common::value::{ColorOrGradient, ScalarOrArray};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::{SceneSymbolMark, SymbolShape};
use avenger_scenegraph::scene_graph::SceneGraph;

use avenger_wgpu::canvas::{Canvas, CanvasDimensions, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use rand::Rng;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
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
    scene_params: SceneParams,
}

struct SceneParams {
    width: f32,
    height: f32,
    shape: SymbolShape,
    x: Vec<f32>,
    y: Vec<f32>,
    fill: Vec<ColorOrGradient>,
    size: Vec<f32>,
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
                    let canvas =
                        web_sys::Element::from(window.canvas().expect("Failed to get canvas"));
                    dst.append_child(&canvas).ok()?;
                    Some(())
                })
                .expect("Couldn't append canvas to document body.");
        }

        let dimensions = CanvasDimensions {
            size: [self.scene_params.width, self.scene_params.height],
            scale: self.scene_params.scale,
        };

        let mut canvas =
            pollster::block_on(WindowCanvas::new(window, dimensions, Default::default()))
                .expect("Failed to create canvas");

        let scene_graph = make_sg(
            self.scene_params.width,
            self.scene_params.height,
            &self.scene_params.shape,
            &self.scene_params.x,
            &self.scene_params.y,
            &self.scene_params.fill,
            &self.scene_params.size,
            0.0,
            0.0,
        );

        canvas.set_scene(&scene_graph).unwrap();

        // Request initial redraw
        canvas.window().request_redraw();

        self.canvas = Some(canvas);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
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
                    // Drop the canvas before exiting
                    self.canvas.take(); // Using take() instead of setting to None
                    event_loop.exit();
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
                                event_loop.exit();
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
                WindowEvent::CursorMoved { position, .. } => {
                    let scene_graph = make_sg(
                        self.scene_params.width,
                        self.scene_params.height,
                        &self.scene_params.shape,
                        &self.scene_params.x,
                        &self.scene_params.y,
                        &self.scene_params.fill,
                        &self.scene_params.size,
                        (position.x / self.scene_params.scale as f64) as f32 - 100.0,
                        (position.y / self.scene_params.scale as f64) as f32 - 100.0,
                    );
                    canvas.set_scene(&scene_graph).unwrap();
                    canvas.window().request_redraw();
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
            tracing_subscriber::registry()
                .with(fmt::layer().with_span_events(FmtSpan::CLOSE))
                .with(EnvFilter::from_default_env())
                .init();
        }
    }

    // Initialize scene parameters
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

    let n = 100000;
    for _ in 0..n {
        x.push(rng.gen::<f32>() * inner_width + margin);
        y.push(rng.gen::<f32>() * inner_height + margin);
        size.push(rng.gen::<f32>() * 300.0 + 100.0);
        fill.push(ColorOrGradient::Color([0.5, 0.5, rng.gen::<f32>(), 0.4]));
    }

    let scene_params = SceneParams {
        width,
        height,
        shape,
        x,
        y,
        fill,
        size,
        scale,
    };

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    let mut app = App {
        canvas: None,
        scene_params,
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
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
            marks: vec![SceneMark::Symbol(SceneSymbolMark {
                name: "scatter".to_string(),
                clip: false,
                shapes: vec![shape.clone()],
                stroke_width: None,
                len: x.len() as u32,
                x: ScalarOrArray::Array(x),
                y: ScalarOrArray::Array(y),
                fill: ScalarOrArray::Array(fill),
                size: ScalarOrArray::Array(size),
                stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                angle: ScalarOrArray::Scalar(0.0),
                indices: None,
                gradients: vec![],
                shape_index: ScalarOrArray::Scalar(0),
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
