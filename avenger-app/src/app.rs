use avenger_common::time::Instant;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::WindowEvent;
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_image::{ImageCache, ImageResolver};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::alarm::Alarm;
use crate::error::AvengerAppError;
use crate::task::{Attach, Runner, Spawner, Task};
use crate::waker::{AppWaker, Refresh};

/// Scene updates and redraw requests returned by [`AvengerApp::update`] and
/// [`AvengerApp::refresh`]. Install the changed scenes, then draw a frame if
/// `layers` is nonempty or `redraw` is true.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Indices of changed scenes in [`AvengerApp::scenes`], in ascending order.
    pub layers: Vec<usize>,
    /// Request a redraw even if no layer changed, for example when an image arrives.
    pub redraw: bool,
}

/// Lower layers' scenes and hit-testing trees, plus the app's text engine.
pub struct BuildContext<'a> {
    scenes: &'a [Arc<SceneGraph>],
    trees: &'a [SceneGraphRTree],
    current: &'a [bool],
    text_engine: &'a LabelEngine,
}

impl<'a> BuildContext<'a> {
    /// The scenes of the layers below, from the bottom.
    pub fn scenes_below(&self) -> &'a [Arc<SceneGraph>] {
        self.scenes
    }

    /// The hit-testing trees of the layers below, from the bottom.
    pub fn trees_below(&self) -> &'a [SceneGraphRTree] {
        self.trees
    }

    /// Whether the hit-testing tree of `layer`, one of the layers below, shows that layer's
    /// latest scene. A layer whose scene rebuilds without its tree, as during a pan, has a stale
    /// tree until the tree builds again, and a layer above shows nothing that a stale tree
    /// places. Panics when `layer` isn't below the layer that builds.
    pub fn tree_is_current(&self, layer: usize) -> bool {
        self.current[layer]
    }

    /// The engine that measures the app's text.
    pub fn text_engine(&self) -> &'a LabelEngine {
        self.text_engine
    }
}

/// A layer's scene, with the rows behind its marks, which the layer's hit-testing tree pairs with
/// the marks' instances.
pub struct BuiltScene {
    pub scene: SceneGraph,
    /// The rows behind the scene's marks, by mark name.
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

/// A layer of an app: a scene graph that the app draws over the layers below it, with its own
/// z-index stack, its own hit-testing tree, and its own event streams. A layer rebuilds without
/// the layers below it, and every rebuild of a layer rebuilds the layers above it.
#[async_trait]
pub trait Layer<State: Clone + Send + Sync + 'static> {
    /// Build this layer's scene, with the rows behind its marks, from app state and lower
    /// layers, using the window's size and origin.
    async fn build(
        &self,
        state: &mut State,
        context: &BuildContext<'_>,
    ) -> Result<BuiltScene, AvengerAppError>;

    /// Return this layer's event streams in dispatch order. A handler's rebuild request applies
    /// to this layer and all layers above it.
    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        Vec::new()
    }
}

#[derive(Clone)]
pub struct AvengerApp<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// The app's layers, from the bottom.
    layers: Vec<Arc<dyn Layer<State>>>,
    event_stream_manager: EventStreamManager<State>,
    /// Each layer's latest scene, by layer.
    scenes: Vec<Arc<SceneGraph>>,
    /// Each layer's hit-testing tree, by layer.
    trees: Vec<SceneGraphRTree>,
    /// Whether each layer's tree shows its latest scene, by layer.
    current: Vec<bool>,
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

    /// Create an app with `initial_state` and layers ordered from bottom to top.
    /// Register each layer's streams and build its scene and hit-testing tree in that order.
    /// Return an error if `layers` is empty or a layer fails to build.
    pub async fn try_new(
        initial_state: State,
        layers: Vec<Arc<dyn Layer<State>>>,
        text_engine: LabelEngine,
    ) -> Result<Self, AvengerAppError> {
        if layers.is_empty() {
            return Err(AvengerAppError::InternalError(
                "an app needs at least one layer".to_string(),
            ));
        }
        let mut event_stream_manager = EventStreamManager::new(initial_state);
        for (index, layer) in layers.iter().enumerate() {
            for (config, handler) in layer.streams() {
                event_stream_manager.register_handler(index, config, handler);
            }
        }
        let mut scenes = Vec::with_capacity(layers.len());
        let mut trees = Vec::with_capacity(layers.len());
        let current = vec![true; layers.len()];
        for (index, layer) in layers.iter().enumerate() {
            let context = BuildContext {
                scenes: &scenes,
                trees: &trees,
                current: &current[..index],
                text_engine: &text_engine,
            };
            let built = layer
                .build(event_stream_manager.state_mut(), &context)
                .await?;
            trees.push(tree(&built, &text_engine, index));
            scenes.push(Arc::new(built.scene));
        }
        let waker = AppWaker::new();

        Ok(Self {
            layers,
            event_stream_manager,
            scenes,
            trees,
            current,
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

    /// Consume the pending refresh, run the wakes that are due, and return scene updates and
    /// redraw requests.
    /// A pending `Refresh::Rebuild` rebuilds every layer and tree.
    pub async fn refresh(&mut self) -> Changes {
        let pending = self.waker.take();
        let statuses = self
            .event_stream_manager
            .dispatch_due_wakes(&self.trees, Instant::now())
            .await;
        self.set_alarm();
        let all = pending == Some(Refresh::Rebuild);
        let from = if all {
            Some(0)
        } else {
            lowest_rebuild(&statuses)
        };
        Changes {
            layers: self.rebuild(from, all, &statuses).await,
            redraw: pending == Some(Refresh::Redraw),
        }
    }

    /// Set the alarm for the next wake that the event streams wait for.
    fn set_alarm(&self) {
        if let Some(at) = self.event_stream_manager.next_wake() {
            self.alarm.set(at);
        }
    }

    /// Build the layers from `from` up again, and return them. A layer builds its tree again
    /// when its status in `statuses` asks for it, when `all` rebuilds every layer and tree, or
    /// when a layer below it rebuilt, and otherwise keeps a tree that no longer shows its scene. A layer whose build fails logs its error and keeps its
    /// previous scene, and the layers above it still build. A build of every layer clears the
    /// pending refresh first, since the new scenes show the state as it is, and a request during
    /// the build raises it again.
    async fn rebuild(
        &mut self,
        from: Option<usize>,
        all: bool,
        statuses: &[UpdateStatus],
    ) -> Vec<usize> {
        let Some(from) = from else {
            return Vec::new();
        };
        if from == 0 {
            self.waker.take();
        }
        let mut rebuilt = Vec::new();
        for index in from..self.layers.len() {
            let context = BuildContext {
                scenes: &self.scenes[..index],
                trees: &self.trees[..index],
                current: &self.current[..index],
                text_engine: &self.text_engine,
            };
            let built = match self.layers[index]
                .build(self.event_stream_manager.state_mut(), &context)
                .await
            {
                Ok(built) => built,
                Err(e) => {
                    eprintln!("Failed to build layer {index}: {e:?}");
                    continue;
                }
            };
            let own = statuses
                .get(index)
                .is_some_and(|status| status.rebuild_geometry);
            let builds_tree = all || index > from || own;
            if builds_tree {
                self.trees[index] = tree(&built, &self.text_engine, index);
            }
            self.current[index] = builds_tree;
            self.scenes[index] = Arc::new(built.scene);
            rebuilt.push(index);
        }
        rebuilt
    }

    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.event_stream_manager.get_watched_files()
    }

    /// Update the state of the app with `event`, and build again the lowest layer whose status
    /// asks for it and every layer above it.
    pub async fn update(&mut self, event: &WindowEvent, instant: Instant) -> Changes {
        let statuses = self
            .event_stream_manager
            .dispatch_event(event, &self.trees, instant)
            .await;
        self.set_alarm();
        Changes {
            layers: self
                .rebuild(lowest_rebuild(&statuses), false, &statuses)
                .await,
            redraw: false,
        }
    }

    /// Each layer's latest scene, from the bottom.
    pub fn scenes(&self) -> &[Arc<SceneGraph>] {
        &self.scenes
    }

    /// The engine that measures the app's text, to draw its scenes with.
    pub fn text_engine(&self) -> &LabelEngine {
        &self.text_engine
    }
}

/// The hit-testing tree of the scene that layer `index` built, paired with the build's rows. Rows
/// that don't match the scene are logged and left out, so the tree still takes hits.
fn tree(built: &BuiltScene, text_engine: &LabelEngine, index: usize) -> SceneGraphRTree {
    SceneGraphRTree::from_scene_graph_with_data(&built.scene, &built.data, text_engine)
        .unwrap_or_else(|e| {
            eprintln!("Rows of layer {index} don't match its scene: {e}");
            SceneGraphRTree::from_scene_graph(&built.scene, text_engine)
        })
}

/// The lowest layer whose status asks for a new scene or tree.
fn lowest_rebuild(statuses: &[UpdateStatus]) -> Option<usize> {
    (statuses.iter()).position(|status| status.rerender || status.rebuild_geometry)
}
