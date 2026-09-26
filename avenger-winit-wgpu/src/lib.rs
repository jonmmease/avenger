use avenger_app::app::AvengerApp;
mod background;
mod host_update;
mod render_invalidation;
pub use host_update::{
    HostUpdateInstallOutcome, HostUpdateSender, HostUpdateSubmitError, HostUpdateSubmitOutcome,
    PreparedHostUpdate,
};
mod wake_scheduler;
use avenger_common::{canvas::CanvasDimensions, cursor::CursorStyle, time::Instant};
#[cfg(not(target_arch = "wasm32"))]
use avenger_eventstream::runtime::{InputSession, KeyboardPolicy};
use avenger_eventstream::runtime::{RuntimeHostCommand, RuntimeWakeEvent};
use avenger_eventstream::window::{
    CanvasResizeEvent, WindowEvent as AvengerWindowEvent, WindowResizeEvent,
};
#[cfg(not(target_arch = "wasm32"))]
use avenger_eventstream::{runtime::LogicalRect, window::ClipboardEvent};
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
use render_invalidation::send_render_invalidation_event;
use std::{
    collections::VecDeque,
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use wake_scheduler::{send_event_after, RuntimeWakeScheduler};
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
    BackgroundTaskReady {
        host_generation: u64,
        event: RuntimeWakeEvent,
    },
    SetWindowTitle(String),
    ExitRequested,
    CanvasFrameResize {
        host_generation: u64,
        size: [f32; 2],
        settled: bool,
    },
    ResizeSettled {
        size: [f32; 2],
        generation: u64,
    },
    RuntimeWake {
        event: RuntimeWakeEvent,
        ticket: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WinitWgpuHostInitError {
    EventLoop(String),
    FileWatcher(String),
    BackgroundTasks(String),
}

impl fmt::Display for WinitWgpuHostInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(message) => write!(f, "failed to build native event loop: {message}"),
            Self::FileWatcher(message) => {
                write!(f, "failed to initialize app file watcher: {message}")
            }
            Self::BackgroundTasks(message) => {
                write!(f, "failed to attach background tasks: {message}")
            }
        }
    }
}

impl std::error::Error for WinitWgpuHostInitError {}

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

mod canvas_frame;
use canvas_frame::{CanvasFrameEventOutcome, CanvasFrameState};
pub use canvas_frame::{CanvasFrameOptions, WindowSceneSizing};

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
    resize_settle_generation: std::cell::Cell<u64>,
    canvas_frame: Option<CanvasFrameState>,
    canvas_config: CanvasConfig,
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
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
    hub_epoch_at_last_evaluation_start: std::rc::Rc<std::cell::Cell<u64>>,
    pub file_watcher: Option<FileWatcher>,
    window_id: Option<winit::window::WindowId>,
    coalesced_event_count: usize,
    stale_canvas_resize_count: usize,
    pending_canvas_resize: Option<CanvasResizeEvent>,
    fatal_error: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    clipboard: Option<arboard::Clipboard>,
    #[cfg(not(target_arch = "wasm32"))]
    input_session: Option<InputSession>,
    #[cfg(not(target_arch = "wasm32"))]
    composition_session: Option<Option<InputSession>>,
    #[cfg(not(target_arch = "wasm32"))]
    keyboard_policy: Option<KeyboardPolicy>,
    #[cfg(not(target_arch = "wasm32"))]
    pointer_captured: bool,
    #[cfg(not(target_arch = "wasm32"))]
    modifiers: keyboard::ModifiersState,
    #[cfg(target_arch = "wasm32")]
    text_agent: std::rc::Rc<std::cell::RefCell<Option<TextAgentHost>>>,
    #[cfg(target_arch = "wasm32")]
    clipboard_payload_provider: Option<ClipboardPayloadProvider>,
    prepared_host_updates: Arc<Mutex<VecDeque<PreparedHostUpdate<State>>>>,
    latest_host_request_epoch: Arc<AtomicU64>,
    installed_host_generation: u64,
    wake_scheduler: std::rc::Rc<RuntimeWakeScheduler>,
    background: std::rc::Rc<std::cell::RefCell<background::BackgroundHost>>,

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

        let wake_scheduler = std::rc::Rc::new(RuntimeWakeScheduler::new(event_proxy.clone()));
        let attachment = background::attach(
            avenger_app.background_tasks(),
            event_proxy.clone(),
            0,
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime.handle().clone(),
        )
        .map_err(|error| WinitWgpuHostInitError::BackgroundTasks(error.to_string()))?;
        let mut background = background::BackgroundHost::default();
        background.install(0, attachment);

        let winit_app = Self {
            canvas: std::rc::Rc::new(std::cell::RefCell::new(None)),
            scale: options.scale,
            window_attributes: options.window_attributes,
            window_scene_sizing: options.window_scene_sizing,
            resize_settle_delay_ms: options.resize_settle_delay_ms,
            interaction_settle_delay_ms: options.interaction_settle_delay_ms,
            interaction_settle_generation: Arc::new(AtomicU64::new(0)),
            resize_settle_generation: std::cell::Cell::new(0),
            canvas_frame: options.canvas_frame.map(CanvasFrameState::new),
            canvas_config: options.canvas_config,
            event_proxy,
            _render_invalidation_subscription: render_invalidation_subscription,
            render_invalidation_hub: options.render_invalidation_hub,
            avenger_app: std::rc::Rc::new(std::cell::RefCell::new(avenger_app)),
            render_pending: false,
            render_invalidation_pending: false,
            pending_startup_render_invalidation: None,
            last_requested_render_invalidation_epoch: 0,
            last_rendered_render_invalidation_epoch: 0,
            hub_epoch_at_last_evaluation_start: Default::default(),
            file_watcher,
            window_id: None,
            coalesced_event_count: 0,
            stale_canvas_resize_count: 0,
            pending_canvas_resize: None,
            fatal_error: None,
            wake_scheduler,
            background: std::rc::Rc::new(std::cell::RefCell::new(background)),
            prepared_host_updates: Default::default(),
            latest_host_request_epoch: Default::default(),
            installed_host_generation: 0,
            #[cfg(not(target_arch = "wasm32"))]
            clipboard: None,
            #[cfg(not(target_arch = "wasm32"))]
            input_session: None,
            #[cfg(not(target_arch = "wasm32"))]
            composition_session: None,
            #[cfg(not(target_arch = "wasm32"))]
            keyboard_policy: None,
            #[cfg(not(target_arch = "wasm32"))]
            pointer_captured: false,
            #[cfg(not(target_arch = "wasm32"))]
            modifiers: keyboard::ModifiersState::default(),
            #[cfg(target_arch = "wasm32")]
            text_agent: Default::default(),
            #[cfg(target_arch = "wasm32")]
            clipboard_payload_provider: options.clipboard_payload_provider,
            #[cfg(not(target_arch = "wasm32"))]
            last_redraw: None,
            #[cfg(not(target_arch = "wasm32"))]
            tokio_runtime,
        };

        Ok((winit_app, event_loop))
    }

    /// Take a fatal initialization error recorded by the event-loop handler.
    pub fn take_fatal_error(&mut self) -> Option<String> {
        self.fatal_error.take()
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
                let wake_scheduler = self.wake_scheduler.clone();
                let text_agent = self.text_agent.clone();
                let canvas_shared = self.canvas.clone();
                let evaluation_epoch = self.hub_epoch_at_last_evaluation_start.clone();
                let hub = self.render_invalidation_hub.clone();

                #[allow(clippy::await_holding_refcell_ref)]
                let update_future = async move {
                    let hub_epoch_before = hub.as_ref().map(|hub| hub.epoch());
                    let update_result = app_clone
                        .borrow_mut()
                        .update_with_status(&event_clone, Instant::now())
                        .await;

                    match update_result {
                        Ok(update) => {
                            if let (true, Some(epoch)) = (update.scene_graph.is_some(), hub_epoch_before) {
                                evaluation_epoch.set(evaluation_epoch.get().max(epoch));
                            }
                            if let Some(cursor) = update.status.cursor {
                                if let Some(canvas) = canvas_shared.borrow().as_ref() {
                                    canvas.window().set_cursor(cursor_style_to_winit(cursor));
                                }
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
                                        return;
                                    }
                                }
                            }
                            apply_browser_host_commands(update.status.commands, &wake_scheduler, &canvas_shared, &text_agent);

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
                    self.hub_epoch_at_last_evaluation_start.set(self.hub_epoch_at_last_evaluation_start.get().max(epoch));
                }
                if let Some(cursor) = scene_graph_opt.status.cursor {
                    self.set_cursor(cursor_style_to_winit(cursor));
                }
                let commands = std::mem::take(&mut scene_graph_opt.status.commands);
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
                            return;
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
                self.apply_runtime_host_commands(commands);
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
        let generation = self.resize_settle_generation.get() + 1;
        self.resize_settle_generation.set(generation);
        send_event_after(
            self.event_proxy.clone(),
            WinitWgpuEvent::ResizeSettled { size, generation },
            std::time::Duration::from_millis(delay_ms),
        );
    }

    fn schedule_interaction_settle(&self) {
        let Some(delay_ms) = self.interaction_settle_delay_ms else {
            return;
        };
        let generation = self
            .interaction_settle_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        send_event_after(
            self.event_proxy.clone(),
            WinitWgpuEvent::App(AvengerWindowEvent::InteractionSettled { generation }),
            std::time::Duration::from_millis(delay_ms),
        );
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
            WindowEvent::Focused(false) => frame.handle_focus_lost(),
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
            let _ = self
                .event_proxy
                .send_event(WinitWgpuEvent::CanvasFrameResize {
                    host_generation: self.installed_host_generation,
                    size,
                    settled: false,
                });
            tracing::trace!(
                target: "avenger_winit_wgpu::resize",
                width = size[0],
                height = size[1],
                pointer_ms = pointer_start.elapsed().as_secs_f64() * 1000.0,
                "canvas_frame.pointer queued CanvasResize"
            );
        }
        if let Some(size) = outcome.resize_settled {
            let _ = self
                .event_proxy
                .send_event(WinitWgpuEvent::CanvasFrameResize {
                    host_generation: self.installed_host_generation,
                    size,
                    settled: true,
                });
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
                } => self.wake_scheduler.request(key, deadline, generation),
                RuntimeHostCommand::CancelWakeup { key } => self.wake_scheduler.cancel(&key),
                RuntimeHostCommand::SetInputSession { session } => self.input_session = session,
                RuntimeHostCommand::SetKeyboardPolicy { policy } => self.keyboard_policy = policy,
                RuntimeHostCommand::SetPointerCapture { captured } => {
                    self.pointer_captured = captured
                }
                RuntimeHostCommand::SetImeAllowed { allowed } => {
                    if let Some(canvas) = self.canvas.borrow().as_ref() {
                        canvas.window().set_ime_allowed(allowed);
                    }
                }
                RuntimeHostCommand::SetImeCursorArea { rect } => {
                    if let Some(canvas) = self.canvas.borrow().as_ref() {
                        let scale = self.scale as f64;
                        let rect = rect.unwrap_or_else(|| {
                            LogicalRect::new(0.0, 0.0, 0.0, 0.0)
                                .expect("zero IME rectangle is finite")
                        });
                        let (position, size) = physical_ime_cursor_area(rect, scale);
                        canvas.window().set_ime_cursor_area(position, size);
                    }
                }
                RuntimeHostCommand::SetClipboardPayload { .. } => {}
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
    fn stamp_native_input(&mut self, event: AvengerWindowEvent) -> AvengerWindowEvent {
        use avenger_eventstream::window::ImeEvent;
        let session = match &event {
            AvengerWindowEvent::Ime(ImeEvent::Preedit { .. }) => {
                self.composition_session
                    .get_or_insert_with(|| self.input_session.clone());
                self.composition_session.clone().flatten()
            }
            AvengerWindowEvent::Ime(ImeEvent::Commit(_) | ImeEvent::Disabled) => self
                .composition_session
                .take()
                .unwrap_or_else(|| self.input_session.clone()),
            _ => self.input_session.clone(),
        };
        event.with_input_session(session)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_native_clipboard_shortcut(&mut self, event: &WindowEvent) -> bool {
        if self
            .keyboard_policy
            .as_ref()
            .is_some_and(|p| !p.text_shortcuts)
        {
            return false;
        }
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
        let event = self.stamp_native_input(AvengerWindowEvent::Clipboard(clipboard_event));
        self.dispatch_avenger_event(event, false);
        true
    }

    #[cfg(target_arch = "wasm32")]
    fn setup_wasm_canvas(&self, window: &Arc<winit::window::Window>) {
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
        let mut host = TextAgentHost::new_with_clipboard_payload_provider(
            canvas,
            self.event_proxy.clone(),
            self.clipboard_payload_provider.clone(),
        )
        .expect("failed to install wasm text agent");
        host.install_winit_keyboard_policy(window.clone())
            .expect("failed to install wasm keyboard policy");
        *self.text_agent.borrow_mut() = Some(host);
    }
}

impl<State: Clone + Send + Sync + 'static> Drop for WinitWgpuAvengerApp<State> {
    fn drop(&mut self) {
        // Shut down before the runtime drops, even if async canvas setup holds a clone.
        self.background.borrow_mut().shutdown();
    }
}

impl<State> ApplicationHandler<WinitWgpuEvent> for WinitWgpuAvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = match event_loop.create_window(self.window_attributes.clone()) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fatal_error = Some(format!("failed to create native window: {error}"));
                self.background.borrow_mut().shutdown();
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

        let initial_commands = self.avenger_app.borrow_mut().take_host_commands();
        let canvas_future = WindowCanvas::new(window, dimensions, self.canvas_config.clone());

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                use wasm_bindgen::JsCast;

                let event_proxy = self.event_proxy.clone();
                let text_agent = self.text_agent.clone();
                let wake_scheduler = self.wake_scheduler.clone();
                let render_invalidation_hub = self.render_invalidation_hub.clone();
                let render_generation = self.installed_host_generation;
                let background = self.background.clone();
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
                                background.borrow_mut().shutdown();
                                let _ = event_proxy.send_event(WinitWgpuEvent::ExitRequested);
                                return;
                            }
                            *canvas_shared.borrow_mut() = Some(canvas);
                            apply_browser_host_commands(initial_commands, &wake_scheduler, &canvas_shared, &text_agent);
                            background.borrow_mut().activate();
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
                            background.borrow_mut().shutdown();
                            let _ = event_proxy.send_event(WinitWgpuEvent::ExitRequested);
                        }
                    }
                };
                let callback = wasm_bindgen::closure::Closure::once_into_js(move || {
                    spawn_local(setup_future);
                });
                web_sys::window()
                    .and_then(|window| {
                        window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                callback.unchecked_ref(),
                                0,
                            )
                            .ok()
                    })
                    .expect("schedule wasm canvas setup");
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
                            self.background.borrow_mut().shutdown();
                            event_loop.exit();
                            return;
                        }
                        *canvas_shared.borrow_mut() = Some(canvas);
                        self.apply_runtime_host_commands(initial_commands);
                        self.background.borrow_mut().activate();
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
                        self.background.borrow_mut().shutdown();
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.background.borrow_mut().shutdown();
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
            WinitWgpuEvent::BackgroundTaskReady {
                host_generation,
                event,
            } => {
                let current = self.background.borrow().accepts(host_generation);
                if current {
                    self.dispatch_avenger_event(AvengerWindowEvent::RuntimeWake(event), true);
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
                self.background.borrow_mut().shutdown();
                _event_loop.exit();
            }
            WinitWgpuEvent::CanvasFrameResize {
                host_generation,
                size,
                settled,
            } => {
                if host_generation == self.installed_host_generation {
                    let event = if settled {
                        AvengerWindowEvent::CanvasResizeSettled(CanvasResizeEvent { size })
                    } else {
                        AvengerWindowEvent::CanvasResize(CanvasResizeEvent { size })
                    };
                    if let Some(force) = self.user_event_force(&event) {
                        self.dispatch_avenger_event(event, force);
                    }
                }
            }
            WinitWgpuEvent::ResizeSettled { size, generation } => {
                if generation == self.resize_settle_generation.get() {
                    self.dispatch_avenger_event(
                        AvengerWindowEvent::WindowResizeSettled(WindowResizeEvent { size }),
                        true,
                    );
                }
            }
            WinitWgpuEvent::RuntimeWake { event, ticket } => {
                if self.wake_scheduler.claim(&event, ticket) {
                    self.dispatch_avenger_event(AvengerWindowEvent::RuntimeWake(event), true);
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

        // The DOM text agent combines focus on the canvas and its hidden input.
        #[cfg(target_arch = "wasm32")]
        if matches!(event, WindowEvent::Focused(_)) {
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

        #[cfg(not(target_arch = "wasm32"))]
        if self.pointer_captured && matches!(event, WindowEvent::CursorLeft { .. }) {
            self.pointer_captured = false;
            self.dispatch_avenger_event(AvengerWindowEvent::PointerCaptureLost, false);
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
                    self.background.borrow_mut().shutdown();
                    _event_loop.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                        canvas.clear_tooltip();
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
                                    self.background.borrow_mut().shutdown();
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
                        #[cfg(not(target_arch = "wasm32"))]
                        let event = self.stamp_native_input(event);
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
        AvengerWindowEvent::TextInput(_) => "TextInput",
        AvengerWindowEvent::PointerCaptureLost => "PointerCaptureLost",
        AvengerWindowEvent::FocusEntered { .. } => "FocusEntered",
        AvengerWindowEvent::Ime(_) => "Ime",
        AvengerWindowEvent::Clipboard(_) => "Clipboard",
        AvengerWindowEvent::ModifiersChanged(_) => "ModifiersChanged",
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

#[cfg(test)]
mod input_tests {
    use super::*;
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
    fn ime_cursor_area_scales_root_logical_pixels_to_physical_pixels() {
        let rect = LogicalRect::new(10.0, 20.0, 30.0, 12.0).unwrap();
        let (position, size) = physical_ime_cursor_area(rect, 2.0);
        assert_eq!(position, PhysicalPosition::new(20.0, 40.0));
        assert_eq!(size, PhysicalSize::new(60, 24));
    }
}

#[cfg(target_arch = "wasm32")]
fn apply_browser_host_commands(
    commands: Vec<RuntimeHostCommand>,
    scheduler: &RuntimeWakeScheduler,
    canvas: &std::cell::RefCell<Option<WindowCanvas<'static>>>,
    text_agent: &std::cell::RefCell<Option<TextAgentHost>>,
) {
    let mut text_commands = Vec::new();
    for command in commands {
        match command {
            RuntimeHostCommand::RequestWakeup {
                key,
                deadline,
                generation,
            } => scheduler.request(key, deadline, generation),
            RuntimeHostCommand::CancelWakeup { key } => scheduler.cancel(&key),
            RuntimeHostCommand::UpdateTooltip(update) => {
                if let Some(canvas) = canvas.borrow_mut().as_mut() {
                    if let Err(error) = canvas.set_tooltip_update(update) {
                        log::error!("failed to update tooltip: {error}");
                    }
                }
            }
            command => text_commands.push(command),
        }
    }
    if let Some(host) = text_agent.borrow_mut().as_mut() {
        host.apply_commands(text_commands);
    }
}
