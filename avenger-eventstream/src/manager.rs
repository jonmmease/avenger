use crate::scene::{
    ModifiersState, SceneClickEvent, SceneCursorMovedEvent, SceneDoubleClickEvent,
    SceneFileChangedEvent, SceneGraphEvent, SceneGraphEventType, SceneKeyPressEvent,
    SceneKeyReleaseEvent, SceneMouseCancelEvent, SceneMouseDownEvent, SceneMouseEnterEvent,
    SceneMouseLeaveEvent, SceneMouseUpEvent, SceneMouseWheelEvent,
};
use crate::stream::{EventStream, EventStreamConfig, UpdateStatus};
use crate::window::{ElementState, Key, MouseButton, NamedKey, WindowEvent, WindowKeyboardInput};
use async_trait::async_trait;
use avenger_common::time::{Duration, Instant};
use avenger_geometry::query::{Query, QueryShape, Relation};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::mark::MarkInstance;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

/// A handler's view of its layer's hit-testing tree and the trees below it.
/// Dereferences to the handler's own tree.
#[derive(Clone, Copy)]
pub struct LayerTrees<'a> {
    trees: &'a [SceneGraphRTree],
    layer: usize,
}

impl<'a> LayerTrees<'a> {
    /// Create a handler's view of `trees` at `layer`. Trees are ordered from bottom to top.
    /// Panics if `layer` is out of bounds.
    pub fn new(trees: &'a [SceneGraphRTree], layer: usize) -> Self {
        assert!(layer < trees.len(), "layer {layer} of {}", trees.len());
        Self { trees, layer }
    }

    /// The handler's own layer, counted from the bottom.
    pub fn layer(&self) -> usize {
        self.layer
    }

    /// The trees of the layers below the handler's own, from the bottom.
    pub fn below(&self) -> &'a [SceneGraphRTree] {
        &self.trees[..self.layer]
    }
}

impl Deref for LayerTrees<'_> {
    type Target = SceneGraphRTree;

    fn deref(&self) -> &SceneGraphRTree {
        &self.trees[self.layer]
    }
}

#[async_trait]
pub trait EventStreamHandler<State: Clone + Send + Sync + 'static>: Send + Sync {
    /// Handle `event`, with the trees of the handler's layer and the layers below it.
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut State,
        rtree: &LayerTrees<'_>,
    ) -> UpdateStatus;
}

/// Dispatches events from top to bottom and to each layer's streams in registration order.
/// Consumption stops further delivery, except that a press's owner still hears the press's
/// pointer events. Without a press, an interactive hit stops pointer events at that layer. Key,
/// window, and file events continue until consumed.
///
/// A press belongs to the lowest layer it reaches, its owner. Subsequent pointer events are
/// offered to that layer and those above it. Consumption in an upper layer cannot block delivery
/// to the owner. Clicks still require matching press and release targets.
/// See [`Self::dispatch_event`] for cancellation and hover behavior.
#[derive(Clone)]
pub struct EventStreamManager<State: Clone + Send + Sync + 'static> {
    state: State,
    /// The streams, each in its layer, in the order that they were added.
    streams: Vec<EventStream<State>>,
    /// The mark under the pointer in each layer, by layer, where the pointer reaches the layer.
    current_marks: Vec<Option<MarkInstance>>,
    last_click: Option<(Instant, [f32; 2])>,
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
    /// The mark under the press in each layer, by layer, which a click needs it to end on.
    marks: Vec<Option<MarkInstance>>,
    /// The layer that owns the press: the lowest layer that heard it, where a stream consumed
    /// it, where an interactive mark of the layer's lay under it, or else the bottom layer. The
    /// press's moves, release, and clicks reach the layers from the top down to the owner, and
    /// always the owner.
    owner: usize,
}

impl<State: Clone + Send + Sync + 'static> EventStreamManager<State> {
    pub fn new(state: State) -> Self {
        Self {
            state,
            streams: Vec::new(),
            current_marks: Vec::new(),
            last_click: None,
            double_click_threshold: Duration::from_millis(500),
            double_click_distance: 5.0,
            current_cursor_position: None,
            press: None,
            modifiers: ModifiersState::default(),
        }
    }

    /// Register a handler with `config` in `layer`, counted from the bottom, after the streams
    /// that the layer already has.
    pub fn register_handler(
        &mut self,
        layer: usize,
        config: EventStreamConfig,
        handler: Arc<dyn EventStreamHandler<State>>,
    ) {
        self.streams.push(EventStream::new(layer, config, handler));
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
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

    /// Dispatch `event` to the layers whose hit-testing `trees` come from the bottom, and return
    /// each layer's update status, by layer.
    ///
    /// Focus loss during a press sends `MouseCancel` to the press's owner and the layers above
    /// it. A stream that receives it closes an active `between` interval when its start
    /// configuration includes `MouseDown`. Hover tracking uses each layer's tree. Without a
    /// press, layers below the topmost interactive hit have no hovered mark. During a press, the
    /// owner and the layers above it keep theirs, and the layers below the owner have none.
    pub async fn dispatch_event(
        &mut self,
        event: &WindowEvent,
        trees: &[SceneGraphRTree],
        instant: Instant,
    ) -> Vec<UpdateStatus> {
        let mut statuses = vec![UpdateStatus::default(); trees.len()];
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
        // The pointer events during a press go to its owner and the layers above it.
        let owner = self.press.as_ref().map(|press| press.owner);

        let scene_event = match event {
            WindowEvent::MouseInput(input) => {
                let Some(position) = position else {
                    return statuses;
                };
                match input.state {
                    ElementState::Pressed => {
                        let down = SceneGraphEvent::MouseDown(SceneMouseDownEvent {
                            position,
                            button: input.button,
                            mark_instance: None,
                            modifiers,
                        });
                        let owner = self
                            .dispatch_from_top(
                                &down,
                                &marks,
                                None,
                                None,
                                trees,
                                instant,
                                &mut statuses,
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
                                    trees,
                                    instant,
                                    &mut statuses,
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
                self.track_marks(e.position, &marks, owner, trees, instant, &mut statuses)
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
                self.track_marks(e.position, &marks, owner, trees, instant, &mut statuses)
                    .await;
                None
            }
            WindowEvent::CursorLeft => {
                let position = position.unwrap_or_default();
                let none = vec![None; trees.len()];
                self.track_marks(position, &none, owner, trees, instant, &mut statuses)
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
                        trees,
                        instant,
                        &mut statuses,
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
                trees,
                instant,
                &mut statuses,
            )
            .await;
        }
        statuses
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

    /// Run the wakes that are due at `now`, earliest first: each debounced stream whose burst
    /// settled handles the burst's last event, in its own layer of `trees`. Returns each layer's
    /// update status, by layer.
    pub async fn dispatch_due_wakes(
        &mut self,
        trees: &[SceneGraphRTree],
        now: Instant,
    ) -> Vec<UpdateStatus> {
        let mut due: Vec<(Instant, usize)> = self
            .streams
            .iter()
            .enumerate()
            .filter(|(_, stream)| stream.layer < trees.len())
            .filter_map(|(index, stream)| Some((stream.settles_at()?, index)))
            .filter(|(settles_at, _)| *settles_at <= now)
            .collect();
        due.sort();
        let mut statuses = vec![UpdateStatus::default(); trees.len()];
        for (_, index) in due {
            let stream = &mut self.streams[index];
            if let Some(event) = stream.settle() {
                stream.last_handled_time = Some(now);
                let layer = stream.layer;
                let status = (stream.handler)
                    .handle(&event, &mut self.state, &LayerTrees::new(trees, layer))
                    .await;
                statuses[layer] = statuses[layer].merge(&status);
            }
        }
        statuses
    }

    /// Dispatch `event` to the layers from the top, each with the mark under it in that layer,
    /// and return the lowest layer that heard it. A layer that `only` leaves out doesn't hear it,
    /// but a mark of the layer's under the event still stops it. The event stops at a stream that
    /// consumes it, and a pointer event stops at a layer whose interactive mark lies under it.
    /// With the `owner` of a press, a pointer event reaches the layers from the top down to the
    /// owner, wherever it lies, and the owner hears it whatever the layers above it consume.
    /// Other events ignore `owner`.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_from_top(
        &mut self,
        event: &SceneGraphEvent,
        marks: &[Option<MarkInstance>],
        only: Option<&[bool]>,
        owner: Option<usize>,
        trees: &[SceneGraphRTree],
        instant: Instant,
        statuses: &mut [UpdateStatus],
    ) -> usize {
        let owner = owner.filter(|_| event.is_pointer());
        let blocks = owner.is_none() && event.is_pointer();
        let hears = |layer: usize| only.is_none_or(|only| only[layer]);
        let mut lowest = trees.len().saturating_sub(1);
        for layer in (owner.unwrap_or(0)..trees.len()).rev() {
            lowest = layer;
            let event = event.with_mark(marks[layer].clone());
            if hears(layer)
                && self
                    .dispatch_in_layer(layer, &event, trees, instant, &mut statuses[layer])
                    .await
            {
                // A stream above the owner that consumes the event keeps it from the layers
                // between them, but the owner still hears it, so its gesture goes on.
                if let Some(owner) = owner.filter(|owner| *owner < layer && hears(*owner)) {
                    let event = event.with_mark(marks[owner].clone());
                    self.dispatch_in_layer(owner, &event, trees, instant, &mut statuses[owner])
                        .await;
                    lowest = owner;
                }
                break;
            }
            if blocks && marks[layer].is_some() {
                break;
            }
        }
        lowest
    }

    /// Dispatch `event` to `layer`'s streams, in the order that they were added, and return
    /// whether one of them consumed it.
    async fn dispatch_in_layer(
        &mut self,
        layer: usize,
        event: &SceneGraphEvent,
        trees: &[SceneGraphRTree],
        instant: Instant,
        status: &mut UpdateStatus,
    ) -> bool {
        let view = LayerTrees::new(trees, layer);
        for stream in self
            .streams
            .iter_mut()
            .filter(|stream| stream.layer == layer)
        {
            if stream.matches_and_update(event, event.mark_instance(), instant) {
                // A debounced stream takes the event into its burst, and handles it later
                // unless it's a burst's first event that the stream handles at once.
                if stream.debounce(event, instant).unwrap_or(true) {
                    stream.last_handled_time = Some(instant);
                    *status =
                        status.merge(&stream.handler.handle(event, &mut self.state, &view).await);
                }
                if stream.config.consume {
                    return true;
                }
            }
        }
        false
    }

    /// Compare each layer's hovered mark with its previous mark. Send `MouseLeave`
    /// for the old mark and `MouseEnter` for the new mark when they differ.
    /// Without a press, layers below the topmost interactive hit have no hovered mark. During a
    /// press, the owner and the layers above it keep theirs, and the layers below it have none.
    async fn track_marks(
        &mut self,
        position: [f32; 2],
        marks: &[Option<MarkInstance>],
        owner: Option<usize>,
        trees: &[SceneGraphRTree],
        instant: Instant,
        statuses: &mut [UpdateStatus],
    ) {
        self.current_marks.resize(trees.len(), None);
        let reached =
            owner.unwrap_or_else(|| (marks.iter()).rposition(Option::is_some).unwrap_or(0));
        let modifiers = self.modifiers;
        for layer in (0..trees.len()).rev() {
            let current = if layer >= reached {
                marks[layer].clone()
            } else {
                None
            };
            if self.current_marks[layer] == current {
                continue;
            }
            if let Some(mark_instance) = self.current_marks[layer].take() {
                let leave = SceneGraphEvent::MouseLeave(SceneMouseLeaveEvent {
                    position,
                    mark_instance,
                    modifiers,
                });
                self.dispatch_in_layer(layer, &leave, trees, instant, &mut statuses[layer])
                    .await;
            }
            if let Some(mark_instance) = current.clone() {
                let enter = SceneGraphEvent::MouseEnter(SceneMouseEnterEvent {
                    position,
                    mark_instance,
                    modifiers,
                });
                self.dispatch_in_layer(layer, &enter, trees, instant, &mut statuses[layer])
                    .await;
            }
            self.current_marks[layer] = current;
        }
    }

    /// Dispatch the click that a release of `button` at `position` ends, or the double click
    /// when it follows a left click near it soon enough, to the layers that `same` marks.
    #[allow(clippy::too_many_arguments)]
    async fn click(
        &mut self,
        position: [f32; 2],
        button: MouseButton,
        marks: &[Option<MarkInstance>],
        same: &[bool],
        owner: Option<usize>,
        trees: &[SceneGraphRTree],
        instant: Instant,
        statuses: &mut [UpdateStatus],
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
        self.dispatch_from_top(&event, marks, Some(same), owner, trees, instant, statuses)
            .await;
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
