//! The navigation tool that pans and zooms a map projection.

use std::sync::Arc;
use std::time::Duration;

use avenger_geo::{PanMode, Projection};
use avenger_layout::Rect;

use super::{install, notch_factor, Navigate, NavigationState, SETTLE_DELAY};
use avenger_app::app::{Tool, ToolInstall};

type Area<State> = Arc<dyn Fn(&State) -> Rect + Send + Sync>;
type Select<State> = Arc<dyn Fn(&mut State) -> Option<&mut Projection> + Send + Sync>;
type Home<State> = Arc<dyn Fn(&State) -> Option<Projection> + Send + Sync>;
type Limits<State> = Arc<dyn Fn(&State, &Projection) -> (f64, f64) + Send + Sync>;

/// Pans and zooms an app's map projection, for the gestures in the map's area of a scene. The
/// app keeps the projection in its state, and its build draws the map with it in the area's
/// frame, whose origin is the area's top-left corner. A drag pans and the wheel zooms through
/// [`Projection::pan`] and [`Projection::zoom`] in the tool's [`PanMode`], and the wheel stops
/// at the scale limits. The double-click restores the home, which is the projection at the
/// tool's first gesture unless the app gives one. The [module documentation](super) lists the
/// gestures.
pub struct MapPanZoom<State> {
    scene: String,
    area: Area<State>,
    projection: Select<State>,
    mode: PanMode,
    home: Option<Home<State>>,
    limits: Option<Limits<State>>,
    settle_delay: Duration,
}

impl<State: Send + Sync + 'static> MapPanZoom<State> {
    /// Pan and zoom the projection that `projection` reaches, which is `None` until the app has
    /// one, in `mode`, for the gestures in the area of `scene` that `area` reaches.
    pub fn new(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        projection: impl Fn(&mut State) -> Option<&mut Projection> + Send + Sync + 'static,
        mode: PanMode,
    ) -> Self {
        Self {
            scene: scene.to_string(),
            area: Arc::new(area),
            projection: Arc::new(projection),
            mode,
            home: None,
            limits: None,
            settle_delay: SETTLE_DELAY,
        }
    }

    /// The projection that the double-click restores, such as the projection fitted to the map,
    /// or `None` to leave the projection as it is.
    pub fn with_home(
        mut self,
        home: impl Fn(&State) -> Option<Projection> + Send + Sync + 'static,
    ) -> Self {
        self.home = Some(Arc::new(home));
        self
    }

    /// The least and greatest scales that the wheel reaches from a projection, such as the
    /// fitted scale's fraction and the deepest zoom of the map's tiles.
    pub fn with_scale_limits(
        mut self,
        limits: impl Fn(&State, &Projection) -> (f64, f64) + Send + Sync + 'static,
    ) -> Self {
        self.limits = Some(Arc::new(limits));
        self
    }

    /// How long a drag or the wheel stays still before the view comes to rest, 100 ms by
    /// default.
    pub fn with_settle_delay(mut self, delay: Duration) -> Self {
        self.settle_delay = delay;
        self
    }
}

impl<State: Send + Sync + 'static> Tool<State> for MapPanZoom<State> {
    /// The drag, and the projections at the press and at the first gesture.
    type State = NavigationState<Projection>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        let map = Map {
            area: self.area.clone(),
            projection: self.projection.clone(),
            mode: self.mode,
            home: self.home.clone(),
            limits: self.limits.clone(),
        };
        install(tool, &self.scene, self.settle_delay, Arc::new(map));
    }
}

/// The projection that a [`MapPanZoom`] moves.
struct Map<State> {
    area: Area<State>,
    projection: Select<State>,
    mode: PanMode,
    home: Option<Home<State>>,
    limits: Option<Limits<State>>,
}

impl<State> Map<State> {
    /// Replace the projection with `projection`, when the app has one.
    fn set(&self, app: &mut State, projection: Projection) -> bool {
        match (self.projection)(app) {
            Some(current) => {
                *current = projection;
                true
            }
            None => false,
        }
    }
}

/// `position` in the frame of an area at `area`.
fn local(area: Rect, position: [f32; 2]) -> [f64; 2] {
    [
        f64::from(position[0] - area.x),
        f64::from(position[1] - area.y),
    ]
}

impl<State: Send + Sync + 'static> Navigate<State> for Map<State> {
    type View = Projection;

    fn area(&self, app: &State) -> Rect {
        (self.area)(app)
    }

    fn view(&self, app: &mut State) -> Option<Projection> {
        (self.projection)(app).cloned()
    }

    fn pan(
        &self,
        app: &mut State,
        start: &Projection,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool {
        match start.pan(self.mode, local(area, from), local(area, to)) {
            Ok(panned) => self.set(app, panned),
            Err(error) => {
                eprintln!("MapPanZoom couldn't pan the projection: {error}");
                false
            }
        }
    }

    fn zoom(&self, app: &mut State, area: Rect, at: [f32; 2], notches: f64) -> bool {
        let Some(current) = (self.projection)(app).cloned() else {
            return false;
        };
        let scale = current.view.map_or(1.0, |view| view.scale);
        let (min, max) =
            (self.limits.as_ref()).map_or((0.0, f64::INFINITY), |limits| limits(app, &current));
        let wanted = scale * notch_factor(notches);
        let factor = wanted.min(max).max(min) / scale;
        // At a limit, the wheel turns toward it change nothing.
        if factor == 1.0 {
            return false;
        }
        match current.zoom(self.mode, local(area, at), factor) {
            Ok(zoomed) => self.set(app, zoomed),
            Err(error) => {
                eprintln!("MapPanZoom couldn't zoom the projection: {error}");
                false
            }
        }
    }

    fn home(&self, app: &mut State, first: Option<&Projection>) -> bool {
        let home = match &self.home {
            Some(home) => home(app),
            None => first.cloned(),
        };
        home.is_some_and(|home| self.set(app, home))
    }
}
