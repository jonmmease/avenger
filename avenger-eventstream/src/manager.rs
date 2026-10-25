use crate::scene::{
    ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent,
    SceneFileChangedEvent, SceneGraphEvent, SceneGraphEventType, SceneKeyPressEvent,
    SceneKeyReleaseEvent, SceneMouseCancelEvent, SceneMouseDownEvent, SceneMouseEnterEvent,
    SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::stream::{EventStream, EventStreamConfig, Rebuild};
use crate::window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput};
use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::query::{Query, QueryShape, Relation};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

/// A handler's view of its scene's hit-testing tree and the trees of the scenes below it.
/// Dereferences to the handler's own tree.
#[derive(Clone, Copy)]
pub struct SceneTrees<'a> {
    trees: &'a [SceneGraphRTree],
    scene: usize,
}

impl<'a> SceneTrees<'a> {
    /// Create a handler's view of `trees` at `scene`. Trees are ordered from bottom to top.
    /// Panics if `scene` is out of bounds.
    pub fn new(trees: &'a [SceneGraphRTree], scene: usize) -> Self {
        assert!(scene < trees.len(), "scene {scene} of {}", trees.len());
        Self { trees, scene }
    }

    /// The handler's scene index, counted from the bottom.
    pub fn scene(&self) -> usize {
        self.scene
    }

    /// Hit-testing trees below the handler's scene, ordered from bottom to top.
    pub fn below(&self) -> &'a [SceneGraphRTree] {
        &self.trees[..self.scene]
    }
}

impl Deref for SceneTrees<'_> {
    type Target = SceneGraphRTree;

    fn deref(&self) -> &SceneGraphRTree {
        &self.trees[self.scene]
    }
}

/// Handles stream events using mutable state borrowed from the caller.
#[async_trait]
pub trait EventStreamHandler<State: Send + Sync + 'static>: Send + Sync {
    /// Return whether the handler accepts an event that matches the stream configuration.
    /// The stream handles only accepted events, and a consuming stream consumes only those.
    /// The handler can use state and scene trees to decide. Accepts every event by default.
    fn accepts(&self, _event: &SceneGraphEvent, _state: &State, _trees: &SceneTrees<'_>) -> bool {
        true
    }

    /// Handle `event` and return the scenes to rebuild.
    /// `trees` provides the hit-testing trees for this scene and the scenes below it.
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        trees: &SceneTrees<'_>,
    ) -> Rebuild;
}

/// Dispatches events to scenes from top to bottom, and within each scene to its
/// streams in registration order. Consumption stops further delivery, except that
/// the scene owning a press still receives its pointer events. Without a press, an
/// interactive hit stops pointer events at that scene. Key, window, and file events
/// continue until consumed.
///
/// The lowest scene reached by a press owns it. Later pointer events are offered to
/// that scene and those above it. Clicks require matching press and release targets.
/// See [`Self::dispatch_event`] for cancellation and hover behavior.
///
/// Each dispatch borrows handler state from the caller. The manager retains only
/// stream and input-tracking state.
pub struct EventStreamManager<State: Send + Sync + 'static> {
    /// Event streams and their scene indices, in registration order.
    streams: Vec<EventStream<State>>,
    /// Hovered marks indexed by scene, limited to scenes reached by the pointer.
    current_marks: Vec<Option<MarkInstance>>,
    last_click: Option<(Instant, [f32; 2])>,
    /// Time, position, button, and consecutive count of the last press.
    last_press: Option<(Instant, [f32; 2], MouseButton, u32)>,
    double_click_threshold: Duration,
    // Double-click distance threshold (e.g., 5 pixels)
    double_click_distance: f32,
    // Track current cursor position
    current_cursor_position: Option<[f32; 2]>,
    /// The press that the pointer holds, until its release.
    press: Option<Press>,
    modifiers: ModifiersState,
}

#[derive(Clone)]
struct Press {
    button: MouseButton,
    /// Press targets indexed by scene. A click requires the release target to match.
    marks: Vec<Option<MarkInstance>>,
    /// The lowest scene reached by the press, stopped by consumption, an interactive hit,
    /// or the bottom of the stack. It continues to receive pointer events even if a higher
    /// scene consumes them.
    owner: usize,
}

impl<State: Send + Sync + 'static> Default for EventStreamManager<State> {
    fn default() -> Self {
        Self::new()
    }
}

impl<State: Send + Sync + 'static> EventStreamManager<State> {
    /// Create a manager with no registered event streams.
    pub fn new() -> Self {
        Self {
            streams: Vec::new(),
            current_marks: Vec::new(),
            last_click: None,
            last_press: None,
            double_click_threshold: Duration::from_millis(500),
            double_click_distance: 5.0,
            current_cursor_position: None,
            press: None,
            modifiers: ModifiersState::default(),
        }
    }

    /// Append a handler with `config` to `scene`, indexed from the bottom.
    pub fn register_handler(
        &mut self,
        scene: usize,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        self.streams.push(EventStream::new(scene, config, handler));
    }

    fn update_modifiers(&mut self, input: &WindowKeyboardInput) {
        match (input.key, input.state) {
            (Key::Named(NamedKey::Shift), ElementState::Pressed) => self.modifiers.shift = true,
            (Key::Named(NamedKey::Shift), ElementState::Released) => self.modifiers.shift = false,
            (Key::Named(NamedKey::Control), ElementState::Pressed) => self.modifiers.control = true,
            (Key::Named(NamedKey::Control), ElementState::Released) => {
                self.modifiers.control = false
            }
            (Key::Named(NamedKey::Alt), ElementState::Pressed) => self.modifiers.alt = true,
            (Key::Named(NamedKey::Alt), ElementState::Released) => self.modifiers.alt = false,
            (Key::Named(NamedKey::Super), ElementState::Pressed) => self.modifiers.meta = true,
            (Key::Named(NamedKey::Super), ElementState::Released) => self.modifiers.meta = false,
            _ => {}
        }
    }

    /// Get all file paths that are watched by registered event streams
    pub fn get_watched_files(&self) -> Vec<PathBuf> {
        self.streams
            .iter()
            .flat_map(|stream| {
                stream
                    .config
                    .types
                    .iter()
                    .filter_map(|t| match t {
                        SceneGraphEventType::FileChanged(file_path) => Some(file_path.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Dispatch `event` using `trees` in bottom-to-top order and merge handler rebuild requests.
    ///
    /// Focus loss during a press sends `MouseCancel` to the owner and scenes above it.
    /// A receiving stream closes an active `between` interval if its start configuration
    /// includes `MouseDown`.
    ///
    /// Each scene tracks hover using its own tree. Without a press, scenes below the
    /// topmost interactive hit have no hovered mark. During a press, only the owner and
    /// scenes above it track hovered marks.
    pub async fn dispatch_event(
        &mut self,
        event: &WindowEvent,
        state: &mut State,
        trees: &[SceneGraphRTree],
        instant: Instant,
    ) -> Rebuild {
        let mut rebuild = Rebuild::none();
        // Update modifier state based on keyboard events
        if let WindowEvent::KeyboardInput(input) = event {
            self.update_modifiers(input);
        }
        if let Some(position) = event.position() {
            self.current_cursor_position = Some(position);
        }
        let position = self.current_cursor_position;
        let marks: Vec<Option<MarkInstance>> = (trees.iter())
            .map(|tree| position.and_then(|position| mark_at(tree, position)))
            .collect();
        let modifiers = self.modifiers;
        // The pointer events during a press go to its owner and the scenes above it.
        let owner = self.press.as_ref().map(|press| press.owner);

        let scene_event = match event {
            WindowEvent::MouseInput(input) => {
                let Some(position) = position else {
                    return rebuild;
                };
                match input.state {
                    ElementState::Pressed => {
                        let count = self.press_count(position, input.button, instant);
                        let down = SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                            position,
                            button: input.button,
                            mark_instance: None,
                            modifiers,
                            count,
                        });
                        let owner = self
                            .dispatch_from_top(
                                &down,
                                &marks,
                                None,
                                None,
                                state,
                                trees,
                                instant,
                                &mut rebuild,
                            )
                            .await;
                        self.press = Some(Press {
                            button: input.button,
                            marks: marks.clone(),
                            owner,
                        });
                        None
                    }
                    ElementState::Released => {
                        let press = self.press.take();
                        if let Some(press) =
                            press.as_ref().filter(|press| press.button == input.button)
                        {
                            // Only matching press and release targets produce clicks. A release
                            // with no eligible target must not update double-click history.
                            let same: Vec<bool> = (press.marks.iter().zip(&marks))
                                .map(|(down, up)| down == up)
                                .collect();
                            if same.iter().skip(press.owner).any(|same| *same) {
                                self.click(
                                    position,
                                    input.button,
                                    &marks,
                                    &same,
                                    owner,
                                    state,
                                    trees,
                                    instant,
                                    &mut rebuild,
                                )
                                .await;
                            }
                        }
                        Some(SceneGraphEvent::MouseUp(SceneMouseUpEvent {
                            position,
                            button: input.button,
                            mark_instance: None,
                            modifiers,
                        }))
                    }
                }
            }
            WindowEvent::CursorMoved(e) => {
                self.track_marks(
                    e.position,
                    &marks,
                    owner,
                    state,
                    trees,
                    instant,
                    &mut rebuild,
                )
                .await;
                Some(SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
                    position: e.position,
                    mark_instance: None,
                    modifiers,
                }))
            }
            WindowEvent::MouseWheel(e) => position.map(|position| {
                SceneGraphEvent::MouseWheel(SceneMouseWheelEvent {
                    position,
                    delta: e.delta,
                    mark_instance: None,
                    modifiers,
                })
            }),
            WindowEvent::KeyboardInput(e) => position.map(|position| {
                if e.state == ElementState::Pressed {
                    SceneGraphEvent::KeyPress(SceneKeyPressEvent {
                        position,
                        key: e.key,
                        mark_instance: None,
                        modifiers,
                    })
                } else {
                    SceneGraphEvent::KeyRelease(SceneKeyReleaseEvent {
                        position,
                        key: e.key,
                        mark_instance: None,
                        modifiers,
                    })
                }
            }),
            // A touch moves the mark under the pointer, though it has no scene event of its own.
            WindowEvent::Touch(e) => {
                self.track_marks(
                    e.position,
                    &marks,
                    owner,
                    state,
                    trees,
                    instant,
                    &mut rebuild,
                )
                .await;
                None
            }
            WindowEvent::CursorLeft => {
                let position = position.unwrap_or_default();
                let none = vec![None; trees.len()];
                self.track_marks(position, &none, owner, state, trees, instant, &mut rebuild)
                    .await;
                Some(SceneGraphEvent::CursorLeft)
            }
            WindowEvent::WindowResize(e) => Some(SceneGraphEvent::WindowResize(e.clone())),
            WindowEvent::WindowMoved(e) => Some(SceneGraphEvent::WindowMoved(e.clone())),
            WindowEvent::WindowFocused(focused) => {
                // Focus loss may prevent delivery of the release, so cancel the active press.
                if let Some(press) = self.press.take_if(|_| !*focused) {
                    let cancel = SceneGraphEvent::MouseCancel(SceneMouseCancelEvent {
                        position: position.unwrap_or_default(),
                        button: press.button,
                        mark_instance: None,
                        modifiers,
                    });
                    self.dispatch_from_top(
                        &cancel,
                        &marks,
                        None,
                        owner,
                        state,
                        trees,
                        instant,
                        &mut rebuild,
                    )
                    .await;
                }
                Some(SceneGraphEvent::WindowFocused(*focused))
            }
            WindowEvent::WindowCloseRequested => Some(SceneGraphEvent::WindowCloseRequested),
            WindowEvent::FileChanged(e) => {
                Some(SceneGraphEvent::FileChanged(SceneFileChangedEvent {
                    file_path: e.file_path.clone(),
                    error: e.error.clone(),
                }))
            }
            _ => None,
        };
        if let Some(scene_event) = scene_event {
            self.dispatch_from_top(
                &scene_event,
                &marks,
                None,
                owner,
                state,
                trees,
                instant,
                &mut rebuild,
            )
            .await;
        }
        rebuild
    }

    /// When the earliest wake is due: the earliest time that a debounced stream's burst settles,
    /// for the host's timer. Dispatch the wakes then with
    /// [`EventStreamManager::dispatch_due_wakes`].
    pub fn next_wake(&self) -> Option<Instant> {
        self.streams
            .iter()
            .filter_map(EventStream::settles_at)
            .min()
    }

    /// Dispatch debounced events due at `now`, earliest first.
    /// Each handler receives the last event in its burst and its scene view of `trees`.
    /// Return the merged rebuild requests.
    pub async fn dispatch_due_wakes(
        &mut self,
        state: &mut State,
        trees: &[SceneGraphRTree],
        now: Instant,
    ) -> Rebuild {
        let mut due: Vec<(Instant, usize)> = self
            .streams
            .iter()
            .enumerate()
            .filter(|(_, stream)| stream.scene < trees.len())
            .filter_map(|(index, stream)| Some((stream.settles_at()?, index)))
            .filter(|(settles_at, _)| *settles_at <= now)
            .collect();
        due.sort();
        let mut rebuild = Rebuild::none();
        for (_, index) in due {
            let stream = &mut self.streams[index];
            if let Some(event) = stream.settle() {
                stream.last_handled_time = Some(now);
                let view = SceneTrees::new(trees, stream.scene);
                let named = (stream.handler).handle(&event, state, &view).await;
                rebuild.merge(named);
            }
        }
        rebuild
    }

    /// Dispatch from top to bottom with each scene's target mark and return the lowest
    /// scene reached. `only` filters delivery, but excluded scenes can still block pointer
    /// events with an interactive hit.
    ///
    /// Consumption stops delivery. Without a press, an interactive hit also stops pointer
    /// events. With `owner`, pointer events are offered down to that scene, which receives
    /// them even if a higher scene consumes them. Other events ignore `owner`.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_from_top(
        &mut self,
        event: &SceneGraphEvent,
        marks: &[Option<MarkInstance>],
        only: Option<&[bool]>,
        owner: Option<usize>,
        state: &mut State,
        trees: &[SceneGraphRTree],
        instant: Instant,
        rebuild: &mut Rebuild,
    ) -> usize {
        let owner = owner.filter(|_| event.is_pointer());
        let blocks = owner.is_none() && event.is_pointer();
        let hears = |scene: usize| only.is_none_or(|only| only[scene]);
        let mut lowest = trees.len().saturating_sub(1);
        for scene in (owner.unwrap_or(0)..trees.len()).rev() {
            lowest = scene;
            let event = event.with_mark(marks[scene].clone());
            if hears(scene)
                && self
                    .dispatch_in_scene(scene, &event, state, trees, instant, rebuild)
                    .await
            {
                // A stream above the owner that consumes the event keeps it from the scenes
                // between them, but the owner still hears it, so its gesture goes on.
                if let Some(owner) = owner.filter(|owner| *owner < scene && hears(*owner)) {
                    let event = event.with_mark(marks[owner].clone());
                    self.dispatch_in_scene(owner, &event, state, trees, instant, rebuild)
                        .await;
                    lowest = owner;
                }
                break;
            }
            if blocks && marks[scene].is_some() {
                break;
            }
        }
        lowest
    }

    /// Dispatch `event` to streams in registration order within `scene`.
    /// Return whether a stream consumed the event.
    async fn dispatch_in_scene(
        &mut self,
        scene: usize,
        event: &SceneGraphEvent,
        state: &mut State,
        trees: &[SceneGraphRTree],
        instant: Instant,
        rebuild: &mut Rebuild,
    ) -> bool {
        let view = SceneTrees::new(trees, scene);
        for stream in self
            .streams
            .iter_mut()
            .filter(|stream| stream.scene == scene)
        {
            if stream.matches_and_update(event, event.mark_instance(), instant)
                && stream.handler.accepts(event, state, &view)
            {
                // A debounced stream takes the event into its burst, and handles it later
                // unless it's a burst's first event that the stream handles at once.
                if stream.debounce(event, instant).unwrap_or(true) {
                    stream.last_handled_time = Some(instant);
                    let named = (stream.handler).handle(event, state, &view).await;
                    rebuild.merge(named);
                }
                if stream.config.consume {
                    return true;
                }
            }
        }
        false
    }

    /// Compare each scene's hovered mark with its previous mark. Send `MouseLeave`
    /// for the old mark and `MouseEnter` for the new mark when they differ.
    /// Without a press, scenes below the topmost interactive hit have no hovered mark. During a
    /// press, the owner and the scenes above it keep theirs, and the scenes below it have none.
    #[allow(clippy::too_many_arguments)]
    async fn track_marks(
        &mut self,
        position: [f32; 2],
        marks: &[Option<MarkInstance>],
        owner: Option<usize>,
        state: &mut State,
        trees: &[SceneGraphRTree],
        instant: Instant,
        rebuild: &mut Rebuild,
    ) {
        self.current_marks.resize(trees.len(), None);
        let reached =
            owner.unwrap_or_else(|| (marks.iter()).rposition(Option::is_some).unwrap_or(0));
        let modifiers = self.modifiers;
        for scene in (0..trees.len()).rev() {
            let current = if scene >= reached {
                marks[scene].clone()
            } else {
                None
            };
            if self.current_marks[scene] == current {
                continue;
            }
            if let Some(mark_instance) = self.current_marks[scene].take() {
                let leave = SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                    position,
                    mark_instance,
                    modifiers,
                });
                self.dispatch_in_scene(scene, &leave, state, trees, instant, rebuild)
                    .await;
            }
            if let Some(mark_instance) = current.clone() {
                let enter = SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                    position,
                    mark_instance,
                    modifiers,
                });
                self.dispatch_in_scene(scene, &enter, state, trees, instant, rebuild)
                    .await;
            }
            self.current_marks[scene] = current;
        }
    }

    /// Dispatch a click to scenes marked by `same`. A left release produces a double-click
    /// if it follows a nearby left click within the time threshold.
    #[allow(clippy::too_many_arguments)]
    async fn click(
        &mut self,
        position: [f32; 2],
        button: MouseButton,
        marks: &[Option<MarkInstance>],
        same: &[bool],
        owner: Option<usize>,
        state: &mut State,
        trees: &[SceneGraphRTree],
        instant: Instant,
        rebuild: &mut Rebuild,
    ) {
        let modifiers = self.modifiers;
        let double = button == MouseButton::Left
            && self.last_click.is_some_and(|(last_time, last_position)| {
                let distance =
                    (position[0] - last_position[0]).hypot(position[1] - last_position[1]);
                instant.duration_since(last_time) <= self.double_click_threshold
                    && distance <= self.double_click_distance
            });
        let event = if double {
            self.last_click = None;
            SceneGraphEvent::DoubleClick(SceneDoubleClickEvent {
                position,
                mark_instance: None,
                modifiers,
            })
        } else {
            if button == MouseButton::Left {
                self.last_click = Some((instant, position));
            }
            SceneGraphEvent::Click(SceneClickEvent {
                position,
                button,
                mark_instance: None,
                modifiers,
            })
        };
        self.dispatch_from_top(
            &event,
            marks,
            Some(same),
            owner,
            state,
            trees,
            instant,
            rebuild,
        )
        .await;
    }

    /// Count consecutive presses of `button` within the double-click time and distance
    /// thresholds. Reset to 1 when the button changes or either threshold is exceeded.
    fn press_count(&mut self, position: [f32; 2], button: MouseButton, instant: Instant) -> u32 {
        let count = match self.last_press {
            Some((time, at, pressed, count))
                if pressed == button
                    && instant.duration_since(time) <= self.double_click_threshold
                    && (position[0] - at[0]).hypot(position[1] - at[1])
                        <= self.double_click_distance =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_press = Some((instant, position, button, count));
        count
    }

    pub fn modifiers(&self) -> ModifiersState {
        self.modifiers
    }
}

/// The topmost interactive mark of `tree` at `position`.
fn mark_at(tree: &SceneGraphRTree, position: [f32; 2]) -> Option<MarkInstance> {
    let topmost = Query::new(QueryShape::Point(position), Relation::Intersects).limit(1);
    (tree.query(&topmost).first()).map(|instance| instance.mark_instance.clone())
}
