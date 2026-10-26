use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use avenger_app::app::{AvengerApp, Changes};
use avenger_common::canvas::CanvasDimensions;
use avenger_common::time::Instant;
use avenger_eventstream::window::{WindowEvent as AvengerWindowEvent, WindowResizeEvent};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, WindowCanvas};
use avenger_wgpu::error::AvengerWgpuError;

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{self, ModifiersState};
use winit::window::{Window, WindowAttributes, WindowId};

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
    /// Last reported copy text, available to the shortcut without waiting for an app update.
    copy_text: Rc<RefCell<String>>,
    /// Keyboard modifiers used to recognize the copy shortcut.
    modifiers: ModifiersState,
    /// The latest cursor move skipped while a render was pending.
    skipped_move: SkippedMove,
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
            skipped_move: SkippedMove::default(),
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
        if let Some(canvas) = window.canvas() {
            self.listen_for_paste(canvas);
        }
    }

    /// Send the app the text of each paste while the canvas has focus. winit prevents the
    /// default action of every key press on the canvas, which would cancel the paste that
    /// Command+V or Control+V starts, so a listener that captures the key first keeps it from
    /// winit. The paste event's text needs no clipboard permission.
    #[cfg(target_arch = "wasm32")]
    fn listen_for_paste(&self, canvas: web_sys::HtmlCanvasElement) {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let focused = {
            let (document, canvas) = (document.clone(), web_sys::Element::from(canvas));
            move || document.active_element().as_ref() == Some(&canvas)
        };
        let keys = {
            let focused = focused.clone();
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
                move |event: web_sys::KeyboardEvent| {
                    // Shift and Caps Lock make the key "V".
                    let v = event.key().eq_ignore_ascii_case("v");
                    let paste = (event.ctrl_key() || event.meta_key()) && v;
                    if paste && focused() {
                        event.stop_immediate_propagation();
                    }
                },
            )
        };
        let proxy = self.proxy.clone();
        let paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
            move |event: web_sys::ClipboardEvent| {
                let data = event.clipboard_data().filter(|_| focused());
                let Some(text) = data.and_then(|data| data.get_data("text/plain").ok()) else {
                    return;
                };
                event.prevent_default();
                let _ = proxy.send_event(WinitWgpuEvent::App(AvengerWindowEvent::Paste(text)));
            },
        );
        let target: &web_sys::EventTarget = document.as_ref();
        let added = target
            .add_event_listener_with_callback_and_bool(
                "keydown",
                keys.as_ref().unchecked_ref(),
                true,
            )
            .and_then(|_| {
                target.add_event_listener_with_callback("paste", paste.as_ref().unchecked_ref())
            });
        if let Err(e) = added {
            log::error!("Failed to listen for pastes: {e:?}");
        }
        // The listeners last as long as the page.
        keys.forget();
        paste.forget();
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
    /// Skip replaceable events while a render is pending, and send the latest skipped cursor
    /// move before the next event that the host dispatches.
    fn dispatch(&mut self, event: AvengerWindowEvent) {
        for event in self.skipped_move.admit(event, self.render_pending.get()) {
            self.run(Operation::Update(event));
        }
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
            scale: self.scale,
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
    /// Physical pixels per unit of the app's coordinates.
    scale: f32,
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
        canvas.window().set_cursor(app.cursor());
        *self.copy_text.borrow_mut() = app.copy_text().to_string();
        set_text_entry(canvas.window(), app.text_entry(), self.scale);
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

    /// Install scene changes and request a redraw if a scene changed or `changes.redraw` is
    /// true.
    /// Apply cursor and text entry changes, and store updated text for the copy shortcut.
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
            canvas.window().set_cursor(cursor);
        }
        if let Some(text) = &changes.copy_text {
            *self.copy_text.borrow_mut() = text.clone();
        }
        if let Some(caret) = changes.text_entry {
            set_text_entry(canvas.window(), caret, self.scale);
        }
    }
}

/// Let input methods compose text only while a text entry has focus, so that keys reach the app
/// elsewhere, and place their candidate windows beside the entry's `caret`. The caret is in the
/// app's coordinates, which `scale` converts to physical pixels, as the canvas draws them,
/// whatever the display's scale factor.
fn set_text_entry(window: &Window, caret: Option<[f32; 4]>, scale: f32) {
    window.set_ime_allowed(caret.is_some());
    if let Some([x, y, width, height]) = caret {
        let position = PhysicalPosition::new(x * scale, y * scale);
        window.set_ime_cursor_area(position, PhysicalSize::new(width * scale, height * scale));
    }
}

/// The latest cursor move that the host skipped while a render was pending. The host sends it
/// before the next event that it dispatches, or once the pending frame draws, so the app sees
/// where the pointer ended, such as where a drag released.
#[derive(Default)]
struct SkippedMove(Option<AvengerWindowEvent>);

impl SkippedMove {
    /// Return the events to dispatch for `event`: none while a render is pending and `event`
    /// can wait, keeping `event` if it is a cursor move. Otherwise, the skipped move, unless
    /// `event` is a newer move, and then `event`.
    fn admit(
        &mut self,
        event: AvengerWindowEvent,
        render_pending: bool,
    ) -> Vec<AvengerWindowEvent> {
        let moved = matches!(event, AvengerWindowEvent::CursorMoved(_));
        if render_pending && event.skip_if_render_pending() {
            if moved {
                self.0 = Some(event);
            }
            return Vec::new();
        }
        let skipped = self.0.take().filter(|_| !moved);
        skipped.into_iter().chain([event]).collect()
    }

    /// Take the skipped move, to send once the pending frame draws.
    fn take(&mut self) -> Option<AvengerWindowEvent> {
        self.0.take()
    }
}

/// A clipboard shortcut: C, X or V pressed with Control or Super (Command on macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    Copy,
    Cut,
    Paste,
}

impl Shortcut {
    fn of(key: &keyboard::Key, pressed: bool, modifiers: ModifiersState) -> Option<Self> {
        let command = modifiers.control_key() || modifiers.super_key();
        let keyboard::Key::Character(c) = key else {
            return None;
        };
        match c.to_ascii_lowercase().as_str() {
            _ if !pressed || !command => None,
            "c" => Some(Self::Copy),
            "x" => Some(Self::Cut),
            "v" => Some(Self::Paste),
            _ => None,
        }
    }
}

/// System clipboard used by the copy shortcut.
#[derive(Default)]
struct Clipboard {
    /// Native clipboard, opened on first copy and retained to keep copied text available on
    /// Linux.
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

    /// The clipboard's text, for a native paste. The browser's pastes arrive through the
    /// page's paste event instead, so this returns `None` there.
    fn read(&mut self) -> Option<String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.native.is_none() {
                self.native = arboard::Clipboard::new()
                    .map_err(|e| log::error!("Failed to open the clipboard: {e}"))
                    .ok();
            }
            let text = self.native.as_mut()?.get_text();
            text.map_err(|e| log::warn!("Failed to paste: {e}")).ok()
        }
        #[cfg(target_arch = "wasm32")]
        None
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

        // Copy and cut write the app's copy text, and their keys still reach the app, which
        // deletes a cut selection. A native paste reads the clipboard, and the app hears the
        // pasted text after the key.
        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.modifiers = modifiers.state();
        }
        let shortcut = match &event {
            WindowEvent::KeyboardInput { event: key, .. } => {
                let pressed = key.state == ElementState::Pressed && !key.repeat;
                Shortcut::of(&key.logical_key, pressed, self.modifiers)
            }
            _ => None,
        };
        if matches!(shortcut, Some(Shortcut::Copy | Shortcut::Cut)) {
            let text = self.copy_text.borrow().clone();
            if !text.is_empty() {
                self.clipboard.write(&text);
            }
        }
        let pasted = match shortcut {
            Some(Shortcut::Paste) => self.clipboard.read(),
            _ => None,
        };

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
                // Escape reaches the app to clear selections. Only a close request exits the host.
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
                    // Once the frame draws, the app sees where the pointer moved meanwhile.
                    if !self.render_pending.get() {
                        if let Some(moved) = self.skipped_move.take() {
                            self.dispatch(moved);
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
        if let Some(text) = pasted {
            self.dispatch(AvengerWindowEvent::Paste(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_eventstream::window::{
        ElementState as Press, MouseButton, WindowCursorMoved, WindowMouseInput,
    };

    /// A cursor move skipped while a render is pending comes before the next event that the
    /// host dispatches, unless a newer move replaces it, and waits for the frame otherwise.
    #[test]
    fn a_skipped_move_comes_before_the_next_event() {
        let moved = |x| AvengerWindowEvent::CursorMoved(WindowCursorMoved { position: [x, 0.0] });
        let release = AvengerWindowEvent::MouseInput(WindowMouseInput {
            state: Press::Released,
            button: MouseButton::Left,
        });
        let mut skipped = SkippedMove::default();
        assert!(skipped.admit(moved(1.0), true).is_empty());
        assert!(skipped.admit(moved(2.0), true).is_empty());
        assert_eq!(skipped.admit(release.clone(), true), [moved(2.0), release]);
        assert!(skipped.admit(moved(3.0), true).is_empty());
        assert_eq!(skipped.admit(moved(4.0), false), [moved(4.0)]);
        assert!(skipped.admit(moved(5.0), true).is_empty());
        assert_eq!(skipped.take(), Some(moved(5.0)));
    }

    /// The clipboard shortcuts are C, X and V, pressed with Control or Super.
    #[test]
    fn clipboard_shortcuts_are_c_x_and_v_with_control_or_command() {
        let key = |c: &str| keyboard::Key::Character(c.into());
        let of = Shortcut::of;
        assert_eq!(
            of(&key("c"), true, ModifiersState::CONTROL),
            Some(Shortcut::Copy)
        );
        assert_eq!(
            of(&key("C"), true, ModifiersState::SUPER),
            Some(Shortcut::Copy)
        );
        assert_eq!(
            of(&key("x"), true, ModifiersState::SUPER),
            Some(Shortcut::Cut)
        );
        assert_eq!(
            of(&key("v"), true, ModifiersState::CONTROL),
            Some(Shortcut::Paste)
        );
        assert_eq!(of(&key("c"), true, ModifiersState::empty()), None);
        assert_eq!(of(&key("c"), false, ModifiersState::CONTROL), None);
        assert_eq!(of(&key("a"), true, ModifiersState::CONTROL), None);
    }
}
