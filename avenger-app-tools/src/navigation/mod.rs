//! Navigation tools for continuous scales (`scales` feature) and map projections
//! (`geo` feature), and [`Navigation`], which moves the views of any [`ViewModel`], such as the
//! rotation of a polar plot. Each supports these gestures within its configured areas:
//!
//! - A left-button drag pans from the view at the press, keeping the same location
//!   under the pointer, once the pointer moves more than 4 pixels from the press, so a click
//!   doesn't pan. Moves are throttled to one every 8 ms.
//! - The wheel zooms around the pointer by a factor of 1.2 per notch, capped at two
//!   notches per event. One line or 100 pixels of scroll movement counts as a notch.
//! - Pan and zoom updates request the scene without its tree. Each gesture tracks its
//!   pending changes and requests a tree rebuild when it settles. Drags settle on pause,
//!   release, or cancellation. Wheel zoom settles when scrolling stops. The default
//!   pause delay is 100 ms. Pointer movement alone does not settle wheel zoom.
//! - A double-click restores the home view and requests the scene with its tree.
//!
//! Drags start only inside the configured area. Wheel events and double-clicks apply
//! only while the pointer is inside. Tools with separate areas can share a scene.
//! Navigation tools do not consume events.
//!
//! Each tool requests an open-hand cursor over its area and a closed hand during a drag.

#[cfg(feature = "geo")]
mod map_pan_zoom;
#[cfg(feature = "scales")]
mod pan_zoom;

#[cfg(feature = "geo")]
pub use map_pan_zoom::{KeyedMapPanZoom, MapPanZoom};
#[cfg(feature = "scales")]
pub use pan_zoom::{KeyedPanZoom, PanZoom};

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_layout::Rect;

use avenger_app::app::{Tool, ToolHandler, ToolInstall};

use crate::CLICK_DISTANCE;

/// Default idle time before a drag or wheel gesture settles.
const SETTLE_DELAY: Duration = Duration::from_millis(100);

/// The state of a navigation tool: the active drag, the pending pan and zoom changes, whether the
/// pointer is over an area, and the view of each area at its first gesture, used as the default
/// home view.
pub struct NavigationState<K, V> {
    drag: Option<Drag<K, V>>,
    /// The area whose drag has panned since its view last settled.
    dragged: Option<K>,
    /// The areas whose views the wheel has zoomed since they last settled.
    zoomed: Vec<K>,
    first: Vec<(K, V)>,
    /// Whether the pointer is over an area.
    over: bool,
}

impl<K, V> Default for NavigationState<K, V> {
    fn default() -> Self {
        Self {
            drag: None,
            dragged: None,
            zoomed: Vec::new(),
            first: Vec::new(),
            over: false,
        }
    }
}

impl<K: PartialEq, V> NavigationState<K, V> {
    /// Return a closed-hand cursor during a drag, an open hand over an area, or `None`.
    fn cursor(&self) -> Option<CursorIcon> {
        if self.drag.is_some() {
            Some(CursorIcon::Grabbing)
        } else {
            self.over.then_some(CursorIcon::Grab)
        }
    }

    /// The view of `key` at its first gesture.
    fn first(&self, key: &K) -> Option<&V> {
        (self.first.iter())
            .find(|(first, _)| first == key)
            .map(|(_, view)| view)
    }

    /// Keep `view` as the first view of `key`, unless `key` has one.
    fn keep_first(&mut self, key: &K, view: impl FnOnce() -> Option<V>)
    where
        K: Clone,
    {
        if self.first(key).is_none() {
            if let Some(view) = view() {
                self.first.push((key.clone(), view));
            }
        }
    }
}

/// The area, the view, and the pointer position at press time, plus the last processed pointer
/// position.
struct Drag<K, V> {
    key: K,
    start: V,
    from: [f32; 2],
    at: [f32; 2],
    /// Whether the pointer has moved farther from the press than a click allows.
    far: bool,
}

/// The views that a navigation moves, one for each of a set of keyed areas, and how a drag, the
/// wheel, and a double click move them. The scales of `KeyedPanZoom` and the projections of
/// `KeyedMapPanZoom` are view models, and a [`Navigation`] moves the views of any other, such as
/// the rotation of a polar plot or the camera of a 3D plot, with the same gestures.
pub trait ViewModel<State>: Send + Sync + 'static {
    /// The key of an area.
    type Key: Clone + PartialEq + Send + Sync + 'static;

    /// A snapshot of the view, at a drag's press and at an area's first gesture.
    type View: Clone + Send + Sync + 'static;

    /// The areas of the scene where the tool's gestures apply, each with its key.
    fn areas(&self, app: &State) -> Vec<(Self::Key, Rect)>;

    /// Return the current view of `key`, or `None` if unavailable.
    fn view(&self, app: &mut State, key: &Self::Key) -> Option<Self::View>;

    /// Pan `start` so the location at `from` moves to `to` within `area`.
    /// Return whether the operation updated the view.
    fn pan(
        &self,
        app: &mut State,
        key: &Self::Key,
        start: &Self::View,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool;

    /// Zoom the view of `key` around `at` within `area` by `notches` wheel notches.
    /// Return whether the operation updated the view.
    fn zoom(
        &self,
        app: &mut State,
        key: &Self::Key,
        area: Rect,
        at: [f32; 2],
        notches: f64,
    ) -> bool;

    /// Restore the home view of `key`, using its first gesture snapshot if needed.
    /// Return whether a home view was applied.
    fn home(&self, app: &mut State, key: &Self::Key, first: Option<&Self::View>) -> bool;

    /// Report that the view of `key` settled after a gesture changed it, and return the scenes
    /// to rebuild besides the tool's scene with its tree. The default reports nothing.
    fn settled(&self, _app: &mut State, _key: &Self::Key) -> Rebuild {
        Rebuild::none()
    }
}

/// The first area that contains `position`.
fn area_at<K>(areas: Vec<(K, Rect)>, position: [f32; 2]) -> Option<(K, Rect)> {
    areas.into_iter().find(|(_, area)| area.contains(position))
}

/// The area of `key`.
fn area_of<K: PartialEq>(areas: Vec<(K, Rect)>, key: &K) -> Option<Rect> {
    (areas.into_iter())
        .find(|(own, _)| own == key)
        .map(|(_, rect)| rect)
}

/// Convert wheel notches to a scale multiplier.
fn notch_factor(notches: f64) -> f64 {
    1.2_f64.powf(notches)
}

/// Convert scroll movement to notches: one line or 100 pixels per notch, clamped to `[-2, 2]`.
fn notches(delta: MouseScrollDelta) -> f64 {
    let notches = match delta {
        MouseScrollDelta::LineDelta(x, y) => f64::from(x + y),
        MouseScrollDelta::PixelDelta(x, y) => (x + y) / 100.0,
    };
    notches.clamp(-2.0, 2.0)
}

/// Moves the views of a [`ViewModel`] within `scene` with the gestures of the
/// [module documentation](self).
pub struct Navigation<M> {
    scene: String,
    model: Arc<M>,
    settle_delay: Duration,
}

impl<M> Navigation<M> {
    /// Create a tool that moves the views of `model` within `scene`.
    pub fn new(scene: &str, model: M) -> Self {
        Self {
            scene: scene.to_string(),
            model: Arc::new(model),
            settle_delay: SETTLE_DELAY,
        }
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(mut self, delay: Duration) -> Self {
        self.settle_delay = delay;
        self
    }
}

impl<State, M> Tool<State> for Navigation<M>
where
    State: Send + Sync + 'static,
    M: ViewModel<State>,
{
    type State = NavigationState<M::Key, M::View>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        install(tool, &self.scene, self.settle_delay, self.model.clone());
    }

    fn cursor(&self, _app: &State, own: &Self::State) -> Option<CursorIcon> {
        own.cursor()
    }
}

/// Attach navigation event streams to `scene`.
fn install<State, N>(
    tool: &mut ToolInstall<'_, State, NavigationState<N::Key, N::View>>,
    scene: &str,
    settle_delay: Duration,
    view: Arc<N>,
) where
    State: Send + Sync + 'static,
    N: ViewModel<State>,
{
    let on = |types: &[SceneGraphEventType]| EventStreamConfig {
        types: types.to_vec(),
        ..Default::default()
    };
    let settle = || Some(DebounceConfig::new(settle_delay.as_millis() as u64));
    let moved = [SceneGraphEventType::CursorMoved];
    let wheel = [SceneGraphEventType::MouseWheel];
    let streams = [
        (
            on(&[
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ]),
            Gesture::Hover,
        ),
        (on(&[SceneGraphEventType::MouseDown]), Gesture::Press),
        (
            EventStreamConfig {
                // Limit pan updates to one every 8 ms.
                throttle: Some(8),
                ..on(&moved)
            },
            Gesture::Move,
        ),
        (
            EventStreamConfig {
                debounce: settle(),
                ..on(&moved)
            },
            Gesture::Pause,
        ),
        (on(&[SceneGraphEventType::MouseUp]), Gesture::Release),
        (on(&[SceneGraphEventType::MouseCancel]), Gesture::Cancel),
        (on(&wheel), Gesture::Wheel),
        (
            EventStreamConfig {
                debounce: settle(),
                ..on(&wheel)
            },
            Gesture::WheelStop,
        ),
        (on(&[SceneGraphEventType::DoubleClick]), Gesture::Home),
    ];
    for (config, gesture) in streams {
        let handler = Gestures {
            gesture,
            scene: scene.to_string(),
            view: view.clone(),
        };
        tool.stream(scene, config, handler);
    }
}

/// Actions handled by navigation event streams.
#[derive(Clone, Copy)]
enum Gesture {
    /// Track whether the pointer is over an area to choose the cursor.
    Hover,
    /// A press in an area starts a drag.
    Press,
    /// A move pans the drag's view.
    Move,
    /// Apply the last drag position and request its tree after a pause.
    /// The throttle may have skipped the last move.
    Pause,
    /// Apply the release position and end the drag.
    Release,
    /// Cancel the drag at its last processed position.
    Cancel,
    /// The wheel zooms the view.
    Wheel,
    /// Request the tree after wheel zoom settles.
    WheelStop,
    /// A double-click restores the view's home.
    Home,
}

struct Gestures<N> {
    gesture: Gesture,
    scene: String,
    view: Arc<N>,
}

impl<N> Gestures<N> {
    /// Pan the active drag to `to` if the pointer position changed.
    fn pan<State>(
        &self,
        app: &mut State,
        own: &mut NavigationState<N::Key, N::View>,
        to: [f32; 2],
    ) -> Rebuild
    where
        N: ViewModel<State>,
    {
        let Some(drag) = &mut own.drag else {
            return Rebuild::none();
        };
        drag.far |= (to[0] - drag.from[0]).hypot(to[1] - drag.from[1]) > CLICK_DISTANCE;
        if drag.at == to || !drag.far {
            return Rebuild::none();
        }
        drag.at = to;
        // An area that left the app, as in a swap of its content, pans nothing.
        let Some(area) = area_of(self.view.areas(app), &drag.key) else {
            return Rebuild::none();
        };
        if !self
            .view
            .pan(app, &drag.key, &drag.start, area, drag.from, to)
        {
            return Rebuild::none();
        }
        own.dragged = Some(drag.key.clone());
        Rebuild::scene(&self.scene)
    }

    /// Request the scene and tree, and report each settled view, for the areas whose views
    /// changed since they last settled.
    fn rest<State>(&self, app: &mut State, keys: Vec<N::Key>) -> Rebuild
    where
        N: ViewModel<State>,
    {
        if keys.is_empty() {
            return Rebuild::none();
        }
        let mut rebuild = Rebuild::tree(&self.scene);
        for key in &keys {
            rebuild.merge(self.view.settled(app, key));
        }
        rebuild
    }
}

#[async_trait]
impl<State, N> ToolHandler<State, NavigationState<N::Key, N::View>> for Gestures<N>
where
    State: Send + Sync + 'static,
    N: ViewModel<State>,
{
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        app: &mut State,
        own: &mut NavigationState<N::Key, N::View>,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        match (self.gesture, event) {
            (Gesture::Hover, SceneGraphEvent::CursorMoved(moved)) => {
                own.over = area_at(self.view.areas(app), moved.position).is_some();
                Rebuild::none()
            }
            (Gesture::Hover, _) => {
                own.over = false;
                Rebuild::none()
            }
            (Gesture::Press, SceneGraphEvent::MouseDown(press)) => {
                if press.button != MouseButton::Left {
                    return Rebuild::none();
                }
                let Some((key, _)) = area_at(self.view.areas(app), press.position) else {
                    return Rebuild::none();
                };
                own.drag = self.view.view(app, &key).map(|start| Drag {
                    key: key.clone(),
                    start,
                    from: press.position,
                    at: press.position,
                    far: false,
                });
                let start = own.drag.as_ref().map(|drag| drag.start.clone());
                own.keep_first(&key, || start);
                Rebuild::none()
            }
            (Gesture::Move, SceneGraphEvent::CursorMoved(moved)) => {
                self.pan(app, own, moved.position)
            }
            (Gesture::Pause, SceneGraphEvent::CursorMoved(moved)) => {
                self.pan(app, own, moved.position);
                let dragged = own.dragged.take().into_iter().collect();
                self.rest(app, dragged)
            }
            (Gesture::Release, SceneGraphEvent::MouseUp(release)) => {
                if release.button != MouseButton::Left || own.drag.is_none() {
                    return Rebuild::none();
                }
                self.pan(app, own, release.position);
                own.drag = None;
                let dragged = own.dragged.take().into_iter().collect();
                self.rest(app, dragged)
            }
            (Gesture::Cancel, _) => {
                if own.drag.take().is_none() {
                    return Rebuild::none();
                }
                let dragged = own.dragged.take().into_iter().collect();
                self.rest(app, dragged)
            }
            (Gesture::Wheel, SceneGraphEvent::MouseWheel(wheel)) => {
                let notches = notches(wheel.delta);
                let Some((key, area)) = area_at(self.view.areas(app), wheel.position) else {
                    return Rebuild::none();
                };
                if notches == 0.0 {
                    return Rebuild::none();
                }
                own.keep_first(&key, || self.view.view(app, &key));
                if !self.view.zoom(app, &key, area, wheel.position, notches) {
                    return Rebuild::none();
                }
                if !own.zoomed.contains(&key) {
                    own.zoomed.push(key);
                }
                Rebuild::scene(&self.scene)
            }
            (Gesture::WheelStop, _) => {
                let zoomed = std::mem::take(&mut own.zoomed);
                self.rest(app, zoomed)
            }
            (Gesture::Home, SceneGraphEvent::DoubleClick(click)) => {
                let Some((key, _)) = area_at(self.view.areas(app), click.position) else {
                    return Rebuild::none();
                };
                if !self.view.home(app, &key, own.first(&key)) {
                    return Rebuild::none();
                }
                if own.dragged.as_ref() == Some(&key) {
                    own.dragged = None;
                }
                own.zoomed.retain(|zoomed| zoomed != &key);
                self.rest(app, vec![key])
            }
            _ => Rebuild::none(),
        }
    }
}
