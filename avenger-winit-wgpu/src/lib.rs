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
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{self, ModifiersState};
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
    State: Send + Sync + 'static,
{
    canvas: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    scale: f32,
    pub avenger_app: Rc<RefCell<AvengerApp<State>>>,
    /// Whether a scene was installed since the last frame.
    /// Replaceable events are dropped until that frame draws.
    render_pending: Rc<Cell<bool>>,
    /// The operations on the app that wait for the one that runs.
    operations: Rc<RefCell<Operations>>,
    pub file_watcher: Option<FileWatcher>,
    window_id: Option<winit::window::WindowId>,
    proxy: EventLoopProxy<WinitWgpuEvent>,
    /// The text that the copy shortcut writes, as the app last gave it, so copying never waits
    /// for the app.
    copy_text: Rc<RefCell<String>>,
    /// The keyboard's modifiers, which the copy shortcut reads.
    modifiers: ModifiersState,
    clipboard: Clipboard,

    #[cfg(not(target_arch = "wasm32"))]
    tokio_runtime: tokio::runtime::Runtime,
}

impl<State> WinitWgpuAvengerApp<State>
where
    State: Send + Sync + 'static,
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
            copy_text: Rc::default(),
            modifiers: ModifiersState::empty(),
            clipboard: Clipboard::default(),
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
    /// Create the canvas, build scenes at the actual window size, and draw them.
    Start {
        window: Window,
        dimensions: CanvasDimensions,
        config: CanvasConfig,
    },
    /// Update the app with an event, and install the scenes that it builds.
    Update(AvengerWindowEvent),
    /// Process a refresh: install any rebuilt scenes, or draw again for a redraw.
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
    State: Send + Sync + 'static,
{
    /// Dispatch a window event and install any rebuilt scenes.
    /// Skip replaceable events while a render is pending.
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
            copy_text: self.copy_text.clone(),
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
    State: Send + Sync + 'static,
{
    app: Rc<RefCell<AvengerApp<State>>>,
    canvas: Rc<RefCell<Option<WindowCanvas<'static>>>>,
    operations: Rc<RefCell<Operations>>,
    render_pending: Rc<Cell<bool>>,
    proxy: EventLoopProxy<WinitWgpuEvent>,
    copy_text: Rc<RefCell<String>>,
}

impl<State> Host<State>
where
    State: Send + Sync + 'static,
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

    /// Create the canvas and draw scenes at the actual window size.
    async fn start(&self, window: Window, dimensions: CanvasDimensions, config: CanvasConfig) {
        let mut canvas = match WindowCanvas::new(window, dimensions, config).await {
            Ok(canvas) => canvas,
            Err(e) => {
                log::error!("Failed to create canvas: {e:?}");
                return;
            }
        };
        // The window may open at a different size than requested, so rebuild the scenes
        // at its actual size.
        let size = canvas.dimensions().size;
        let built = self.app.borrow().size();
        if size != built {
            let event = AvengerWindowEvent::WindowResize(WindowResizeEvent { size });
            self.update(&event).await;
        }
        let app = self.app.borrow();
        for (index, scene) in app.scenes().iter().enumerate() {
            if let Err(e) = canvas.set_layer(index, scene, app.text_engine()) {
                log::error!("Failed to set scene {index}: {e:?}");
            }
        }
        canvas.window().set_cursor(cursor_icon(app.cursor()));
        *self.copy_text.borrow_mut() = app.copy_text().to_string();
        drop(app);
        canvas.window().request_redraw();
        *self.canvas.borrow_mut() = Some(canvas);
        // Requests that came before the host listened, such as images that arrived during the
        // first build, are pending.
        let _ = self.proxy.send_event(WinitWgpuEvent::Refresh);
    }

    /// Update the app with an event, returning the scenes that it builds.
    #[allow(clippy::await_holding_refcell_ref)]
    async fn update(&self, event: &AvengerWindowEvent) -> Changes {
        self.app.borrow_mut().update(event, Instant::now()).await
    }

    /// Process the pending refresh once the canvas exists.
    /// Earlier requests remain pending until after the initial scenes are installed.
    #[allow(clippy::await_holding_refcell_ref)]
    async fn refresh(&self) -> Changes {
        if self.canvas.borrow().is_none() {
            return Changes::default();
        }
        self.app.borrow_mut().refresh().await
    }

    /// Install changed scenes and request a redraw when any changed or `changes.redraw` is true.
    /// Show a changed cursor, and keep changed copy text for the copy shortcut.
    fn install(&self, changes: &Changes) {
        let mut canvas = self.canvas.borrow_mut();
        let Some(canvas) = canvas.as_mut() else {
            return;
        };
        let app = self.app.borrow();
        for &index in &changes.scenes {
            if let Err(e) = canvas.set_layer(index, &app.scenes()[index], app.text_engine()) {
                log::error!("Failed to set scene {index}: {e:?}");
            }
        }
        if !changes.scenes.is_empty() {
            self.render_pending.set(true);
        }
        if !changes.scenes.is_empty() || changes.redraw {
            canvas.window().request_redraw();
        }
        if let Some(cursor) = changes.cursor {
            canvas.window().set_cursor(cursor_icon(cursor));
        }
        if let Some(text) = &changes.copy_text {
            *self.copy_text.borrow_mut() = text.clone();
        }
    }
}

/// Winit's cursor for an app's cursor style.
fn cursor_icon(style: CursorStyle) -> CursorIcon {
    match style {
        CursorStyle::Default => CursorIcon::Default,
        CursorStyle::Grab => CursorIcon::Grab,
        CursorStyle::Grabbing => CursorIcon::Grabbing,
        CursorStyle::Text => CursorIcon::Text,
    }
}

/// Whether a key press is the copy shortcut: C with Control, or with Command on macOS, which
/// winit calls the super key.
fn copy_shortcut(key: &keyboard::Key, pressed: bool, modifiers: ModifiersState) -> bool {
    let c = matches!(key, keyboard::Key::Character(c) if c.eq_ignore_ascii_case("c"));
    pressed && c && (modifiers.control_key() || modifiers.super_key())
}

/// The system clipboard, which the copy shortcut writes the app's copy text to.
#[derive(Default)]
struct Clipboard {
    /// The native clipboard, opened at the first copy and kept, since on Linux the copied text
    /// lasts only while its clipboard does.
    #[cfg(not(target_arch = "wasm32"))]
    native: Option<arboard::Clipboard>,
}

impl Clipboard {
    fn write(&mut self, text: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.native.is_none() {
                self.native = arboard::Clipboard::new()
                    .map_err(|e| log::error!("Failed to open the clipboard: {e}"))
                    .ok();
            }
            if let Some(clipboard) = &mut self.native {
                if let Err(e) = clipboard.set_text(text) {
                    log::error!("Failed to copy: {e}");
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(window) = web_sys::window() else {
                return;
            };
            let written = window.navigator().clipboard().write_text(text);
            spawn_local(async move {
                if let Err(e) = wasm_bindgen_futures::JsFuture::from(written).await {
                    log::error!("Failed to copy: {e:?}");
                }
            });
        }
    }
}

impl<State> ApplicationHandler<WinitWgpuEvent> for WinitWgpuAvengerApp<State>
where
    State: Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(WindowAttributes::default().with_resizable(true))
            .expect("Failed to create window");

        #[cfg(target_arch = "wasm32")]
        self.setup_wasm_canvas(&window);

        self.window_id = Some(window.id());
        let dimensions = CanvasDimensions {
            size: self.avenger_app.borrow().size(),
            scale: self.scale,
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

        // The copy shortcut writes the app's copy text, and its key still reaches the app.
        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.modifiers = modifiers.state();
        }
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            let pressed = key.state == ElementState::Pressed && !key.repeat;
            if copy_shortcut(&key.logical_key, pressed, self.modifiers) {
                let text = self.copy_text.borrow().clone();
                if !text.is_empty() {
                    self.clipboard.write(&text);
                }
            }
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
                // Escape is the app's, such as to clear a selection, so only closing the window
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

#[cfg(test)]
mod tests {
    use super::*;

    /// C copies with Control or Command, once for each press, and not alone or on release.
    #[test]
    fn the_copy_shortcut_is_c_with_control_or_command() {
        let c = keyboard::Key::Character("c".into());
        assert!(copy_shortcut(&c, true, ModifiersState::CONTROL));
        assert!(copy_shortcut(&c, true, ModifiersState::SUPER));
        let upper = keyboard::Key::Character("C".into());
        assert!(copy_shortcut(&upper, true, ModifiersState::SUPER));
        assert!(!copy_shortcut(&c, true, ModifiersState::empty()));
        assert!(!copy_shortcut(&c, false, ModifiersState::CONTROL));
        let v = keyboard::Key::Character("v".into());
        assert!(!copy_shortcut(&v, true, ModifiersState::CONTROL));
    }
}
