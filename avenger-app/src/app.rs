use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use std::any::{type_name, Any};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use avenger_eventstream::manager::{EventStreamHandler, EventStreamManager, SceneTrees};
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, EventStreamConfig, Level, Rebuild};
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

/// Scene updates, redraw requests, and host outputs returned by [`AvengerApp::update`] and
/// [`AvengerApp::refresh`]. Install the changed scenes, then draw a frame if `scenes` is
/// nonempty or `redraw` is true.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// Indices of scenes set or cleared by the build, in ascending order.
    /// These index [`AvengerApp::scenes`].
    pub scenes: Vec<usize>,
    /// Request a redraw even if no scene changed, for example when an image arrives.
    pub redraw: bool,
    /// Updated cursor, or `None` if unchanged. See [`AvengerApp::cursor`].
    pub cursor: Option<CursorIcon>,
    /// Updated text for the copy shortcut, or `None` if unchanged. See
    /// [`AvengerApp::copy_text`].
    pub copy_text: Option<String>,
    /// The focused text entry's updated caret area, or `None` if unchanged. `Some(None)` means
    /// that no text entry has focus any more. See [`AvengerApp::text_entry`].
    pub text_entry: Option<Option<[f32; 4]>>,
}

/// A scene graph and the data rows associated with its mark instances.
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
    /// Declare the app's scenes with their names, z-order, and event streams.
    fn scenes(&self) -> Vec<Scene<State>>;

    /// Set or clear each requested scene owned by the app.
    async fn build(
        &self,
        state: &mut State,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError>;

    /// Return the app's cursor preference, used when no tool requests a cursor.
    /// Defaults to `None`. See [`AvengerApp::cursor`].
    fn cursor(&self, _state: &State) -> Option<CursorIcon> {
        None
    }
}

/// A reusable interaction with private state, event streams, and optional scenes.
#[async_trait]
pub trait Tool<State: Send + Sync + 'static>: Send + Sync + 'static {
    /// Private state owned by the app and passed to this tool's handlers and update method.
    type State: Default + Send + Sync + 'static;

    /// Declare the tool's scenes, if it has any, and attach its streams.
    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>);

    /// Set or clear the tool's scenes that need it. The app calls every tool's update on every
    /// build, after [`AppBuild::build`], and each tool decides from `Scenes` whether it has
    /// anything to do. The update reads the app state but can't change it: handlers change the
    /// state before the requested scenes build, and updates draw from the result.
    ///
    /// Tools update in order of each tool's lowest scene z-order. Tools without scenes use
    /// `i32::MIN`, and ties keep registration order. Tools with interleaved scenes must account
    /// for this order when reading other scenes.
    async fn update(
        &self,
        _app: &State,
        _own: &mut Self::State,
        _scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        Ok(())
    }

    /// Return the tool's cursor preference, or `None` to defer to other tools and the app.
    /// Defaults to `None`. See [`AvengerApp::cursor`].
    fn cursor(&self, _app: &State, _own: &Self::State) -> Option<CursorIcon> {
        None
    }

    /// Return text for the host's copy shortcut, such as selected text, or `None` to defer to
    /// other tools. Defaults to `None`. See [`AvengerApp::copy_text`].
    fn copy_text<'a>(&'a self, _app: &'a State, _own: &'a Self::State) -> Option<&'a str> {
        None
    }

    /// Return the caret area of a text entry that the tool draws while it has keyboard focus,
    /// such as a text input's, or `None` to defer to other tools. Defaults to `None`. See
    /// [`AvengerApp::text_entry`].
    fn text_entry(&self, _app: &State, _own: &Self::State) -> Option<[f32; 4]> {
        None
    }
}

/// Handles a tool's events with mutable access to app state and private tool state.
#[async_trait]
pub trait ToolHandler<State: Send + Sync + 'static, Own: Send + Sync + 'static>:
    Send + Sync
{
    /// Decide what the stream does with `event`, which its configuration matches, from the app
    /// state and the tool's state, as [`EventStreamHandler::disposition`] does for an app's
    /// stream. Handles every event by default.
    fn disposition(
        &self,
        _event: &SceneGraphEvent,
        _app: &State,
        _own: &Own,
        _trees: &SceneTrees<'_>,
    ) -> Disposition {
        Disposition::Handle
    }

    /// Handle `event` and return the scenes to rebuild.
    /// `trees` provides the hit-testing trees for this scene and the scenes below it.
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
        app: &State,
        own: &mut (dyn Any + Send + Sync),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError>;

    fn cursor(&self, app: &State, own: &(dyn Any + Send + Sync)) -> Option<CursorIcon>;

    fn copy_text<'a>(&'a self, app: &'a State, own: &'a (dyn Any + Send + Sync))
        -> Option<&'a str>;

    fn text_entry(&self, app: &State, own: &(dyn Any + Send + Sync)) -> Option<[f32; 4]>;
}

#[async_trait]
impl<State: Send + Sync + 'static, T: Tool<State>> ErasedTool<State> for T {
    async fn update(
        &self,
        app: &State,
        own: &mut (dyn Any + Send + Sync),
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let own = (own.downcast_mut::<T::State>()).expect("a tool's update receives its state");
        Tool::update(self, app, own, scenes).await
    }

    fn cursor(&self, app: &State, own: &(dyn Any + Send + Sync)) -> Option<CursorIcon> {
        let own = (own.downcast_ref::<T::State>()).expect("a tool's cursor receives its state");
        Tool::cursor(self, app, own)
    }

    fn copy_text<'a>(
        &'a self,
        app: &'a State,
        own: &'a (dyn Any + Send + Sync),
    ) -> Option<&'a str> {
        let own = (own.downcast_ref::<T::State>()).expect("a tool's copy text receives its state");
        Tool::copy_text(self, app, own)
    }

    fn text_entry(&self, app: &State, own: &(dyn Any + Send + Sync)) -> Option<[f32; 4]> {
        let own = (own.downcast_ref::<T::State>()).expect("a tool's text entry receives its state");
        Tool::text_entry(self, app, own)
    }
}

/// App state and private tool states borrowed by event handlers.
/// Tool states are stored in registration order.
struct States<State> {
    app: State,
    tools: Vec<Box<dyn Any + Send + Sync>>,
}

/// Adapts an app handler to the combined app and tool state.
struct AppHandler<H>(H);

#[async_trait]
impl<State, H> EventStreamHandler<States<State>> for AppHandler<H>
where
    State: Send + Sync + 'static,
    H: EventStreamHandler<State>,
{
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        states: &States<State>,
        trees: &SceneTrees<'_>,
    ) -> Disposition {
        self.0.disposition(event, &states.app, trees)
    }

    async fn handle(
        &self,
        event: &SceneGraphEvent,
        states: &mut States<State>,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        self.0.handle(event, &mut states.app, trees).await
    }
}

/// Adapts a tool handler to app state and the private state at `tool`.
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
    fn disposition(
        &self,
        event: &SceneGraphEvent,
        states: &States<State>,
        trees: &SceneTrees<'_>,
    ) -> Disposition {
        let own = (states.tools[self.tool].downcast_ref::<Own>())
            .expect("a tool's handler receives its tool's state");
        self.handler.disposition(event, &states.app, own, trees)
    }

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

/// Scene ownership used for diagnostics and tool update ordering.
#[derive(Clone, Debug)]
enum Declarer {
    App,
    /// The tool index and type name.
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

/// Attaches a stream after scene indices are assigned.
type Attachment<State> = Box<dyn FnOnce(&mut EventStreamManager<States<State>>, usize) + Send>;

/// Scene declarations and pending stream attachments.
struct Declarations<State: Send + Sync + 'static> {
    scenes: Vec<Declared>,
    /// Stream attachments in registration order, keyed by scene name.
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

/// Configures tools and window size for an app with declared scenes.
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
    /// Set the initial window size, in logical pixels. Defaults to 800 by 600.
    /// The host can replace it with the actual window size.
    pub fn size(mut self, size: [f32; 2]) -> Self {
        self.size = size;
        self
    }

    /// Register `tool` with a new default state of its own.
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

    /// Build the app and all scene graphs and hit-testing trees.
    ///
    /// Fails if there are no scenes, a name or z-order is duplicated, a stream names an
    /// undeclared scene, or the initial build fails.
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
        // Order tools by their lowest scene. Tools without scenes use the minimum z-order.
        // Stable sorting preserves registration order for ties.
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
            cursor: CursorIcon::Default,
            copy_text: String::new(),
            text_entry: None,
        };
        let (_, first_error) = app.run_build(vec![Some(Level::Tree); count]).await;
        if let Some(error) = first_error {
            return Err(error);
        }
        app.report_outputs(&mut Changes::default());
        Ok(app)
    }
}

/// An app event stream: its configuration and handler.
type AppStream<State> = (
    EventStreamConfig,
    Arc<dyn EventStreamHandler<States<State>>>,
);

/// A named scene declared by [`AppBuild::scenes`], with a z-order and app event streams.
/// App streams receive events before tool streams attached to the same scene.
pub struct Scene<State: Send + Sync + 'static> {
    name: String,
    z: i32,
    streams: Vec<AppStream<State>>,
}

impl<State: Send + Sync + 'static> Scene<State> {
    /// Create a scene named `name` at z-order `z`.
    /// Scenes draw from bottom to top and receive events from top to bottom.
    pub fn new(name: &str, z: i32) -> Self {
        Self {
            name: name.to_string(),
            z,
            streams: Vec::new(),
        }
    }

    /// Append an app event stream to this scene.
    pub fn stream(
        mut self,
        config: EventStreamConfig,
        handler: impl EventStreamHandler<State> + 'static,
    ) -> Self {
        self.streams.push((config, Arc::new(AppHandler(handler))));
        self
    }
}

/// Registers scenes and event streams during [`Tool::install`].
pub struct ToolInstall<'a, State: Send + Sync + 'static, Own> {
    declarations: &'a mut Declarations<State>,
    tool: usize,
    declarer: Declarer,
    _own: PhantomData<fn() -> Own>,
}

impl<State: Send + Sync + 'static, Own: Send + Sync + 'static> ToolInstall<'_, State, Own> {
    /// Declare a scene named `name` at z-order `z`.
    /// The name and z-order must be unique within the app.
    pub fn scene(&mut self, name: &str, z: i32) -> &mut Self {
        let declarer = self.declarer.clone();
        self.declarations.scene(name, z, declarer);
        self
    }

    /// Append a tool event stream to the declared scene `scene`.
    /// The scene can belong to the app or a tool.
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

/// Scene identity and ownership.
struct Slot {
    name: String,
    declarer: Declarer,
}

/// Scene access during a build, including requests, graphs, trees, and changes.
/// Scenes that are not replaced retain their graphs, trees, and prepared GPU resources.
pub struct Scenes<'a> {
    slots: &'a mut [Slot],
    graphs: &'a mut [Arc<SceneGraph>],
    trees: &'a mut [SceneGraphRTree],
    /// Requested rebuild level for each scene.
    requested: &'a [Option<Level>],
    /// Tracks scenes set or cleared during this build.
    changed: Vec<bool>,
    size: [f32; 2],
    text_engine: &'a LabelEngine,
}

impl Scenes<'_> {
    /// Return the index of `name`. Panics if the scene is undeclared.
    fn index(&self, name: &str) -> usize {
        (self.slots.iter())
            .position(|slot| slot.name == name)
            .unwrap_or_else(|| panic!("no scene is named {name:?}"))
    }

    /// Return whether `name` was requested for this build.
    /// Panics if the scene is undeclared.
    pub fn requested(&self, name: &str) -> bool {
        self.requested[self.index(name)].is_some()
    }

    /// Return whether `name` was requested at [`Level::Tree`].
    /// This includes initial builds, nonzero resizes, full rebuilds, and gesture settling.
    /// Panics if the scene is undeclared.
    pub fn requested_tree(&self, name: &str) -> bool {
        self.requested[self.index(name)] == Some(Level::Tree)
    }

    /// Replace `name` with `built` and rebuild its hit-testing tree immediately.
    ///
    /// An explicit [`Level::Scene`] request preserves the old tree and marks it stale.
    /// Scenes set without a request also get a new tree, so dependent scenes have current hits.
    /// Returns an error if the scene is undeclared.
    pub fn set(&mut self, name: &str, built: BuiltScene) -> Result<(), AvengerAppError> {
        let index = self.find(name)?;
        if self.requested[index] == Some(Level::Scene) {
            self.trees[index].mark_stale();
        } else {
            self.trees[index] = tree(&built, self.text_engine, name);
        }
        self.graphs[index] = Arc::new(built.scene);
        self.changed[index] = true;
        Ok(())
    }

    /// Clear the marks and hit-testing tree of `name`.
    /// Returns an error if the scene is undeclared.
    pub fn clear(&mut self, name: &str) -> Result<(), AvengerAppError> {
        let index = self.find(name)?;
        let empty = empty_scene(self.size);
        self.trees[index] = SceneGraphRTree::from_scene_graph(&empty, self.text_engine);
        self.graphs[index] = Arc::new(empty);
        self.changed[index] = true;
        Ok(())
    }

    fn find(&self, name: &str) -> Result<usize, AvengerAppError> {
        (self.slots.iter())
            .position(|slot| slot.name == name)
            .ok_or_else(|| AvengerAppError::InvalidApp(format!("no scene is named {name:?}")))
    }

    /// Return the stored hit-testing tree for `name`, including any update made in this build.
    /// The tree may be stale. Panics if the scene is undeclared.
    pub fn tree(&self, name: &str) -> &SceneGraphRTree {
        &self.trees[self.index(name)]
    }

    /// Return whether the hit-testing tree for `name` matches its latest graph.
    /// Panics if the scene is undeclared.
    pub fn is_current(&self, name: &str) -> bool {
        self.trees[self.index(name)].is_current()
    }

    /// Return whether this build set or cleared `name`.
    /// Panics if the scene is undeclared.
    pub fn changed(&self, name: &str) -> bool {
        self.changed[self.index(name)]
    }

    /// Iterate over scenes below `name`, from highest to lowest z-order.
    /// Panics if the scene is undeclared.
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

/// Read access to a scene returned by [`Scenes::below`].
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

    /// The stored hit-testing tree, which may be stale.
    pub fn tree(&self) -> &'a SceneGraphRTree {
        &self.scenes.trees[self.index]
    }

    /// Whether the scene's tree shows its latest graph.
    pub fn is_current(&self) -> bool {
        self.scenes.trees[self.index].is_current()
    }

    /// Whether this build set or cleared the scene.
    pub fn changed(&self) -> bool {
        self.scenes.changed[self.index]
    }
}

/// An interactive app with shared app state, tools, and named scenes in z-order.
pub struct AvengerApp<State: Send + Sync + 'static> {
    build: Box<dyn AppBuild<State>>,
    tools: Vec<Box<dyn ErasedTool<State>>>,
    /// Tool indices in update order.
    update_order: Vec<usize>,
    states: States<State>,
    /// Event streams that borrow `states` during dispatch.
    manager: EventStreamManager<States<State>>,
    /// Scene metadata in z-order.
    slots: Vec<Slot>,
    /// Scene graphs in z-order.
    graphs: Vec<Arc<SceneGraph>>,
    /// Hit-testing trees in z-order.
    trees: Vec<SceneGraphRTree>,
    /// The window's size.
    size: [f32; 2],
    text_engine: LabelEngine,
    waker: AppWaker,
    /// Wakes the host when the next debounced event is due.
    alarm: Alarm,
    images: Option<Arc<dyn ImageResolver>>,
    /// Runs background jobs on the attached native runtime or the browser runtime.
    spawner: Option<Spawner>,
    /// Tasks waiting for a native runtime.
    waiting_tasks: Vec<Arc<dyn Attach>>,
    /// Last reported cursor.
    cursor: CursorIcon,
    /// Last reported text for the copy shortcut.
    copy_text: String,
    /// Last reported caret area of the focused text entry.
    text_entry: Option<[f32; 4]>,
}

impl<State: Send + Sync + 'static> AvengerApp<State> {
    /// Create a builder with app state, scene declarations, and a text engine.
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

    /// Read the app state.
    pub fn state(&self) -> &State {
        &self.states.app
    }

    /// Mutate the app state. To show the change, request [`Refresh::Rebuild`] on the app's
    /// waker, which rebuilds every scene with its tree.
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

    /// Process the pending refresh and due event-stream wakeups.
    /// Returns scene updates and redraw requests. [`Refresh::Rebuild`] requests every
    /// scene with its tree.
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
        let mut changes = Changes {
            scenes: self.run_build(requested).await.0,
            redraw: pending == Some(Refresh::Redraw),
            ..Default::default()
        };
        self.report_outputs(&mut changes);
        changes
    }

    /// Set the alarm for the next wake that the event streams wait for.
    fn set_alarm(&self) {
        if let Some(at) = self.manager.next_wake() {
            self.alarm.set(at);
        }
    }

    /// Merge rebuild requests, keeping the higher level for each scene.
    /// Log requests for undeclared scenes.
    fn add_requests(&self, rebuild: &Rebuild, requested: &mut [Option<Level>]) {
        for (name, level) in rebuild.iter() {
            match self.slots.iter().position(|slot| slot.name == name) {
                Some(index) => requested[index] = requested[index].max(Some(level)),
                None => eprintln!("A handler requested the undeclared scene {name:?}"),
            }
        }
    }

    /// Run the app build and tool updates if any scene was requested.
    /// Return the changed scene indices and the first build or update error. Errors are
    /// logged, and changes made before an error are retained. A full rebuild clears the
    /// pending refresh before it starts, so requests made during the build remain pending.
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
            if let Err(error) = self.tools[tool].update(&*state, own, &mut scenes).await {
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

    /// Return the file paths watched by registered event streams.
    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.manager.get_watched_files()
    }

    /// Dispatch `event` and rebuild the scenes requested by its handlers.
    /// A resize with both dimensions at least one logical pixel also requests every
    /// scene with its tree.
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
        let mut changes = Changes {
            scenes: self.run_build(requested).await.0,
            ..Default::default()
        };
        self.report_outputs(&mut changes);
        changes
    }

    /// Resolve cursor, copy-text and text entry preferences and report changed values in
    /// `changes`.
    fn report_outputs(&mut self, changes: &mut Changes) {
        let States { app, tools: owns } = &self.states;
        // Later updates take priority, using the same ordering as Tool::update.
        let mut answers = self
            .update_order
            .iter()
            .rev()
            .map(|&tool| (&self.tools[tool], &*owns[tool]));
        let cursor = (answers.clone())
            .find_map(|(tool, own)| tool.cursor(app, own))
            .or_else(|| self.build.cursor(app))
            .unwrap_or_default();
        let entry = (answers.clone()).find_map(|(tool, own)| {
            let caret = tool.text_entry(app, own)?;
            Some((tool, own, caret))
        });
        let text_entry = entry.map(|(_, _, caret)| caret);
        // A focused text entry owns copy and cut, even without a selection, as focus does in
        // browsers.
        let copy_text = match entry {
            Some((tool, own, _)) => tool.copy_text(app, own),
            None => answers.find_map(|(tool, own)| tool.copy_text(app, own)),
        }
        .unwrap_or_default();
        if cursor != self.cursor {
            self.cursor = cursor;
            changes.cursor = Some(cursor);
        }
        // The text is copied only when it changes, since the app asks after every update.
        if copy_text != self.copy_text {
            self.copy_text = copy_text.to_string();
            changes.copy_text = Some(self.copy_text.clone());
        }
        if text_entry != self.text_entry {
            self.text_entry = text_entry;
            changes.text_entry = Some(text_entry);
        }
    }

    /// Return the current cursor for the canvas.
    /// Tools are queried in reverse update order, as defined by [`Tool::update`]. The first
    /// `Some` value wins. If no tool requests a cursor, use [`AppBuild::cursor`], then
    /// [`CursorIcon::Default`].
    pub fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    /// Return the current text for the host's copy shortcut.
    /// While a text entry has focus, the tool that draws it provides the text, as the text entry
    /// owns copy and cut. Otherwise, tools are queried in reverse update order, as defined by
    /// [`Tool::update`], and the first `Some` value wins. Return an empty string if no tool
    /// provides text.
    pub fn copy_text(&self) -> &str {
        &self.copy_text
    }

    /// Return the caret area of the text entry that has keyboard focus, as `[x, y, width,
    /// height]` in window coordinates, or `None` when no text entry has focus. While a text
    /// entry has focus, the host lets input methods compose text, such as accented letters from
    /// dead keys, and places their candidate windows beside the caret. It sends their text as
    /// [`WindowEvent::Composing`](avenger_eventstream::window::WindowEvent::Composing) and
    /// [`WindowEvent::Composed`](avenger_eventstream::window::WindowEvent::Composed). Tools are
    /// queried in reverse update order, as defined by [`Tool::update`]. The first `Some` value
    /// wins.
    pub fn text_entry(&self) -> Option<[f32; 4]> {
        self.text_entry
    }

    /// Return scene graphs in ascending z-order. Cleared or unset scenes have empty graphs.
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

/// Create an empty scene at the window size.
fn empty_scene(size: [f32; 2]) -> SceneGraph {
    SceneGraph {
        marks: Vec::new(),
        width: size[0],
        height: size[1],
        origin: [0.0; 2],
    }
}

/// Build a hit-testing tree with the rows in `built`.
/// If the rows do not match the marks, log the error and build the tree without rows.
fn tree(built: &BuiltScene, text_engine: &LabelEngine, name: &str) -> SceneGraphRTree {
    SceneGraphRTree::from_scene_graph_with_data(&built.scene, &built.data, text_engine)
        .unwrap_or_else(|e| {
            eprintln!("Rows of the scene {name:?} don't match its graph: {e}");
            SceneGraphRTree::from_scene_graph(&built.scene, text_engine)
        })
}
