use avenger_common::time::Instant;
use std::any::{type_name, Any};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{EventStreamConfig, Level, Rebuild};
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
/// `scenes` is nonempty or `redraw` is true.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// The positions in [`AvengerApp::scenes`] of the scenes that the build set or cleared, in
    /// ascending order.
    pub scenes: Vec<usize>,
    /// Request a redraw even if no scene changed, for example when an image arrives.
    pub redraw: bool,
}

/// A scene graph, with the rows behind its marks, which the scene's hit-testing tree pairs with
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

/// Declares the app's own scenes and draws them from its state.
#[async_trait]
pub trait AppBuild<State: Send + Sync + 'static>: Send + Sync {
    /// The app's scenes, each with its name, its z-order, and the app's streams on it.
    fn scenes(&self) -> Vec<Scene<State>>;

    /// Set or clear every one of the app's scenes that `scenes` reports as requested.
    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError>;
}

/// A reusable interaction: the scenes that it declares, the streams that it attaches to its
/// scenes or the app's, its private state, and its part of each build.
#[async_trait]
pub trait Tool<State: Send + Sync + 'static>: Send + Sync + 'static {
    /// The tool's private state, which the app keeps and only the tool's handlers and update see.
    type State: Default + Send + Sync + 'static;

    /// Declare the tool's scenes, if it has any, and attach its streams.
    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>);

    /// Set or clear the tool's scenes that need it. The app calls it on every build, after the
    /// app's build and the updates of the tools whose scenes lie lower in the z-order, so a tool
    /// reads the latest graphs of the scenes below its own.
    async fn update(
        &self,
        _app: &mut State,
        _own: &mut Self::State,
        _scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        Ok(())
    }
}

/// Handles the events of a tool's stream, with the app's state and the tool's own state.
#[async_trait]
pub trait ToolHandler<State: Send + Sync + 'static, Own: Send + Sync + 'static>:
    Send + Sync
{
    /// Handle `event`, with the trees of the handler's scene and the scenes below it, and name
    /// the scenes that the handling changes.
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        app: &mut State,
        own: &mut Own,
        trees: &SceneTrees<'_>,
    ) -> Rebuild;
}

/// A tool, with the type of its state erased.
#[async_trait]
trait ErasedTool<State: Send + Sync + 'static>: Send + Sync {
    async fn update(
        &self,
        app: &mut State,
        own: &mut (dyn Any + Send + Sync),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError>;
}

#[async_trait]
impl<State: Send + Sync + 'static, T: Tool<State>> ErasedTool<State> for T {
    async fn update(
        &self,
        app: &mut State,
        own: &mut (dyn Any + Send + Sync),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let own = (own.downcast_mut::<T::State>()).expect("a tool's update receives its state");
        Tool::update(self, app, own, scenes).await
    }
}

/// What the app's handlers and its tools' handlers change, which each dispatch lends them: the
/// app's state, and each tool's own state, in the order that the app added the tools.
struct States<State> {
    app: State,
    tools: Vec<Box<dyn Any + Send + Sync>>,
}

/// An app's handler, which changes the app's state.
struct AppHandler<H>(H);

#[async_trait]
impl<State, H> EventStreamHandler<States<State>> for AppHandler<H>
where
    State: Send + Sync + 'static,
    H: EventStreamHandler<State>,
{
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        states: &mut States<State>,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        self.0.handle(event, &mut states.app, trees).await
    }
}

/// A tool's handler, which changes the app's state and the state of the tool at `tool`.
struct ToolAdapter<State, Own> {
    tool: usize,
    handler: Box<dyn ToolHandler<State, Own>>,
}

#[async_trait]
impl<State, Own> EventStreamHandler<States<State>> for ToolAdapter<State, Own>
where
    State: Send + Sync + 'static,
    Own: Send + Sync + 'static,
{
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        states: &mut States<State>,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let States { app, tools } = states;
        let own = (tools[self.tool].downcast_mut::<Own>())
            .expect("a tool's handler receives its tool's state");
        self.handler.handle(event, app, own, trees).await
    }
}

/// Who declared a scene, for messages about it and for ordering the tools' updates.
#[derive(Clone, Debug)]
enum Declarer {
    App,
    /// The tool at an index, by its type's name.
    Tool(usize, &'static str),
}

impl std::fmt::Display for Declarer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Declarer::App => f.write_str("the app"),
            Declarer::Tool(_, name) => write!(f, "the tool {name}"),
        }
    }
}

/// A scene that an app or a tool declared.
struct Declared {
    name: String,
    z: i32,
    declarer: Declarer,
}

/// Attaches a stream to the scene at an index, once the builder orders the scenes.
type Attachment<State> = Box<dyn FnOnce(&mut EventStreamManager<States<State>>, usize) + Send>;

/// The scenes that an app and its tools declared, and the streams that they attached.
struct Declarations<State: Send + Sync + 'static> {
    scenes: Vec<Declared>,
    /// The streams, each with the name of its scene, in the order that they were attached.
    streams: Vec<(String, Attachment<State>)>,
}

impl<State: Send + Sync + 'static> Declarations<State> {
    fn scene(&mut self, name: &str, z: i32, declarer: Declarer) {
        self.scenes.push(Declared {
            name: name.to_string(),
            z,
            declarer,
        });
    }

    fn stream(&mut self, scene: &str, attach: Attachment<State>) {
        self.streams.push((scene.to_string(), attach));
    }
}

/// Adds an app's tools to the scenes and streams that its build declared.
pub struct AppBuilder<State: Send + Sync + 'static> {
    states: States<State>,
    manager: EventStreamManager<States<State>>,
    build: Box<dyn AppBuild<State>>,
    tools: Vec<Box<dyn ErasedTool<State>>>,
    declarations: Declarations<State>,
    size: [f32; 2],
    text_engine: LabelEngine,
}

impl<State: Send + Sync + 'static> AppBuilder<State> {
    /// The window's size until the host reports the window's actual size, 800 by 600 by default.
    pub fn size(mut self, size: [f32; 2]) -> Self {
        self.size = size;
        self
    }

    /// Add `tool` after the tools added so far, with a new default state of its own.
    pub fn tool<T: Tool<State>>(mut self, tool: T) -> Self {
        self.states.tools.push(Box::new(T::State::default()));
        let index = self.states.tools.len() - 1;
        let mut install = ToolInstall {
            declarations: &mut self.declarations,
            tool: index,
            declarer: Declarer::Tool(index, type_name::<T>()),
            _own: PhantomData,
        };
        tool.install(&mut install);
        self.tools.push(Box::new(tool));
        self
    }

    /// The app, after its first build, which requests every scene with its tree. Fails when the
    /// app has no scene, when two scenes share a name or a z-order, when a stream names a scene
    /// that no one declared, or when the first build fails.
    pub async fn build(self) -> Result<AvengerApp<State>, AvengerAppError> {
        let Self {
            states,
            mut manager,
            build,
            tools,
            declarations:
                Declarations {
                    mut scenes,
                    streams,
                },
            size,
            text_engine,
        } = self;
        if scenes.is_empty() {
            return Err(AvengerAppError::InvalidApp(
                "an app needs a scene".to_string(),
            ));
        }
        scenes.sort_by_key(|scene| scene.z);
        for pair in scenes.windows(2) {
            if pair[0].z == pair[1].z {
                return Err(AvengerAppError::InvalidApp(format!(
                    "the scenes {:?} and {:?} share the z-order {}",
                    pair[0].name, pair[1].name, pair[0].z
                )));
            }
        }
        for (index, scene) in scenes.iter().enumerate() {
            if scenes[..index].iter().any(|other| other.name == scene.name) {
                return Err(AvengerAppError::InvalidApp(format!(
                    "two scenes are named {:?}",
                    scene.name
                )));
            }
        }
        for (name, attach) in streams {
            let Some(index) = scenes.iter().position(|scene| scene.name == name) else {
                return Err(AvengerAppError::InvalidApp(format!(
                    "a stream attaches to the undeclared scene {name:?}"
                )));
            };
            attach(&mut manager, index);
        }
        // The tools update from the lowest of their scenes up, so each reads the latest graphs
        // below its own. Tools without scenes update first, and ties keep the order of adding.
        let lowest = |tool: usize| {
            (scenes.iter())
                .find(|scene| matches!(scene.declarer, Declarer::Tool(index, _) if index == tool))
                .map_or(i32::MIN, |scene| scene.z)
        };
        let mut update_order: Vec<usize> = (0..tools.len()).collect();
        update_order.sort_by_key(|&tool| lowest(tool));
        let empty = empty_scene(size);
        let tree = SceneGraphRTree::from_scene_graph(&empty, &text_engine);
        let count = scenes.len();
        let waker = AppWaker::new();
        let mut app = AvengerApp {
            build,
            tools,
            update_order,
            states,
            manager,
            slots: (scenes.into_iter())
                .map(|scene| Slot {
                    name: scene.name,
                    declarer: scene.declarer,
                    current: true,
                })
                .collect(),
            graphs: (0..count).map(|_| Arc::new(empty_scene(size))).collect(),
            trees: vec![tree; count],
            size,
            text_engine,
            alarm: Alarm::new(waker.clone()),
            waker,
            images: None,
            #[cfg(not(target_arch = "wasm32"))]
            spawner: None,
            #[cfg(target_arch = "wasm32")]
            spawner: Some(Spawner::Browser),
            waiting_tasks: Vec::new(),
        };
        let (_, first_error) = app.run_build(vec![Some(Level::Tree); count]).await;
        match first_error {
            Some(error) => Err(error),
            None => Ok(app),
        }
    }
}

/// A stream of the app's on one of its scenes: its config, and its handler.
type AppStream<State> = (
    EventStreamConfig,
    Arc<dyn EventStreamHandler<States<State>>>,
);

/// One of the app's scenes, as [`AppBuild::scenes`] declares it: its name, its z-order, and the
/// app's streams on it. In a scene, the app's streams hear events before the tools' streams.
pub struct Scene<State: Send + Sync + 'static> {
    name: String,
    z: i32,
    streams: Vec<AppStream<State>>,
}

impl<State: Send + Sync + 'static> Scene<State> {
    /// The scene `name`, at `z` in the order that the scenes draw and hear events.
    pub fn new(name: &str, z: i32) -> Self {
        Self {
            name: name.to_string(),
            z,
            streams: Vec::new(),
        }
    }

    /// The scene, with a stream of the app's after the streams that it already has.
    pub fn stream(
        mut self,
        config: EventStreamConfig,
        handler: impl EventStreamHandler<State> + 'static,
    ) -> Self {
        self.streams.push((config, Arc::new(AppHandler(handler))));
        self
    }
}

/// Declares a tool's scenes and attaches its streams, as [`Tool::install`] does.
pub struct ToolInstall<'a, State: Send + Sync + 'static, Own> {
    declarations: &'a mut Declarations<State>,
    tool: usize,
    declarer: Declarer,
    _own: PhantomData<fn() -> Own>,
}

impl<State: Send + Sync + 'static, Own: Send + Sync + 'static> ToolInstall<'_, State, Own> {
    /// Declare the tool's scene `name`, at `z` in the order that the scenes draw and hear
    /// events.
    pub fn scene(&mut self, name: &str, z: i32) -> &mut Self {
        let declarer = self.declarer.clone();
        self.declarations.scene(name, z, declarer);
        self
    }

    /// Attach a stream of the tool's to the scene `scene`, the tool's or the app's, after the
    /// streams that the scene already has.
    pub fn stream(
        &mut self,
        scene: &str,
        config: EventStreamConfig,
        handler: impl ToolHandler<State, Own> + 'static,
    ) -> &mut Self {
        let handler = ToolAdapter {
            tool: self.tool,
            handler: Box::new(handler),
        };
        let attach = move |manager: &mut EventStreamManager<States<State>>, index: usize| {
            manager.register_handler(index, config, Arc::new(handler));
        };
        self.declarations.stream(scene, Box::new(attach));
        self
    }
}

/// What the app keeps for a scene besides its graph and tree.
struct Slot {
    name: String,
    declarer: Declarer,
    /// Whether the scene's tree shows its latest graph.
    current: bool,
}

/// The app's scenes, as a build sees them: which ones handlers requested, and each scene's
/// graph, tree, and changes. A scene that the build leaves alone keeps its graph, its tree, and
/// the GPU resources that the host prepared for it.
pub struct Scenes<'a> {
    slots: &'a mut [Slot],
    graphs: &'a mut [Arc<SceneGraph>],
    trees: &'a mut [SceneGraphRTree],
    /// The level that each scene was requested at, by scene.
    requested: &'a [Option<Level>],
    /// Whether this build set or cleared each scene, by scene.
    changed: Vec<bool>,
    size: [f32; 2],
    text_engine: &'a LabelEngine,
}

impl Scenes<'_> {
    /// The position of the scene `name`. Panics when no scene has that name.
    fn index(&self, name: &str) -> usize {
        (self.slots.iter())
            .position(|slot| slot.name == name)
            .unwrap_or_else(|| panic!("no scene is named {name:?}"))
    }

    /// Whether a handler or a trigger requested the scene `name`. Panics when no scene has that
    /// name.
    pub fn requested(&self, name: &str) -> bool {
        self.requested[self.index(name)].is_some()
    }

    /// Whether the scene `name` was requested with its tree, as for the first build, a resize, a
    /// refresh, and the end of a gesture. Panics when no scene has that name.
    pub fn requested_tree(&self, name: &str) -> bool {
        self.requested[self.index(name)] == Some(Level::Tree)
    }

    /// Replace the scene `name`'s graph and rows with `built`. The scene's tree is rebuilt at
    /// once, unless the scene was requested without its tree, as during a pan, when the tree goes
    /// stale. A scene that a build sets without a request of its own, as a tool sets its scene
    /// when a scene below it changes, gets its tree, so its marks take hits where they draw.
    /// Fails when no scene has that name.
    pub fn set(&mut self, name: &str, built: BuiltScene) -> Result<(), AvengerAppError> {
        let index = self.find(name)?;
        let with_tree = self.requested[index] != Some(Level::Scene);
        if with_tree {
            self.trees[index] = tree(&built, self.text_engine, name);
        }
        self.slots[index].current = with_tree;
        self.graphs[index] = Arc::new(built.scene);
        self.changed[index] = true;
        Ok(())
    }

    /// Remove the scene `name`'s marks and tree. Fails when no scene has that name.
    pub fn clear(&mut self, name: &str) -> Result<(), AvengerAppError> {
        let index = self.find(name)?;
        let empty = empty_scene(self.size);
        self.trees[index] = SceneGraphRTree::from_scene_graph(&empty, self.text_engine);
        self.slots[index].current = true;
        self.graphs[index] = Arc::new(empty);
        self.changed[index] = true;
        Ok(())
    }

    fn find(&self, name: &str) -> Result<usize, AvengerAppError> {
        (self.slots.iter())
            .position(|slot| slot.name == name)
            .ok_or_else(|| AvengerAppError::InvalidApp(format!("no scene is named {name:?}")))
    }

    /// The scene `name`'s hit-testing tree as it stands, including a tree that `set` rebuilt
    /// earlier in this build. Panics when no scene has that name.
    pub fn tree(&self, name: &str) -> &SceneGraphRTree {
        &self.trees[self.index(name)]
    }

    /// Whether the scene `name`'s tree shows its latest graph. Panics when no scene has that
    /// name.
    pub fn is_current(&self, name: &str) -> bool {
        self.slots[self.index(name)].current
    }

    /// Whether this build set or cleared the scene `name`. Panics when no scene has that name.
    pub fn changed(&self, name: &str) -> bool {
        self.changed[self.index(name)]
    }

    /// The scenes under the scene `name` in z-order, from the top down. Panics when no scene has
    /// that name.
    pub fn below(&self, name: &str) -> impl Iterator<Item = SceneView<'_>> + '_ {
        let index = self.index(name);
        (0..index).rev().map(|below| SceneView {
            scenes: self,
            index: below,
        })
    }

    /// The window's size.
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// The engine that measures the app's text.
    pub fn text_engine(&self) -> &LabelEngine {
        self.text_engine
    }
}

/// A scene that [`Scenes::below`] gives.
#[derive(Clone, Copy)]
pub struct SceneView<'a> {
    scenes: &'a Scenes<'a>,
    index: usize,
}

impl<'a> SceneView<'a> {
    /// The scene's latest graph.
    pub fn graph(&self) -> &'a SceneGraph {
        &self.scenes.graphs[self.index]
    }

    /// The scene's hit-testing tree, as it stands.
    pub fn tree(&self) -> &'a SceneGraphRTree {
        &self.scenes.trees[self.index]
    }

    /// Whether the scene's tree shows its latest graph.
    pub fn is_current(&self) -> bool {
        self.scenes.slots[self.index].current
    }

    /// Whether this build set or cleared the scene.
    pub fn changed(&self) -> bool {
        self.scenes.changed[self.index]
    }
}

/// An app: its state, its build, its tools, and the named scenes that they draw, from the bottom
/// of the z-order up.
pub struct AvengerApp<State: Send + Sync + 'static> {
    build: Box<dyn AppBuild<State>>,
    tools: Vec<Box<dyn ErasedTool<State>>>,
    /// The tools by their indices, in the order that they update.
    update_order: Vec<usize>,
    states: States<State>,
    /// The streams, which each dispatch lends `states`.
    manager: EventStreamManager<States<State>>,
    /// What the app keeps for each scene, by scene.
    slots: Vec<Slot>,
    /// Each scene's latest graph, by scene.
    graphs: Vec<Arc<SceneGraph>>,
    /// Each scene's hit-testing tree, by scene.
    trees: Vec<SceneGraphRTree>,
    /// The window's size.
    size: [f32; 2],
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

impl<State: Send + Sync + 'static> AvengerApp<State> {
    /// A builder of an app with `state`, whose scenes `build` declares and draws, with
    /// `text_engine`.
    pub fn builder(
        state: State,
        build: impl AppBuild<State> + 'static,
        text_engine: LabelEngine,
    ) -> AppBuilder<State> {
        let mut declarations = Declarations {
            scenes: Vec::new(),
            streams: Vec::new(),
        };
        for scene in build.scenes() {
            declarations.scene(&scene.name, scene.z, Declarer::App);
            for (config, handler) in scene.streams {
                let attach = move |manager: &mut EventStreamManager<States<State>>,
                                   index: usize| {
                    manager.register_handler(index, config, handler);
                };
                declarations.stream(&scene.name, Box::new(attach));
            }
        }
        AppBuilder {
            states: States {
                app: state,
                tools: Vec::new(),
            },
            manager: EventStreamManager::new(),
            build: Box::new(build),
            tools: Vec::new(),
            declarations,
            size: [800.0, 600.0],
            text_engine,
        }
    }

    pub fn state(&self) -> &State {
        &self.states.app
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.states.app
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
    /// redraw requests. A pending `Refresh::Rebuild` requests every scene with its tree.
    pub async fn refresh(&mut self) -> Changes {
        let pending = self.waker.take();
        let rebuild = (self.manager)
            .dispatch_due_wakes(&mut self.states, &self.trees, Instant::now())
            .await;
        self.set_alarm();
        let mut requested = vec![None; self.slots.len()];
        if pending == Some(Refresh::Rebuild) {
            requested.fill(Some(Level::Tree));
        }
        self.add_requests(&rebuild, &mut requested);
        Changes {
            scenes: self.run_build(requested).await.0,
            redraw: pending == Some(Refresh::Redraw),
        }
    }

    /// Set the alarm for the next wake that the event streams wait for.
    fn set_alarm(&self) {
        if let Some(at) = self.manager.next_wake() {
            self.alarm.set(at);
        }
    }

    /// Add the scenes that `rebuild` names to `requested`, keeping the higher level for each,
    /// and log the names of scenes that no one declared.
    fn add_requests(&self, rebuild: &Rebuild, requested: &mut [Option<Level>]) {
        for (name, level) in rebuild.iter() {
            match self.slots.iter().position(|slot| slot.name == name) {
                Some(index) => requested[index] = requested[index].max(Some(level)),
                None => eprintln!("A handler requested the undeclared scene {name:?}"),
            }
        }
    }

    /// Run the app's build and then each tool's update, when anything is `requested`, and
    /// return the scenes that they set or cleared, with the first error of a build or an update.
    /// A build or an update that fails logs its error, and the scenes that it set before the
    /// error keep their changes. A build that requests every scene with its tree clears the
    /// pending refresh first, since its scenes show the state as it is, and a request during
    /// the build raises it again.
    async fn run_build(
        &mut self,
        requested: Vec<Option<Level>>,
    ) -> (Vec<usize>, Option<AvengerAppError>) {
        if requested.iter().all(Option::is_none) {
            return (Vec::new(), None);
        }
        if requested.iter().all(|level| *level == Some(Level::Tree)) {
            self.waker.take();
        }
        let States {
            app: state,
            tools: tool_states,
        } = &mut self.states;
        let mut scenes = Scenes {
            slots: &mut self.slots,
            graphs: &mut self.graphs,
            trees: &mut self.trees,
            requested: &requested,
            changed: vec![false; requested.len()],
            size: self.size,
            text_engine: &self.text_engine,
        };
        let mut first_error = None;
        if let Err(error) = self.build.build(state, &mut scenes).await {
            eprintln!("The app's build failed: {error}");
            first_error.get_or_insert(error);
        }
        for &tool in &self.update_order {
            let own = tool_states[tool].as_mut();
            if let Err(error) = self.tools[tool].update(state, own, &mut scenes).await {
                eprintln!("A tool's update failed: {error}");
                first_error.get_or_insert(error);
            }
        }
        for (index, slot) in scenes.slots.iter().enumerate() {
            if requested[index].is_some() && !scenes.changed[index] {
                eprintln!(
                    "The scene {:?} was requested, but {} neither set nor cleared it",
                    slot.name, slot.declarer
                );
            }
        }
        let changed = (scenes.changed.iter().enumerate())
            .filter_map(|(index, changed)| changed.then_some(index))
            .collect();
        (changed, first_error)
    }

    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.manager.get_watched_files()
    }

    /// Update the app with `event`, and run a build for the scenes that its handlers requested.
    /// A resize to a nonzero size, in both directions, also requests every scene with its tree.
    pub async fn update(&mut self, event: &WindowEvent, instant: Instant) -> Changes {
        let mut requested = vec![None; self.slots.len()];
        if let WindowEvent::WindowResize(resize) = event {
            if resize.size.iter().all(|&length| length >= 1.0) {
                self.size = resize.size;
                requested.fill(Some(Level::Tree));
            }
        }
        let rebuild = (self.manager)
            .dispatch_event(event, &mut self.states, &self.trees, instant)
            .await;
        self.set_alarm();
        self.add_requests(&rebuild, &mut requested);
        Changes {
            scenes: self.run_build(requested).await.0,
            redraw: false,
        }
    }

    /// Each scene's latest graph, from the bottom of the z-order up. A scene that is cleared or
    /// never set has an empty graph.
    pub fn scenes(&self) -> &[Arc<SceneGraph>] {
        &self.graphs
    }

    /// The window's size.
    pub fn size(&self) -> [f32; 2] {
        self.size
    }

    /// The engine that measures the app's text, to draw its scenes with.
    pub fn text_engine(&self) -> &LabelEngine {
        &self.text_engine
    }
}

/// A scene with no marks, at the window's `size`.
fn empty_scene(size: [f32; 2]) -> SceneGraph {
    SceneGraph {
        marks: Vec::new(),
        width: size[0],
        height: size[1],
        origin: [0.0; 2],
    }
}

/// The hit-testing tree of `built`, paired with the build's rows. Rows that don't match the
/// scene are logged and left out, so the tree still takes hits.
fn tree(built: &BuiltScene, text_engine: &LabelEngine, name: &str) -> SceneGraphRTree {
    SceneGraphRTree::from_scene_graph_with_data(&built.scene, &built.data, text_engine)
        .unwrap_or_else(|e| {
            eprintln!("Rows of the scene {name:?} don't match its graph: {e}");
            SceneGraphRTree::from_scene_graph(&built.scene, text_engine)
        })
}
