use avenger_app::app::AvengerApp;
use avenger_common::canvas::CanvasDimensions;
use avenger_eventstream::window::WindowEvent as AvengerWindowEvent;
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};

pub struct WinitWgpuAvengerApp<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    canvas: Option<WindowCanvas<'a>>,
    scale: f32,
    avenger_app: AvengerApp<State>,
    render_pending: bool,
    tokio_runtime: tokio::runtime::Runtime,
}

impl<'a, State> WinitWgpuAvengerApp<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn new(
        avenger_app: AvengerApp<State>,
        scale: f32,
        tokio_runtime: tokio::runtime::Runtime,
    ) -> Self {
        Self {
            canvas: None,
            scale,
            avenger_app,
            render_pending: false,
            tokio_runtime,
        }
    }
}

impl<'a, State> ApplicationHandler for WinitWgpuAvengerApp<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(false))
            .expect("Failed to create window");

        let dimensions = CanvasDimensions {
            size: [
                self.avenger_app.scene_graph().width,
                self.avenger_app.scene_graph().height,
            ],
            scale: self.scale,
        };

        let mut canvas = self
            .tokio_runtime
            .block_on(WindowCanvas::new(window, dimensions, Default::default()))
            .expect("Failed to create canvas");

        // Initial render
        canvas.set_scene(&self.avenger_app.scene_graph()).unwrap();

        // Request initial redraw
        self.render_pending = true;
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
                    let start_time = Instant::now(); // Start timing

                    canvas.update();

                    println!("render");

                    match canvas.render() {
                        Ok(_) => {
                            self.render_pending = false;
                        }
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

                    let duration = start_time.elapsed(); // Calculate elapsed time
                    println!("Render time: {:?}", duration); // Print the duration
                }
                event => {
                    if let Some(event) = AvengerWindowEvent::from_winit_event(event, self.scale) {
                        if let Some(scene_graph) = self
                            .tokio_runtime
                            .block_on(self.avenger_app.update(&event, Instant::now()))
                            .expect("Failed to update app")
                        {
                            if !self.render_pending || !event.skip_if_render_pending() {
                                // if true {
                                println!("update scene graph");
                                canvas.set_scene(&scene_graph).unwrap();
                                self.render_pending = true;
                                canvas.window().request_redraw();
                            }
                        }
                    }
                }
            }
        }
    }
}
