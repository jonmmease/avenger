//! Pan and zoom tools for map projections.

use std::sync::Arc;
use std::time::Duration;

use avenger_eventstream::stream::Rebuild;
use avenger_geo::{PanMode, Projection};
use avenger_layout::Rect;

use super::{install, notch_factor, Navigate, NavigationState, SETTLE_DELAY};
use avenger_app::app::{Tool, ToolInstall};
use avenger_common::cursor::CursorIcon;

type Areas<State, K> = Arc<dyn Fn(&State) -> Vec<(K, Rect)> + Send + Sync>;
type Select<State, K> =
    Arc<dyn for<'a> Fn(&'a mut State, &K) -> Option<&'a mut Projection> + Send + Sync>;
type Mode<State, K> = Arc<dyn Fn(&State, &K) -> PanMode + Send + Sync>;
type Home<State, K> = Arc<dyn Fn(&mut State, &K) -> Option<Projection> + Send + Sync>;
type Limits<State, K> = Arc<dyn Fn(&State, &K, &Projection) -> (f64, f64) + Send + Sync>;
type Settle<State, K> = Arc<dyn Fn(&mut State, &K) -> Rebuild + Send + Sync>;

/// Pans and zooms an app-owned map projection within a scene area.
///
/// The app draws the map in local coordinates with the area's top-left corner as the
/// origin. The tool uses [`Projection::pan`] and [`Projection::zoom`] in its [`PanMode`],
/// with optional scale limits for wheel zoom. A double-click restores the projection
/// saved at the first gesture, unless [`Self::with_home`] supplies another home view.
/// See the [module documentation](super) for gesture details. A [`KeyedMapPanZoom`] moves
/// the projections of each of a set of areas.
pub struct MapPanZoom<State>(KeyedMapPanZoom<State, ()>);

impl<State: Send + Sync + 'static> MapPanZoom<State> {
    /// Create a tool for the projection selected from app state, using `mode`.
    /// `area` selects the interaction rectangle within `scene`. The projection selector
    /// can return `None` when no projection is available.
    pub fn new(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        projection: impl Fn(&mut State) -> Option<&mut Projection> + Send + Sync + 'static,
        mode: PanMode,
    ) -> Self {
        Self(KeyedMapPanZoom::new(
            scene,
            move |state| vec![((), area(state))],
            move |state, _| projection(state),
            move |_, _| mode,
        ))
    }

    /// Set the callback that supplies the home projection on double-click, such as a projection
    /// fitted to the map rectangle. Supply a home when something other than the tool also
    /// changes the projection. The callback can also restore the home view itself, such as by
    /// dropping a view that replaces a fitted projection, and return `None`.
    pub fn with_home(
        self,
        home: impl Fn(&mut State) -> Option<Projection> + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.with_home(move |state, _| home(state)))
    }

    /// Set a callback that returns the minimum and maximum scales for wheel zoom.
    /// The callback receives the app state and current projection.
    pub fn with_scale_limits(
        self,
        limits: impl Fn(&State, &Projection) -> (f64, f64) + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .with_scale_limits(move |state, _, projection| limits(state, projection)),
        )
    }

    /// Set the callback for the end of a gesture that changed the projection: a drag that
    /// pauses, ends, or is canceled, a wheel zoom that stops, and a double-click that restores
    /// the home view. Return the scenes to rebuild besides the tool's scene, which rebuilds with
    /// its tree.
    pub fn on_settle(
        self,
        on_settle: impl Fn(&mut State) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.on_settle(move |state, _| on_settle(state)))
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(self, delay: Duration) -> Self {
        Self(self.0.with_settle_delay(delay))
    }
}

impl<State: Send + Sync + 'static> Tool<State> for MapPanZoom<State> {
    /// Private gesture state and projection snapshots.
    type State = NavigationState<(), Projection>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        self.0.install(tool);
    }

    fn cursor(&self, app: &State, own: &Self::State) -> Option<CursorIcon> {
        self.0.cursor(app, own)
    }
}

/// Pans and zooms the map projection of each of a set of areas, which the app names with keys,
/// such as the maps of a dashboard.
///
/// The areas come from the app state on each event, and a press, a wheel event, or a
/// double-click acts on the first area that contains the pointer, as for
/// [`KeyedPanZoom`](crate::navigation::KeyedPanZoom). Each map draws in its area's coordinates
/// from the area's top-left corner, and pans and zooms in the [`PanMode`] of its key.
pub struct KeyedMapPanZoom<State, K> {
    scene: String,
    areas: Areas<State, K>,
    projection: Select<State, K>,
    mode: Mode<State, K>,
    home: Option<Home<State, K>>,
    limits: Option<Limits<State, K>>,
    on_settle: Option<Settle<State, K>>,
    settle_delay: Duration,
}

impl<State, K> KeyedMapPanZoom<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a tool for the projection that `projection` selects for each key, in the areas that
    /// `areas` gives within `scene`, using the mode that `mode` gives for the key. The projection
    /// selector can return `None` for a key without a projection.
    pub fn new(
        scene: &str,
        areas: impl Fn(&State) -> Vec<(K, Rect)> + Send + Sync + 'static,
        projection: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut Projection>
            + Send
            + Sync
            + 'static,
        mode: impl Fn(&State, &K) -> PanMode + Send + Sync + 'static,
    ) -> Self {
        Self {
            scene: scene.to_string(),
            areas: Arc::new(areas),
            projection: Arc::new(projection),
            mode: Arc::new(mode),
            home: None,
            limits: None,
            on_settle: None,
            settle_delay: SETTLE_DELAY,
        }
    }

    /// Set the callback that supplies the home projection of a key on double-click. See
    /// [`MapPanZoom::with_home`].
    pub fn with_home(
        mut self,
        home: impl Fn(&mut State, &K) -> Option<Projection> + Send + Sync + 'static,
    ) -> Self {
        self.home = Some(Arc::new(home));
        self
    }

    /// Set a callback that returns the minimum and maximum scales of a key for wheel zoom. The
    /// callback receives the app state, the key, and the key's current projection.
    pub fn with_scale_limits(
        mut self,
        limits: impl Fn(&State, &K, &Projection) -> (f64, f64) + Send + Sync + 'static,
    ) -> Self {
        self.limits = Some(Arc::new(limits));
        self
    }

    /// Set the callback for the end of a gesture that changed the projection of a key. See
    /// [`MapPanZoom::on_settle`].
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, &K) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_settle = Some(Arc::new(on_settle));
        self
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(mut self, delay: Duration) -> Self {
        self.settle_delay = delay;
        self
    }
}

impl<State, K> Tool<State> for KeyedMapPanZoom<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    /// Private gesture state and projection snapshots.
    type State = NavigationState<K, Projection>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        let maps = Maps {
            areas: self.areas.clone(),
            projection: self.projection.clone(),
            mode: self.mode.clone(),
            home: self.home.clone(),
            limits: self.limits.clone(),
            on_settle: self.on_settle.clone(),
        };
        install(tool, &self.scene, self.settle_delay, Arc::new(maps));
    }

    fn cursor(&self, _app: &State, own: &Self::State) -> Option<CursorIcon> {
        own.cursor()
    }
}

/// The projections that a [`KeyedMapPanZoom`] moves.
struct Maps<State, K> {
    areas: Areas<State, K>,
    projection: Select<State, K>,
    mode: Mode<State, K>,
    home: Option<Home<State, K>>,
    limits: Option<Limits<State, K>>,
    on_settle: Option<Settle<State, K>>,
}

impl<State, K> Maps<State, K> {
    /// Replace the projection of `key` with `projection`, when the app has one.
    fn set(&self, app: &mut State, key: &K, projection: Projection) -> bool {
        match (self.projection)(app, key) {
            Some(current) => {
                *current = projection;
                true
            }
            None => false,
        }
    }
}

/// Convert a window position to coordinates relative to `area`.
fn local(area: Rect, position: [f32; 2]) -> [f64; 2] {
    [
        f64::from(position[0] - area.x),
        f64::from(position[1] - area.y),
    ]
}

impl<State, K> Navigate<State> for Maps<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    type Key = K;
    type View = Projection;

    fn areas(&self, app: &State) -> Vec<(K, Rect)> {
        (self.areas)(app)
    }

    fn view(&self, app: &mut State, key: &K) -> Option<Projection> {
        (self.projection)(app, key).cloned()
    }

    fn pan(
        &self,
        app: &mut State,
        key: &K,
        start: &Projection,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool {
        let mode = (self.mode)(app, key);
        match start.pan(mode, local(area, from), local(area, to)) {
            Ok(panned) => self.set(app, key, panned),
            Err(error) => {
                eprintln!("MapPanZoom couldn't pan the projection: {error}");
                false
            }
        }
    }

    fn zoom(&self, app: &mut State, key: &K, area: Rect, at: [f32; 2], notches: f64) -> bool {
        let Some(current) = (self.projection)(app, key).cloned() else {
            return false;
        };
        let scale = current.view.map_or(1.0, |view| view.scale);
        let (min, max) = (self.limits.as_ref())
            .map_or((0.0, f64::INFINITY), |limits| limits(app, key, &current));
        let wanted = scale * notch_factor(notches);
        let factor = wanted.min(max).max(min) / scale;
        // At a limit, the wheel turns toward it change nothing.
        if factor == 1.0 {
            return false;
        }
        let mode = (self.mode)(app, key);
        match current.zoom(mode, local(area, at), factor) {
            Ok(zoomed) => self.set(app, key, zoomed),
            Err(error) => {
                eprintln!("MapPanZoom couldn't zoom the projection: {error}");
                false
            }
        }
    }

    fn home(&self, app: &mut State, key: &K, first: Option<&Projection>) -> bool {
        let home = match (&self.home, first) {
            (Some(home), _) => home(app, key),
            (None, Some(first)) => Some(first.clone()),
            (None, None) => return false,
        };
        if let Some(home) = home {
            self.set(app, key, home);
        }
        true
    }

    fn settled(&self, app: &mut State, key: &K) -> Rebuild {
        self.on_settle
            .as_ref()
            .map_or_else(Rebuild::none, |on_settle| on_settle(app, key))
    }
}
