use avenger_app::app::AvengerApp;
use avenger_common::canvas::CanvasDimensions;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::runtime::{RuntimeHostCommand, RuntimeWakeEvent};
use winit::window::CursorIcon;
mod wake_scheduler;
use avenger_eventstream::window::WindowEvent as AvengerWindowEvent;
use avenger_wgpu::canvas::{Canvas, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use wake_scheduler::RuntimeWakeScheduler;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{WindowAttributes, WindowId};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

#[cfg(not(target_arch = "wasm32"))]
mod file_watcher;
#[cfg(not(target_arch = "wasm32"))]
pub use file_watcher::FileWatcher;

#[cfg(target_arch = "wasm32")]
pub struct FileWatcher;

/// Application events and scheduled work delivered on the window event loop.
#[derive(Clone)]
pub enum WinitWgpuEvent {
    App(AvengerWindowEvent),
    #[doc(hidden)]
    RuntimeWake {
        event: RuntimeWakeEvent,
        ticket: u64,
    },
}

pub struct WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    canvas: std::rc::Rc<std::cell::RefCell<Option<WindowCanvas<'static>>>>,
    scale: f32,
    wake_scheduler: std::rc::Rc<RuntimeWakeScheduler>,
    pub avenger_app: std::rc::Rc<std::cell::RefCell<AvengerApp<State>>>,
    render_pending: bool,
    pub file_watcher: Option<FileWatcher>,
    window_id: Option<winit::window::WindowId>,

    #[cfg(not(target_arch = "wasm32"))]
    tokio_runtime: tokio::runtime::Runtime,
}

impl<State> WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn new_and_event_loop(
        avenger_app: AvengerApp<State>,
        scale: f32,
        #[cfg(not(target_arch = "wasm32"))] tokio_runtime: tokio::runtime::Runtime,
    ) -> (Self, EventLoop<WinitWgpuEvent>) {
        // Create event loop with AvengerWindowEvent as custom event type
        let event_loop = EventLoop::<WinitWgpuEvent>::with_user_event()
            .build()
            .expect("Failed to build event loop");

        // File watching is only supported on desktop
        #[cfg(not(target_arch = "wasm32"))]
        let file_watcher = {
            let watched_files = avenger_app.get_watched_files();
            if !watched_files.is_empty() {
                Some(
                    FileWatcher::new(event_loop.create_proxy(), watched_files)
                        .expect("Failed to create file watcher"),
                )
            } else {
                None
            }
        };
        #[cfg(target_arch = "wasm32")]
        let file_watcher = None;

        let winit_app = Self {
            canvas: std::rc::Rc::new(std::cell::RefCell::new(None)),
            scale,
            wake_scheduler: std::rc::Rc::new(RuntimeWakeScheduler::new(event_loop.create_proxy())),
            avenger_app: std::rc::Rc::new(std::cell::RefCell::new(avenger_app)),
            render_pending: false,
            file_watcher,
            window_id: None,
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        };

        (winit_app, event_loop)
    }

    fn dispatch_avenger_event(&mut self, event: AvengerWindowEvent) {
        if self.render_pending && event.skip_if_render_pending() {
            return;
        }
        let app = self.avenger_app.clone();
        let canvas = self.canvas.clone();
        let scheduler = self.wake_scheduler.clone();
        // Browser builders may await local tasks while the application stays borrowed.
        #[allow(clippy::await_holding_refcell_ref)]
        let update = async move {
            let result = app
                .borrow_mut()
                .update_with_status(&event, Instant::now())
                .await;
            let update = match result {
                Ok(update) => update,
                Err(error) => {
                    log::error!("failed to update application: {error}");
                    return false;
                }
            };
            for command in update.status.commands {
                match command {
                    RuntimeHostCommand::RequestWakeup {
                        key,
                        deadline,
                        generation,
                    } => scheduler.request(key, deadline, generation),
                    RuntimeHostCommand::CancelWakeup { key } => scheduler.cancel(&key),
                }
            }
            if let Some(canvas) = canvas.borrow_mut().as_mut() {
                if let Some(cursor) = update.status.cursor {
                    canvas.window().set_cursor(cursor_style_to_winit(cursor));
                }
                if let Some(scene) = update.scene_graph {
                    if let Err(error) = canvas.set_scene(&scene) {
                        log::error!("failed to set scene: {error}");
                        return false;
                    }
                    canvas.window().request_redraw();
                    return true;
                }
            }
            false
        };
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            update.await;
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.render_pending |= self.tokio_runtime.block_on(update);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn setup_wasm_canvas(&self, window: &winit::window::Window) {
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
}

impl<State> ApplicationHandler<WinitWgpuEvent> for WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(false))
            .expect("Failed to create window");

        #[cfg(target_arch = "wasm32")]
        self.setup_wasm_canvas(&window);

        self.window_id = Some(window.id());
        let canvas_shared = self.canvas.clone();

        // Extract scene graph and dimensions in a limited scope to avoid RefCell conflicts
        let (scene_graph, dimensions) = {
            let app_borrowed = self.avenger_app.borrow();
            let scene_graph = app_borrowed.scene_graph().clone();
            let dimensions = CanvasDimensions {
                size: [
                    app_borrowed.scene_graph().width,
                    app_borrowed.scene_graph().height,
                ],
                scale: self.scale,
            };
            (scene_graph, dimensions)
        };

        let canvas_future = WindowCanvas::new(window, dimensions, Default::default());

        let setup_future = async move {
            match canvas_future.await {
                Ok(mut canvas) => {
                    canvas.set_scene(&scene_graph).unwrap();
                    canvas.window().request_redraw();
                    *canvas_shared.borrow_mut() = Some(canvas);
                }
                Err(e) => {
                    log::error!("Failed to create canvas: {e:?}");
                }
            }
        };

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                spawn_local(setup_future);
            } else {
                self.tokio_runtime.block_on(setup_future);
            }
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: WinitWgpuEvent) {
        match event {
            WinitWgpuEvent::App(event) => self.dispatch_avenger_event(event),
            WinitWgpuEvent::RuntimeWake { event, ticket } => {
                if self.wake_scheduler.claim(&event, ticket) {
                    self.dispatch_avenger_event(AvengerWindowEvent::RuntimeWake(event));
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

        // Handle input event first with limited canvas borrow scope
        let input_handled = {
            let mut canvas_borrowed = self.canvas.borrow_mut();
            match canvas_borrowed.as_mut() {
                Some(canvas) => canvas.input(&event),
                None => return, // Canvas not ready yet
            }
        };

        if !input_handled {
            match event {
                WindowEvent::CloseRequested => {
                    self.dispatch_avenger_event(AvengerWindowEvent::WindowCloseRequested);
                    *self.canvas.borrow_mut() = None;
                    _event_loop.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.resize(physical_size);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
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
                                wgpu::SurfaceError::Other => {
                                    log::error!("Other surface error");
                                }
                            },
                            Err(err) => {
                                log::error!("{err:?}");
                            }
                        }
                    }
                }
                event => {
                    if let Some(event) = AvengerWindowEvent::from_winit_event(event, self.scale) {
                        self.dispatch_avenger_event(event);
                    }
                }
            }
        }
    }
}

fn cursor_style_to_winit(style: CursorStyle) -> CursorIcon {
    match style {
        CursorStyle::Default => CursorIcon::Default,
        CursorStyle::Pointer => CursorIcon::Pointer,
        CursorStyle::Text => CursorIcon::Text,
        CursorStyle::Crosshair => CursorIcon::Crosshair,
        CursorStyle::Grab => CursorIcon::Grab,
        CursorStyle::Grabbing => CursorIcon::Grabbing,
        CursorStyle::ResizeHorizontal => CursorIcon::EwResize,
        CursorStyle::ResizeVertical => CursorIcon::NsResize,
        CursorStyle::ResizeNwSe => CursorIcon::NwseResize,
        CursorStyle::ResizeNeSw => CursorIcon::NeswResize,
    }
}
