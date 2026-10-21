use avenger_common::time::Instant;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_common::cursor::CursorStyle;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::WindowEvent;
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_image::{ImageCache, ImageResolver};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::alarm::Alarm;
use crate::error::AvengerAppError;
use crate::task::{Attach, Runner, Spawner, Task};
use crate::waker::{AppWaker, Refresh};

/// What the host does after [`AvengerApp::update`] or [`AvengerApp::refresh`]: install the new
/// scene and the new overlay, show the new cursor, and draw a frame when the scene or the overlay
/// is new or `redraw` asks for one.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub scene: Option<Arc<SceneGraph>>,
    /// The new overlay, which the host draws over the scene.
    pub overlay: Option<Arc<SceneGraph>>,
    /// Draw a frame though the scene and the overlay are the same, as when images arrive.
    pub redraw: bool,
    /// The cursor that the state and the overlays now call for, when it differs from the one
    /// before.
    pub cursor: Option<CursorStyle>,
    /// The text that copying now takes, when it differs from the text before: empty when
    /// there's nothing to copy.
    pub copy_text: Option<String>,
}

/// A scene, and the rows behind its marks.
#[derive(Debug, Clone)]
pub struct BuiltScene {
    pub scene: SceneGraph,
    /// The rows behind the scene's marks, which the hit-testing tree pairs with the marks'
    /// instances.
    pub data: SceneData,
}

impl From<SceneGraph> for BuiltScene {
    /// A scene with no rows behind its marks.
    fn from(scene: SceneGraph) -> Self {
        Self {
            scene,
            data: SceneData::new(),
        }
    }
}

#[async_trait]
pub trait SceneGraphBuilder<State: Clone + Send + Sync + 'static> {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError>;
}

/// Builds an overlay: marks that the host draws over the scene, such as a tooltip. The app
/// builds its overlays without building the scene or its hit-testing tree, so they can follow
/// every pointer move.
pub trait OverlayBuilder<State: Clone + Send + Sync + 'static> {
    /// The overlay's marks, in the scene's coordinates, over `scene` and the rows behind its
    /// marks, `data`, from the build that drew it, and over `beneath`, the marks of the overlays
    /// that the app took before this one, such as a pinned tooltip's. The scene and its rows can
    /// be newer than the hit-testing tree, when a handler asked for a scene and not for its tree.
    /// Each scene build shares a new scene, and the overlay builds that follow a hover share the
    /// same one, so an overlay that indexes the scene can tell them apart with [`Arc::ptr_eq`].
    /// When the build fails, the app logs the error and draws nothing for the overlay until its
    /// next build, and the scene and the other overlays still draw.
    fn build(
        &self,
        state: &State,
        scene: &Arc<SceneGraph>,
        beneath: &[SceneMark],
        data: &SceneData,
        text_engine: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError>;

    /// The event streams that drive the overlay, which the app registers with it, after the
    /// app's own. An overlay that keeps its own state, such as the focus a tooltip shows, changes
    /// it in their handlers, and asks for an overlay build with [`UpdateStatus::overlay`].
    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        Vec::new()
    }

    /// The cursor that the overlay calls for, as a view of its own state, such as a text cursor
    /// over text that it selects, or `None` to leave the cursor to the overlays beneath it and
    /// the app's state.
    fn cursor(&self, _state: &State) -> Option<CursorStyle> {
        None
    }

    /// The text that copying takes from the overlay, as a view of its own state, such as the
    /// text that it selects, or `None` for none.
    fn copy_text(&self, _state: &State) -> Option<String> {
        None
    }
}

/// The cursor that a state calls for.
type CursorOf<State> = Arc<dyn Fn(&State) -> CursorStyle + Send + Sync>;

#[derive(Clone)]
pub struct AvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
    /// The overlays' builders, whose marks draw in order.
    overlay_builders: Vec<Arc<dyn OverlayBuilder<State>>>,
    event_stream_manager: EventStreamManager<State>,
    rtree: SceneGraphRTree,
    scene_graph: Arc<SceneGraph>,
    /// The rows behind the scene's marks, from the build that drew it, which can be newer than
    /// the tree's.
    scene_data: SceneData,
    overlay: Arc<SceneGraph>,
    /// The cursor as a view of the state, which [`AvengerApp::with_cursor`] sets.
    cursor_of: Option<CursorOf<State>>,
    /// The cursor that the state and the overlays last called for.
    cursor: CursorStyle,
    /// The text that copying last took, empty for none.
    copy_text: String,
    text_engine: LabelEngine,
    waker: AppWaker,
    /// Wakes the host when the next wake that the event streams wait for is due.
    alarm: Alarm,
    images: Option<Arc<dyn ImageResolver>>,
    /// Where the app's tasks run their jobs: the app's runtime on native targets, once it has
    /// one, and the browser's runtime in the browser.
    spawner: Option<Spawner>,
    /// The tasks that wait for the app's runtime.
    waiting_tasks: Vec<Arc<dyn Attach>>,
}

impl<State> AvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// Get a mutable reference to the app state
    pub fn app_state_mut(&mut self) -> &mut State {
        self.event_stream_manager.state_mut()
    }

    pub async fn try_new(
        initial_state: State,
        scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
        stream_callbacks: Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)>,
        text_engine: LabelEngine,
    ) -> Result<Self, AvengerAppError> {
        let mut event_stream_manager = EventStreamManager::new(initial_state);
        for (config, handler) in stream_callbacks {
            event_stream_manager.register_handler(config, handler);
        }
        // Build initial scene graph and rtree
        let built = scene_graph_builder
            .build(event_stream_manager.state_mut())
            .await?;
        let rtree = tree(&built.scene, &built.data, &text_engine)?;
        let overlay = Arc::new(overlay(&built.scene, Vec::new()));
        let waker = AppWaker::new();

        Ok(Self {
            scene_graph_builder,
            overlay_builders: Vec::new(),
            event_stream_manager,
            rtree,
            scene_graph: Arc::new(built.scene),
            scene_data: built.data,
            overlay,
            cursor_of: None,
            cursor: CursorStyle::Default,
            copy_text: String::new(),
            text_engine,
            alarm: Alarm::new(waker.clone()),
            waker,
            images: None,
            #[cfg(not(target_arch = "wasm32"))]
            spawner: None,
            #[cfg(target_arch = "wasm32")]
            spawner: Some(Spawner::Browser),
            waiting_tasks: Vec::new(),
        })
    }

    /// Show the cursor that `cursor` calls for from the state, such as a grabbing hand while a
    /// drag lasts. The app reads it again after each update and refresh, whatever the handlers
    /// asked for, so a handler changes the cursor by changing the state. An overlay that calls
    /// for a cursor, as [`OverlayBuilder::cursor`] does, takes precedence.
    pub fn with_cursor(
        mut self,
        cursor: impl Fn(&State) -> CursorStyle + Send + Sync + 'static,
    ) -> Self {
        self.cursor_of = Some(Arc::new(cursor));
        self.changed_views(&mut Changes::default());
        self
    }

    /// Add an overlay over those that the app has, and register its event streams, which hear
    /// each event before those of the overlays beneath and the app's own, so a stream of the
    /// overlay's that consumes an event takes it from them. The app builds its overlays now, and
    /// again with each new scene and whenever an event stream's handler asks for them. Returns the
    /// overlay's error when it fails to build now.
    pub fn with_overlay(
        mut self,
        builder: Arc<dyn OverlayBuilder<State>>,
    ) -> Result<Self, AvengerAppError> {
        builder.build(
            self.event_stream_manager.state(),
            &self.scene_graph,
            &self.overlay.marks,
            &self.scene_data,
            &self.text_engine,
        )?;
        // Each overlay's streams hear events in a layer of their own over the layers beneath.
        let layer = self.overlay_builders.len() + 1;
        for (config, handler) in builder.streams() {
            self.event_stream_manager
                .register_handler_in_layer(layer, config, handler);
        }
        self.overlay_builders.push(builder);
        self.build_overlay();
        self.changed_views(&mut Changes::default());
        Ok(self)
    }

    /// Run the app's tasks' jobs on `runtime`, a multi-thread Tokio runtime. A current-thread
    /// runtime runs its tasks only while a thread blocks on it, so the app rejects one.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_runtime(
        mut self,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, AvengerAppError> {
        if runtime.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
            return Err(AvengerAppError::InternalError(
                "background tasks need a multi-thread Tokio runtime".to_string(),
            ));
        }
        let spawner = Spawner::Runtime(runtime);
        for task in self.waiting_tasks.drain(..) {
            task.attach(Runner::new(spawner.clone(), self.waker.clone()));
        }
        self.spawner = Some(spawner);
        Ok(self)
    }

    /// Run `task`'s jobs on the app's runtime, and request a rebuild on the app's waker when
    /// each of its requests finishes or fails. On native targets, its jobs wait for
    /// [`AvengerApp::with_runtime`].
    pub fn with_task<K, T>(mut self, task: &Task<K, T>) -> Self
    where
        K: Send + Sync + 'static,
        T: Send + Sync + 'static,
    {
        match &self.spawner {
            Some(spawner) => task.attach(Runner::new(spawner.clone(), self.waker.clone())),
            None => self.waiting_tasks.push(Arc::new(task.clone())),
        }
        self
    }

    /// Draw the scenes' image resources through `images`. Hosts give the cache to their
    /// canvases, and each image that arrives or fails requests a redraw on the app's waker.
    pub fn with_images(mut self, images: ImageCache) -> Self {
        let waker = self.waker.clone();
        images.set_on_change(move || waker.request(Refresh::Redraw));
        self.images = Some(Arc::new(images));
        self
    }

    /// The resolver of the scenes' image resources, for the host's canvas.
    pub fn images(&self) -> Option<Arc<dyn ImageResolver>> {
        self.images.clone()
    }

    /// The waker through which background work asks the host to refresh, for the app's state
    /// to hold.
    pub fn waker(&self) -> AppWaker {
        self.waker.clone()
    }

    /// Take the waker's pending refresh, run the wakes that are due, and do what the refresh and
    /// the wakes' handlers ask, as [`AvengerApp::update`] does for an event's handlers. A rebuild
    /// builds the scene and its hit-testing tree again. Returns what the host must do.
    pub async fn refresh(&mut self) -> Result<Changes, AvengerAppError> {
        let pending = self.waker.take();
        let status = self
            .event_stream_manager
            .dispatch_due_wakes(&self.rtree, Instant::now())
            .await;
        self.set_alarm();
        let rebuild = pending == Some(Refresh::Rebuild);
        let mut changes = Changes {
            redraw: pending == Some(Refresh::Redraw),
            ..Default::default()
        };
        if rebuild || status.rerender || status.rebuild_geometry {
            self.build_scene().await?;
            if rebuild || status.rebuild_geometry {
                self.rebuild_tree().await?;
            }
            changes.scene = Some(self.scene_graph.clone());
        }
        if changes.scene.is_some() || status.overlay {
            changes.overlay = self.build_overlay();
        }
        self.changed_views(&mut changes);
        Ok(changes)
    }

    /// Set the alarm for the next wake that the event streams wait for.
    fn set_alarm(&self) {
        if let Some(at) = self.event_stream_manager.next_wake() {
            self.alarm.set(at);
        }
    }

    /// Build the scene and the rows behind its marks from the state. The pending refresh clears
    /// first, since the new scene shows the state as it is, and a request during the build
    /// raises it again.
    async fn build_scene(&mut self) -> Result<(), AvengerAppError> {
        self.waker.take();
        match self
            .scene_graph_builder
            .build(self.event_stream_manager.state_mut())
            .await
        {
            Ok(built) => {
                self.scene_graph = Arc::new(built.scene);
                self.scene_data = built.data;
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to build scene graph: {e:?}");
                Err(AvengerAppError::InternalError(
                    "Failed to build scene graph".to_string(),
                ))
            }
        }
    }

    /// Build the scene's hit-testing tree with its rows, and dispatch
    /// [`TreeRebuilt`](avenger_eventstream::scene::SceneGraphEvent::TreeRebuilt), so a hover
    /// finds its focus again. The overlay builds after every scene build, so the event's
    /// handlers change only the state that the overlay shows: the app ignores what they ask
    /// for. When the rows don't match the scene, the tree takes the scene's hits without rows,
    /// so hits never find the instances where an older scene drew them, and the error returns.
    async fn rebuild_tree(&mut self) -> Result<(), AvengerAppError> {
        let (rtree, result) = match tree(&self.scene_graph, &self.scene_data, &self.text_engine) {
            Ok(rtree) => (rtree, Ok(())),
            Err(err) => (
                SceneGraphRTree::from_scene_graph(&self.scene_graph, &self.text_engine),
                Err(err),
            ),
        };
        self.rtree = rtree;
        self.event_stream_manager
            .dispatch_tree_rebuilt(&self.rtree, Instant::now())
            .await;
        result
    }

    /// Read the views of the state and the overlays again, and put those that changed in
    /// `changes`. The topmost overlay that calls for a cursor or offers text to copy gives it,
    /// and else the state's cursor does.
    fn changed_views(&mut self, changes: &mut Changes) {
        let state = self.event_stream_manager.state();
        let cursor = (self.overlay_builders.iter().rev())
            .find_map(|builder| builder.cursor(state))
            .or_else(|| self.cursor_of.as_ref().map(|cursor| cursor(state)))
            .unwrap_or_default();
        let copy_text = (self.overlay_builders.iter().rev())
            .find_map(|builder| builder.copy_text(state))
            .unwrap_or_default();
        if cursor != self.cursor {
            self.cursor = cursor;
            changes.cursor = Some(cursor);
        }
        if copy_text != self.copy_text {
            changes.copy_text = Some(copy_text.clone());
            self.copy_text = copy_text;
        }
    }

    /// Build the overlays from the state, over the scene, and return them as one overlay. An
    /// overlay that fails to build logs its error and draws nothing. Without overlays, there's no
    /// new overlay.
    fn build_overlay(&mut self) -> Option<Arc<SceneGraph>> {
        if self.overlay_builders.is_empty() {
            return None;
        }
        let mut marks = Vec::new();
        for builder in &self.overlay_builders {
            match builder.build(
                self.event_stream_manager.state(),
                &self.scene_graph,
                &marks,
                &self.scene_data,
                &self.text_engine,
            ) {
                Ok(built) => marks.extend(built),
                Err(err) => log::error!("Failed to build an overlay: {err}"),
            }
        }
        self.overlay = Arc::new(overlay(&self.scene_graph, marks));
        Some(self.overlay.clone())
    }

    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.event_stream_manager.get_watched_files()
    }

    /// Update the state of the app without rebuilding the scene graph
    pub async fn update_state(&mut self, event: &WindowEvent, instant: Instant) -> UpdateStatus {
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await;
        self.set_alarm();
        update_status
    }

    /// Update the state of the app, and build the scene, its hit-testing tree, and the overlay
    /// as the handlers ask. A status that asks only for the overlay builds neither the scene nor
    /// its tree.
    pub async fn update(
        &mut self,
        event: &WindowEvent,
        instant: Instant,
    ) -> Result<Changes, AvengerAppError> {
        let update_status = self
            .event_stream_manager
            .dispatch_event(event, &self.rtree, instant)
            .await;
        self.set_alarm();
        let mut changes = Changes::default();
        if update_status.rerender || update_status.rebuild_geometry {
            self.build_scene().await?;
            if update_status.rebuild_geometry {
                self.rebuild_tree().await?;
            }
            changes.scene = Some(self.scene_graph.clone());
        }
        if changes.scene.is_some() || update_status.overlay {
            changes.overlay = self.build_overlay();
        }
        self.changed_views(&mut changes);
        Ok(changes)
    }

    pub fn scene_graph(&self) -> &SceneGraph {
        &self.scene_graph
    }

    /// The marks drawn over the scene, as the overlay builders last built them.
    pub fn overlay(&self) -> &SceneGraph {
        &self.overlay
    }

    /// The cursor that the state and the overlays last called for, or the default cursor
    /// without either.
    pub fn cursor(&self) -> CursorStyle {
        self.cursor
    }

    /// The text that copying last took from the overlays, empty for none.
    pub fn copy_text(&self) -> &str {
        &self.copy_text
    }

    /// The engine that measures the app's text, to draw its scenes with.
    pub fn text_engine(&self) -> &LabelEngine {
        &self.text_engine
    }
}

/// The hit-testing tree of `scene`, whose instances `data` gives rows.
fn tree(
    scene: &SceneGraph,
    data: &SceneData,
    text_engine: &LabelEngine,
) -> Result<SceneGraphRTree, AvengerAppError> {
    SceneGraphRTree::from_scene_graph_with_data(scene, data, text_engine)
        .map_err(|e| AvengerAppError::InternalError(format!("scene data: {e}")))
}

/// An overlay of `marks` over `scene`, at the scene's size.
fn overlay(scene: &SceneGraph, marks: Vec<SceneMark>) -> SceneGraph {
    SceneGraph {
        marks,
        width: scene.width,
        height: scene.height,
        origin: scene.origin,
    }
}
