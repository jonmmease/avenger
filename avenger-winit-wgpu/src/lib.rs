use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use avenger_app::app::{AvengerApp, Changes};
use avenger_common::canvas::CanvasDimensions;
use avenger_common::cursor::CursorStyle;
use avenger_common::time::Instant;
use avenger_eventstream::window::{WindowEvent as AvengerWindowEvent, WindowResizeEvent};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

#[cfg(not(target_arch = "wasm32"))]
mod file_watcher;
#[cfg(not(target_arch = "wasm32"))]
pub use file_watcher::FileWatcher;

#[cfg(target_arch = "wasm32")]
pub struct FileWatcher;

/// The host's user events.
#[derive(Debug, Clone)]
pub enum WinitWgpuEvent {
    /// An event for the app, such as a changed file.
    App(AvengerWindowEvent),
    /// The app's waker asks for a refresh.
    Refresh,
}

pub struct WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    canvas: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    scale: f32,
    pub avenger_app: Rc<RefCell<AvengerApp<State>>>,
    /// Whether a scene installed since the last frame, so an event that a later one replaces
    /// can wait for the next.
    render_pending: Rc<Cell<bool>>,
    /// The operations on the app that wait for the one that runs.
    operations: Rc<RefCell<Operations>>,
    pub file_watcher: Option<FileWatcher>,
    window_id: Option<winit::window::WindowId>,
    proxy: EventLoopProxy<WinitWgpuEvent>,

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
        let event_loop = EventLoop::<WinitWgpuEvent>::with_user_event()
            .build()
            .expect("Failed to build event loop");

        // The app's waker sends a refresh event. The proxy is Send on every platform, and Sync
        // on some, so a mutex shares it between threads.
        let proxy = std::sync::Mutex::new(event_loop.create_proxy());
        avenger_app.waker().set_notify(move || {
            if let Ok(proxy) = proxy.lock() {
                let _ = proxy.send_event(WinitWgpuEvent::Refresh);
            }
        });

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
            canvas: Rc::new(RefCell::new(None)),
            scale,
            avenger_app: Rc::new(RefCell::new(avenger_app)),
            render_pending: Rc::default(),
            operations: Rc::default(),
            file_watcher,
            window_id: None,
            proxy: event_loop.create_proxy(),
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        };

        (winit_app, event_loop)
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

/// An operation on the app. Each holds the app across its awaits, so the host runs them one at a
/// time, in order.
enum Operation {
    /// Create the window's canvas, build the scene at the size that the window took, and draw
    /// it.
    Start {
        window: Window,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    },
    /// Update the app with an event, and install the scene and the overlay that it builds.
    Update(AvengerWindowEvent),
    /// Do what the app's pending refresh asks: draw the scene again, or install the scene and
    /// the overlay that the app builds again.
    Refresh,
}

/// The operations that wait, and whether one runs.
#[derive(Default)]
struct Operations {
    waiting: VecDeque<Operation>,
    running: bool,
}

impl<State> WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// Updates the app with a window event, and shows the scene that the app builds for it.
    /// While a render is pending, an event that a later one replaces is skipped.
    fn dispatch(&mut self, event: AvengerWindowEvent) {
        if self.render_pending.get() && event.skip_if_render_pending() {
            return;
        }
        self.run(Operation::Update(event));
    }

    /// Run `operation` after the operations that wait, at once when none runs. In the
    /// browser, an operation that awaits lets events arrive, which wait their turn.
    fn run(&mut self, operation: Operation) {
        {
            let mut operations = self.operations.borrow_mut();
            operations.waiting.push_back(operation);
            if operations.running {
                return;
            }
            operations.running = true;
        }
        let host = Host {
            app: self.avenger_app.clone(),
            canvas: self.canvas.clone(),
            operations: self.operations.clone(),
            render_pending: self.render_pending.clone(),
            proxy: self.proxy.clone(),
        };
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                spawn_local(host.drain());
            } else {
                self.tokio_runtime.block_on(host.drain());
            }
        }
    }
}

/// What the operations share with the host.
struct Host<State>
where
    State: Clone + Send + Sync + 'static,
{
    app: Rc<RefCell<AvengerApp<State>>>,
    canvas: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    operations: Rc<RefCell<Operations>>,
    render_pending: Rc<Cell<bool>>,
    proxy: EventLoopProxy<WinitWgpuEvent>,
}

impl<State> Host<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// Run the waiting operations in order until none waits.
    async fn drain(self) {
        loop {
            let operation = {
                let mut operations = self.operations.borrow_mut();
                match operations.waiting.pop_front() {
                    Some(operation) => operation,
                    None => {
                        operations.running = false;
                        return;
                    }
                }
            };
            let changes = match operation {
                Operation::Start {
                    window,
                    dimensions,
                    config,
                } => {
                    self.start(window, dimensions, config).await;
                    Changes::default()
                }
                Operation::Update(event) => self.update(&event).await,
                Operation::Refresh => self.refresh().await,
            };
            self.install(&changes);
        }
    }

    /// Create the canvas, and draw the app's scene at the size that the window took.
    async fn start(&self, window: Window, dimensions: CanvasDimensions, config: CanvasConfig) {
        let mut canvas = match WindowCanvas::new(window, dimensions, config).await {
            Ok(canvas) => canvas,
            Err(e) => {
                log::error!("Failed to create canvas: {e:?}");
                return;
            }
        };
        // The window can take another size than the scene asked for, so the app builds its
        // scene again at the size that the window took.
        let size = canvas.dimensions().size;
        let built = {
            let app = self.app.borrow();
            [app.scene_graph().width, app.scene_graph().height]
        };
        if size != built {
            let event = AvengerWindowEvent::WindowResize(WindowResizeEvent { size });
            self.update(&event).await;
        }
        let app = self.app.borrow();
        let installed = canvas
            .set_scene(app.scene_graph(), app.text_engine())
            .and_then(|()| canvas.set_overlay(app.overlay(), app.text_engine()));
        match installed {
            Ok(()) => canvas.window().request_redraw(),
            Err(e) => log::error!("Failed to set scene: {e:?}"),
        }
        canvas.window().set_cursor(cursor_icon(app.cursor()));
        drop(app);
        *self.canvas.borrow_mut() = Some(canvas);
        // Requests that came before the host listened, such as images that arrived during the
        // first build, are pending.
        let _ = self.proxy.send_event(WinitWgpuEvent::Refresh);
    }

    /// Update the app with an event, returning the scene and the overlay that it builds.
    #[allow(clippy::await_holding_refcell_ref)]
    async fn update(&self, event: &AvengerWindowEvent) -> Changes {
        match self.app.borrow_mut().update(event, Instant::now()).await {
            Ok(changes) => changes,
            Err(e) => {
                log::error!("Failed to update app: {e:?}");
                Changes::default()
            }
        }
    }

    /// Do what the app's pending refresh asks. Until the canvas exists, the request stays
    /// pending for the refresh that follows the first scene.
    #[allow(clippy::await_holding_refcell_ref)]
    async fn refresh(&self) -> Changes {
        if self.canvas.borrow().is_none() {
            return Changes::default();
        }
        match self.app.borrow_mut().refresh().await {
            Ok(changes) => changes,
            Err(e) => {
                log::error!("Failed to refresh app: {e:?}");
                Changes::default()
            }
        }
    }

    /// Install the new scene and overlay on the canvas, and draw a frame when there's either or
    /// the changes ask for a redraw.
    fn install(&self, changes: &Changes) {
        let mut canvas = self.canvas.borrow_mut();
        let Some(canvas) = canvas.as_mut() else {
            return;
        };
        let app = self.app.borrow();
        if let Some(scene) = &changes.scene {
            if let Err(e) = canvas.set_scene(scene, app.text_engine()) {
                log::error!("Failed to set scene: {e:?}");
            }
        }
        if let Some(overlay) = &changes.overlay {
            if let Err(e) = canvas.set_overlay(overlay, app.text_engine()) {
                log::error!("Failed to set overlay: {e:?}");
            }
        }
        if changes.scene.is_some() || changes.overlay.is_some() {
            self.render_pending.set(true);
        }
        if changes.scene.is_some() || changes.overlay.is_some() || changes.redraw {
            canvas.window().request_redraw();
        }
        if let Some(cursor) = changes.cursor {
            canvas.window().set_cursor(cursor_icon(cursor));
        }
    }
}

/// Winit's cursor for an app's cursor style.
fn cursor_icon(style: CursorStyle) -> CursorIcon {
    match style {
        CursorStyle::Default => CursorIcon::Default,
        CursorStyle::Pointer => CursorIcon::Pointer,
        CursorStyle::Crosshair => CursorIcon::Crosshair,
        CursorStyle::Grab => CursorIcon::Grab,
        CursorStyle::Grabbing => CursorIcon::Grabbing,
        CursorStyle::Move => CursorIcon::Move,
        CursorStyle::Text => CursorIcon::Text,
        CursorStyle::ResizeHorizontal => CursorIcon::EwResize,
        CursorStyle::ResizeVertical => CursorIcon::NsResize,
        CursorStyle::ResizeNwSe => CursorIcon::NwseResize,
        CursorStyle::ResizeNeSw => CursorIcon::NeswResize,
    }
}

impl<State> ApplicationHandler<WinitWgpuEvent> for WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(true))
            .expect("Failed to create window");

        #[cfg(target_arch = "wasm32")]
        self.setup_wasm_canvas(&window);

        self.window_id = Some(window.id());
        let dimensions = {
            let app = self.avenger_app.borrow();
            CanvasDimensions {
                size: [app.scene_graph().width, app.scene_graph().height],
                scale: self.scale,
            }
        };
        let config = CanvasConfig {
            images: self.avenger_app.borrow().images(),
            ..Default::default()
        };
        self.run(Operation::Start {
            window,
            dimensions,
            config,
        });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WinitWgpuEvent) {
        match event {
            WinitWgpuEvent::Refresh => self.run(Operation::Refresh),
            // A changed file, or another event for the app.
            WinitWgpuEvent::App(event) => self.dispatch(event),
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
                // Escape is the app's, such as to cancel a gesture, so only closing the window
                // ends it.
                WindowEvent::CloseRequested => {
                    *self.canvas.borrow_mut() = None;
                    _event_loop.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.resize(physical_size);
                        canvas.window().request_redraw();
                    }
                    // The app builds its scene again at the window's new size.
                    if let Some(event) = AvengerWindowEvent::from_winit_event(
                        WindowEvent::Resized(physical_size),
                        self.scale,
                    ) {
                        self.dispatch(event);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.update();

                        match canvas.render() {
                            Ok(_) => {
                                self.render_pending.set(false);
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
                        self.dispatch(event);
                    }
                }
            }
        }
    }
}
