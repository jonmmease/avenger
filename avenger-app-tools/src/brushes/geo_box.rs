//! Geographic box brushes, which select ranges of longitude and latitude on maps.

use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_geo::paths::geometry_path;
use avenger_geo::{Projection, Projector};
use avenger_geometry::geo_types::{Geometry, LineString, Point, Polygon};
use avenger_layout::Rect;
use avenger_scenegraph::marks::mark::SceneMark;

use super::binding::{Binding, Change};
use super::brush::{
    part_cursor, state, state_mut, Brush, Claim, Drawing, ErasedBrush, Own, HANDLE_TOLERANCE,
};
use super::draw::{
    clipped, handle_marks, local, local_clamped, outline, path_mark, stroked, Drawn,
};
use super::style::BrushStyle;

/// The step between the vertices of a box's edges, in degrees, as a graticule densifies its
/// parallels.
const PRECISION: f64 = 2.5;

/// The longest step, in pixels, between the places along the pointer's path whose longitudes a
/// drag adds up. Each step changes the longitude by less than 180 degrees on any map wider than
/// 20 pixels, so the sum follows the direction of the drag.
const SWEEP_STEP: f32 = 10.0;

/// The latitude where a box's drawing stops short of a pole, where its meridians meet.
const POLE: f64 = 90.0 - 1e-6;

/// The ranges that a [`GeoBoxBrush`] selects, in degrees. The longitude pair runs east from its
/// first value to its second, so a box across the antimeridian has a first value greater than
/// its second, and `[-180, 180]` spans every longitude. The latitude pair runs from south to
/// north. An axis that the brush does not cover is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeoBox {
    pub lon: Option<[f64; 2]>,
    pub lat: Option<[f64; 2]>,
}

impl GeoBox {
    /// Whether the box contains the point `[longitude, latitude]`, in degrees, including its
    /// edges. An axis that is `None` contains every value.
    pub fn contains(&self, point: [f64; 2]) -> bool {
        Span::of(Some(*self)).contains(point)
    }
}

/// A box with its longitude range unwrapped: from `west` east by `width` degrees, and from
/// `south` to `north`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    west: f64,
    width: f64,
    south: f64,
    north: f64,
}

impl Span {
    /// The span of `value`. An axis that the value leaves out covers the sphere.
    fn of(value: Option<GeoBox>) -> Self {
        let (west, width) = match value.and_then(|value| value.lon) {
            Some([west, east]) if east >= west => (west, east - west),
            Some([west, east]) => (west, east - west + 360.0),
            None => (-180.0, 360.0),
        };
        let [south, north] = value.and_then(|value| value.lat).unwrap_or([-90.0, 90.0]);
        Self {
            west,
            width,
            south,
            north,
        }
    }

    fn east(&self) -> f64 {
        self.west + self.width
    }

    fn contains(&self, [lon, lat]: [f64; 2]) -> bool {
        (lon - self.west).rem_euclid(360.0) <= self.width && self.south <= lat && lat <= self.north
    }

    /// The box of the span, on the axes that `covers` names.
    fn geo_box(&self, covers: [bool; 2]) -> GeoBox {
        let lon = covers[0].then(|| {
            if self.width >= 360.0 {
                return [-180.0, 180.0];
            }
            let west = wrap(self.west);
            let east = west + self.width;
            [west, if east > 180.0 { east - 360.0 } else { east }]
        });
        let lat = covers[1].then_some([self.south, self.north]);
        GeoBox { lon, lat }
    }

    /// The latitudes where the drawing stops, short of the poles.
    fn drawn_lats(&self) -> (f64, f64) {
        (self.south.max(-POLE), self.north.min(POLE))
    }

    /// The span's ring in degrees, with its inside on the left, as projection takes it: east
    /// along its south edge, north along its east edge, west along its north edge, and south
    /// along its west edge.
    fn ring(&self) -> Vec<(f64, f64)> {
        let (south, north) = self.drawn_lats();
        let (west, east) = (self.west, self.east());
        let mut ring: Vec<(f64, f64)> = steps(west, east).map(|lon| (lon, south)).collect();
        ring.extend(steps(south, north).skip(1).map(|lat| (east, lat)));
        ring.extend(steps(east, west).skip(1).map(|lon| (lon, north)));
        ring.extend(steps(north, south).skip(1).map(|lat| (west, lat)));
        ring
    }

    /// The edges that a press can move, each with its part: the meridians on the west and the
    /// east for a brush that covers longitude, and the parallels on the south and the north for
    /// one that covers latitude.
    fn edges(&self, covers: [bool; 2]) -> Vec<([i8; 2], LineString<f64>)> {
        let (south, north) = self.drawn_lats();
        let (west, east) = (self.west, self.east());
        let meridian = |lon: f64| steps(south, north).map(|lat| (lon, lat)).collect();
        let parallel = |lat: f64| steps(west, east).map(|lon| (lon, lat)).collect();
        let mut edges = Vec::new();
        if covers[0] {
            edges.extend([([-1, 0], meridian(west)), ([1, 0], meridian(east))]);
        }
        if covers[1] {
            edges.extend([([0, -1], parallel(south)), ([0, 1], parallel(north))]);
        }
        edges
    }

    /// The corners, each with its part: west or east, then south or north.
    fn corners(&self) -> [([i8; 2], (f64, f64)); 4] {
        let (south, north) = self.drawn_lats();
        let (west, east) = (self.west, self.east());
        [
            ([-1, -1], (west, south)),
            ([1, -1], (east, south)),
            ([-1, 1], (west, north)),
            ([1, 1], (east, north)),
        ]
    }
}

/// `lon` from -180 up to 180 degrees.
fn wrap(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Values from `from` to `to`, at most 2.5 degrees apart, with both ends.
fn steps(from: f64, to: f64) -> impl Iterator<Item = f64> {
    let count = ((to - from).abs() / PRECISION).ceil().max(1.0) as usize;
    (0..=count).map(move |step| from + (to - from) * step as f64 / count as f64)
}

/// The pixel of `(lon, lat)`, or `None` where the projection clips it.
fn project_point(projector: &Projector, (lon, lat): (f64, f64)) -> Option<[f32; 2]> {
    match projector.project(&Point::new(lon, lat)) {
        Geometry::Point(point) if point.x().is_finite() && point.y().is_finite() => {
            Some([point.x() as f32, point.y() as f32])
        }
        _ => None,
    }
}

/// The longitude and latitude under the pixel `at`, or `None` where the map is not: where the
/// pixel doesn't invert, or inverts to a place that the projection draws elsewhere, as past the
/// edge of a flat map, where longitudes wrap.
pub(crate) fn place(projector: &Projector, at: [f32; 2]) -> Option<[f64; 2]> {
    let (lon, lat) = projector.invert(f64::from(at[0]), f64::from(at[1]))?;
    let drawn = project_point(projector, (lon, lat))?;
    ((drawn[0] - at[0]).hypot(drawn[1] - at[1]) <= 1.0).then_some([lon, lat])
}

/// The pieces of `line` that the projection draws, in pixels.
fn project_line(projector: &Projector, line: &LineString<f64>) -> Vec<Vec<[f32; 2]>> {
    match projector.project(line) {
        Geometry::MultiLineString(lines) => (lines.0.iter())
            .map(|line| line.coords().map(|c| [c.x as f32, c.y as f32]).collect())
            .collect(),
        _ => Vec::new(),
    }
}

/// The distance from `at` to the nearest segment of `lines`.
fn distance(at: [f32; 2], lines: &[Vec<[f32; 2]>]) -> f32 {
    let segment = |a: [f32; 2], b: [f32; 2]| {
        let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
        let length = dx * dx + dy * dy;
        let along = if length > 0.0 {
            (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (at[0] - a[0] - along * dx).hypot(at[1] - a[1] - along * dy)
    };
    (lines.iter().flat_map(|line| line.windows(2)))
        .map(|pair| segment(pair[0], pair[1]))
        .fold(f32::INFINITY, f32::min)
}

pub(crate) type ProjectionOf<State, A> =
    Box<dyn Fn(&State, &A) -> Option<Projection> + Send + Sync>;

/// Selects ranges of longitude and latitude with a box between two meridians and two parallels
/// on a map.
///
/// A `lon_lat` box selects both ranges, a `lat` box selects a band between two parallels, and a
/// `lon` box selects a slice between two meridians. The brush draws the box through the map's
/// projection, so a globe shows it as a curved quadrilateral and a flat map cuts it at the
/// antimeridian. The gestures and the callbacks are those of an
/// [`IntervalBrush`](super::IntervalBrush), with [`GeoBox`] values.
///
/// The longitude range follows the longitude that the pointer sweeps, so a drag across the
/// antimeridian selects the region that it crossed, and the range spans at most 360 degrees. A
/// press where the map is not, such as outside a globe, draws no box, and a drag off the map
/// keeps the last place that the pointer was over. Latitudes stop at the poles.
///
/// The map draws in its area's coordinates from the area's top-left corner, as for
/// [`MapPanZoom`](crate::navigation::MapPanZoom). For a brush over several maps, use a
/// [`KeyedGeoBoxBrush`].
pub struct GeoBoxBrush<State>(KeyedGeoBoxBrush<State, ()>);

impl<State: Send + Sync + 'static> GeoBoxBrush<State> {
    /// Create a brush named `id` that selects ranges of longitude and latitude.
    pub fn lon_lat(id: &str) -> Self {
        Self(KeyedGeoBoxBrush::lon_lat(id))
    }

    /// Create a brush named `id` that selects bands between two parallels.
    pub fn lat(id: &str) -> Self {
        Self(KeyedGeoBoxBrush::lat(id))
    }

    /// Create a brush named `id` that selects slices between two meridians.
    pub fn lon(id: &str) -> Self {
        Self(KeyedGeoBoxBrush::lon(id))
    }

    /// Set the map's area, in scene coordinates.
    pub fn area(self, area: impl Fn(&State) -> Rect + Send + Sync + 'static) -> Self {
        Self(self.0.areas(move |state| vec![((), area(state))]))
    }

    /// Set the map's projection, or `None` while the map has none.
    pub fn projection(
        self,
        projection: impl Fn(&State) -> Option<Projection> + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.projection(move |state, _| projection(state)))
    }

    /// Set how the brush reads its box from app state, or `None` for no box.
    pub fn value(self, value: impl Fn(&State) -> Option<GeoBox> + Send + Sync + 'static) -> Self {
        Self(self.0.value(move |state, _| value(state)))
    }

    /// Set the callback for each processed move that changes the box, and for a click that
    /// clears it.
    pub fn on_change(
        self,
        on_change: impl Fn(&mut State, Option<GeoBox>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_change(move |state, _, geo_box| on_change(state, geo_box)),
        )
    }

    /// Set the callback for the end of a gesture that changed the box.
    pub fn on_settle(
        self,
        on_settle: impl Fn(&mut State, Option<GeoBox>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_settle(move |state, _, geo_box| on_settle(state, geo_box)),
        )
    }

    /// Set when the brush claims presses, such as in a selection mode. Defaults to always.
    pub fn active(self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        Self(self.0.active(active))
    }
}

impl<State: Send + Sync + 'static> From<GeoBoxBrush<State>> for Brush<State> {
    fn from(brush: GeoBoxBrush<State>) -> Self {
        brush.0.into()
    }
}

/// Selects ranges of longitude and latitude with a box on each of a set of maps, as a
/// [`GeoBoxBrush`] does on one. The app names each map's area with a key of type `A`, and the
/// callbacks receive the key with the area's box.
pub struct KeyedGeoBoxBrush<State, A> {
    id: String,
    /// Whether the brush covers longitude and latitude.
    covers: [bool; 2],
    binding: Binding<State, A, Option<GeoBox>>,
    projection: ProjectionOf<State, A>,
}

impl<State, A> KeyedGeoBoxBrush<State, A>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush named `id` that selects ranges of longitude and latitude.
    pub fn lon_lat(id: &str) -> Self {
        Self::new(id, [true, true])
    }

    /// Create a brush named `id` that selects bands between two parallels.
    pub fn lat(id: &str) -> Self {
        Self::new(id, [false, true])
    }

    /// Create a brush named `id` that selects slices between two meridians.
    pub fn lon(id: &str) -> Self {
        Self::new(id, [true, false])
    }

    fn new(id: &str, covers: [bool; 2]) -> Self {
        Self {
            id: id.to_string(),
            covers,
            binding: Binding::default(),
            projection: Box::new(|_, _| None),
        }
    }

    /// Set the maps' areas, each with its key and its rectangle in scene coordinates. Where
    /// areas overlap, a press edits the box of the last one.
    pub fn areas(
        mut self,
        areas: impl Fn(&State) -> Vec<(A, Rect)> + Send + Sync + 'static,
    ) -> Self {
        self.binding.areas = Box::new(areas);
        self
    }

    /// Set the projection of each map, or `None` where an area has none.
    pub fn projection(
        mut self,
        projection: impl Fn(&State, &A) -> Option<Projection> + Send + Sync + 'static,
    ) -> Self {
        self.projection = Box::new(projection);
        self
    }

    /// Set how the brush reads each area's box from app state, or `None` for no box.
    pub fn value(
        mut self,
        value: impl Fn(&State, &A) -> Option<GeoBox> + Send + Sync + 'static,
    ) -> Self {
        self.binding.value = Box::new(value);
        self
    }

    /// Set the callback for each processed move that changes an area's box, and for a click
    /// that clears it.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, &A, Option<GeoBox>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_change = Box::new(on_change);
        self
    }

    /// Set the callback for the end of a gesture that changed an area's box.
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, &A, Option<GeoBox>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_settle = Box::new(on_settle);
        self
    }

    /// Set when the brush claims presses, such as in a selection mode. Defaults to always.
    pub fn active(mut self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.binding.active = Box::new(active);
        self
    }

    /// The projector of `key`'s map, or `None` for an area without a projection.
    fn projector(&self, app: &State, key: &A) -> Result<Option<Projector>, String> {
        let Some(projection) = (self.projection)(app, key) else {
            return Ok(None);
        };
        (projection.build()).map(Some).map_err(|error| {
            format!(
                "geographic box brush {:?} couldn't build its projection: {error}",
                self.id
            )
        })
    }

    /// The part of the box of `span` under the area's pixel `at`: for each axis, -1 near its
    /// western or southern edge, 1 near its eastern or northern edge, and 0 between them.
    fn part(&self, projector: &Projector, span: &Span, at: [f32; 2]) -> Option<[i8; 2]> {
        if self.covers == [true, true] {
            for (part, corner) in span.corners() {
                let near = project_point(projector, corner)
                    .is_some_and(|p| (p[0] - at[0]).hypot(p[1] - at[1]) <= HANDLE_TOLERANCE);
                if near {
                    return Some(part);
                }
            }
        }
        let nearest = (span.edges(self.covers).into_iter())
            .map(|(part, edge)| (distance(at, &project_line(projector, &edge)), part))
            .filter(|(distance, _)| *distance <= HANDLE_TOLERANCE)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, part)) = nearest {
            return Some(part);
        }
        span.contains(place(projector, at)?).then_some([0, 0])
    }

    /// The box that a press at `at` would edit, in the last area that has one there.
    fn target(&self, app: &State, at: [f32; 2]) -> Option<Target<A>> {
        (self.binding.pressable_areas(app).into_iter().rev()).find_map(|(key, area)| {
            if !area.contains(at) {
                return None;
            }
            let value = (self.binding.value)(app, &key)?;
            let projector = self.projector(app, &key).ok()??;
            let span = Span::of(Some(value));
            let part = self.part(&projector, &span, local(at, area))?;
            Some(Target {
                key,
                area,
                projector,
                span,
                part,
            })
        })
    }

    /// The first area where a press at `at` would draw a new box, with the press in the area's
    /// pixels and the place under it.
    fn new_area(&self, app: &State, at: [f32; 2]) -> Option<(A, [f32; 2], [f64; 2])> {
        (self.binding.pressable_areas(app).into_iter()).find_map(|(key, area)| {
            if !area.contains(at) {
                return None;
            }
            let projector = self.projector(app, &key).ok()??;
            let at = local(at, area);
            Some((key, at, place(&projector, at)?))
        })
    }

    /// The span of `gesture` at its pointer's last place.
    fn next(&self, gesture: &Gesture<A>) -> Span {
        let [_, lat] = gesture.last;
        let lat = lat.clamp(-90.0, 90.0);
        let [lon0, lat0] = gesture.anchor;
        let swept = gesture.swept;
        match gesture.op {
            Op::New => {
                let mut span = Span::of(None);
                if self.covers[0] {
                    span.west = lon0 + swept.min(0.0);
                    span.width = swept.abs().min(360.0);
                }
                if self.covers[1] {
                    span.south = lat0.min(lat);
                    span.north = lat0.max(lat);
                }
                span
            }
            Op::Move(base) => {
                let mut span = base;
                if self.covers[0] {
                    span.west = base.west + swept;
                }
                if self.covers[1] {
                    let delta = (lat - lat0).clamp(-90.0 - base.south, 90.0 - base.north);
                    span.south += delta;
                    span.north += delta;
                }
                span
            }
            Op::Resize(part, base) => {
                let mut span = base;
                match part[0] {
                    -1 => {
                        span.west += swept;
                        span.width -= swept;
                    }
                    1 => span.width += swept,
                    _ => {}
                }
                if span.width < 0.0 {
                    span.west += span.width;
                    span.width = -span.width;
                }
                span.width = span.width.min(360.0);
                match part[1] {
                    -1 => span.south = lat,
                    1 => span.north = lat,
                    _ => {}
                }
                if span.south > span.north {
                    (span.south, span.north) = (span.north, span.south);
                }
                span
            }
        }
    }

    /// Follow the pointer to `at`, moved inside the area, during `gesture`, and report a changed
    /// value through `on_change` if `report`. A pointer off the map leaves the box where it was.
    /// Return `None` when the value stays the same.
    fn moved(
        &self,
        app: &mut State,
        gesture: &mut Gesture<A>,
        at: [f32; 2],
        report: bool,
    ) -> Option<Rebuild> {
        let area = self.binding.rect(app, &gesture.change.area)?;
        let projector = self.projector(app, &gesture.change.area).ok()??;
        // Follow the pointer's path in short steps, however far apart the moves come.
        let (from, to) = (gesture.pointer, local_clamped(at, area));
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / SWEEP_STEP).ceil();
        let steps = steps.max(1.0) as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let at = [0, 1].map(|axis| from[axis] + t * (to[axis] - from[axis]));
            if let Some([lon, lat]) = place(&projector, at) {
                gesture.swept += wrap(lon - gesture.last[0]);
                gesture.last = [lon, lat];
            }
        }
        gesture.pointer = to;
        let next = Some(self.next(gesture).geo_box(self.covers));
        (gesture.change).update(app, &self.binding, next, report)
    }

    /// What the chrome of `span`'s box shows in `area`.
    fn view(&self, projector: &Projector, area: Rect, span: Span, handles: bool) -> View {
        let ring: LineString<f64> = span.ring().into_iter().collect();
        let fill = projector.project(&Polygon::new(ring, Vec::new()));
        let edges: Vec<Vec<Vec<[f32; 2]>>> = (span.edges(self.covers).into_iter())
            .map(|(_, edge)| project_line(projector, &edge))
            .collect();
        let mut points = Vec::new();
        if handles {
            if self.covers == [true, true] {
                let corners = span.corners().into_iter();
                points.extend(corners.filter_map(|(_, corner)| project_point(projector, corner)));
            }
            // A handle at the middle of the longest piece of each edge on the map.
            for pieces in &edges {
                let longest = pieces.iter().max_by_key(|piece| piece.len());
                if let Some(middle) = longest.and_then(|piece| piece.get(piece.len() / 2)) {
                    points.push(*middle);
                }
            }
        }
        View {
            area,
            fill: Some(fill),
            edges: edges.into_iter().flatten().collect(),
            handles: points,
        }
    }

    fn marks(&self, view: &View, style: &BrushStyle) -> Vec<SceneMark> {
        let Some(fill) = &view.fill else {
            return Vec::new();
        };
        let edges = outline(view.edges.iter().map(Vec::as_slice), false);
        let mut marks = vec![
            path_mark("brush_box", geometry_path(fill), style.fill, None),
            stroked("brush_edges", &edges, style),
        ];
        marks.extend(handle_marks(&view.handles, style));
        vec![clipped(&self.id, view.area, marks)]
    }
}

/// A geographic box brush's gesture, the area whose box is under the pointer, and each area's
/// drawing.
struct GeoBoxState<A> {
    gesture: Option<Gesture<A>>,
    /// The area whose box a press under the pointer would edit.
    hot: Option<A>,
    drawn: Drawn<A, View>,
}

impl<A> Default for GeoBoxState<A> {
    fn default() -> Self {
        Self {
            gesture: None,
            hot: None,
            drawn: Drawn::default(),
        }
    }
}

struct Gesture<A> {
    op: Op,
    /// The longitude and latitude under the press.
    anchor: [f64; 2],
    /// The pointer's last position, in the area's pixels from its top-left corner.
    pointer: [f32; 2],
    /// The longitude and latitude under the pointer's last place on the map.
    last: [f64; 2],
    /// How far east the pointer has swept since the press, in degrees of longitude.
    swept: f64,
    change: Change<A, Option<GeoBox>>,
}

#[derive(Clone, Copy)]
enum Op {
    /// Draw a new box from the press.
    New,
    /// Move the box that was at the span at the press.
    Move(Span),
    /// Move the edges that the part names, of the box that was at the span at the press.
    Resize([i8; 2], Span),
}

/// A box that a press would edit, and the part of it under the press.
struct Target<A> {
    key: A,
    area: Rect,
    projector: Projector,
    span: Span,
    part: [i8; 2],
}

/// What an area's chrome shows, in the area's pixels from its top-left corner. The brush draws
/// the area again when it changes.
#[derive(Clone, PartialEq)]
struct View {
    area: Rect,
    /// The box's projected fill, or `None` without a box.
    fill: Option<Geometry<f64>>,
    /// The pieces of the box's edges that the projection draws.
    edges: Vec<Vec<[f32; 2]>>,
    handles: Vec<[f32; 2]>,
}

impl<State, A> ErasedBrush<State> for KeyedGeoBoxBrush<State, A>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    fn new_state(&self) -> Own {
        Box::new(GeoBoxState::<A>::default())
    }

    fn claim(&self, app: &State, at: [f32; 2]) -> Option<Claim> {
        if let Some(target) = self.target(app, at) {
            let [lon, lat] = target.part;
            return Some(Claim::Edit(part_cursor([lon, -lat])));
        }
        self.new_area(app, at).map(|_| Claim::New)
    }

    fn press(
        &self,
        app: &State,
        own: &mut Own,
        claim: Claim,
        at: [f32; 2],
        _shift: bool,
        _trees: &SceneTrees<'_>,
    ) {
        state_mut::<GeoBoxState<A>>(own).gesture = match claim {
            Claim::Edit(_) => self.target(app, at).and_then(|target| {
                let pointer = local(at, target.area);
                let anchor = place(&target.projector, pointer)?;
                let op = match target.part {
                    [0, 0] => Op::Move(target.span),
                    part => Op::Resize(part, target.span),
                };
                Some(Gesture {
                    op,
                    anchor,
                    pointer,
                    last: anchor,
                    swept: 0.0,
                    change: self.binding.change(app, target.key),
                })
            }),
            Claim::New => self
                .new_area(app, at)
                .map(|(area, pointer, anchor)| Gesture {
                    op: Op::New,
                    anchor,
                    pointer,
                    last: anchor,
                    swept: 0.0,
                    change: self.binding.change(app, area),
                }),
        };
    }

    fn drag(&self, app: &mut State, own: &mut Own, at: [f32; 2], report: bool) -> Option<Rebuild> {
        let gesture = state_mut::<GeoBoxState<A>>(own).gesture.as_mut()?;
        self.moved(app, gesture, at, report)
    }

    fn flush(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<GeoBoxState<A>>(own).gesture.as_mut()?;
        gesture.change.report(app, &self.binding)
    }

    /// End the gesture at `at`. A click clears the area's box. A drag settles its last value.
    fn release(
        &self,
        app: &mut State,
        own: &mut Own,
        at: [f32; 2],
        click: bool,
    ) -> Option<Rebuild> {
        let mut gesture = state_mut::<GeoBoxState<A>>(own).gesture.take()?;
        if click {
            return Some(gesture.change.clear(app, &self.binding));
        }
        self.moved(app, &mut gesture, at, false);
        Some(gesture.change.settle(app, &self.binding))
    }

    fn cancel(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<GeoBoxState<A>>(own).gesture.take()?;
        Some(gesture.change.cancel(app, &self.binding))
    }

    fn cursor(&self, own: &Own) -> Option<CursorIcon> {
        let gesture = state::<GeoBoxState<A>>(own).gesture.as_ref()?;
        Some(match gesture.op {
            Op::New => CursorIcon::Crosshair,
            Op::Move(_) => CursorIcon::Move,
            Op::Resize([lon, lat], _) => part_cursor([lon, -lat]),
        })
    }

    fn hover(&self, app: &State, own: &mut Own, at: Option<[f32; 2]>) -> bool {
        let hot = at
            .and_then(|at| self.target(app, at))
            .map(|target| target.key);
        let own = state_mut::<GeoBoxState<A>>(own);
        let changed = own.hot != hot;
        own.hot = hot;
        changed
    }

    fn draw(&self, app: &State, own: &mut Own, style: &BrushStyle) -> Drawing {
        let own = state_mut::<GeoBoxState<A>>(own);
        let active = (self.binding.active)(app);
        let mut drawing = Drawing::default();
        let mut views = Vec::new();
        for (key, area) in self.binding.usable_areas(app) {
            let change = (own.gesture.as_ref())
                .map(|gesture| &gesture.change)
                .filter(|change| change.area == key);
            let handles = active && (change.is_some() || own.hot.as_ref() == Some(&key));
            let projector = match self.projector(app, &key) {
                Ok(projector) => projector,
                Err(error) => {
                    drawing.error.get_or_insert(error);
                    None
                }
            };
            // A value that a change interval keeps from the app shows while it waits.
            let kept = change.and_then(Change::kept).copied();
            let value = kept.unwrap_or_else(|| (self.binding.value)(app, &key));
            let view = match (value, projector) {
                (Some(value), Some(projector)) => {
                    self.view(&projector, area, Span::of(Some(value)), handles)
                }
                _ => View {
                    area,
                    fill: None,
                    edges: Vec::new(),
                    handles: Vec::new(),
                },
            };
            views.push((key, view));
        }
        (own.drawn).update(views, &mut drawing, |view| self.marks(view, style));
        drawing
    }
}

impl<State, A> From<KeyedGeoBoxBrush<State, A>> for Brush<State>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    fn from(brush: KeyedGeoBoxBrush<State, A>) -> Self {
        Brush(Arc::new(brush))
    }
}
