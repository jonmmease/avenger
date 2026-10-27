//! Pan and zoom tools for continuous scales.

use std::sync::Arc;
use std::time::Duration;

use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::ConfiguredScale;

use super::{install, notch_factor, Navigate, NavigationState, SETTLE_DELAY};
use avenger_app::app::{Tool, ToolInstall};
use avenger_common::cursor::CursorIcon;

type Areas<State, K> = Arc<dyn Fn(&State) -> Vec<(K, Rect)> + Send + Sync>;
type Select<State, K> =
    Arc<dyn for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale> + Send + Sync>;
type Home<State, K> = Arc<dyn Fn(&State, &K) -> [Option<ConfiguredScale>; 2] + Send + Sync>;
type Settle<State, K> = Arc<dyn Fn(&mut State, &K) -> Rebuild + Send + Sync>;
type Bounds<State, K> = Arc<dyn Fn(&State, &K) -> [Option<[f64; 2]>; 2] + Send + Sync>;

/// The view of the x and y scales of an area.
type View = [Option<ConfiguredScale>; 2];

/// A scale selector with the signature that ties the scale's lifetime to the state's.
fn select<State, K>(
    select: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale>
        + Send
        + Sync
        + 'static,
) -> Select<State, K> {
    Arc::new(select)
}

/// Pans and zooms the app's x scale, y scale, or both within a scene area.
///
/// The app owns the scales and sets their ranges during layout. Each range must span
/// the area, in any units. The tool uses the range direction and pointer movement as
/// a fraction of the area to pan and zoom through [`ConfiguredScale::pan`] and
/// [`ConfiguredScale::zoom`]. Linear and time scales support these operations.
///
/// A double-click restores the domains saved at the first gesture, unless [`Self::with_home`]
/// supplies the home scales. See the [module documentation](super) for gesture details. A
/// [`KeyedPanZoom`] moves the scales of each of a set of areas.
pub struct PanZoom<State>(KeyedPanZoom<State, ()>);

impl<State: Send + Sync + 'static> PanZoom<State> {
    /// Create a tool for the scales selected by `x` and `y` in app state.
    /// `area` selects the interaction rectangle within `scene`.
    pub fn xy(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        x: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
        y: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self::new(
            scene,
            area,
            [
                Some(select(move |state, _| Some(x(state)))),
                Some(select(move |state, _| Some(y(state)))),
            ],
        )
    }

    /// Create a tool for the x scale selected from app state, within `area` of `scene`.
    pub fn x(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        x: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self::new(
            scene,
            area,
            [Some(select(move |state, _| Some(x(state)))), None],
        )
    }

    /// Create a tool for the y scale selected from app state, within `area` of `scene`.
    pub fn y(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        y: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self::new(
            scene,
            area,
            [None, Some(select(move |state, _| Some(y(state))))],
        )
    }

    fn new(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        scales: [Option<Select<State, ()>>; 2],
    ) -> Self {
        let areas: Areas<State, ()> = Arc::new(move |state| vec![((), area(state))]);
        Self(KeyedPanZoom::new(scene, areas, scales))
    }

    /// Set the callback that supplies the home scales on double-click, x then y, such as the
    /// scales that the app first drew. The tool restores the domain of each scale that it moves
    /// from the home scale in the same place, and a `None` leaves that scale as it is. Supply a
    /// home when something other than the tool also changes the scales, such as a box zoom.
    pub fn with_home(
        self,
        home: impl Fn(&State) -> [Option<ConfiguredScale>; 2] + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.with_home(move |state, _| home(state)))
    }

    /// Set the callback for the end of a gesture that changed the scales: a drag that pauses,
    /// ends, or is canceled, a wheel zoom that stops, and a double-click that restores the home
    /// view. Return the scenes to rebuild besides the tool's scene, which rebuilds with its tree.
    pub fn on_settle(
        self,
        on_settle: impl Fn(&mut State) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.on_settle(move |state, _| on_settle(state)))
    }

    /// Keep each scale's domain inside `[low, high]` bounds, x then y, as
    /// [`KeyedPanZoom::bounds`] describes.
    pub fn bounds(
        self,
        bounds: impl Fn(&State) -> [Option<[f64; 2]>; 2] + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.bounds(move |state, _| bounds(state)))
    }

    /// Keep each scale's domain at least this wide, x then y, as [`KeyedPanZoom::min_span`]
    /// describes.
    pub fn min_span(self, min_span: [Option<f64>; 2]) -> Self {
        Self(self.0.min_span(min_span))
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(self, delay: Duration) -> Self {
        Self(self.0.with_settle_delay(delay))
    }
}

impl<State: Send + Sync + 'static> Tool<State> for PanZoom<State> {
    /// Private gesture state and scale snapshots.
    type State = NavigationState<(), View>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        self.0.install(tool);
    }

    fn cursor(&self, app: &State, own: &Self::State) -> Option<CursorIcon> {
        self.0.cursor(app, own)
    }
}

/// Pans and zooms the x scale, the y scale, or both of each of a set of areas, which the app
/// names with keys, such as the panels of a facet grid.
///
/// The areas come from the app state on each event, so a change of the app's panels changes
/// where the tool acts. A press, a wheel event, or a double-click acts on the first area that
/// contains the pointer, and a drag keeps the key of its press. The scale selectors return the
/// scales of a key, or `None` for a key without a scale on that axis. Panels that share a scale
/// return the same scale, so a pan in one panel moves them all.
pub struct KeyedPanZoom<State, K> {
    scene: String,
    areas: Areas<State, K>,
    /// The x and y scales that the tool moves.
    scales: [Option<Select<State, K>>; 2],
    home: Option<Home<State, K>>,
    on_settle: Option<Settle<State, K>>,
    bounds: Option<Bounds<State, K>>,
    min_span: [Option<f64>; 2],
    settle_delay: Duration,
}

impl<State, K> KeyedPanZoom<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a tool for the scales that `x` and `y` select for each key, in the areas that
    /// `areas` gives within `scene`.
    pub fn xy(
        scene: &str,
        areas: impl Fn(&State) -> Vec<(K, Rect)> + Send + Sync + 'static,
        x: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale> + Send + Sync + 'static,
        y: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale> + Send + Sync + 'static,
    ) -> Self {
        Self::new(scene, Arc::new(areas), [Some(select(x)), Some(select(y))])
    }

    /// Create a tool for the x scale that `x` selects for each key.
    pub fn x(
        scene: &str,
        areas: impl Fn(&State) -> Vec<(K, Rect)> + Send + Sync + 'static,
        x: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale> + Send + Sync + 'static,
    ) -> Self {
        Self::new(scene, Arc::new(areas), [Some(select(x)), None])
    }

    /// Create a tool for the y scale that `y` selects for each key.
    pub fn y(
        scene: &str,
        areas: impl Fn(&State) -> Vec<(K, Rect)> + Send + Sync + 'static,
        y: impl for<'a> Fn(&'a mut State, &K) -> Option<&'a mut ConfiguredScale> + Send + Sync + 'static,
    ) -> Self {
        Self::new(scene, Arc::new(areas), [None, Some(select(y))])
    }

    fn new(scene: &str, areas: Areas<State, K>, scales: [Option<Select<State, K>>; 2]) -> Self {
        Self {
            scene: scene.to_string(),
            areas,
            scales,
            home: None,
            on_settle: None,
            bounds: None,
            min_span: [None; 2],
            settle_delay: SETTLE_DELAY,
        }
    }

    /// Set the callback that supplies the home scales of a key on double-click, x then y. See
    /// [`PanZoom::with_home`].
    pub fn with_home(
        mut self,
        home: impl Fn(&State, &K) -> [Option<ConfiguredScale>; 2] + Send + Sync + 'static,
    ) -> Self {
        self.home = Some(Arc::new(home));
        self
    }

    /// Set the callback for the end of a gesture that changed the scales of a key. See
    /// [`PanZoom::on_settle`].
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, &K) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_settle = Some(Arc::new(on_settle));
        self
    }

    /// Keep each scale's domain inside `[low, high]` bounds for each key, x then y, in the values
    /// that [`ConfiguredScale::invert_scalar`] gives, which are epoch milliseconds for a time
    /// scale. A pan stops at a bound, and a zoom out stops at the width of the bounds. An axis
    /// without bounds moves freely.
    pub fn bounds(
        mut self,
        bounds: impl Fn(&State, &K) -> [Option<[f64; 2]>; 2] + Send + Sync + 'static,
    ) -> Self {
        self.bounds = Some(Arc::new(bounds));
        self
    }

    /// Keep each scale's domain at least this wide, x then y, in the values of
    /// [`Self::bounds`]. A zoom in stops at the minimum span.
    pub fn min_span(mut self, min_span: [Option<f64>; 2]) -> Self {
        self.min_span = min_span;
        self
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(mut self, delay: Duration) -> Self {
        self.settle_delay = delay;
        self
    }
}

impl<State, K> Tool<State> for KeyedPanZoom<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    /// Private gesture state and scale snapshots.
    type State = NavigationState<K, View>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        let scales = Scales {
            areas: self.areas.clone(),
            scales: self.scales.clone(),
            home: self.home.clone(),
            on_settle: self.on_settle.clone(),
            bounds: self.bounds.clone(),
            min_span: self.min_span,
        };
        install(tool, &self.scene, self.settle_delay, Arc::new(scales));
    }

    fn cursor(&self, _app: &State, own: &Self::State) -> Option<CursorIcon> {
        own.cursor()
    }
}

/// The scales that a [`KeyedPanZoom`] moves.
struct Scales<State, K> {
    areas: Areas<State, K>,
    scales: [Option<Select<State, K>>; 2],
    home: Option<Home<State, K>>,
    on_settle: Option<Settle<State, K>>,
    bounds: Option<Bounds<State, K>>,
    min_span: [Option<f64>; 2],
}

impl<State, K> Scales<State, K> {
    /// The bounds of each axis of `key`.
    fn bounds(&self, app: &State, key: &K) -> [Option<[f64; 2]>; 2] {
        self.bounds
            .as_ref()
            .map_or([None; 2], |bounds| bounds(app, key))
    }
}

/// Return the start and length of `area` along `axis` (0 for x, 1 for y).
fn span(area: Rect, axis: usize) -> (f32, f32) {
    if axis == 0 {
        (area.x, area.width)
    } else {
        (area.y, area.height)
    }
}

/// The direction of `scale`'s range: 1 when it increases, and -1 when it decreases.
fn direction(scale: &ConfiguredScale) -> Result<f64, AvengerScaleError> {
    let (start, end) = scale.numeric_interval_range()?;
    Ok(if end < start { -1.0 } else { 1.0 })
}

/// The smallest and largest domain values that `scale` shows across its range.
fn shown(scale: &ConfiguredScale) -> Option<[f64; 2]> {
    let (start, end) = scale.numeric_interval_range().ok()?;
    let [a, b] = [start, end].map(|end| scale.invert_scalar(end).ok());
    Some([a?.min(b?), a?.max(b?)])
}

/// `scale` moved back inside `bounds`, keeping its span, or set to the bounds when it is wider.
fn bounded(scale: ConfiguredScale, bounds: Option<[f64; 2]>) -> ConfiguredScale {
    let (Some([low, high]), Some([start, end])) = (bounds, shown(&scale)) else {
        return scale;
    };
    let width = end - start;
    if width >= high - low {
        return scale.with_domain_interval((low, high));
    }
    let shift = if start < low {
        low - start
    } else if end > high {
        high - end
    } else {
        return scale;
    };
    // A pan by `delta` moves the domain by `-delta` times its width.
    scale.clone().pan(-shift / width).unwrap_or(scale)
}

impl<State, K> Navigate<State> for Scales<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + PartialEq + Send + Sync + 'static,
{
    type Key = K;
    type View = View;

    fn areas(&self, app: &State) -> Vec<(K, Rect)> {
        (self.areas)(app)
    }

    fn view(&self, app: &mut State, key: &K) -> Option<View> {
        let scales = self.scales.each_ref();
        Some(scales.map(|select| select.as_ref().and_then(|select| select(app, key).cloned())))
    }

    fn pan(
        &self,
        app: &mut State,
        key: &K,
        start: &View,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool {
        let bounds = self.bounds(app, key);
        let mut changed = false;
        for (axis, (select, start)) in self.scales.iter().zip(start).enumerate() {
            let (_, length) = span(area, axis);
            let (Some(select), Some(start), true) = (select, start, length > 0.0) else {
                continue;
            };
            let Some(scale) = select(app, key) else {
                continue;
            };
            let moved = direction(scale).and_then(|direction| {
                let delta = f64::from((to[axis] - from[axis]) / length) * direction;
                let at_press = scale.clone().with_domain(start.domain().clone());
                at_press.pan(delta)
            });
            match moved {
                Ok(moved) => {
                    *scale = bounded(moved, bounds[axis]);
                    changed = true;
                }
                Err(error) => eprintln!("PanZoom couldn't pan a scale: {error}"),
            }
        }
        changed
    }

    fn zoom(&self, app: &mut State, key: &K, area: Rect, at: [f32; 2], notches: f64) -> bool {
        let bounds = self.bounds(app, key);
        let mut changed = false;
        for (axis, select) in self.scales.iter().enumerate() {
            let (start, length) = span(area, axis);
            let (Some(select), true) = (select, length > 0.0) else {
                continue;
            };
            let Some(scale) = select(app, key) else {
                continue;
            };
            // Zooming in by a factor shrinks the domain by it, down to the minimum span and up
            // to the width of the bounds. At a limit, the wheel turns toward it change nothing.
            let mut factor = notch_factor(-notches);
            if let Some([low, high]) = shown(scale) {
                let width = high - low;
                if let Some(min) = self.min_span[axis] {
                    factor = factor.max((min / width).min(1.0));
                }
                if let Some([low, high]) = bounds[axis] {
                    factor = factor.min(((high - low) / width).max(1.0));
                }
            }
            if factor == 1.0 {
                continue;
            }
            let fraction = f64::from((at[axis] - start) / length);
            let zoomed = direction(scale).and_then(|direction| {
                let anchor = if direction < 0.0 {
                    1.0 - fraction
                } else {
                    fraction
                };
                scale.clone().zoom(anchor, factor)
            });
            match zoomed {
                Ok(zoomed) => {
                    *scale = bounded(zoomed, bounds[axis]);
                    changed = true;
                }
                Err(error) => eprintln!("PanZoom couldn't zoom a scale: {error}"),
            }
        }
        changed
    }

    fn home(&self, app: &mut State, key: &K, first: Option<&View>) -> bool {
        let home = match (&self.home, first) {
            (Some(home), _) => home(app, key),
            (None, Some(first)) => first.clone(),
            (None, None) => return false,
        };
        for (select, home) in self.scales.iter().zip(home) {
            if let (Some(select), Some(home)) = (select, home) {
                if let Some(scale) = select(app, key) {
                    *scale = scale.clone().with_domain(home.domain().clone());
                }
            }
        }
        true
    }

    fn settled(&self, app: &mut State, key: &K) -> Rebuild {
        self.on_settle
            .as_ref()
            .map_or_else(Rebuild::none, |on_settle| on_settle(app, key))
    }
}
