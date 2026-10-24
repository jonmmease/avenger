//! Tools that pan and zoom an app's views: [`PanZoom`] moves continuous scales, behind the
//! `scales` feature, and [`MapPanZoom`] moves a map projection, behind the `geo` feature. Both
//! take the same gestures in their area of a scene:
//!
//! - A press remembers the view, and a drag sets the view to the remembered view panned from the
//!   press to the pointer, at most once each 8 ms, so the place under the press stays under the
//!   pointer. Each move requests the scene without its tree.
//! - The wheel zooms the view about the pointer by 20% a notch, at most two notches for one
//!   event, and requests the scene without its tree. A line is a notch, and so are 100 pixels.
//! - A drag's pause for the settle delay, its release, and a canceled press bring the drag's
//!   changes to rest, and the wheel's stopping for the settle delay brings its zooms to rest.
//!   Each requests the scene with its tree when its own gesture moved the view since the tree
//!   last caught up, so a pause of the pointer doesn't settle the wheel.
//! - A double-click restores the view's home, and requests the scene with its tree.
//!
//! A gesture that starts outside the area doesn't belong to the tool, so two tools on one scene
//! each take the gestures in their own area. The tools consume no events, so the streams that
//! hear events after them, such as the app's own, still hear the gestures.

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
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_layout::Rect;

use avenger_app::app::{ToolHandler, ToolInstall};

/// How long a drag or the wheel stays still before the view comes to rest, unless a tool sets
/// its own delay.
const SETTLE_DELAY: Duration = Duration::from_millis(100);

/// What a navigation tool keeps between events: its drag, whether a drag and whether the wheel
/// moved its view since its scene's tree last caught up, and its view at its first gesture.
pub struct NavigationState<V> {
    drag: Option<Drag<V>>,
    dragged: bool,
    zoomed: bool,
    first: Option<V>,
}

impl<V> Default for NavigationState<V> {
    fn default() -> Self {
        Self {
            drag: None,
            dragged: false,
            zoomed: false,
            first: None,
        }
    }
}

/// A drag: the view at its press, where the press was, and where the view was last panned to.
struct Drag<V> {
    start: V,
    from: [f32; 2],
    at: [f32; 2],
}

/// What a navigation tool moves: a view, which a drag pans from the view at its press and the
/// wheel zooms about the pointer.
trait Navigate<State>: Send + Sync + 'static {
    /// The view, as a drag remembers it at its press.
    type View: Clone + Send + Sync + 'static;

    /// The area of the scene where the tool's gestures apply.
    fn area(&self, app: &State) -> Rect;

    /// The view now, or `None` while the app has none.
    fn view(&self, app: &mut State) -> Option<Self::View>;

    /// Set the view to `start` panned so that the place at `from` lies at `to`, in an area at
    /// `area`. Returns whether the view changed.
    fn pan(
        &self,
        app: &mut State,
        start: &Self::View,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool;

    /// Zoom the view by `notches` wheel notches about `at`, in an area at `area`. Returns whether
    /// the view changed.
    fn zoom(&self, app: &mut State, area: Rect, at: [f32; 2], notches: f64) -> bool;

    /// Restore the view's home, given the view at the tool's first gesture. Returns whether the
    /// tool has a home to restore.
    fn home(&self, app: &mut State, first: Option<&Self::View>) -> bool;
}

/// The zoom factor of `notches` wheel notches, by which a view's scale grows.
fn notch_factor(notches: f64) -> f64 {
    1.2_f64.powf(notches)
}

/// The wheel notches of a wheel event's `delta`: a line is a notch, and so are 100 pixels, as a
/// browser reports a notch, and one event turns at most two notches.
fn notches(delta: MouseScrollDelta) -> f64 {
    let notches = match delta {
        MouseScrollDelta::LineDelta(x, y) => f64::from(x + y),
        MouseScrollDelta::PixelDelta(x, y) => (x + y) / 100.0,
    };
    notches.clamp(-2.0, 2.0)
}

/// Attach the streams of the gestures that move `view` to the tool's scene `scene`.
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
        (on(&[SceneGraphEventType::MouseDown]), Gesture::Press),
        (
            EventStreamConfig {
                // At most one pan a frame.
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

/// What a stream of a navigation tool's gestures does with its events.
#[derive(Clone, Copy)]
enum Gesture {
    /// A press in the area starts a drag.
    Press,
    /// A move pans the drag's view.
    Move,
    /// A pause of the pointer brings a drag's changes to rest, after a last pan to where the
    /// pointer paused, since the throttle can drop the drag's last move.
    Pause,
    /// A release ends the drag, after a last pan to where the pointer let go.
    Release,
    /// A canceled press ends the drag where its last move left it.
    Cancel,
    /// The wheel zooms the view.
    Wheel,
    /// The wheel's stopping brings its zooms to rest.
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
    /// Pan the drag's view to `to`, unless it's there already.
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

    /// Bring a gesture's changes to rest: request the scene with its tree, when the gesture moved
    /// the view since the tree last caught up, as `moved` records.
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
