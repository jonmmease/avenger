mod file_watcher;

use avenger_app::app::AvengerApp;
use avenger_common::canvas::CanvasDimensions;
use avenger_eventstream::window::{WindowEvent as AvengerWindowEvent, WindowFileChangedEvent};
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use std::path::PathBuf;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard;
use winit::keyboard::NamedKey;
use winit::window::{WindowAttributes, WindowId};

pub use file_watcher::FileWatcher;

pub struct WinitWgpuAvengerApp<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    canvas: Option<WindowCanvas<'a>>,
    scale: f32,
    pub avenger_app: AvengerApp<State>,
    render_pending: bool,
    tokio_runtime: tokio::runtime::Runtime,
    file_watcher: Option<FileWatcher>,
}

impl<'a, State> WinitWgpuAvengerApp<'a, State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn new_and_event_loop(
        avenger_app: AvengerApp<State>,
        scale: f32,
        tokio_runtime: tokio::runtime::Runtime,
    ) -> (Self, EventLoop<AvengerWindowEvent>) {

        // Create event loop with AvengerWindowEvent as custom event type
        // We save a proxy and return the original event loop
        let event_loop = EventLoop::<AvengerWindowEvent>::with_user_event().build().expect("Failed to build event loop");
        let watched_files = avenger_app.get_watched_files();
        let file_watcher = if !watched_files.is_empty() {
            Some(FileWatcher::new(event_loop.create_proxy(), watched_files).expect("Failed to create file watcher"))
        } else {
            None
        };

        // println the watched file
        let winit_app = Self {
            canvas: None,
            scale,
            avenger_app,
            render_pending: false,
            tokio_runtime,
            file_watcher,
        };

        (winit_app, event_loop)
    }
}

impl<'a, State> ApplicationHandler<AvengerWindowEvent> for WinitWgpuAvengerApp<'a, State>
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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AvengerWindowEvent) {
        // Process file change events and other custom events
        if !self.render_pending || !event.skip_if_render_pending() {
            if let Some(canvas) = &mut self.canvas {

                match self
                    .tokio_runtime
                    .block_on(self.avenger_app.update(&event, Instant::now()))
                {
                    Ok(Some(scene_graph)) => {
                        canvas.set_scene(&scene_graph).unwrap();
                        canvas.window().request_redraw();
                    }
                    Ok(None) => {
                        // No update needed
                    }
                    Err(e) => {
                        eprintln!("Failed to update app with user event: {:?}", e);
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
        let canvas = match &mut self.canvas {
            Some(canvas) => canvas,
            None => return,
        };
        
        // User events are handled through the application::event method
        
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

                    let _duration = start_time.elapsed(); // Calculate elapsed time
                                                         // println!("Render time: {:?}", _duration); // Print the duration
                }
                event => {
                    if let Some(event) = AvengerWindowEvent::from_winit_event(event, self.scale) {
                        if !self.render_pending || !event.skip_if_render_pending() {
                            if let Some(scene_graph) = self
                                .tokio_runtime
                                .block_on(self.avenger_app.update(&event, Instant::now()))
                                .expect("Failed to update app")
                            {
                                // println!("update scene graph");
                                canvas.set_scene(&scene_graph).unwrap();
                                self.render_pending = true;
                                canvas.window().request_redraw();
                            }
                        } else {
                            // println!("skip update scene graph");
                        }
                    }
                }
            }
        }
    }
}