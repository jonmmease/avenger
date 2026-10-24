//! Pan and zoom tools for continuous scales.

use std::sync::Arc;
use std::time::Duration;

use avenger_layout::Rect;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::ConfiguredScale;

use super::{install, notch_factor, Navigate, NavigationState, SETTLE_DELAY};
use avenger_app::app::{Tool, ToolInstall};

type Area<State> = Arc<dyn Fn(&State) -> Rect + Send + Sync>;
type Select<State> = Arc<dyn Fn(&mut State) -> &mut ConfiguredScale + Send + Sync>;

/// Pans and zooms the app's x scale, y scale, or both within a scene area.
///
/// The app owns the scales and sets their ranges during layout. Each range must span
/// the area, in any units. The tool uses the range direction and pointer movement as
/// a fraction of the area to pan and zoom through [`ConfiguredScale::pan`] and
/// [`ConfiguredScale::zoom`]. Linear scales support these operations.
///
/// A double-click restores the domains saved at the first gesture. See the
/// [module documentation](super) for gesture details.
pub struct PanZoom<State> {
    scene: String,
    area: Area<State>,
    /// The x and y scales that the tool moves.
    scales: [Option<Select<State>>; 2],
    settle_delay: Duration,
}

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
            Arc::new(area),
            [Some(Arc::new(x)), Some(Arc::new(y))],
        )
    }

    /// Create a tool for the x scale selected from app state, within `area` of `scene`.
    pub fn x(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        x: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self::new(scene, Arc::new(area), [Some(Arc::new(x)), None])
    }

    /// Create a tool for the y scale selected from app state, within `area` of `scene`.
    pub fn y(
        scene: &str,
        area: impl Fn(&State) -> Rect + Send + Sync + 'static,
        y: impl Fn(&mut State) -> &mut ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self::new(scene, Arc::new(area), [None, Some(Arc::new(y))])
    }

    fn new(scene: &str, area: Area<State>, scales: [Option<Select<State>>; 2]) -> Self {
        Self {
            scene: scene.to_string(),
            area,
            scales,
            settle_delay: SETTLE_DELAY,
        }
    }

    /// Set the idle time before a drag or wheel gesture settles. Defaults to 100 ms.
    pub fn with_settle_delay(mut self, delay: Duration) -> Self {
        self.settle_delay = delay;
        self
    }
}

impl<State: Send + Sync + 'static> Tool<State> for PanZoom<State> {
    /// Private gesture state and scale snapshots.
    type State = NavigationState<[Option<ConfiguredScale>; 2]>;

    fn install(&self, tool: &mut ToolInstall<'_, State, Self::State>) {
        let scales = Scales {
            area: self.area.clone(),
            scales: self.scales.clone(),
        };
        install(tool, &self.scene, self.settle_delay, Arc::new(scales));
    }
}

/// The scales that a [`PanZoom`] moves.
struct Scales<State> {
    area: Area<State>,
    scales: [Option<Select<State>>; 2],
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

impl<State: Send + Sync + 'static> Navigate<State> for Scales<State> {
    type View = [Option<ConfiguredScale>; 2];

    fn area(&self, app: &State) -> Rect {
        (self.area)(app)
    }

    fn view(&self, app: &mut State) -> Option<Self::View> {
        let scales = self.scales.each_ref();
        Some(scales.map(|select| select.as_ref().map(|select| select(app).clone())))
    }

    fn pan(
        &self,
        app: &mut State,
        start: &Self::View,
        area: Rect,
        from: [f32; 2],
        to: [f32; 2],
    ) -> bool {
        let mut changed = false;
        for (axis, (select, start)) in self.scales.iter().zip(start).enumerate() {
            let (_, length) = span(area, axis);
            let (Some(select), Some(start), true) = (select, start, length > 0.0) else {
                continue;
            };
            let scale = select(app);
            let moved = direction(scale).and_then(|direction| {
                let delta = f64::from((to[axis] - from[axis]) / length) * direction;
                let at_press = scale.clone().with_domain(start.domain().clone());
                at_press.pan(delta)
            });
            match moved {
                Ok(moved) => {
                    *scale = moved;
                    changed = true;
                }
                Err(error) => eprintln!("PanZoom couldn't pan a scale: {error}"),
            }
        }
        changed
    }

    fn zoom(&self, app: &mut State, area: Rect, at: [f32; 2], notches: f64) -> bool {
        let mut changed = false;
        for (axis, select) in self.scales.iter().enumerate() {
            let (start, length) = span(area, axis);
            let (Some(select), true) = (select, length > 0.0) else {
                continue;
            };
            let scale = select(app);
            let fraction = f64::from((at[axis] - start) / length);
            let zoomed = direction(scale).and_then(|direction| {
                let anchor = if direction < 0.0 {
                    1.0 - fraction
                } else {
                    fraction
                };
                // Zooming in by a factor shrinks the domain by it.
                scale.clone().zoom(anchor, notch_factor(-notches))
            });
            match zoomed {
                Ok(zoomed) => {
                    *scale = zoomed;
                    changed = true;
                }
                Err(error) => eprintln!("PanZoom couldn't zoom a scale: {error}"),
            }
        }
        changed
    }

    fn home(&self, app: &mut State, first: Option<&Self::View>) -> bool {
        let Some(first) = first else {
            return false;
        };
        for (select, first) in self.scales.iter().zip(first) {
            if let (Some(select), Some(first)) = (select, first) {
                let scale = select(app);
                *scale = scale.clone().with_domain(first.domain().clone());
            }
        }
        true
    }
}
