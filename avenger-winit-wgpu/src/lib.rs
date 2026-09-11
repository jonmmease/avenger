use avenger_app::app::AvengerApp;
use avenger_common::{canvas::CanvasDimensions, cursor::CursorStyle, time::Instant};
#[cfg(not(target_arch = "wasm32"))]
use avenger_eventstream::runtime::{
    LogicalRect, RuntimeHostCommand, RuntimeWakeEvent, RuntimeWakeKey,
};
#[cfg(not(target_arch = "wasm32"))]
use avenger_eventstream::window::ClipboardEvent;
use avenger_eventstream::window::{
    CanvasResizeEvent, WindowEvent as AvengerWindowEvent, WindowResizeEvent,
};
use avenger_resource::{
    RenderInvalidation, RenderInvalidationHub, RenderInvalidationReason,
    RenderInvalidationSchedule, RenderInvalidationSubscription,
};
pub use avenger_wgpu::{
    canvas::CanvasConfig,
    image_resources::{
        ImageResourceResolver, WgpuImagePlaceholder, WgpuImageResourceConfig,
        WgpuMissingImagePolicy,
    },
};
use avenger_wgpu::{
    canvas::{Canvas, CanvasFrameOverlay, WindowCanvas},
    error::AvengerWgpuError,
};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::{
    collections::VecDeque,
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalSize, Size},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{CursorIcon, WindowAttributes, WindowId},
};
#[cfg(not(target_arch = "wasm32"))]
use winit::{dpi::PhysicalPosition, keyboard};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

#[cfg(any(test, target_arch = "wasm32"))]
mod text_agent;
#[cfg(target_arch = "wasm32")]
pub use text_agent::TextAgentHost;

/// Synchronous text supplied to a browser copy/cut callback for the currently
/// focused canvas control.
pub type ClipboardPayloadProvider = Arc<dyn Fn() -> Option<String> + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
mod file_watcher;
#[cfg(not(target_arch = "wasm32"))]
pub use file_watcher::FileWatcher;

#[cfg(target_arch = "wasm32")]
pub struct FileWatcher;

#[derive(Clone)]
pub enum WinitWgpuEvent {
    App(AvengerWindowEvent),
    RenderInvalidated {
        host_generation: u64,
        invalidation: RenderInvalidation,
    },
    HostUpdateReady,
    SetWindowTitle(String),
    ExitRequested,
}

/// A fully prepared application generation for event-loop-thread installation.
pub struct PreparedHostUpdate<State>
where
    State: Clone + Send + Sync + 'static,
{
    pub generation: u64,
    /// Monotonic source-change epoch this update was prepared from.
    pub request_epoch: u64,
    pub app: AvengerApp<State>,
    pub render_invalidation_hub: Option<RenderInvalidationHub>,
    /// Resolver that received this generation's image-resource requests.
    /// The host installs it before rendering the replacement scene.
    pub image_resource_resolver: Option<Arc<dyn ImageResourceResolver>>,
    pub window_title: Option<String>,
    pub window_scene_sizing: WindowSceneSizing,
    pub canvas_frame: Option<CanvasFrameOptions>,
    pub completion: Option<std::sync::mpsc::SyncSender<HostUpdateInstallOutcome>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostUpdateInstallOutcome {
    Installed,
    Superseded,
    Failed(String),
}

/// Thread-safe producer for prepared native host updates.
#[derive(Clone)]
pub struct HostUpdateSender<State>
where
    State: Clone + Send + Sync + 'static,
{
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
    queue: Arc<Mutex<VecDeque<PreparedHostUpdate<State>>>>,
    latest_request_epoch: Arc<AtomicU64>,
}

impl<State> HostUpdateSender<State>
where
    State: Clone + Send + Sync + 'static,
{
    pub fn submit(
        &self,
        update: PreparedHostUpdate<State>,
    ) -> Result<HostUpdateSubmitOutcome, HostUpdateSubmitError> {
        if update.request_epoch < self.latest_request_epoch.load(Ordering::Acquire) {
            complete_host_update(update, HostUpdateInstallOutcome::Superseded);
            return Ok(HostUpdateSubmitOutcome::Superseded);
        }
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| HostUpdateSubmitError::QueuePoisoned)?;
        for pending in queue.drain(..) {
            complete_host_update(pending, HostUpdateInstallOutcome::Superseded);
        }
        queue.push_back(update);
        drop(queue);
        self.event_proxy
            .send_event(WinitWgpuEvent::HostUpdateReady)
            .map_err(|_| HostUpdateSubmitError::EventLoopClosed)?;
        Ok(HostUpdateSubmitOutcome::Queued)
    }

    /// Publish the newest source-change epoch before starting reload work.
    /// Prepared updates from older epochs are rejected both at submission and
    /// again on the event-loop thread.
    pub fn mark_request_epoch(&self, epoch: u64) {
        self.latest_request_epoch.fetch_max(epoch, Ordering::AcqRel);
    }

    pub fn set_window_title(&self, title: impl Into<String>) -> Result<(), HostUpdateSubmitError> {
        self.event_proxy
            .send_event(WinitWgpuEvent::SetWindowTitle(title.into()))
            .map_err(|_| HostUpdateSubmitError::EventLoopClosed)
    }

    /// Request clean event-loop termination from another thread.
    pub fn request_exit(&self) -> Result<(), HostUpdateSubmitError> {
        self.event_proxy
            .send_event(WinitWgpuEvent::ExitRequested)
            .map_err(|_| HostUpdateSubmitError::EventLoopClosed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostUpdateSubmitOutcome {
    Queued,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostUpdateSubmitError {
    QueuePoisoned,
    EventLoopClosed,
}

impl fmt::Display for HostUpdateSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueuePoisoned => f.write_str("prepared host-update queue is poisoned"),
            Self::EventLoopClosed => f.write_str("winit event loop is closed"),
        }
    }
}

impl std::error::Error for HostUpdateSubmitError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WinitWgpuHostInitError {
    EventLoop(String),
    FileWatcher(String),
}

impl fmt::Display for WinitWgpuHostInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(message) => write!(f, "failed to build native event loop: {message}"),
            Self::FileWatcher(message) => {
                write!(f, "failed to initialize app file watcher: {message}")
            }
        }
    }
}

impl std::error::Error for WinitWgpuHostInitError {}

fn take_latest_host_update<State>(
    queue: &mut VecDeque<PreparedHostUpdate<State>>,
    installed_generation: u64,
    latest_request_epoch: u64,
) -> Option<PreparedHostUpdate<State>>
where
    State: Clone + Send + Sync + 'static,
{
    let mut latest: Option<PreparedHostUpdate<State>> = None;
    while let Some(candidate) = queue.pop_front() {
        if candidate.generation <= installed_generation
            || candidate.request_epoch < latest_request_epoch
        {
            complete_host_update(candidate, HostUpdateInstallOutcome::Superseded);
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|current| candidate.generation > current.generation)
        {
            if let Some(previous) = latest.replace(candidate) {
                complete_host_update(previous, HostUpdateInstallOutcome::Superseded);
            }
        } else {
            complete_host_update(candidate, HostUpdateInstallOutcome::Superseded);
        }
    }
    latest
}

fn complete_host_update<State>(
    mut update: PreparedHostUpdate<State>,
    outcome: HostUpdateInstallOutcome,
) where
    State: Clone + Send + Sync + 'static,
{
    complete_host_update_sender(update.completion.take(), outcome);
}

fn complete_host_update_sender(
    completion: Option<std::sync::mpsc::SyncSender<HostUpdateInstallOutcome>>,
    outcome: HostUpdateInstallOutcome,
) {
    if let Some(completion) = completion {
        let _ = completion.try_send(outcome);
    }
}

fn cancel_transient_input_after_host_update(
    pending_canvas_resize: &mut Option<CanvasResizeEvent>,
    interaction_settle_generation: &AtomicU64,
) {
    *pending_canvas_resize = None;
    interaction_settle_generation.fetch_add(1, Ordering::Relaxed);
}

fn send_render_invalidation_event(
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
    host_generation: u64,
    invalidation: RenderInvalidation,
) {
    match invalidation.schedule {
        RenderInvalidationSchedule::Now => {
            let _ = event_proxy.send_event(WinitWgpuEvent::RenderInvalidated {
                host_generation,
                invalidation,
            });
        }
        RenderInvalidationSchedule::After(delay) => {
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                let _ = event_proxy.send_event(WinitWgpuEvent::RenderInvalidated {
                    host_generation,
                    invalidation,
                });
            });

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsCast;

                let delay_ms = delay.as_millis().min(i32::MAX as u128) as i32;
                let callback = wasm_bindgen::closure::Closure::once(move || {
                    let _ = event_proxy.send_event(WinitWgpuEvent::RenderInvalidated {
                        host_generation,
                        invalidation,
                    });
                });
                web_sys::window()
                    .and_then(|window| {
                        window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                callback.as_ref().unchecked_ref(),
                                delay_ms,
                            )
                            .ok()
                    })
                    .expect("schedule wasm render invalidation");
                callback.forget();
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeRuntimeWakeScheduler {
    active: Arc<Mutex<HashMap<RuntimeWakeKey, u64>>>,
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeRuntimeWakeScheduler {
    fn new(event_proxy: EventLoopProxy<WinitWgpuEvent>) -> Self {
        Self {
            active: Arc::new(Mutex::new(HashMap::new())),
            event_proxy,
        }
    }

    fn request(&self, key: RuntimeWakeKey, deadline: Instant, generation: u64) {
        let Ok(mut active) = self.active.lock() else {
            log::error!("runtime wake scheduler lock is poisoned; dropping wake request");
            return;
        };
        active.insert(key.clone(), generation);
        drop(active);
        let active = self.active.clone();
        let event_proxy = self.event_proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            let should_dispatch = claim_runtime_wakeup(&active, &key, generation);
            if should_dispatch {
                let _ = event_proxy.send_event(WinitWgpuEvent::App(
                    AvengerWindowEvent::RuntimeWake(RuntimeWakeEvent { key, generation }),
                ));
            }
        });
    }

    fn cancel(&self, key: &RuntimeWakeKey) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(key);
        } else {
            log::error!("runtime wake scheduler lock is poisoned; dropping cancellation");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn claim_runtime_wakeup(
    active: &Mutex<HashMap<RuntimeWakeKey, u64>>,
    key: &RuntimeWakeKey,
    generation: u64,
) -> bool {
    let Ok(mut active) = active.lock() else {
        log::error!("runtime wake scheduler lock is poisoned; dropping scheduled wake");
        return false;
    };
    if active.get(key) == Some(&generation) {
        active.remove(key);
        true
    } else {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn physical_ime_cursor_area(
    rect: LogicalRect,
    scale: f64,
) -> (PhysicalPosition<f64>, PhysicalSize<u32>) {
    (
        PhysicalPosition::new(f64::from(rect.x()) * scale, f64::from(rect.y()) * scale),
        PhysicalSize::new(
            (f64::from(rect.width()) * scale).round().max(0.0) as u32,
            (f64::from(rect.height()) * scale).round().max(0.0) as u32,
        ),
    )
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for NativeRuntimeWakeScheduler {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.clear();
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowSceneSizing {
    #[default]
    SurfaceFollowsWindow,
    MatchSceneGraph,
    MatchSceneGraphAxes {
        width: bool,
        height: bool,
    },
}

impl WindowSceneSizing {
    fn matching_axes(self) -> (bool, bool) {
        match self {
            Self::SurfaceFollowsWindow => (false, false),
            Self::MatchSceneGraph => (true, true),
            Self::MatchSceneGraphAxes { width, height } => (width, height),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrameOptions {
    pub resize_width: bool,
    pub resize_height: bool,
    pub min_size: [f32; 2],
    pub extra_window_size: [f32; 2],
    pub handle_thickness: f32,
}

impl Default for CanvasFrameOptions {
    fn default() -> Self {
        Self {
            resize_width: false,
            resize_height: false,
            min_size: [120.0, 120.0],
            extra_window_size: [320.0, 240.0],
            handle_thickness: 8.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasFrameHandle {
    Right,
    Bottom,
    Corner,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeClipboardShortcut {
    Cut,
    Copy,
    Paste,
}

#[cfg(not(target_arch = "wasm32"))]
fn native_clipboard_shortcut(
    logical_key: &keyboard::Key,
    state: ElementState,
    repeat: bool,
    modifiers: keyboard::ModifiersState,
) -> Option<NativeClipboardShortcut> {
    if state != ElementState::Pressed || repeat || modifiers.alt_key() {
        return None;
    }

    #[cfg(target_os = "macos")]
    let command_pressed = modifiers.super_key();
    #[cfg(not(target_os = "macos"))]
    let command_pressed = modifiers.control_key();
    if !command_pressed {
        return None;
    }

    let keyboard::Key::Character(character) = logical_key else {
        return None;
    };
    if character.eq_ignore_ascii_case("x") {
        Some(NativeClipboardShortcut::Cut)
    } else if character.eq_ignore_ascii_case("c") {
        Some(NativeClipboardShortcut::Copy)
    } else if character.eq_ignore_ascii_case("v") {
        Some(NativeClipboardShortcut::Paste)
    } else {
        None
    }
}

impl CanvasFrameHandle {
    fn resizes_width(self) -> bool {
        matches!(self, Self::Right | Self::Corner)
    }

    fn resizes_height(self) -> bool {
        matches!(self, Self::Bottom | Self::Corner)
    }

    fn cursor(self) -> CursorIcon {
        match self {
            Self::Right => CursorIcon::EResize,
            Self::Bottom => CursorIcon::SResize,
            Self::Corner => CursorIcon::SeResize,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CanvasFrameDrag {
    handle: CanvasFrameHandle,
    start_pointer: [f32; 2],
    start_canvas_size: [f32; 2],
}

#[derive(Clone, Debug)]
struct CanvasFrameState {
    options: CanvasFrameOptions,
    canvas_size: [f32; 2],
    hover_handle: Option<CanvasFrameHandle>,
    active_drag: Option<CanvasFrameDrag>,
    last_cursor_position: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, Default)]
struct CanvasFrameEventOutcome {
    consumed: bool,
    cursor: Option<CursorIcon>,
    redraw_overlay: bool,
    resize: Option<[f32; 2]>,
    resize_settled: Option<[f32; 2]>,
}

impl CanvasFrameState {
    fn new(options: CanvasFrameOptions) -> Self {
        Self {
            options,
            canvas_size: [0.0, 0.0],
            hover_handle: None,
            active_drag: None,
            last_cursor_position: None,
        }
    }

    fn update_scene_size(&mut self, size: [f32; 2]) {
        if let Some(drag) = self.active_drag {
            if !drag.handle.resizes_width() {
                self.canvas_size[0] = size[0];
            }
            if !drag.handle.resizes_height() {
                self.canvas_size[1] = size[1];
            }
        } else {
            self.canvas_size = size;
        }
    }

    fn initial_window_size(&self) -> [f32; 2] {
        [
            self.canvas_size[0] + self.options.extra_window_size[0],
            self.canvas_size[1] + self.options.extra_window_size[1],
        ]
    }

    fn overlay(&self) -> CanvasFrameOverlay {
        CanvasFrameOverlay {
            size: self.canvas_size,
            resize_width: self.options.resize_width,
            resize_height: self.options.resize_height,
            handle_thickness: self.options.handle_thickness,
        }
    }

    fn hit_test(&self, position: [f32; 2]) -> Option<CanvasFrameHandle> {
        let [x, y] = position;
        let [width, height] = self.canvas_size;
        let thickness = self.options.handle_thickness.max(1.0);
        let near_right = self.options.resize_width
            && x >= width - thickness
            && x <= width + thickness
            && y >= 0.0
            && y <= height + thickness;
        let near_bottom = self.options.resize_height
            && y >= height - thickness
            && y <= height + thickness
            && x >= 0.0
            && x <= width + thickness;

        match (near_right, near_bottom) {
            (true, true) if self.options.resize_width && self.options.resize_height => {
                Some(CanvasFrameHandle::Corner)
            }
            (true, _) => Some(CanvasFrameHandle::Right),
            (_, true) => Some(CanvasFrameHandle::Bottom),
            _ => None,
        }
    }

    fn handle_cursor_moved(&mut self, position: [f32; 2]) -> CanvasFrameEventOutcome {
        self.last_cursor_position = Some(position);

        if let Some(drag) = self.active_drag {
            let size = self.drag_size(drag, position);
            let changed = self.canvas_size != size;
            self.canvas_size = size;
            return CanvasFrameEventOutcome {
                consumed: true,
                cursor: Some(drag.handle.cursor()),
                redraw_overlay: changed,
                resize: changed.then_some(size),
                resize_settled: None,
            };
        }

        let hover_handle = self.hit_test(position);
        let changed = self.hover_handle != hover_handle;
        self.hover_handle = hover_handle;
        CanvasFrameEventOutcome {
            consumed: hover_handle.is_some(),
            cursor: Some(hover_handle.map_or(CursorIcon::Default, CanvasFrameHandle::cursor)),
            redraw_overlay: changed,
            resize: None,
            resize_settled: None,
        }
    }

    fn handle_cursor_left(&mut self) -> CanvasFrameEventOutcome {
        if self.active_drag.is_some() {
            return CanvasFrameEventOutcome::default();
        }
        self.last_cursor_position = None;
        let changed = self.hover_handle.take().is_some();
        CanvasFrameEventOutcome {
            consumed: false,
            cursor: Some(CursorIcon::Default),
            redraw_overlay: changed,
            resize: None,
            resize_settled: None,
        }
    }

    fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
    ) -> CanvasFrameEventOutcome {
        if button != MouseButton::Left {
            return CanvasFrameEventOutcome::default();
        }

        match state {
            ElementState::Pressed => {
                let Some(position) = self.last_cursor_position else {
                    return CanvasFrameEventOutcome::default();
                };
                let handle = self.hover_handle.or_else(|| self.hit_test(position));
                if let Some(handle) = handle {
                    self.active_drag = Some(CanvasFrameDrag {
                        handle,
                        start_pointer: position,
                        start_canvas_size: self.canvas_size,
                    });
                    return CanvasFrameEventOutcome {
                        consumed: true,
                        cursor: Some(handle.cursor()),
                        redraw_overlay: true,
                        resize: None,
                        resize_settled: None,
                    };
                }
                CanvasFrameEventOutcome::default()
            }
            ElementState::Released => {
                let Some(_drag) = self.active_drag.take() else {
                    return CanvasFrameEventOutcome::default();
                };
                self.hover_handle = self
                    .last_cursor_position
                    .and_then(|position| self.hit_test(position));
                CanvasFrameEventOutcome {
                    consumed: true,
                    cursor: Some(
                        self.hover_handle
                            .map_or(CursorIcon::Default, CanvasFrameHandle::cursor),
                    ),
                    redraw_overlay: true,
                    resize: Some(self.canvas_size),
                    resize_settled: Some(self.canvas_size),
                }
            }
        }
    }

    fn drag_size(&self, drag: CanvasFrameDrag, position: [f32; 2]) -> [f32; 2] {
        let mut size = self.canvas_size;
        if drag.handle.resizes_width() {
            size[0] = (drag.start_canvas_size[0] + position[0] - drag.start_pointer[0])
                .max(self.options.min_size[0]);
        }
        if drag.handle.resizes_height() {
            size[1] = (drag.start_canvas_size[1] + position[1] - drag.start_pointer[1])
                .max(self.options.min_size[1]);
        }
        size
    }
}

#[derive(Clone)]
pub struct WinitWgpuAvengerAppOptions {
    pub scale: f32,
    pub window_attributes: WindowAttributes,
    pub window_scene_sizing: WindowSceneSizing,
    pub resize_settle_delay_ms: Option<u64>,
    pub interaction_settle_delay_ms: Option<u64>,
    pub canvas_frame: Option<CanvasFrameOptions>,
    pub canvas_config: CanvasConfig,
    pub render_invalidation_hub: Option<RenderInvalidationHub>,
    pub clipboard_payload_provider: Option<ClipboardPayloadProvider>,
}

impl WinitWgpuAvengerAppOptions {
    pub fn new(scale: f32) -> Self {
        Self {
            scale,
            window_attributes: WindowAttributes::default().with_resizable(false),
            window_scene_sizing: WindowSceneSizing::SurfaceFollowsWindow,
            resize_settle_delay_ms: None,
            interaction_settle_delay_ms: Some(120),
            canvas_frame: None,
            canvas_config: CanvasConfig::default(),
            render_invalidation_hub: None,
            clipboard_payload_provider: None,
        }
    }

    pub fn window_attributes(mut self, window_attributes: WindowAttributes) -> Self {
        self.window_attributes = window_attributes;
        self
    }

    pub fn window_scene_sizing(mut self, window_scene_sizing: WindowSceneSizing) -> Self {
        self.window_scene_sizing = window_scene_sizing;
        self
    }

    pub fn resize_settle_delay_ms(mut self, resize_settle_delay_ms: Option<u64>) -> Self {
        self.resize_settle_delay_ms = resize_settle_delay_ms;
        self
    }

    pub fn interaction_settle_delay_ms(mut self, interaction_settle_delay_ms: Option<u64>) -> Self {
        self.interaction_settle_delay_ms = interaction_settle_delay_ms;
        self
    }

    pub fn canvas_frame(mut self, canvas_frame: Option<CanvasFrameOptions>) -> Self {
        self.canvas_frame = canvas_frame;
        self
    }

    pub fn canvas_config(mut self, canvas_config: CanvasConfig) -> Self {
        self.canvas_config = canvas_config;
        self
    }

    pub fn render_invalidation_hub(mut self, hub: RenderInvalidationHub) -> Self {
        self.render_invalidation_hub = Some(hub);
        self
    }

    pub fn clipboard_payload_provider(mut self, provider: ClipboardPayloadProvider) -> Self {
        self.clipboard_payload_provider = Some(provider);
        self
    }
}

pub struct WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    canvas: std::rc::Rc<std::cell::RefCell<Option<WindowCanvas<'static>>>>,
    scale: f32,
    window_attributes: WindowAttributes,
    window_scene_sizing: WindowSceneSizing,
    resize_settle_delay_ms: Option<u64>,
    interaction_settle_delay_ms: Option<u64>,
    interaction_settle_generation: Arc<AtomicU64>,
    canvas_frame: Option<CanvasFrameState>,
    canvas_config: CanvasConfig,
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
    prepared_host_updates: Arc<Mutex<VecDeque<PreparedHostUpdate<State>>>>,
    latest_host_request_epoch: Arc<AtomicU64>,
    installed_host_generation: u64,
    _render_invalidation_subscription: Option<RenderInvalidationSubscription>,
    /// The hub itself, kept for startup replay: proxy events sent before the
    /// event loop runs are dropped by winit, so invalidations that fire
    /// during app init only survive as hub state.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    render_invalidation_hub: Option<RenderInvalidationHub>,
    pub avenger_app: std::rc::Rc<std::cell::RefCell<AvengerApp<State>>>,
    render_pending: bool,
    render_invalidation_pending: bool,
    /// Latest invalidation that arrived before the canvas existed; replayed
    /// by `resumed()` after the initial scene installs.
    pending_startup_render_invalidation: Option<RenderInvalidation>,
    last_requested_render_invalidation_epoch: u64,
    last_rendered_render_invalidation_epoch: u64,
    /// Hub epoch snapshotted just before the most recent evaluation
    /// (event-driven or invalidation-driven scene rebuild) started. Delayed
    /// invalidations older than this are redundant: that evaluation
    /// re-derived the session's deferred needs and re-requested any wake-up
    /// still required.
    hub_epoch_at_last_evaluation_start: u64,
    pub file_watcher: Option<FileWatcher>,
    window_id: Option<winit::window::WindowId>,
    coalesced_event_count: usize,
    stale_canvas_resize_count: usize,
    pending_canvas_resize: Option<CanvasResizeEvent>,
    fatal_error: Option<String>,

    #[cfg(not(target_arch = "wasm32"))]
    clipboard: Option<arboard::Clipboard>,
    #[cfg(not(target_arch = "wasm32"))]
    modifiers: keyboard::ModifiersState,
    #[cfg(not(target_arch = "wasm32"))]
    runtime_wake_scheduler: NativeRuntimeWakeScheduler,
    #[cfg(target_arch = "wasm32")]
    text_agent: std::rc::Rc<std::cell::RefCell<Option<TextAgentHost>>>,
    #[cfg(target_arch = "wasm32")]
    clipboard_payload_provider: Option<ClipboardPayloadProvider>,

    /// Phase 7 re-baseline: instant of the previous rendered frame (native only),
    /// used to log inter-frame delta / fps alongside surface_render_ms.
    #[cfg(not(target_arch = "wasm32"))]
    last_redraw: Option<Instant>,

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
        Self::new_and_event_loop_with_options(
            avenger_app,
            WinitWgpuAvengerAppOptions::new(scale),
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        )
    }

    pub fn new_and_event_loop_with_options(
        avenger_app: AvengerApp<State>,
        options: WinitWgpuAvengerAppOptions,
        #[cfg(not(target_arch = "wasm32"))] tokio_runtime: tokio::runtime::Runtime,
    ) -> (Self, EventLoop<WinitWgpuEvent>) {
        Self::try_new_and_event_loop_with_options(
            avenger_app,
            options,
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        )
        .expect("failed to initialize winit/wgpu host")
    }

    /// Construct a native host without panicking on event-loop or legacy app
    /// file-watcher initialization failures.
    pub fn try_new_and_event_loop_with_options(
        mut avenger_app: AvengerApp<State>,
        mut options: WinitWgpuAvengerAppOptions,
        #[cfg(not(target_arch = "wasm32"))] tokio_runtime: tokio::runtime::Runtime,
    ) -> Result<(Self, EventLoop<WinitWgpuEvent>), WinitWgpuHostInitError> {
        // Preserve an app's explicit context unless the host supplies its own.
        let text_engine = if options.canvas_config.text_engine.is_none()
            && options.canvas_config.font_resolution == CanvasConfig::default().font_resolution
        {
            avenger_app.text_engine().clone()
        } else {
            options.canvas_config.resolved_text_engine()
        };
        avenger_app.set_text_engine(text_engine.clone());
        options.canvas_config.text_engine = Some(text_engine);
        // Create event loop with WinitWgpuEvent as custom event type
        let event_loop = EventLoop::<WinitWgpuEvent>::with_user_event()
            .build()
            .map_err(|error| WinitWgpuHostInitError::EventLoop(error.to_string()))?;
        let event_proxy = event_loop.create_proxy();
        let prepared_host_updates = Arc::new(Mutex::new(VecDeque::new()));
        let latest_host_request_epoch = Arc::new(AtomicU64::new(0));
        let render_invalidation_subscription =
            options.render_invalidation_hub.as_ref().map(|hub| {
                let event_proxy_for_subscription = event_proxy.clone();
                let subscription = hub.subscribe(Arc::new(move |invalidation| {
                    send_render_invalidation_event(
                        event_proxy_for_subscription.clone(),
                        0,
                        invalidation,
                    );
                }));
                if let Some(invalidation) = hub.latest_invalidation() {
                    send_render_invalidation_event(event_proxy.clone(), 0, invalidation);
                }
                subscription
            });

        // File watching is only supported on desktop
        #[cfg(not(target_arch = "wasm32"))]
        let file_watcher = {
            let watched_files = avenger_app.get_watched_files();
            if !watched_files.is_empty() {
                Some(
                    FileWatcher::new(event_proxy.clone(), watched_files)
                        .map_err(|error| WinitWgpuHostInitError::FileWatcher(error.to_string()))?,
                )
            } else {
                None
            }
        };
        #[cfg(target_arch = "wasm32")]
        let file_watcher = None;

        #[cfg(not(target_arch = "wasm32"))]
        let runtime_wake_scheduler = NativeRuntimeWakeScheduler::new(event_proxy.clone());

        let winit_app = Self {
            canvas: std::rc::Rc::new(std::cell::RefCell::new(None)),
            scale: options.scale,
            window_attributes: options.window_attributes,
            window_scene_sizing: options.window_scene_sizing,
            resize_settle_delay_ms: options.resize_settle_delay_ms,
            interaction_settle_delay_ms: options.interaction_settle_delay_ms,
            interaction_settle_generation: Arc::new(AtomicU64::new(0)),
            canvas_frame: options.canvas_frame.map(CanvasFrameState::new),
            canvas_config: options.canvas_config,
            event_proxy,
            prepared_host_updates,
            latest_host_request_epoch,
            installed_host_generation: 0,
            _render_invalidation_subscription: render_invalidation_subscription,
            render_invalidation_hub: options.render_invalidation_hub,
            avenger_app: std::rc::Rc::new(std::cell::RefCell::new(avenger_app)),
            render_pending: false,
            render_invalidation_pending: false,
            pending_startup_render_invalidation: None,
            last_requested_render_invalidation_epoch: 0,
            last_rendered_render_invalidation_epoch: 0,
            hub_epoch_at_last_evaluation_start: 0,
            file_watcher,
            window_id: None,
            coalesced_event_count: 0,
            stale_canvas_resize_count: 0,
            pending_canvas_resize: None,
            fatal_error: None,
            #[cfg(not(target_arch = "wasm32"))]
            clipboard: None,
            #[cfg(not(target_arch = "wasm32"))]
            modifiers: keyboard::ModifiersState::default(),
            #[cfg(not(target_arch = "wasm32"))]
            runtime_wake_scheduler,
            #[cfg(target_arch = "wasm32")]
            text_agent: std::rc::Rc::new(std::cell::RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            clipboard_payload_provider: options.clipboard_payload_provider,
            #[cfg(not(target_arch = "wasm32"))]
            last_redraw: None,
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        };

        Ok((winit_app, event_loop))
    }

    /// Return a thread-safe handle for publishing fully prepared replacement
    /// applications to this host's event-loop thread.
    pub fn host_update_sender(&self) -> HostUpdateSender<State> {
        HostUpdateSender {
            event_proxy: self.event_proxy.clone(),
            queue: Arc::clone(&self.prepared_host_updates),
            latest_request_epoch: Arc::clone(&self.latest_host_request_epoch),
        }
    }

    /// Take a fatal initialization error recorded by the event-loop handler.
    pub fn take_fatal_error(&mut self) -> Option<String> {
        self.fatal_error.take()
    }

    fn install_latest_prepared_host_update(&mut self) {
        let update = {
            let Ok(mut queue) = self.prepared_host_updates.lock() else {
                log::error!("prepared host-update queue is poisoned");
                return;
            };
            take_latest_host_update(
                &mut queue,
                self.installed_host_generation,
                self.latest_host_request_epoch.load(Ordering::Acquire),
            )
        };
        let Some(update) = update else {
            return;
        };
        let mut update = update;
        update
            .app
            .set_text_engine(self.canvas_config.resolved_text_engine());
        let completion = update.completion.take();

        let scene_graph = update.app.scene_graph_arc();
        let mut replacement_frame = update.canvas_frame.map(CanvasFrameState::new);
        if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
            canvas.clear_tooltip();
            if let Err(error) = install_scene_graph(
                canvas,
                &scene_graph,
                update.window_scene_sizing,
                self.scale,
                replacement_frame.as_mut(),
            ) {
                log::error!("failed to install prepared host update: {error:?}");
                let old_scene = self.avenger_app.borrow().scene_graph_arc();
                let _ = install_scene_graph(
                    canvas,
                    &old_scene,
                    self.window_scene_sizing,
                    self.scale,
                    self.canvas_frame.as_mut(),
                );
                complete_host_update_sender(
                    completion,
                    HostUpdateInstallOutcome::Failed(format!("{error:?}")),
                );
                return;
            }
            if let Some(resolver) = update.image_resource_resolver.take() {
                canvas.set_image_resource_resolver(resolver);
            }
            if let Some(title) = update.window_title.as_deref() {
                canvas.window().set_title(title);
            }
        } else {
            if let Some(resolver) = update.image_resource_resolver.take() {
                self.canvas_config.image_resource_config.resolver = Some(resolver);
            }
            if let Some(title) = update.window_title.as_deref() {
                self.window_attributes = self.window_attributes.clone().with_title(title);
            }
        }

        self.window_scene_sizing = update.window_scene_sizing;
        self.canvas_frame = replacement_frame;
        *self.avenger_app.borrow_mut() = update.app;
        self._render_invalidation_subscription = None;
        self.render_invalidation_hub = update.render_invalidation_hub;
        self.last_requested_render_invalidation_epoch = 0;
        self.last_rendered_render_invalidation_epoch = 0;
        self.hub_epoch_at_last_evaluation_start = 0;
        self.pending_startup_render_invalidation = None;
        self.render_invalidation_pending = false;
        cancel_transient_input_after_host_update(
            &mut self.pending_canvas_resize,
            &self.interaction_settle_generation,
        );
        if let Some(canvas) = self.canvas.borrow().as_ref() {
            // The replacement app and canvas-frame state contain no active
            // gesture. Keep the native cursor in the same reset state rather
            // than retaining `grabbing`/resize feedback from the old app.
            canvas.window().set_cursor(CursorIcon::Default);
        }
        self.installed_host_generation = update.generation;
        let installed_generation = update.generation;

        if let Some(hub) = self.render_invalidation_hub.as_ref() {
            let proxy = self.event_proxy.clone();
            self._render_invalidation_subscription = Some(hub.subscribe(Arc::new({
                let proxy = proxy.clone();
                move |invalidation| {
                    send_render_invalidation_event(
                        proxy.clone(),
                        installed_generation,
                        invalidation,
                    );
                }
            })));
            if let Some(invalidation) = hub.latest_invalidation() {
                send_render_invalidation_event(proxy, installed_generation, invalidation);
            }
        }
        complete_host_update_sender(completion, HostUpdateInstallOutcome::Installed);
    }

    fn dispatch_avenger_event(&mut self, event: AvengerWindowEvent, force: bool) {
        if !force && self.render_pending && event.skip_if_render_pending() {
            if let AvengerWindowEvent::CanvasResize(event) = &event {
                self.pending_canvas_resize = Some(event.clone());
            }
            self.coalesced_event_count += 1;
            tracing::debug!(
                target: "avenger_winit_wgpu::resize",
                event_kind = event_kind_label(&event),
                coalesced_events = self.coalesced_event_count,
                "winit.dispatch coalesced render-pending event"
            );
            return;
        }

        let _span = tracing::debug_span!(
            "winit.dispatch",
            event_kind = event_kind_label(&event),
            force
        )
        .entered();
        let window_scene_sizing = self.window_scene_sizing;
        let scale = self.scale;

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                let app_clone = self.avenger_app.clone();
                let event_clone = event.clone();
                let canvas_shared = self.canvas.clone();
                let text_agent = self.text_agent.clone();

                #[allow(clippy::await_holding_refcell_ref)]
                let update_future = async move {
                    let update_result = app_clone
                        .borrow_mut()
                        .update_with_status(&event_clone, Instant::now())
                        .await;

                    match update_result {
                        Ok(mut update) => {
                            let mut text_commands = Vec::new();
                            for command in std::mem::take(&mut update.status.commands) {
                                match command {
                                    avenger_eventstream::runtime::RuntimeHostCommand::UpdateTooltip(update) => {
                                        if let Some(canvas) = canvas_shared.borrow_mut().as_mut() {
                                            if let Err(error) = canvas.set_tooltip_update(update) {
                                                log::error!("Failed to update tooltip overlay: {error:?}");
                                            }
                                        }
                                    }
                                    other => text_commands.push(other),
                                }
                            }
                            if let Some(host) = text_agent.borrow_mut().as_mut() {
                                host.apply_commands(text_commands);
                            }
                            if let Some(scene_graph) = update.scene_graph {
                                if let Some(host) = text_agent.borrow_mut().as_mut() {
                                    host.set_logical_canvas_size([scene_graph.width, scene_graph.height]);
                                }
                                let mut canvas_borrowed = canvas_shared.borrow_mut();
                                if let Some(canvas) = canvas_borrowed.as_mut() {
                                    if let Err(e) = install_scene_graph(
                                        canvas,
                                        &scene_graph,
                                        window_scene_sizing,
                                        scale,
                                        None,
                                    ) {
                                        log::error!("Failed to set scene: {:?}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to update app: {:?}", e);
                        }
                    }
                };
                spawn_local(update_future);
            } else {
                // For non-WASM, maintain the original precise render_pending logic
                let dispatch_start = Instant::now();
                let app_update_start = Instant::now();
                // Snapshot BEFORE the update: if it evaluates (produces a
                // scene), delayed wake-ups requested before this point are
                // redundant for the delayed-invalidation test, while ones the
                // evaluation itself parks get later epochs and survive.
                let hub_epoch_before = self
                    .render_invalidation_hub
                    .as_ref()
                    .map(|hub| hub.epoch());
                let mut scene_graph_opt = {
                    let mut app = self.avenger_app.borrow_mut();
                    match self
                        .tokio_runtime
                        .block_on(app.update_with_status(&event, Instant::now()))
                    {
                        Ok(update) => update,
                        Err(error) => {
                            log::error!("failed to update app; keeping current scene: {error:?}");
                            return;
                        }
                    }
                };
                let app_update_elapsed = app_update_start.elapsed();
                if let (true, Some(epoch)) = (scene_graph_opt.scene_graph.is_some(), hub_epoch_before)
                {
                    self.hub_epoch_at_last_evaluation_start =
                        self.hub_epoch_at_last_evaluation_start.max(epoch);
                }
                if let Some(cursor) = scene_graph_opt.status.cursor {
                    self.set_cursor(cursor_style_to_winit(cursor));
                }
                let commands = std::mem::take(&mut scene_graph_opt.status.commands);
                self.apply_runtime_host_commands(commands);
                let rerender = scene_graph_opt.scene_graph.is_some();

                if let Some(scene_graph) = scene_graph_opt.scene_graph {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        let install_start = Instant::now();
                        if let Err(err) = install_scene_graph(
                            canvas,
                            &scene_graph,
                            window_scene_sizing,
                            scale,
                            self.canvas_frame.as_mut(),
                        ) {
                            log::error!("Failed to set scene: {err:?}");
                        } else {
                            tracing::debug!(
                                target: "avenger_winit_wgpu::resize",
                                set_scene_ms = install_start.elapsed().as_secs_f64() * 1000.0,
                                "winit.dispatch install_scene_graph"
                            );
                            self.render_pending = true;
                        }
                    }
                }
                tracing::debug!(
                    target: "avenger_winit_wgpu::resize",
                    app_update_ms = app_update_elapsed.as_secs_f64() * 1000.0,
                    queue_ms = dispatch_start.elapsed().as_secs_f64() * 1000.0,
                    rerender,
                    "winit.dispatch complete"
                );
            }
        }
    }

    fn handle_render_invalidation(&mut self, invalidation: RenderInvalidation) {
        // The epoch gates below coalesce redundant queued events, which is
        // only valid for `Now` invalidations where queue order matches epoch
        // order. `After(_)` events get their epoch at REQUEST time but are
        // delivered after their delay, so any immediate invalidation landing
        // inside that window (a tile load, a materialization completing)
        // advances the trackers past them; gating them on the trackers would
        // drop the only scheduled wake-up for a deferred materialization
        // schedule pass or preview consume, and the raster would never
        // refresh at gesture end. Delayed events instead use an
        // evaluation-based test: a delayed wake-up is redundant iff any
        // evaluation started after it was requested, because that evaluation
        // re-derived the session's deferred needs and re-requested whatever
        // wake-up is still pending. (Without this, mid-gesture wheel
        // evaluations each park a soon-stale wake-up whose delivery would
        // force a redundant rebuild between frames — visible scroll chop.)
        let delayed = matches!(invalidation.schedule, RenderInvalidationSchedule::After(_));
        let evaluation_changed = matches!(
            &invalidation.reason,
            RenderInvalidationReason::EvaluationChanged { .. }
        );
        if delayed {
            if invalidation.epoch <= self.hub_epoch_at_last_evaluation_start {
                return;
            }
        } else if evaluation_changed {
            // A render-only invalidation (for example, a just-loaded map tile)
            // can be requested after an evaluation invalidation and render
            // before the custom event for the evaluation reaches us on wasm.
            // Do not let that render-only epoch suppress a required scene
            // rebuild; only an evaluation that started after this request
            // makes it redundant.
            if invalidation.epoch <= self.hub_epoch_at_last_evaluation_start {
                return;
            }
        } else {
            if invalidation.epoch <= self.last_rendered_render_invalidation_epoch {
                return;
            }
            if invalidation.epoch <= self.last_requested_render_invalidation_epoch
                && self.render_invalidation_pending
            {
                return;
            }
        }

        // Startup race: invalidations can arrive before the window/canvas
        // exists (e.g. a fast async materialization completing during app
        // init). Rebuilding now would consume the result and then be
        // overwritten by the initial scene install — defer instead;
        // `resumed()` replays the latest deferred invalidation once the
        // canvas is up.
        if self.canvas.borrow().is_none() {
            tracing::debug!(
                target: "avenger_winit_wgpu::resize",
                epoch = invalidation.epoch,
                reason = ?invalidation.reason,
                "winit render invalidation deferred until canvas creation"
            );
            self.pending_startup_render_invalidation = Some(invalidation);
            return;
        }

        if evaluation_changed && !self.rebuild_scene_graph_for_render_invalidation(&invalidation) {
            return;
        }

        // `.max()` so a late-delivered delayed event never regresses the
        // monotonic trackers below an epoch that already rendered.
        let canvas = self.canvas.borrow();
        let Some(canvas) = canvas.as_ref() else {
            self.last_requested_render_invalidation_epoch = self
                .last_requested_render_invalidation_epoch
                .max(invalidation.epoch);
            return;
        };
        self.last_requested_render_invalidation_epoch = self
            .last_requested_render_invalidation_epoch
            .max(invalidation.epoch);
        self.render_invalidation_pending = true;
        canvas.window().request_redraw();
        tracing::debug!(
            target: "avenger_winit_wgpu::resize",
            epoch = invalidation.epoch,
            schedule = ?invalidation.schedule,
            "winit render invalidated"
        );
    }

    fn rebuild_scene_graph_for_render_invalidation(
        &mut self,
        invalidation: &RenderInvalidation,
    ) -> bool {
        let window_scene_sizing = self.window_scene_sizing;
        let scale = self.scale;

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                let app_clone = self.avenger_app.clone();
                let canvas_shared = self.canvas.clone();
                let text_agent = self.text_agent.clone();
                let invalidation_epoch = invalidation.epoch;
                let hub_epoch_before = self
                    .render_invalidation_hub
                    .as_ref()
                    .map(|hub| hub.epoch());
                if let Some(epoch) = hub_epoch_before {
                    self.hub_epoch_at_last_evaluation_start =
                        self.hub_epoch_at_last_evaluation_start.max(epoch);
                }
                #[allow(clippy::await_holding_refcell_ref)]
                spawn_local(async move {
                    let scene_graph = match app_clone.borrow_mut().rebuild_scene_graph(true).await {
                        Ok(scene_graph) => scene_graph,
                        Err(err) => {
                            log::error!("Failed to rebuild scene graph after render invalidation: {err:?}");
                            return;
                        }
                    };
                    if let Some(host) = text_agent.borrow_mut().as_mut() {
                        host.set_logical_canvas_size([scene_graph.width, scene_graph.height]);
                    }
                    let mut canvas_borrowed = canvas_shared.borrow_mut();
                    let Some(canvas) = canvas_borrowed.as_mut() else {
                        return;
                    };
                    if let Err(err) = install_scene_graph(
                        canvas,
                        &scene_graph,
                        window_scene_sizing,
                        scale,
                        None,
                    ) {
                        log::error!("Failed to set invalidated scene graph: {err:?}");
                        return;
                    }
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        epoch = invalidation_epoch,
                        "winit render invalidation rebuilt scene"
                    );
                });
                true
            } else {
                let rebuild_start = Instant::now();
                // Snapshot BEFORE evaluating: wake-ups the evaluation itself
                // parks get later epochs and must survive the delayed-event
                // redundancy test.
                let hub_epoch_before = self
                    .render_invalidation_hub
                    .as_ref()
                    .map(|hub| hub.epoch());
                let scene_graph = {
                    let mut app = self.avenger_app.borrow_mut();
                    match self.tokio_runtime.block_on(app.rebuild_scene_graph(true)) {
                        Ok(scene_graph) => scene_graph,
                        Err(err) => {
                            log::error!("Failed to rebuild scene graph after render invalidation: {err:?}");
                            return false;
                        }
                    }
                };
                if let Some(epoch) = hub_epoch_before {
                    self.hub_epoch_at_last_evaluation_start =
                        self.hub_epoch_at_last_evaluation_start.max(epoch);
                }

                if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                    let install_start = Instant::now();
                    if let Err(err) = install_scene_graph(
                        canvas,
                        &scene_graph,
                        window_scene_sizing,
                        scale,
                        self.canvas_frame.as_mut(),
                    ) {
                        log::error!("Failed to set invalidated scene graph: {err:?}");
                        return false;
                    }
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        epoch = invalidation.epoch,
                        rebuild_ms = rebuild_start.elapsed().as_secs_f64() * 1000.0,
                        set_scene_ms = install_start.elapsed().as_secs_f64() * 1000.0,
                        "winit render invalidation rebuilt scene"
                    );
                    self.render_pending = true;
                }
                true
            }
        }
    }

    fn dispatch_pending_canvas_resize(&mut self) {
        let Some(event) = self.pending_canvas_resize.take() else {
            return;
        };
        if !self.canvas_resize_size_is_current(event.size) {
            self.stale_canvas_resize_count += 1;
            tracing::debug!(
                target: "avenger_winit_wgpu::resize",
                event_kind = "CanvasResize",
                width = event.size[0],
                height = event.size[1],
                stale_canvas_resizes = self.stale_canvas_resize_count,
                "winit.dispatch stale pending canvas resize drop"
            );
            return;
        }
        tracing::debug!(
            target: "avenger_winit_wgpu::resize",
            width = event.size[0],
            height = event.size[1],
            "winit.dispatch pending canvas resize"
        );
        self.dispatch_avenger_event(AvengerWindowEvent::CanvasResize(event), false);
    }

    fn schedule_resize_settle(&self, size: [f32; 2]) {
        let Some(delay_ms) = self.resize_settle_delay_ms else {
            return;
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            let event_proxy = self.event_proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let _ = event_proxy.send_event(WinitWgpuEvent::App(
                    AvengerWindowEvent::WindowResizeSettled(WindowResizeEvent { size }),
                ));
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = delay_ms;
            let _ = size;
        }
    }

    fn schedule_interaction_settle(&self) {
        let Some(delay_ms) = self.interaction_settle_delay_ms else {
            return;
        };
        let generation = self
            .interaction_settle_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;

        #[cfg(not(target_arch = "wasm32"))]
        {
            let event_proxy = self.event_proxy.clone();
            let settle_generation = self.interaction_settle_generation.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                if settle_generation.load(Ordering::Relaxed) != generation {
                    return;
                }
                let _ = event_proxy.send_event(WinitWgpuEvent::App(
                    AvengerWindowEvent::InteractionSettled { generation },
                ));
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = delay_ms;
            let _ = generation;
        }
    }

    fn user_event_force(&mut self, event: &AvengerWindowEvent) -> Option<bool> {
        match event {
            AvengerWindowEvent::CanvasResize(event) => {
                if !self.canvas_resize_size_is_current(event.size) {
                    self.stale_canvas_resize_count += 1;
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        event_kind = "CanvasResize",
                        width = event.size[0],
                        height = event.size[1],
                        stale_canvas_resizes = self.stale_canvas_resize_count,
                        "winit.dispatch stale canvas resize drop"
                    );
                    return None;
                }
                Some(
                    self.canvas_frame
                        .as_ref()
                        .is_some_and(|frame| frame.active_drag.is_none()),
                )
            }
            AvengerWindowEvent::CanvasResizeSettled(event) => self
                .canvas_resize_size_is_current(event.size)
                .then_some(true)
                .or_else(|| {
                    self.stale_canvas_resize_count += 1;
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        event_kind = "CanvasResizeSettled",
                        width = event.size[0],
                        height = event.size[1],
                        stale_canvas_resizes = self.stale_canvas_resize_count,
                        "winit.dispatch stale canvas resize settled drop"
                    );
                    None
                }),
            AvengerWindowEvent::InteractionSettled { generation } => (*generation
                == self.interaction_settle_generation.load(Ordering::Relaxed))
            .then_some(true),
            _ => Some(false),
        }
    }

    fn canvas_resize_size_is_current(&self, size: [f32; 2]) -> bool {
        let Some(frame) = self.canvas_frame.as_ref() else {
            return true;
        };
        let width_matches =
            !frame.options.resize_width || (frame.canvas_size[0] - size[0]).abs() < 0.5;
        let height_matches =
            !frame.options.resize_height || (frame.canvas_size[1] - size[1]).abs() < 0.5;
        width_matches && height_matches
    }

    fn handle_canvas_frame_event(&mut self, event: &WindowEvent) -> bool {
        let Some(frame) = self.canvas_frame.as_mut() else {
            return false;
        };

        let _span = tracing::trace_span!(
            "canvas_frame.pointer",
            event_kind = winit_event_kind_label(event)
        )
        .entered();
        let pointer_start = Instant::now();
        let outcome = match event {
            WindowEvent::CursorMoved { position, .. } => frame.handle_cursor_moved([
                position.x as f32 / self.scale,
                position.y as f32 / self.scale,
            ]),
            WindowEvent::CursorLeft { .. } => frame.handle_cursor_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                frame.handle_mouse_input(*state, *button)
            }
            _ => CanvasFrameEventOutcome::default(),
        };

        if let Some(cursor) = outcome.cursor {
            self.set_cursor(cursor);
        }
        if outcome.redraw_overlay {
            self.refresh_canvas_frame_overlay();
        }
        if let Some(size) = outcome.resize {
            let _ =
                self.event_proxy
                    .send_event(WinitWgpuEvent::App(AvengerWindowEvent::CanvasResize(
                        CanvasResizeEvent { size },
                    )));
            tracing::trace!(
                target: "avenger_winit_wgpu::resize",
                width = size[0],
                height = size[1],
                pointer_ms = pointer_start.elapsed().as_secs_f64() * 1000.0,
                "canvas_frame.pointer queued CanvasResize"
            );
        }
        if let Some(size) = outcome.resize_settled {
            let _ = self.event_proxy.send_event(WinitWgpuEvent::App(
                AvengerWindowEvent::CanvasResizeSettled(CanvasResizeEvent { size }),
            ));
            tracing::debug!(
                target: "avenger_winit_wgpu::resize",
                width = size[0],
                height = size[1],
                pointer_ms = pointer_start.elapsed().as_secs_f64() * 1000.0,
                "canvas_frame.pointer queued CanvasResizeSettled"
            );
        }

        if outcome.consumed || outcome.redraw_overlay {
            tracing::trace!(
                target: "avenger_winit_wgpu::resize",
                pointer_ms = pointer_start.elapsed().as_secs_f64() * 1000.0,
                consumed = outcome.consumed,
                redraw_overlay = outcome.redraw_overlay,
                "canvas_frame.pointer complete"
            );
        }
        outcome.consumed
    }

    fn set_cursor(&self, cursor: CursorIcon) {
        if let Some(canvas) = self.canvas.borrow().as_ref() {
            canvas.window().set_cursor(cursor);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn apply_runtime_host_commands(&mut self, commands: Vec<RuntimeHostCommand>) {
        for command in commands {
            match command {
                RuntimeHostCommand::RequestWakeup {
                    key,
                    deadline,
                    generation,
                } => self
                    .runtime_wake_scheduler
                    .request(key, deadline, generation),
                RuntimeHostCommand::CancelWakeup { key } => {
                    self.runtime_wake_scheduler.cancel(&key)
                }
                RuntimeHostCommand::SetImeAllowed { allowed } => {
                    if let Some(canvas) = self.canvas.borrow().as_ref() {
                        canvas.window().set_ime_allowed(allowed);
                    }
                }
                RuntimeHostCommand::SetImeCursorArea { rect } => {
                    if let Some(canvas) = self.canvas.borrow().as_ref() {
                        let scale = canvas.window().scale_factor();
                        let rect = rect.unwrap_or_else(|| {
                            LogicalRect::new(0.0, 0.0, 0.0, 0.0)
                                .expect("zero IME rectangle is finite")
                        });
                        let (position, size) = physical_ime_cursor_area(rect, scale);
                        canvas.window().set_ime_cursor_area(position, size);
                    }
                }
                RuntimeHostCommand::WriteClipboard { text } => {
                    if self.clipboard.is_none() {
                        match arboard::Clipboard::new() {
                            Ok(clipboard) => self.clipboard = Some(clipboard),
                            Err(err) => {
                                tracing::warn!(
                                    target: "avenger_winit_wgpu::clipboard",
                                    ?err,
                                    "native clipboard unavailable"
                                );
                                continue;
                            }
                        }
                    }
                    if let Some(clipboard) = self.clipboard.as_mut() {
                        if let Err(err) = clipboard.set_text(text) {
                            tracing::warn!(
                                target: "avenger_winit_wgpu::clipboard",
                                ?err,
                                "failed to write native clipboard text"
                            );
                        }
                    }
                }
                RuntimeHostCommand::UpdateTooltip(update) => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        if let Err(error) = canvas.set_tooltip_update(update) {
                            tracing::warn!(
                                target: "avenger_winit_wgpu::tooltip",
                                ?error,
                                "failed to update tooltip overlay"
                            );
                        }
                    }
                }
            }
        }
    }

    fn refresh_canvas_frame_overlay(&mut self) {
        let overlay = self.canvas_frame.as_ref().map(CanvasFrameState::overlay);
        if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
            canvas.set_frame_overlay(overlay);
            canvas.window().request_redraw();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_native_clipboard_shortcut(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::KeyboardInput { event, .. } = event else {
            return false;
        };
        let Some(shortcut) = native_clipboard_shortcut(
            &event.logical_key,
            event.state,
            event.repeat,
            self.modifiers,
        ) else {
            return false;
        };

        let clipboard_event = match shortcut {
            NativeClipboardShortcut::Cut => ClipboardEvent::Cut,
            NativeClipboardShortcut::Copy => ClipboardEvent::Copy,
            NativeClipboardShortcut::Paste => {
                if self.clipboard.is_none() {
                    match arboard::Clipboard::new() {
                        Ok(clipboard) => self.clipboard = Some(clipboard),
                        Err(err) => {
                            tracing::warn!(
                                target: "avenger_winit_wgpu::clipboard",
                                ?err,
                                "native clipboard unavailable"
                            );
                            return true;
                        }
                    }
                }
                let Some(clipboard) = self.clipboard.as_mut() else {
                    return true;
                };
                match clipboard.get_text() {
                    Ok(text) => ClipboardEvent::Paste(text.into()),
                    Err(err) => {
                        tracing::warn!(
                            target: "avenger_winit_wgpu::clipboard",
                            ?err,
                            "failed to read native clipboard text"
                        );
                        return true;
                    }
                }
            }
        };
        self.dispatch_avenger_event(AvengerWindowEvent::Clipboard(clipboard_event), false);
        true
    }

    #[cfg(target_arch = "wasm32")]
    fn setup_wasm_canvas(&self, window: &winit::window::Window) {
        use winit::platform::web::WindowExtWebSys;

        let canvas = web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| {
                let dst = doc.get_element_by_id("wasm-example")?;
                let canvas = window.canvas().expect("Failed to get canvas");
                dst.append_child(&canvas).ok()?;
                Some(canvas)
            })
            .expect("Couldn't append canvas to document body.");
        let host = TextAgentHost::new_with_clipboard_payload_provider(
            canvas,
            self.event_proxy.clone(),
            self.clipboard_payload_provider.clone(),
        )
        .expect("failed to install wasm text agent");
        *self.text_agent.borrow_mut() = Some(host);
    }
}

impl<State> ApplicationHandler<WinitWgpuEvent> for WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = match event_loop.create_window(self.window_attributes.clone()) {
            Ok(window) => window,
            Err(error) => {
                self.fatal_error = Some(format!("failed to create native window: {error}"));
                event_loop.exit();
                return;
            }
        };

        #[cfg(target_arch = "wasm32")]
        self.setup_wasm_canvas(&window);

        self.window_id = Some(window.id());
        let canvas_shared = self.canvas.clone();

        // Extract scene graph and dimensions in a limited scope to avoid RefCell conflicts
        let (scene_graph, dimensions) = {
            let app_borrowed = self.avenger_app.borrow();
            let scene_graph = app_borrowed.scene_graph().clone();
            let scene_size = [scene_graph.width, scene_graph.height];
            let initial_size = if let Some(frame) = self.canvas_frame.as_mut() {
                frame.update_scene_size(scene_size);
                frame.initial_window_size()
            } else {
                scene_size
            };
            let dimensions = CanvasDimensions {
                size: initial_size,
                scale: self.scale,
            };
            (scene_graph, dimensions)
        };

        #[cfg(target_arch = "wasm32")]
        if let Some(host) = self.text_agent.borrow_mut().as_mut() {
            host.set_logical_canvas_size([scene_graph.width, scene_graph.height]);
        }

        let canvas_future = WindowCanvas::new(window, dimensions, self.canvas_config.clone());

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                use wasm_bindgen::JsCast;

                let event_proxy = self.event_proxy.clone();
                let render_generation = self.installed_host_generation;
                let render_invalidation_hub = self.render_invalidation_hub.clone();
                let setup_future = async move {
                    match canvas_future.await {
                        Ok(mut canvas) => {
                            if let Err(err) = install_scene_graph(
                                &mut canvas,
                                &scene_graph,
                                WindowSceneSizing::SurfaceFollowsWindow,
                                dimensions.scale,
                                None,
                            ) {
                                log::error!("Failed to set initial scene: {err:?}");
                            }
                            *canvas_shared.borrow_mut() = Some(canvas);
                            if let Some(invalidation) = render_invalidation_hub
                                .as_ref()
                                .and_then(|hub| hub.latest_evaluation_invalidation())
                            {
                                send_render_invalidation_event(
                                    event_proxy,
                                    render_generation,
                                    invalidation,
                                );
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to create canvas: {e:?}");
                        }
                    }
                };
                let callback = wasm_bindgen::closure::Closure::once(move || {
                    spawn_local(setup_future);
                });
                web_sys::window()
                    .and_then(|window| {
                        window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                callback.as_ref().unchecked_ref(),
                                0,
                            )
                            .ok()
                    })
                    .expect("schedule wasm canvas setup");
                callback.forget();
            } else {
                match self.tokio_runtime.block_on(canvas_future) {
                    Ok(mut canvas) => {
                        if let Err(err) = install_scene_graph(
                            &mut canvas,
                            &scene_graph,
                            WindowSceneSizing::SurfaceFollowsWindow,
                            dimensions.scale,
                            self.canvas_frame.as_mut(),
                        ) {
                            self.fatal_error =
                                Some(format!("failed to install initial scene: {err:?}"));
                            event_loop.exit();
                            return;
                        }
                        *canvas_shared.borrow_mut() = Some(canvas);
                        // Replay any invalidation that arrived while the
                        // canvas didn't exist yet (e.g. an async
                        // materialization that completed during init) so its
                        // evaluation isn't silently shadowed by the initial
                        // scene installed above.
                        if let Some(invalidation) =
                            self.pending_startup_render_invalidation.take()
                        {
                            tracing::debug!(
                                target: "avenger_winit_wgpu::resize",
                                epoch = invalidation.epoch,
                                reason = ?invalidation.reason,
                                "replaying deferred startup render invalidation"
                            );
                            self.handle_render_invalidation(invalidation);
                        }
                        // Proxy events sent before the event loop runs are
                        // dropped by winit, so an evaluation-affecting
                        // invalidation from app init (e.g. a fast async
                        // materialization) may exist only as hub state.
                        // Replay it; the epoch guards in
                        // handle_render_invalidation dedupe against the
                        // stash replay above.
                        if let Some(invalidation) = self
                            .render_invalidation_hub
                            .as_ref()
                            .and_then(|hub| hub.latest_evaluation_invalidation())
                        {
                            tracing::debug!(
                                target: "avenger_winit_wgpu::resize",
                                epoch = invalidation.epoch,
                                reason = ?invalidation.reason,
                                "replaying hub evaluation invalidation after canvas creation"
                            );
                            self.handle_render_invalidation(invalidation);
                        }
                    }
                    Err(e) => {
                        self.fatal_error = Some(format!("failed to create canvas: {e:?}"));
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WinitWgpuEvent) {
        match event {
            WinitWgpuEvent::App(event) => {
                // Process file change events and other custom events
                let Some(force) = self.user_event_force(&event) else {
                    return;
                };
                self.dispatch_avenger_event(event, force);
            }
            WinitWgpuEvent::RenderInvalidated {
                host_generation,
                invalidation,
            } => {
                if host_generation == self.installed_host_generation {
                    self.handle_render_invalidation(invalidation);
                }
            }
            WinitWgpuEvent::HostUpdateReady => {
                self.install_latest_prepared_host_update();
            }
            WinitWgpuEvent::SetWindowTitle(title) => {
                if let Some(canvas) = self.canvas.borrow().as_ref() {
                    canvas.window().set_title(&title);
                } else {
                    self.window_attributes = self.window_attributes.clone().with_title(title);
                }
            }
            WinitWgpuEvent::ExitRequested => {
                self.dispatch_avenger_event(AvengerWindowEvent::WindowCloseRequested, true);
                *self.canvas.borrow_mut() = None;
                _event_loop.exit();
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

        #[cfg(not(target_arch = "wasm32"))]
        if let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.modifiers = modifiers.state();
        }

        #[cfg(not(target_arch = "wasm32"))]
        if matches!(event, WindowEvent::Focused(false)) {
            self.modifiers = keyboard::ModifiersState::empty();
        }

        #[cfg(not(target_arch = "wasm32"))]
        if self.handle_native_clipboard_shortcut(&event) {
            return;
        }

        if self.handle_canvas_frame_event(&event) {
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
                    self.dispatch_avenger_event(AvengerWindowEvent::WindowCloseRequested, true);
                    *self.canvas.borrow_mut() = None;
                    _event_loop.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.resize(physical_size);
                    }
                    let logical_size = [
                        physical_size.width as f32 / self.scale,
                        physical_size.height as f32 / self.scale,
                    ];
                    self.dispatch_avenger_event(
                        AvengerWindowEvent::WindowResize(WindowResizeEvent { size: logical_size }),
                        true,
                    );
                    self.schedule_resize_settle(logical_size);
                }
                WindowEvent::RedrawRequested => {
                    let mut rendered = false;
                    #[cfg(not(target_arch = "wasm32"))]
                    let render_start = Instant::now();
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.update();

                        match canvas.render() {
                            Ok(_) => {
                                self.render_pending = false;
                                if self.render_invalidation_pending {
                                    self.last_rendered_render_invalidation_epoch =
                                        self.last_requested_render_invalidation_epoch;
                                    self.render_invalidation_pending = false;
                                }
                                rendered = true;
                                // Phase 7 re-baseline: optional continuous render loop to
                                // measure steady-state surface-draw cost / render-only fps.
                                #[cfg(not(target_arch = "wasm32"))]
                                if std::env::var_os("AVENGER_FORCE_REDRAW_LOOP").is_some() {
                                    canvas.window().request_redraw();
                                }
                            }
                            Err(AvengerWgpuError::SurfaceError(err)) => match err {
                                wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                    canvas.resize(canvas.get_size());
                                }
                                wgpu::SurfaceError::OutOfMemory => {
                                    self.fatal_error = Some(
                                        "native rendering stopped because the GPU surface ran out of memory"
                                            .to_string(),
                                    );
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
                    if rendered {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let surface_render_ms = render_start.elapsed().as_secs_f64() * 1000.0;
                            let now = Instant::now();
                            let frame_dt_ms = self
                                .last_redraw
                                .map(|t| now.duration_since(t).as_secs_f64() * 1000.0);
                            self.last_redraw = Some(now);
                            tracing::debug!(
                                target: "avenger_winit_wgpu::resize",
                                surface_render_ms,
                                frame_dt_ms = frame_dt_ms.unwrap_or(0.0),
                                fps = frame_dt_ms.map(|dt| 1000.0 / dt).unwrap_or(0.0),
                                "winit.redraw render"
                            );
                        }
                        self.dispatch_pending_canvas_resize();
                    }
                }
                event => {
                    if matches!(
                        event,
                        WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false)
                    ) {
                        if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                            canvas.clear_tooltip();
                        }
                    }
                    if let Some(event) = AvengerWindowEvent::from_winit_event(event, self.scale) {
                        if event_schedules_interaction_settle(&event) {
                            self.schedule_interaction_settle();
                        }
                        self.dispatch_avenger_event(event, false);
                    }
                }
            }
        }
    }
}

fn install_scene_graph(
    canvas: &mut WindowCanvas<'static>,
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    window_scene_sizing: WindowSceneSizing,
    scale: f32,
    canvas_frame: Option<&mut CanvasFrameState>,
) -> Result<(), AvengerWgpuError> {
    if let Some(frame) = canvas_frame {
        frame.update_scene_size([scene_graph.width, scene_graph.height]);
        canvas.set_frame_overlay(Some(frame.overlay()));
        sync_canvas_size_to_logical_size(
            canvas,
            frame.initial_window_size(),
            window_scene_sizing,
            scale,
        );
    } else {
        canvas.set_frame_overlay(None);
        sync_canvas_size_to_logical_size(
            canvas,
            [scene_graph.width, scene_graph.height],
            window_scene_sizing,
            scale,
        );
    }
    canvas.set_scene(scene_graph)?;
    canvas.window().request_redraw();
    Ok(())
}

fn sync_canvas_size_to_logical_size(
    canvas: &mut WindowCanvas<'static>,
    logical_size: [f32; 2],
    window_scene_sizing: WindowSceneSizing,
    scale: f32,
) {
    let (match_width, match_height) = window_scene_sizing.matching_axes();
    if !match_width && !match_height {
        return;
    }

    let current = canvas.get_size();
    let target = PhysicalSize {
        width: if match_width {
            logical_to_physical(logical_size[0], scale)
        } else {
            current.width
        },
        height: if match_height {
            logical_to_physical(logical_size[1], scale)
        } else {
            current.height
        },
    };

    let width_changed = current.width.abs_diff(target.width) > 1;
    let height_changed = current.height.abs_diff(target.height) > 1;
    if !width_changed && !height_changed {
        return;
    }

    let accepted = canvas
        .window()
        .request_inner_size(Size::Physical(target))
        .map(|accepted| window_accepted_or_requested_size(target, accepted))
        .unwrap_or_else(|| window_accepted_or_requested_size(target, canvas.window().inner_size()));
    canvas.resize(accepted);
}

#[cfg(target_arch = "wasm32")]
fn window_accepted_or_requested_size(
    requested_size: PhysicalSize<u32>,
    _accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    requested_size
}

#[cfg(not(target_arch = "wasm32"))]
fn window_accepted_or_requested_size(
    _requested_size: PhysicalSize<u32>,
    accepted_size: PhysicalSize<u32>,
) -> PhysicalSize<u32> {
    accepted_size
}

fn logical_to_physical(value: f32, scale: f32) -> u32 {
    ((value * scale).round().max(1.0)) as u32
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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

fn event_kind_label(event: &AvengerWindowEvent) -> &'static str {
    match event {
        AvengerWindowEvent::MouseInput(_) => "MouseInput",
        AvengerWindowEvent::CursorMoved(_) => "CursorMoved",
        AvengerWindowEvent::CursorEntered => "CursorEntered",
        AvengerWindowEvent::CursorLeft => "CursorLeft",
        AvengerWindowEvent::MouseWheel(_) => "MouseWheel",
        AvengerWindowEvent::KeyboardInput(_) => "KeyboardInput",
        AvengerWindowEvent::ModifiersChanged(_) => "ModifiersChanged",
        AvengerWindowEvent::Ime(_) => "Ime",
        AvengerWindowEvent::Clipboard(_) => "Clipboard",
        AvengerWindowEvent::RuntimeWake(_) => "RuntimeWake",
        AvengerWindowEvent::Touch(_) => "Touch",
        AvengerWindowEvent::InteractionSettled { .. } => "InteractionSettled",
        AvengerWindowEvent::WindowResize(_) => "WindowResize",
        AvengerWindowEvent::WindowResizeSettled(_) => "WindowResizeSettled",
        AvengerWindowEvent::CanvasResize(_) => "CanvasResize",
        AvengerWindowEvent::CanvasResizeSettled(_) => "CanvasResizeSettled",
        AvengerWindowEvent::WindowMoved(_) => "WindowMoved",
        AvengerWindowEvent::WindowFocused(_) => "WindowFocused",
        AvengerWindowEvent::WindowCloseRequested => "WindowCloseRequested",
        AvengerWindowEvent::FileChanged(_) => "FileChanged",
    }
}

fn event_schedules_interaction_settle(event: &AvengerWindowEvent) -> bool {
    matches!(
        event,
        AvengerWindowEvent::CursorMoved(_)
            | AvengerWindowEvent::MouseInput(_)
            | AvengerWindowEvent::MouseWheel(_)
            | AvengerWindowEvent::Touch(_)
    )
}

fn winit_event_kind_label(event: &WindowEvent) -> &'static str {
    match event {
        WindowEvent::CursorMoved { .. } => "CursorMoved",
        WindowEvent::CursorLeft { .. } => "CursorLeft",
        WindowEvent::MouseWheel { .. } => "MouseWheel",
        WindowEvent::MouseInput { .. } => "MouseInput",
        WindowEvent::Resized(_) => "Resized",
        WindowEvent::RedrawRequested => "RedrawRequested",
        WindowEvent::CloseRequested => "CloseRequested",
        WindowEvent::KeyboardInput { .. } => "KeyboardInput",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSceneBuilder(f32);

    #[async_trait::async_trait]
    impl avenger_app::app::SceneGraphBuilder<()> for TestSceneBuilder {
        async fn build(
            &self,
            _state: &mut (),
        ) -> Result<avenger_scenegraph::scene_graph::SceneGraph, avenger_app::error::AvengerAppError>
        {
            Ok(avenger_scenegraph::scene_graph::SceneGraph {
                marks: Vec::new(),
                width: self.0,
                height: 100.0,
                origin: [0.0, 0.0],
            })
        }
    }

    fn test_app(width: f32) -> AvengerApp<()> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(AvengerApp::try_new(
                (),
                Arc::new(TestSceneBuilder(width)),
                Vec::new(),
            ))
            .unwrap()
    }

    #[test]
    fn prepared_updates_select_only_the_latest_new_generation() {
        let mut queue = VecDeque::from([
            PreparedHostUpdate {
                generation: 2,
                request_epoch: 2,
                app: test_app(200.0),
                render_invalidation_hub: None,
                image_resource_resolver: None,
                window_title: None,
                window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
                canvas_frame: None,
                completion: None,
            },
            PreparedHostUpdate {
                generation: 1,
                request_epoch: 1,
                app: test_app(100.0),
                render_invalidation_hub: None,
                image_resource_resolver: None,
                window_title: None,
                window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
                canvas_frame: None,
                completion: None,
            },
            PreparedHostUpdate {
                generation: 4,
                request_epoch: 4,
                app: test_app(400.0),
                render_invalidation_hub: None,
                image_resource_resolver: None,
                window_title: None,
                window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
                canvas_frame: None,
                completion: None,
            },
        ]);

        let update = take_latest_host_update(&mut queue, 1, 4).unwrap();
        assert_eq!(update.generation, 4);
        assert_eq!(update.app.scene_graph().width, 400.0);
        assert!(queue.is_empty());
    }

    #[test]
    fn event_loop_rejects_and_acknowledges_a_superseded_request_epoch() {
        let (completion, outcome) = std::sync::mpsc::sync_channel(1);
        let mut queue = VecDeque::from([PreparedHostUpdate {
            generation: 2,
            request_epoch: 7,
            app: test_app(200.0),
            render_invalidation_hub: None,
            image_resource_resolver: None,
            window_title: None,
            window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
            canvas_frame: None,
            completion: Some(completion),
        }]);

        assert!(take_latest_host_update(&mut queue, 1, 8).is_none());
        assert_eq!(
            outcome.recv().expect("superseded completion"),
            HostUpdateInstallOutcome::Superseded
        );
    }

    #[test]
    fn host_update_cancels_pending_resize_and_interaction_settle() {
        let mut pending_resize = Some(CanvasResizeEvent {
            size: [720.0, 480.0],
        });
        let settle_generation = AtomicU64::new(11);

        cancel_transient_input_after_host_update(&mut pending_resize, &settle_generation);

        assert!(pending_resize.is_none());
        assert_eq!(settle_generation.load(Ordering::Relaxed), 12);
    }

    fn frame_state(resize_width: bool, resize_height: bool) -> CanvasFrameState {
        let mut state = CanvasFrameState::new(CanvasFrameOptions {
            resize_width,
            resize_height,
            min_size: [120.0, 100.0],
            extra_window_size: [320.0, 240.0],
            handle_thickness: 8.0,
        });
        state.update_scene_size([400.0, 300.0]);
        state
    }

    #[test]
    fn frame_hit_testing_selects_enabled_handles() {
        let width_only = frame_state(true, false);
        assert_eq!(
            width_only.hit_test([398.0, 150.0]),
            Some(CanvasFrameHandle::Right)
        );
        assert_eq!(width_only.hit_test([200.0, 298.0]), None);

        let height_only = frame_state(false, true);
        assert_eq!(
            height_only.hit_test([200.0, 298.0]),
            Some(CanvasFrameHandle::Bottom)
        );
        assert_eq!(height_only.hit_test([398.0, 150.0]), None);

        let both = frame_state(true, true);
        assert_eq!(
            both.hit_test([399.0, 299.0]),
            Some(CanvasFrameHandle::Corner)
        );
        assert_eq!(both.hit_test([200.0, 200.0]), None);
    }

    #[test]
    fn frame_drag_clamps_to_min_size() {
        let mut state = frame_state(true, true);
        state.handle_cursor_moved([400.0, 300.0]);
        let press = state.handle_mouse_input(ElementState::Pressed, MouseButton::Left);
        assert!(press.consumed);

        let drag = state.handle_cursor_moved([20.0, 20.0]);
        assert!(drag.consumed);
        assert_eq!(drag.resize, Some([120.0, 100.0]));
        assert_eq!(state.canvas_size, [120.0, 100.0]);

        let release = state.handle_mouse_input(ElementState::Released, MouseButton::Left);
        assert_eq!(release.resize_settled, Some([120.0, 100.0]));
    }

    #[test]
    fn chart_cursor_styles_map_to_winit_icons() {
        assert_eq!(
            cursor_style_to_winit(CursorStyle::Default),
            CursorIcon::Default
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::Crosshair),
            CursorIcon::Crosshair
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::Pointer),
            CursorIcon::Pointer
        );
        assert_eq!(cursor_style_to_winit(CursorStyle::Text), CursorIcon::Text);
        assert_eq!(cursor_style_to_winit(CursorStyle::Grab), CursorIcon::Grab);
        assert_eq!(
            cursor_style_to_winit(CursorStyle::Grabbing),
            CursorIcon::Grabbing
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::ResizeHorizontal),
            CursorIcon::EwResize
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::ResizeVertical),
            CursorIcon::NsResize
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::ResizeNwSe),
            CursorIcon::NwseResize
        );
        assert_eq!(
            cursor_style_to_winit(CursorStyle::ResizeNeSw),
            CursorIcon::NeswResize
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_clipboard_shortcuts_require_command_and_ignore_repeats() {
        #[cfg(target_os = "macos")]
        let command = keyboard::ModifiersState::SUPER;
        #[cfg(not(target_os = "macos"))]
        let command = keyboard::ModifiersState::CONTROL;

        let shortcut = |character: &str, state, repeat, modifiers| {
            native_clipboard_shortcut(
                &keyboard::Key::Character(character.into()),
                state,
                repeat,
                modifiers,
            )
        };
        assert_eq!(
            shortcut("c", ElementState::Pressed, false, command),
            Some(NativeClipboardShortcut::Copy)
        );
        assert_eq!(
            shortcut("X", ElementState::Pressed, false, command),
            Some(NativeClipboardShortcut::Cut)
        );
        assert_eq!(
            shortcut("v", ElementState::Pressed, false, command),
            Some(NativeClipboardShortcut::Paste)
        );
        assert_eq!(shortcut("c", ElementState::Released, false, command), None);
        assert_eq!(shortcut("c", ElementState::Pressed, true, command), None);
        assert_eq!(
            shortcut(
                "c",
                ElementState::Pressed,
                false,
                command | keyboard::ModifiersState::ALT,
            ),
            None
        );
        assert_eq!(
            shortcut(
                "c",
                ElementState::Pressed,
                false,
                keyboard::ModifiersState::empty(),
            ),
            None
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn runtime_wake_claim_rejects_replaced_and_cancelled_generations() {
        let key = RuntimeWakeKey::new("widget", 4, "commit");
        let active = Mutex::new(HashMap::from([(key.clone(), 2)]));
        assert!(!claim_runtime_wakeup(&active, &key, 1));
        assert!(claim_runtime_wakeup(&active, &key, 2));
        assert!(!claim_runtime_wakeup(&active, &key, 2));

        active.lock().unwrap().insert(key.clone(), 3);
        active.lock().unwrap().remove(&key);
        assert!(!claim_runtime_wakeup(&active, &key, 3));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ime_cursor_area_scales_root_logical_pixels_to_physical_pixels() {
        let rect = LogicalRect::new(10.0, 20.0, 30.0, 12.0).unwrap();
        let (position, size) = physical_ime_cursor_area(rect, 2.0);
        assert_eq!(position, PhysicalPosition::new(20.0, 40.0));
        assert_eq!(size, PhysicalSize::new(60, 24));
    }
}
