use super::*;

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
    /// Selection provider belonging to this application, installed with its scene.
    pub clipboard_payload_provider: Option<ClipboardPayloadProvider>,
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
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| HostUpdateSubmitError::QueuePoisoned)?;
        let outcome = queue_prepared_update(
            &mut queue,
            self.latest_request_epoch.load(Ordering::Acquire),
            update,
        );
        if outcome == HostUpdateSubmitOutcome::Superseded {
            return Ok(outcome);
        }
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

fn queue_prepared_update<State: Clone + Send + Sync + 'static>(
    queue: &mut VecDeque<PreparedHostUpdate<State>>,
    latest_epoch: u64,
    update: PreparedHostUpdate<State>,
) -> HostUpdateSubmitOutcome {
    if update.request_epoch < latest_epoch
        || queue.iter().any(|pending| {
            pending.request_epoch >= latest_epoch && pending.generation >= update.generation
        })
    {
        complete_host_update(update, HostUpdateInstallOutcome::Superseded);
        return HostUpdateSubmitOutcome::Superseded;
    }
    for pending in queue.drain(..) {
        complete_host_update(pending, HostUpdateInstallOutcome::Superseded);
    }
    queue.push_back(update);
    HostUpdateSubmitOutcome::Queued
}

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

impl<State: Clone + Send + Sync + 'static> WinitWgpuAvengerApp<State> {
    /// Return a thread-safe handle for publishing fully prepared replacement
    /// applications to this host's event-loop thread.
    pub fn host_update_sender(&self) -> HostUpdateSender<State> {
        HostUpdateSender {
            event_proxy: self.event_proxy.clone(),
            queue: Arc::clone(&self.prepared_host_updates),
            latest_request_epoch: Arc::clone(&self.latest_host_request_epoch),
        }
    }

    pub(super) fn install_latest_prepared_host_update(&mut self) {
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
        let attachment = match background::attach(
            update.app.background_tasks(),
            self.event_proxy.clone(),
            update.generation,
            #[cfg(not(target_arch = "wasm32"))]
            self.tokio_runtime.handle().clone(),
        ) {
            Ok(attachment) => attachment,
            Err(error) => {
                complete_host_update_sender(
                    completion,
                    HostUpdateInstallOutcome::Failed(error.to_string()),
                );
                return;
            }
        };

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
                canvas.set_image_resource_resolver(resolver.clone());
                self.canvas_config.image_resource_config.resolver = Some(resolver);
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

        self.wake_scheduler.clear();
        self.resize_settle_generation
            .set(self.resize_settle_generation.get() + 1);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.modifiers = keyboard::ModifiersState::empty();
            self.clipboard = None;
            if let Some(canvas) = self.canvas.borrow().as_ref() {
                canvas.window().set_ime_allowed(false);
            }
        }
        self.window_scene_sizing = update.window_scene_sizing;
        self.canvas_frame = replacement_frame;
        *self.avenger_app.borrow_mut() = update.app;
        #[cfg(target_arch = "wasm32")]
        if let Some(host) = self.text_agent.borrow_mut().as_mut() {
            host.reset_for_replacement([scene_graph.width, scene_graph.height]);
            host.set_clipboard_payload_provider(update.clipboard_payload_provider.clone());
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.clipboard_payload_provider = update.clipboard_payload_provider;
        }
        self._render_invalidation_subscription = None;
        self.render_invalidation_hub = update.render_invalidation_hub;
        self.last_requested_render_invalidation_epoch = 0;
        self.last_rendered_render_invalidation_epoch = 0;
        self.hub_epoch_at_last_evaluation_start.set(0);
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
        self.background
            .borrow_mut()
            .install(update.generation, attachment);
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
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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
                clipboard_payload_provider: None,
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
                clipboard_payload_provider: None,
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
                clipboard_payload_provider: None,
                window_title: None,
                window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
                canvas_frame: None,
                completion: None,
            },
        ]);

        let mut old = queue.pop_front().unwrap();
        old.request_epoch = 4;
        assert_eq!(
            queue_prepared_update(&mut queue, 4, old),
            HostUpdateSubmitOutcome::Superseded
        );
        assert_eq!(queue.len(), 2);
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
            clipboard_payload_provider: None,
            window_title: None,
            window_scene_sizing: WindowSceneSizing::MatchSceneGraph,
            canvas_frame: None,
            completion: Some(completion),
        }]);

        let mut newer = PreparedHostUpdate {
            generation: 3,
            request_epoch: 8,
            app: test_app(300.0),
            render_invalidation_hub: None,
            image_resource_resolver: None,
            clipboard_payload_provider: None,
            window_title: None,
            window_scene_sizing: WindowSceneSizing::SurfaceFollowsWindow,
            canvas_frame: None,
            completion: None,
        };
        let old = queue.pop_front().unwrap();
        assert_eq!(
            queue_prepared_update(&mut queue, 8, newer),
            HostUpdateSubmitOutcome::Queued
        );
        assert_eq!(
            queue_prepared_update(&mut queue, 8, old),
            HostUpdateSubmitOutcome::Superseded
        );
        newer = queue.pop_front().unwrap();
        assert_eq!(newer.generation, 3);
        // A request that becomes stale after submission is rejected at installation too.
        queue.push_back(newer);
        assert!(take_latest_host_update(&mut queue, 1, 9).is_none());

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
}
