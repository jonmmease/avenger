//! Navigation tools for continuous scales (`scales` feature) and map projections
//! (`geo` feature). Both support these gestures within their configured area:
//!
//! - A left-button drag pans from the view at the press, keeping the same location
//!   under the pointer. Moves are throttled to one every 8 ms.
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
//! Each tool calls for an open hand cursor while the pointer is over its area, and a closed
//! hand during its drag.

#[cfg(feature = "geo")]
mod map_pan_zoom;
#[cfg(feature = "scales")]
mod pan_zoom;

#[cfg(feature = "geo")]
pub use map_pan_zoom::MapPanZoom;
#[cfg(feature = "scales")]
pub use pan_zoom::PanZoom;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use avenger_common::cursor::CursorStyle;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_layout::Rect;

use avenger_app::app::{ToolHandler, ToolInstall};

/// Default idle time before a drag or wheel gesture settles.
const SETTLE_DELAY: Duration = Duration::from_millis(100);

/// Private navigation state: the active drag, pending pan and zoom changes, the view at the
/// first gesture, which is the home by default, and whether the pointer is over the area.
pub struct NavigationState<V> {
    drag: Option<Drag<V>>,
    dragged: bool,
    zoomed: bool,
    first: Option<V>,
    /// Whether the pointer is over the area.
    over: bool,
}

impl<V> Default for NavigationState<V> {
    fn default() -> Self {
        Self {
            drag: None,
            dragged: false,
            zoomed: false,
            first: None,
            over: false,
        }
    }
}

impl<V> NavigationState<V> {
    /// A closed hand during a drag, and an open hand over the area.
    fn cursor(&self) -> Option<CursorStyle> {
        if self.drag.is_some() {
            Some(CursorStyle::Grabbing)
        } else {
            self.over.then_some(CursorStyle::Grab)
        }
    }
}

/// The view and pointer position at press time, plus the last processed pointer position.
struct Drag<V> {
    start: V,
    from: [f32; 2],
    at: [f32; 2],
}

/// View operations shared by scale and projection navigation.
trait Navigate<State>: Send + Sync + 'static {
    /// A snapshot of the view, at a drag's press and at the tool's first gesture.
    type View: Clone + Send + Sync + 'static;

    /// The area of the scene where the tool's gestures apply.
    fn area(&self, app: &State) -> Rect;

    /// Return the current view, or `None` if unavailable.
    fn view(&self, app: &mut State) -> Option<Self::View>;

    /// Pan `start` so the location at `from` moves to `to` within `area`.
    /// Return whether the operation updated the view.
    fn pan(
        &self,
        app: &mut State,
        start: &Self::View,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool;

    /// Zoom around `at` within `area` by `notches` wheel notches.
    /// Return whether the operation updated the view.
    fn zoom(&self, app: &mut State, area: Rect, at: [f32; 2], notches: f64) -> bool;

    /// Restore the home view, using the first gesture snapshot if needed.
    /// Return whether a home view was applied.
    fn home(&self, app: &mut State, first: Option<&Self::View>) -> bool;
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

/// Attach navigation event streams to `scene`.
fn install<State, N>(
    tool: &mut ToolInstall<'_, State, NavigationState<N::View>>,
    scene: &str,
    settle_delay: Duration,
    view: Arc<N>,
) where
    State: Send + Sync + 'static,
    N: Navigate<State>,
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
    /// Track whether the pointer is over the area, for the cursor.
    Hover,
    /// A press in the area starts a drag.
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
        own: &mut NavigationState<<N as Navigate<State>>::View>,
        to: [f32; 2],
    ) -> Rebuild
    where
        N: Navigate<State>,
    {
        let Some(drag) = &mut own.drag else {
            return Rebuild::none();
        };
        if drag.at == to {
            return Rebuild::none();
        }
        drag.at = to;
        let area = self.view.area(app);
        if !self.view.pan(app, &drag.start, area, drag.from, to) {
            return Rebuild::none();
        }
        own.dragged = true;
        Rebuild::scene(&self.scene)
    }

    /// Request the scene and tree if this gesture has pending changes, then clear the flag.
    fn rest(&self, moved: &mut bool) -> Rebuild {
        if std::mem::take(moved) {
            Rebuild::tree(&self.scene)
        } else {
            Rebuild::none()
        }
    }
}

#[async_trait]
impl<State, N> ToolHandler<State, NavigationState<N::View>> for Gestures<N>
where
    State: Send + Sync + 'static,
    N: Navigate<State>,
{
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        app: &mut State,
        own: &mut NavigationState<N::View>,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        match (self.gesture, event) {
            (Gesture::Hover, SceneGraphEvent::CursorMoved(moved)) => {
                own.over = self.view.area(app).contains(moved.position);
                Rebuild::none()
            }
            (Gesture::Hover, _) => {
                own.over = false;
                Rebuild::none()
            }
            (Gesture::Press, SceneGraphEvent::MouseDown(press)) => {
                if press.button != MouseButton::Left
                    || !self.view.area(app).contains(press.position)
                {
                    return Rebuild::none();
                }
                own.drag = self.view.view(app).map(|start| Drag {
                    start,
                    from: press.position,
                    at: press.position,
                });
                if own.first.is_none() {
                    own.first = own.drag.as_ref().map(|drag| drag.start.clone());
                }
                Rebuild::none()
            }
            (Gesture::Move, SceneGraphEvent::CursorMoved(moved)) => {
                self.pan(app, own, moved.position)
            }
            (Gesture::Pause, SceneGraphEvent::CursorMoved(moved)) => {
                self.pan(app, own, moved.position);
                self.rest(&mut own.dragged)
            }
            (Gesture::Release, SceneGraphEvent::MouseUp(release)) => {
                if release.button != MouseButton::Left || own.drag.is_none() {
                    return Rebuild::none();
                }
                self.pan(app, own, release.position);
                own.drag = None;
                self.rest(&mut own.dragged)
            }
            (Gesture::Cancel, _) => {
                if own.drag.take().is_none() {
                    return Rebuild::none();
                }
                self.rest(&mut own.dragged)
            }
            (Gesture::Wheel, SceneGraphEvent::MouseWheel(wheel)) => {
                let area = self.view.area(app);
                let notches = notches(wheel.delta);
                if notches == 0.0 || !area.contains(wheel.position) {
                    return Rebuild::none();
                }
                if own.first.is_none() {
                    own.first = self.view.view(app);
                }
                if !self.view.zoom(app, area, wheel.position, notches) {
                    return Rebuild::none();
                }
                own.zoomed = true;
                Rebuild::scene(&self.scene)
            }
            (Gesture::WheelStop, _) => self.rest(&mut own.zoomed),
            (Gesture::Home, SceneGraphEvent::DoubleClick(click)) => {
                if !self.view.area(app).contains(click.position)
                    || !self.view.home(app, own.first.as_ref())
                {
                    return Rebuild::none();
                }
                (own.dragged, own.zoomed) = (false, false);
                Rebuild::tree(&self.scene)
            }
            _ => Rebuild::none(),
        }
    }
}
