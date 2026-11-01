//! Lasso brushes, which select the instances inside freehand paths.

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_coords::PlotProjection;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_geometry::geo_types::MultiPolygon;
use avenger_layout::Rect;
#[cfg(feature = "scales")]
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;

use super::binding::{Binding, Change};
use super::brush::{state, state_mut, Brush, Claim, Drawing, ErasedBrush, Own};
use super::coords::{Coords, CoordsOf};
use super::draw::{clipped, local, local_clamped, outline, path_mark, stroked, Drawn};
use super::instances::Instances;
use super::style::BrushStyle;

/// How far the pointer moves, in pixels, before a lasso's path takes another vertex.
const VERTEX_SPACING: f32 = 2.0;

/// The paths that a [`LassoBrush`] drew in an area, and the keys of the instances inside them.
///
/// Each polygon is a ring of vertices in the data of the area's dimensions, as an [`Interval`]
/// holds them: `[x, y]` in the domains of a rectangle's scales, or `[longitude, latitude]` in
/// degrees on a map. Where the area shows no position, as off a globe, a path's region is cut to
/// the part that the area shows, which can leave it in several polygons. Along a dimension that
/// wraps, such as the angle of a whole circle or longitude, each polygon lies within one period
/// of the dimension's data, so a path across the wrap leaves a polygon on each side of it, and a
/// path around the center of a polar plot or a pole of a globe closes through it. A planar test
/// of a point against the polygons therefore tells whether the lasso holds it. A map's rings are
/// also wound as RFC 7946 winds GeoJSON, with the drawn region on the left of each ring, so
/// `avenger_geo::polygon_contains` tests a point against one of them. A point lies inside the
/// lasso when one of its polygons contains it.
///
/// [`Interval`]: super::Interval
#[derive(Clone, Debug, PartialEq)]
pub struct Lasso<K> {
    pub polygons: Vec<Vec<[f64; 2]>>,
    pub keys: Vec<K>,
}

impl<K> Default for Lasso<K> {
    fn default() -> Self {
        Self {
            polygons: Vec::new(),
            keys: Vec::new(),
        }
    }
}

impl<K> Lasso<K> {
    /// Whether the lasso selects nothing.
    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }
}

/// The rings of `region`, each polygon's exterior and holes, in pixels.
fn region_rings(region: &MultiPolygon<f64>) -> Vec<Vec<[f32; 2]>> {
    (region.iter())
        .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors()))
        .map(|ring| ring.coords().map(|at| [at.x as f32, at.y as f32]).collect())
        .collect()
}

/// Selects the instances inside freehand paths in one area.
///
/// A drag draws a path, closed back to its start. As the path grows, the brush reports a
/// [`Lasso`] through `on_change`, with the path in the area's domain and the keys of the
/// instances whose centers lie inside it, and the release reports the last one through
/// `on_settle`. A drag replaces the lasso, and a Shift+drag adds its path and keys to it. A click
/// clears the lasso, and Escape restores the lasso at the press.
///
/// The brush reads the centers of the instances in the scenes below the brushes at the press, so
/// those scenes can redraw the selection during the drag without their trees. Each new vertex
/// adds a triangle from the path's first vertex, and updates the count of turns around each
/// center inside it, so a vertex costs a box check for each center rather than a test against
/// the whole path. A press selects nothing while a target tree is stale, as for a
/// [`PointBrush`](super::PointBrush). A drag keeps its area's rectangle and coordinates from the
/// press, so its polygons and keys agree when the view changes during the drag.
///
/// The area's projection, the Cartesian projection of its rectangle by default, maps its pixels to
/// the range positions of its two dimensions, and the scale of a dimension, with the `scales`
/// feature, maps them to data, as for an [`IntervalBrush`](super::IntervalBrush). A dimension
/// without a scale, such as the longitude of a map, keeps its range positions. For a brush over
/// several areas, use a [`KeyedLassoBrush`].
pub struct LassoBrush<State, K>(KeyedLassoBrush<State, (), K>);

impl<State, K> LassoBrush<State, K>
where
    State: Send + Sync + 'static,
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    /// Create a brush named `id`. Set its area, its coordinates, and the instances that it
    /// selects.
    pub fn new(id: &str) -> Self {
        Self(KeyedLassoBrush::new(id))
    }

    /// Set the area where the brush draws paths, in scene coordinates.
    pub fn area(self, area: impl Fn(&State) -> Rect + Send + Sync + 'static) -> Self {
        Self(self.0.areas(move |state| vec![((), area(state))]))
    }

    /// Set the projection of the area, from its top-left corner, or `None` while the area has none.
    /// Defaults to the area's rectangle.
    pub fn projection(
        self,
        projection: impl Fn(&State) -> Option<Arc<dyn PlotProjection>> + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.projection(move |state, _| projection(state)))
    }

    /// Set the scale of the first dimension.
    #[cfg(feature = "scales")]
    pub fn x_scale(
        self,
        scale: impl Fn(&State) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.x_scale(move |state, _| Some(scale(state))))
    }

    /// Set the scale of the second dimension.
    #[cfg(feature = "scales")]
    pub fn y_scale(
        self,
        scale: impl Fn(&State) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.y_scale(move |state, _| Some(scale(state))))
    }

    /// Set the instances that the brush selects.
    pub fn instances(self, instances: Instances<K>) -> Self {
        Self(self.0.instances(instances))
    }

    /// Set the instances that the brush selects from the app state, as
    /// [`KeyedLassoBrush::instances_from`] describes.
    pub fn instances_from(
        self,
        instances: impl Fn(&State) -> Instances<K> + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.instances_from(instances))
    }

    /// Set how the brush reads its lasso from app state.
    pub fn value(self, value: impl Fn(&State) -> Lasso<K> + Send + Sync + 'static) -> Self {
        Self(self.0.value(move |state, _| value(state)))
    }

    /// Set the callback for each new vertex of a path, and for a click that clears the lasso.
    /// Store the value in app state and return the scenes to rebuild, such as the chart scene
    /// without its tree.
    pub fn on_change(
        self,
        on_change: impl Fn(&mut State, Lasso<K>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_change(move |state, _, lasso| on_change(state, lasso)),
        )
    }

    /// Set the callback for the end of a gesture that changed the lasso: a release, a
    /// cancellation, or a click that cleared it. Return the scenes to rebuild, such as the chart
    /// scene with its tree.
    pub fn on_settle(
        self,
        on_settle: impl Fn(&mut State, Lasso<K>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_settle(move |state, _, lasso| on_settle(state, lasso)),
        )
    }

    /// Set when the brush claims presses, such as in a lasso mode. Defaults to always.
    pub fn active(self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        Self(self.0.active(active))
    }
}

impl<State, K> From<LassoBrush<State, K>> for Brush<State>
where
    State: Send + Sync + 'static,
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    fn from(brush: LassoBrush<State, K>) -> Self {
        brush.0.into()
    }
}

/// Selects the instances inside freehand paths in each of a set of areas, as a [`LassoBrush`]
/// does in one. The app names each area with a key of type `A`, and the callbacks receive the
/// key with the area's lasso.
pub struct KeyedLassoBrush<State, A, K> {
    id: String,
    binding: Binding<State, A, Lasso<K>>,
    coords_of: CoordsOf<State, A>,
    instances: AreaInstancesOf<State, A, K>,
}

/// The instances of each area of a [`KeyedLassoBrush`].
type AreaInstancesOf<State, A, K> = Box<dyn Fn(&State, &A) -> Instances<K> + Send + Sync>;

impl<State, A, K> KeyedLassoBrush<State, A, K>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    /// Create a brush named `id`. Set its areas, their coordinates, and the instances that it
    /// selects.
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            binding: Binding::default(),
            coords_of: CoordsOf::default(),
            instances: Box::new(|_, _| Instances::marks(Vec::<String>::new())),
        }
    }

    /// Set the areas where the brush draws paths, each with its key and its rectangle in scene
    /// coordinates.
    pub fn areas(
        mut self,
        areas: impl Fn(&State) -> Vec<(A, Rect)> + Send + Sync + 'static,
    ) -> Self {
        self.binding.areas = Box::new(areas);
        self
    }

    /// Set the projection of each area, from its top-left corner, or `None` for an area without
    /// one, where the brush draws no path. Defaults to the rectangle of each area.
    pub fn projection(
        mut self,
        projection: impl Fn(&State, &A) -> Option<Arc<dyn PlotProjection>> + Send + Sync + 'static,
    ) -> Self {
        self.coords_of.projection = Some(Box::new(projection));
        self
    }

    /// Set the scale of the first dimension of each area, or `None` for an area whose first
    /// dimension has none, such as the longitude of a map, which selects range positions.
    #[cfg(feature = "scales")]
    pub fn x_scale(
        mut self,
        scale: impl Fn(&State, &A) -> Option<ConfiguredScale> + Send + Sync + 'static,
    ) -> Self {
        self.coords_of.scales[0] = Some(Box::new(scale));
        self
    }

    /// Set the scale of the second dimension of each area, or `None` for an area whose second
    /// dimension has none.
    #[cfg(feature = "scales")]
    pub fn y_scale(
        mut self,
        scale: impl Fn(&State, &A) -> Option<ConfiguredScale> + Send + Sync + 'static,
    ) -> Self {
        self.coords_of.scales[1] = Some(Box::new(scale));
        self
    }

    /// Set the instances that the brush selects.
    pub fn instances(mut self, instances: Instances<K>) -> Self {
        self.instances = Box::new(move |_, _| instances.clone());
        self
    }

    /// Set the instances that the brush selects from the app state, which the brush reads at
    /// each press, so that an app whose marks change, such as an app that swaps its charts,
    /// selects among the marks that it draws.
    pub fn instances_from(
        mut self,
        instances: impl Fn(&State) -> Instances<K> + Send + Sync + 'static,
    ) -> Self {
        self.instances = Box::new(move |state, _| instances(state));
        self
    }

    /// Set the instances that the brush selects in each area, which the brush reads at each
    /// press, as [`KeyedPointBrush::instances`](super::KeyedPointBrush::instances) reads them, so
    /// that each area selects among its own marks, with keys of its own.
    pub fn instances_in(
        mut self,
        instances: impl Fn(&State, &A) -> Instances<K> + Send + Sync + 'static,
    ) -> Self {
        self.instances = Box::new(instances);
        self
    }

    /// Set how the brush reads each area's lasso from app state.
    pub fn value(mut self, value: impl Fn(&State, &A) -> Lasso<K> + Send + Sync + 'static) -> Self {
        self.binding.value = Box::new(value);
        self
    }

    /// Set the callback for each new vertex of a path, and for a click that clears an area's lasso.
    /// Store the value in app state and return the scenes to rebuild, such as the chart scene
    /// without its tree.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, &A, Lasso<K>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_change = Box::new(on_change);
        self
    }

    /// Set the callback for the end of a gesture that changed an area's lasso: a release, a
    /// cancellation, or a click that cleared it. Return the scenes to rebuild, such as the chart
    /// scene with its tree.
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, &A, Lasso<K>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_settle = Box::new(on_settle);
        self
    }

    /// Set when the brush claims presses, such as in a lasso mode. Defaults to always.
    pub fn active(mut self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.binding.active = Box::new(active);
        self
    }

    /// The coordinates of `key`'s area, or `None` for an area without a projection.
    fn coords(&self, app: &State, key: &A, area: Rect) -> Result<Option<Coords>, String> {
        (self.coords_of.coords(app, key, area))
            .map_err(|error| format!("lasso brush {:?} couldn't map its area: {error}", self.id))
    }

    /// The first area where a press at `at` would draw a path.
    fn new_area(&self, app: &State, at: [f32; 2]) -> Option<(A, Rect)> {
        (self.binding.pressable_areas(app).into_iter()).find(|(key, area)| {
            area.contains(at) && matches!(self.coords(app, key, *area), Ok(Some(_)))
        })
    }

    /// Add the vertex at `at` to `gesture`'s path, and set the lasso of the closed path, which
    /// the gesture reports through `on_change` if `report`. Return `None` when the path stays the
    /// same.
    fn grow(
        &self,
        app: &mut State,
        gesture: &mut LassoGesture<A, K>,
        at: [f32; 2],
        report: bool,
    ) -> Option<Rebuild> {
        if !gesture.extend(at) {
            return None;
        }
        let lasso = gesture.lasso();
        let reported =
            lasso.and_then(|lasso| gesture.change.update(app, &self.binding, lasso, report));
        Some(reported.unwrap_or_default())
    }

    /// The marks of `view`.
    fn marks(&self, view: &View, style: &BrushStyle) -> Vec<SceneMark> {
        let mut marks = Vec::new();
        let polygons = view.region.as_ref().map(|region| {
            let rings = region_rings(region);
            outline(rings.iter().map(|ring| (ring.as_slice(), true)))
        });
        if let Some(polygons) = polygons {
            marks.push(path_mark("brush_lasso", polygons.clone(), style.fill, None));
            marks.push(stroked("brush_lasso_outline", &polygons, style));
        }
        // The path draws closed during the drag, as the release closes it.
        if view.path.len() > 1 {
            let path = outline([(view.path.as_slice(), true)]);
            marks.push(path_mark(
                "brush_lasso_path",
                path.clone(),
                style.fill,
                None,
            ));
            marks.push(stroked("brush_lasso_path_outline", &path, style));
        }
        if marks.is_empty() {
            return marks;
        }
        vec![clipped(&self.id, view.area, marks)]
    }
}

/// A lasso brush's path during a drag, and each area's drawing.
struct LassoState<A, K> {
    gesture: Option<LassoGesture<A, K>>,
    drawn: Drawn<A, View>,
}

impl<A, K> Default for LassoState<A, K> {
    fn default() -> Self {
        Self {
            gesture: None,
            drawn: Drawn::default(),
        }
    }
}

struct LassoGesture<A, K> {
    /// The change to the area's lasso. A Shift+drag adds to the lasso at the press.
    change: Change<A, Lasso<K>>,
    /// Whether the path adds to the area's lasso, as a Shift+drag does.
    adds: bool,
    /// The area's rectangle at the press.
    area: Rect,
    /// The area's coordinates at the press.
    coords: Coords,
    /// The path's vertices in the area's pixels from its top-left corner.
    path: Vec<[f32; 2]>,
    /// The target instances in the area at the press, or `None` when a target tree was stale, so
    /// the path selects nothing.
    targets: Option<Targets<K>>,
}

impl<A, K: Clone + Eq + Hash> LassoGesture<A, K> {
    /// Add `at`, moved inside the area, to the path, unless it lies within the vertex spacing of
    /// the last vertex, and count the turns of the closed path around the targets. Return
    /// whether the path grew.
    fn extend(&mut self, at: [f32; 2]) -> bool {
        let at = local_clamped(at, self.area);
        let (first, last) = (self.path[0], self.path[self.path.len() - 1]);
        if (at[0] - last[0]).hypot(at[1] - last[1]) < VERTEX_SPACING {
            return false;
        }
        if let Some(targets) = &mut self.targets {
            targets.add([first, last, at]);
        }
        self.path.push(at);
        true
    }

    /// The lasso of the closed path: its polygons and the keys of the instances inside it, added
    /// to the lasso at the press for a Shift+drag. `None` while the path encloses nothing, or a
    /// target tree was stale at the press.
    fn lasso(&self) -> Option<Lasso<K>> {
        let targets = self.targets.as_ref()?;
        if self.path.len() < 3 {
            return None;
        }
        let polygons = self.coords.enclosed(&self.path);
        if polygons.is_empty() {
            return None;
        }
        let keys = targets.keys();
        if !self.adds {
            return Some(Lasso { polygons, keys });
        }
        let mut lasso = self.change.start.clone();
        lasso.polygons.extend(polygons);
        let mut seen: HashSet<K> = lasso.keys.iter().cloned().collect();
        lasso
            .keys
            .extend(keys.into_iter().filter(|key| seen.insert(key.clone())));
        Some(lasso)
    }
}

/// The keys and centers of the target instances in an area at a press, in the area's pixels from
/// its top-left corner, and how many times a closed path winds around each center.
struct Targets<K> {
    centers: Vec<(K, [f32; 2])>,
    winding: Vec<i32>,
}

impl<K: Clone + Eq + Hash> Targets<K> {
    fn new(centers: Vec<(K, [f32; 2])>) -> Self {
        let winding = vec![0; centers.len()];
        Self { centers, winding }
    }

    /// Count the turns of the triangle `a`, `b`, `c` around each center inside it or on its edge:
    /// one more for a counterclockwise triangle, and one less for a clockwise one. The triangles
    /// from a path's first vertex to each of its edges count the turns of the closed path.
    fn add(&mut self, [a, b, c]: [[f32; 2]; 3]) {
        let cross = |o: [f32; 2], p: [f32; 2], q: [f32; 2]| {
            (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])
        };
        let area = cross(a, b, c);
        if area == 0.0 {
            return;
        }
        let turn = if area > 0.0 { 1 } else { -1 };
        let low = [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])];
        let high = [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])];
        for ((_, at), winding) in self.centers.iter().zip(&mut self.winding) {
            let outside_box =
                at[0] < low[0] || at[0] > high[0] || at[1] < low[1] || at[1] > high[1];
            if outside_box {
                continue;
            }
            let sides = [cross(a, b, *at), cross(b, c, *at), cross(c, a, *at)];
            if sides.iter().all(|side| side * area >= 0.0) {
                *winding += turn;
            }
        }
    }

    /// The keys of the centers that the path winds around, each once, in the order of the
    /// centers.
    fn keys(&self) -> Vec<K> {
        let mut seen = HashSet::new();
        (self.centers.iter().zip(&self.winding))
            .filter(|(_, winding)| **winding != 0)
            .map(|((key, _), _)| key)
            .filter(|key| seen.insert((*key).clone()))
            .cloned()
            .collect()
    }
}

/// What an area's chrome shows, in the area's pixels from its top-left corner. The brush draws
/// the area again when it changes.
#[derive(Clone, PartialEq)]
struct View {
    area: Rect,
    /// The region of the lasso's polygons, or `None` without a lasso.
    region: Option<MultiPolygon<f64>>,
    path: Vec<[f32; 2]>,
}

impl<State, A, K> ErasedBrush<State> for KeyedLassoBrush<State, A, K>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    fn new_state(&self) -> Own {
        Box::new(LassoState::<A, K>::default())
    }

    fn claim(&self, app: &State, at: [f32; 2]) -> Option<Claim> {
        self.new_area(app, at).map(|_| Claim::New)
    }

    /// Start a path at the press, with the centers of the target instances in the area.
    fn press(
        &self,
        app: &State,
        own: &mut Own,
        _claim: Claim,
        at: [f32; 2],
        shift: bool,
        trees: &SceneTrees<'_>,
    ) {
        let gesture = self.new_area(app, at).and_then(|(area, rect)| {
            let coords = self.coords(app, &area, rect).ok()??;
            let centers = (self.instances)(app, &area)
                .centers(trees.below(), rect)
                .ok();
            let centers = centers.map(|centers| {
                (centers.into_iter())
                    .map(|(key, center)| (key, local(center, rect)))
                    .collect()
            });
            Some(LassoGesture {
                change: self.binding.change(app, area),
                adds: shift,
                area: rect,
                coords,
                path: vec![local_clamped(at, rect)],
                targets: centers.map(Targets::new),
            })
        });
        state_mut::<LassoState<A, K>>(own).gesture = gesture;
    }

    fn drag(&self, app: &mut State, own: &mut Own, at: [f32; 2], report: bool) -> Option<Rebuild> {
        let gesture = state_mut::<LassoState<A, K>>(own).gesture.as_mut()?;
        self.grow(app, gesture, at, report)
    }

    fn flush(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<LassoState<A, K>>(own).gesture.as_mut()?;
        gesture.change.report(app, &self.binding)
    }

    /// Close the path at `at` and settle the lasso that it selects. A click clears the area's
    /// lasso.
    fn release(
        &self,
        app: &mut State,
        own: &mut Own,
        at: [f32; 2],
        click: bool,
    ) -> Option<Rebuild> {
        let mut gesture = state_mut::<LassoState<A, K>>(own).gesture.take()?;
        if click {
            return Some(gesture.change.clear(app, &self.binding));
        }
        self.grow(app, &mut gesture, at, false);
        Some(gesture.change.settle(app, &self.binding))
    }

    /// Restore the lassos at the press, if the path reported one, and end the path.
    fn cancel(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<LassoState<A, K>>(own).gesture.take()?;
        Some(gesture.change.cancel(app, &self.binding))
    }

    fn cursor(&self, own: &Own) -> Option<CursorIcon> {
        let own = state::<LassoState<A, K>>(own);
        own.gesture.as_ref().map(|_| CursorIcon::Crosshair)
    }

    /// Draw each area's lasso, or during a drag, the path and, for a Shift+drag, the lasso at the
    /// press.
    fn draw(&self, app: &State, own: &mut Own, style: &BrushStyle) -> Drawing {
        let own = state_mut::<LassoState<A, K>>(own);
        let mut drawing = Drawing::default();
        let mut views = Vec::new();
        for (key, area) in self.binding.usable_areas(app) {
            let gesture = (own.gesture.as_ref()).filter(|gesture| gesture.change.area == key);
            let value;
            let polygons = match gesture {
                Some(gesture) if gesture.adds => &gesture.change.start.polygons,
                Some(_) => &Vec::new(),
                None => {
                    value = (self.binding.value)(app, &key);
                    &value.polygons
                }
            };
            let region = if polygons.is_empty() {
                None
            } else {
                match self.coords(app, &key, area) {
                    Ok(coords) => coords.and_then(|coords| coords.region(polygons)),
                    Err(error) => {
                        drawing.error.get_or_insert(error);
                        None
                    }
                }
            };
            let path = gesture
                .map(|gesture| gesture.path.clone())
                .unwrap_or_default();
            views.push((key, View { area, region, path }));
        }
        (own.drawn).update(views, &mut drawing, |view| self.marks(view, style));
        drawing
    }
}

impl<State, A, K> From<KeyedLassoBrush<State, A, K>> for Brush<State>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
    K: Clone + Eq + Hash + Send + Sync + 'static,
{
    fn from(brush: KeyedLassoBrush<State, A, K>) -> Self {
        Brush(Arc::new(brush))
    }
}
