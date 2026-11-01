//! Interval brushes, which select a range of each dimension of an area through its projection.

use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_coords::PlotProjection;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;
#[cfg(feature = "scales")]
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;

use super::binding::{Binding, Change};
use super::brush::{
    part_cursor, state, state_mut, Brush, Claim, Drawing, ErasedBrush, Own, HANDLE_TOLERANCE,
};
use super::coords::{Coords, CoordsOf, Dimension};
use super::draw::{
    clipped, distance, handle_marks, joined, local, local_clamped, middle, outline, path_mark,
    stroked, Drawn,
};
use super::style::BrushStyle;

/// The longest step, in pixels, between the places along the pointer's path whose positions a
/// drag adds up along a dimension that wraps. Each step changes a longitude by less than half a
/// turn on any map wider than 20 pixels, so the sum follows the direction of the drag.
const SWEEP_STEP: f32 = 10.0;

/// The ranges that an [`IntervalBrush`] selects, one for each dimension of its area's projection:
/// `x` for the first, such as the x of a rectangle, the angle of a polar plot, or the longitude of
/// a map, and `y` for the second. A range is in the domain of the dimension's scale, or in range
/// positions for a dimension without a scale, such as the degrees of a map, and runs from its
/// first value up to its second. Along a dimension that wraps, a range whose first value exceeds
/// its second runs across the wrap, such as the longitudes `[170, -170]` across the antimeridian,
/// and the range of a whole period spans the dimension's extent, such as `[-180, 180]`. A
/// dimension that the brush does not cover is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Interval {
    pub x: Option<[f64; 2]>,
    pub y: Option<[f64; 2]>,
}

impl Interval {
    /// Whether the interval contains the point `[x, y]`, including its bounds. A range whose first
    /// value exceeds its second contains the values from its first value up and from its second
    /// value down, as a range of longitudes across the antimeridian contains the longitudes from
    /// -180 to 180 inside it. A dimension that is `None` contains every value.
    pub fn contains(&self, [x, y]: [f64; 2]) -> bool {
        let within = |range: Option<[f64; 2]>, value: f64| match range {
            None => true,
            Some([first, second]) if first <= second => first <= value && value <= second,
            Some([first, second]) => value >= first || value <= second,
        };
        within(self.x, x) && within(self.y, y)
    }

    fn range(&self, dimension: usize) -> Option<[f64; 2]> {
        [self.x, self.y][dimension]
    }

    fn set(&mut self, dimension: usize, range: Option<[f64; 2]>) {
        if dimension == 0 {
            self.x = range;
        } else {
            self.y = range;
        }
    }
}

/// The shape that an `xy` [`IntervalBrush`] keeps its boxes at, in range positions, which are
/// pixels in a rectangle. A brush over a dimension that wraps ignores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aspect {
    /// The width-to-height ratio of each area's extent, which is the area's own in a rectangle,
    /// so that a zoom to a box scales both dimensions by one factor.
    Area,
    /// A width-to-height ratio in range positions, such as 1 for squares in a rectangle.
    Ratio(f32),
}

/// Selects a range of each dimension of one area with a box.
///
/// The area's [`PlotProjection`] maps its pixels to the range positions of its two dimensions:
/// the `CartesianProjection` of the area's rectangle by default, whose dimensions are its x and y,
/// or the projection that [`projection`](Self::projection) sets, such as the `PolarProjection` of a
/// polar plot or the `MapProjection` of a map from avenger-coords. An `x` box selects a
/// range of the first dimension and spans the second, a `y` box selects a range of the second and
/// spans the first, and an `xy` box selects both. The brush draws a box's edges along the
/// projection's paths, so a box is a rectangle in a rectangle, an annular sector in a polar plot,
/// and a box between two meridians and two parallels on a map, which a flat map cuts at the
/// antimeridian. The app stores the value: `value` reads the interval, and `on_change` and
/// `on_settle` receive the next interval, or `None` when a click clears the box.
///
/// The scale of a dimension, which `x_scale` and `y_scale` set with the `scales` feature, maps the
/// dimension's data to a range that spans the dimension's extent, in any units, as for
/// `PanZoom`, and must invert, as linear, log, pow, and symlog scales
/// do. A dimension without a scale selects range positions, such as the longitudes and latitudes
/// of a map. Along a dimension that wraps, the range follows the positions that the pointer
/// sweeps, so a drag across the wrap selects the region that it crossed, and the range spans at
/// most one period. A press where the area shows no position, such as outside a globe, draws no
/// box, and a drag off it keeps the last position that the pointer was over. For a brush over
/// several areas, such as the panels of a facet grid, use a [`KeyedIntervalBrush`].
pub struct IntervalBrush<State>(KeyedIntervalBrush<State, ()>);

impl<State: Send + Sync + 'static> IntervalBrush<State> {
    /// Create a brush named `id` that selects ranges of the first dimension.
    pub fn x(id: &str) -> Self {
        Self(KeyedIntervalBrush::x(id))
    }

    /// Create a brush named `id` that selects ranges of the second dimension.
    pub fn y(id: &str) -> Self {
        Self(KeyedIntervalBrush::y(id))
    }

    /// Create a brush named `id` that selects ranges of both dimensions.
    pub fn xy(id: &str) -> Self {
        Self(KeyedIntervalBrush::xy(id))
    }

    /// Set the area where the brush draws its box, in scene coordinates.
    pub fn area(self, area: impl Fn(&State) -> Rect + Send + Sync + 'static) -> Self {
        Self(self.0.areas(move |state| vec![((), area(state))]))
    }

    /// Set the projection of the area, from its top-left corner, or `None` while the area has none,
    /// as [`KeyedIntervalBrush::projection`] describes. Defaults to the Cartesian projection of the area's rectangle.
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

    /// Set how the brush reads its interval from app state, or `None` for no box. The tool draws
    /// this value, so a change in app state outside the brush's callbacks shows when the tool's
    /// scene or a scene below it updates.
    pub fn value(self, value: impl Fn(&State) -> Option<Interval> + Send + Sync + 'static) -> Self {
        Self(self.0.value(move |state, _| value(state)))
    }

    /// Set the callback for each processed move that changes the interval, and for a click that
    /// clears it. Store the value in app state and return the scenes to rebuild, such as the
    /// chart scene without its tree.
    pub fn on_change(
        self,
        on_change: impl Fn(&mut State, Option<Interval>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_change(move |state, _, interval| on_change(state, interval)),
        )
    }

    /// Set the callback for the end of a gesture that changed the interval: a release, a
    /// cancellation, or a click that cleared the box. It follows `on_change` with the same value.
    /// Return the scenes to rebuild, such as the chart scene with its tree.
    pub fn on_settle(
        self,
        on_settle: impl Fn(&mut State, Option<Interval>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        Self(
            self.0
                .on_settle(move |state, _, interval| on_settle(state, interval)),
        )
    }

    /// Set whether the brush draws handles on its box, as
    /// [`KeyedIntervalBrush::handles`] describes. Defaults to true.
    pub fn handles(self, handles: bool) -> Self {
        Self(self.0.handles(handles))
    }

    /// Keep the box of an `xy` brush at `aspect`, as [`KeyedIntervalBrush::aspect`] describes.
    pub fn aspect(self, aspect: Aspect) -> Self {
        Self(self.0.aspect(aspect))
    }

    /// Set when the brush claims presses, such as in a selection mode. Defaults to always. An
    /// inactive brush still draws its box, without handles.
    pub fn active(self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        Self(self.0.active(active))
    }
}

impl<State: Send + Sync + 'static> From<IntervalBrush<State>> for Brush<State> {
    fn from(brush: IntervalBrush<State>) -> Self {
        brush.0.into()
    }
}

/// Selects a range of each dimension of each of a set of areas with a box, as an
/// [`IntervalBrush`] does in one.
///
/// The app names each area with a key of type `A`, such as a panel ID, and stores the values:
/// `value` reads an area's interval, and `on_change` and `on_settle` receive the area's key and
/// its next interval, or `None` when a click clears its box. The bindings choose how the areas
/// combine, such as one selection for the figure, one for each area, or one range that every
/// area shows.
pub struct KeyedIntervalBrush<State, A> {
    id: String,
    /// Whether the brush covers the first dimension and the second.
    covers: [bool; 2],
    binding: Binding<State, A, Option<Interval>>,
    coords_of: CoordsOf<State, A>,
    /// Whether the brush draws handles on its boxes.
    handles: bool,
    aspect: Option<Aspect>,
}

impl<State, A> KeyedIntervalBrush<State, A>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush named `id` that selects ranges of the first dimension.
    pub fn x(id: &str) -> Self {
        Self::new(id, [true, false])
    }

    /// Create a brush named `id` that selects ranges of the second dimension.
    pub fn y(id: &str) -> Self {
        Self::new(id, [false, true])
    }

    /// Create a brush named `id` that selects ranges of both dimensions.
    pub fn xy(id: &str) -> Self {
        Self::new(id, [true, true])
    }

    fn new(id: &str, covers: [bool; 2]) -> Self {
        Self {
            id: id.to_string(),
            covers,
            binding: Binding::default(),
            coords_of: CoordsOf::default(),
            handles: true,
            aspect: None,
        }
    }

    /// Set the areas where the brush draws boxes, each with its key and its rectangle in scene
    /// coordinates. Where areas overlap, a press edits the box of the last one.
    pub fn areas(
        mut self,
        areas: impl Fn(&State) -> Vec<(A, Rect)> + Send + Sync + 'static,
    ) -> Self {
        self.binding.areas = Box::new(areas);
        self
    }

    /// Set the projection of each area, from its top-left corner, or `None` for an area without one,
    /// such as a map without a projection, where the brush draws no box. Defaults to the
    /// rectangle of each area.
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

    /// Set how the brush reads each area's interval from app state, or `None` for no box. The
    /// tool draws this value, so a change in app state outside the brush's callbacks shows
    /// when the tool's scene or a scene below it updates.
    pub fn value(
        mut self,
        value: impl Fn(&State, &A) -> Option<Interval> + Send + Sync + 'static,
    ) -> Self {
        self.binding.value = Box::new(value);
        self
    }

    /// Set the callback for each processed move that changes an area's interval, and for a
    /// click that clears it. Store the value in app state and return the scenes to rebuild,
    /// such as the chart scene without its tree.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, &A, Option<Interval>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_change = Box::new(on_change);
        self
    }

    /// Set the callback for the end of a gesture that changed an area's interval: a release, a
    /// cancellation, or a click that cleared the box. It follows `on_change` with the same
    /// value. Return the scenes to rebuild, such as the chart scene with its tree.
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, &A, Option<Interval>) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.binding.on_settle = Box::new(on_settle);
        self
    }

    /// Set whether the brush draws handles on a box while the pointer is over it or the box is
    /// changing. Defaults to true. A box that the app clears at its release, such as a box zoom's,
    /// needs none.
    pub fn handles(mut self, handles: bool) -> Self {
        self.handles = handles;
        self
    }

    /// Keep the boxes of an `xy` brush at `aspect` in range positions, which are pixels in a
    /// rectangle. A new box grows from the press at that shape to reach the pointer, and shrinks
    /// to stay inside its area's extent. A press on a corner resizes the box at that shape from
    /// the opposite corner, and a press on an edge moves the box. Defaults to any shape. `x` and
    /// `y` brushes and brushes over a dimension that wraps ignore it.
    pub fn aspect(mut self, aspect: Aspect) -> Self {
        self.aspect = Some(aspect);
        self
    }

    /// Set when the brush claims presses, such as in a selection mode. Defaults to always. An
    /// inactive brush still draws its boxes, without handles.
    pub fn active(mut self, active: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.binding.active = Box::new(active);
        self
    }

    /// The coordinates of `key`'s area, or `None` for an area without a projection.
    fn coords(&self, app: &State, key: &A, area: Rect) -> Result<Option<Coords>, String> {
        (self.coords_of.coords(app, key, area)).map_err(|error| {
            format!(
                "interval brush {:?} couldn't map its scales: {error}",
                self.id
            )
        })
    }

    /// The width-to-height ratio in range positions that the brush keeps its boxes at, if any.
    fn ratio(&self, coords: &Coords) -> Option<f64> {
        let aspect = self.aspect.filter(|_| self.covers == [true, true])?;
        let [x, y] = &coords.dimensions;
        if x.period.is_some() || y.period.is_some() {
            return None;
        }
        Some(match aspect {
            Aspect::Area => x.length() / y.length(),
            Aspect::Ratio(ratio) => f64::from(ratio),
        })
    }

    /// The box of `value` in range positions. A dimension that the brush does not cover, or that
    /// the value leaves out, spans the whole dimension.
    fn ranges(&self, coords: &Coords, value: &Interval) -> Result<Ranges, String> {
        let mut ranges = coords
            .dimensions
            .each_ref()
            .map(|dimension| dimension.whole());
        for (index, dimension) in coords.dimensions.iter().enumerate() {
            let Some([first, second]) = value.range(index).filter(|_| self.covers[index]) else {
                continue;
            };
            let [a, b] = [dimension.position(first)?, dimension.position(second)?];
            ranges[index] = match dimension.period {
                None => [a.min(b), a.max(b)],
                Some(period) => {
                    // The range runs up from its first value, so a range across the wrap ends
                    // past the period's end.
                    let (start, end) = if dimension.increasing() {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let width = if end < start {
                        end - start + period
                    } else {
                        end - start
                    };
                    [start, start + width.min(period)]
                }
            };
        }
        Ok(ranges)
    }

    /// The interval of the box at `ranges`. `base` is a box and its value, such as the box at a
    /// press, and along each dimension where the box keeps the range of `base`, the interval
    /// keeps the range of its value exactly.
    fn interval(
        &self,
        coords: &Coords,
        ranges: &Ranges,
        base: Option<(&Ranges, &Interval)>,
    ) -> Result<Interval, String> {
        let mut interval = Interval::default();
        for (index, dimension) in coords.dimensions.iter().enumerate() {
            if !self.covers[index] {
                continue;
            }
            if let Some((_, base)) = base.filter(|(kept, _)| kept[index] == ranges[index]) {
                interval.set(index, base.range(index));
                continue;
            }
            let [low, high] = ranges[index];
            let range = match dimension.period {
                None => {
                    let [a, b] = [dimension.value(low)?, dimension.value(high)?];
                    [a.min(b), a.max(b)]
                }
                Some(period) if high - low >= period => {
                    let [start, end] = dimension.whole();
                    let [a, b] = [dimension.value(start)?, dimension.value(end)?];
                    [a.min(b), a.max(b)]
                }
                Some(period) => {
                    // The range starts within the first period of the extent, and ends past the
                    // extent's end where it runs across the wrap.
                    let [start, end] = dimension.whole();
                    let first = start + (low - start).rem_euclid(period);
                    let mut second = first + (high - low);
                    if second > end {
                        second -= period;
                    }
                    let [a, b] = [dimension.value(first)?, dimension.value(second)?];
                    if dimension.increasing() {
                        [a, b]
                    } else {
                        [b, a]
                    }
                }
            };
            interval.set(index, Some(range));
        }
        Ok(interval)
    }

    /// The edges of the box at `ranges` that its area shows, in order around the box from its low
    /// corner: the low edge of the second dimension, the high edge of the first, the high edge of
    /// the second, and the low edge of the first, each along the part of the other dimension that
    /// the box shows. The edges of a dimension that the brush does not cover, and an edge past
    /// the end of its extent, are left out.
    fn edges(&self, coords: &Coords, ranges: &Ranges) -> Vec<Edge> {
        let [[x0, x1], [y0, y1]] = shown(coords, ranges);
        let sides = [
            (1, -1, [x0, y0], [x1, y0]),
            (0, 1, [x1, y0], [x1, y1]),
            (1, 1, [x1, y1], [x0, y1]),
            (0, -1, [x0, y1], [x0, y0]),
        ];
        (sides.into_iter())
            .filter(|(dimension, side, _, _)| self.shows_edge(coords, ranges, *dimension, *side))
            .map(|(dimension, side, from, to)| Edge {
                dimension,
                side,
                pieces: coords.projection.path(from, to),
            })
            .collect()
    }

    /// Whether the box at `ranges` shows its edge at the `side` end of `dimension`, -1 for the low
    /// end and 1 for the high end.
    fn shows_edge(&self, coords: &Coords, ranges: &Ranges, dimension: usize, side: i8) -> bool {
        let at = ranges[dimension][usize::from(side > 0)];
        self.covers[dimension] && coords.dimensions[dimension].shows(at)
    }

    /// The part of the box at `ranges` under the area's pixel `at`: for each dimension, -1 near
    /// the edge at its low end, 1 near the edge at its high end, and 0 elsewhere, or `None` away
    /// from the box. A press within the handle tolerance of an edge moves that edge, and of two
    /// edges, their corner. A box at a fixed aspect resizes only from its corners, so a press on
    /// one edge moves it.
    fn part(&self, coords: &Coords, ranges: &Ranges, at: [f32; 2]) -> Option<[i8; 2]> {
        let mut part = [0, 0];
        let mut nearest = [f32::INFINITY; 2];
        for edge in self.edges(coords, ranges) {
            let gap = distance(at, &edge.pieces);
            let index = edge.dimension;
            // Between two edges equally near, the edge at the high end wins.
            let nearer = gap < nearest[index] || (gap == nearest[index] && edge.side > 0);
            if gap <= HANDLE_TOLERANCE && nearer {
                nearest[index] = gap;
                part[index] = edge.side;
            }
        }
        if part != [0, 0] {
            if self.ratio(coords).is_some() && (part[0] == 0) != (part[1] == 0) {
                return Some([0, 0]);
            }
            return Some(part);
        }
        let position = coords.projection.position(at)?;
        let shown = shown(coords, ranges);
        let inside = (coords.dimensions.iter().enumerate())
            .all(|(index, dimension)| dimension.contains(shown[index], position[index]));
        inside.then_some([0, 0])
    }

    /// The cursor over `part` of a box at the area's pixel `at`, from the direction on the screen
    /// in which the part moves outward there.
    fn part_cursor(&self, coords: &Coords, part: [i8; 2], at: [f32; 2]) -> CursorIcon {
        if part == [0, 0] {
            return CursorIcon::Move;
        }
        let outward = coords.projection.position(at).and_then(|position| {
            let inward: [f64; 2] = std::array::from_fn(|index| {
                let step = coords.dimensions[index].length() * 1e-3;
                position[index] - f64::from(part[index]) * step
            });
            let [from, to] = [
                coords.projection.pixel(inward)?,
                coords.projection.pixel(position)?,
            ];
            Some([to[0] - from[0], to[1] - from[1]])
        });
        let Some([dx, dy]) = outward else {
            return part_cursor(part);
        };
        if part[0] != 0 && part[1] != 0 {
            if (dx > 0.0) == (dy > 0.0) {
                CursorIcon::NwseResize
            } else {
                CursorIcon::NeswResize
            }
        } else if dx.abs() > dy.abs() {
            CursorIcon::EwResize
        } else {
            CursorIcon::NsResize
        }
    }

    /// The box that a press at `at` would edit, in the last area that has one there.
    fn target(&self, app: &State, at: [f32; 2]) -> Option<Target<A>> {
        (self.binding.pressable_areas(app).into_iter().rev()).find_map(|(key, area)| {
            let value = (self.binding.value)(app, &key)?;
            let coords = self.coords(app, &key, area).ok()??;
            let ranges = self.ranges(&coords, &value).ok()?;
            let part = self.part(&coords, &ranges, local(at, area))?;
            let cursor = self.part_cursor(&coords, part, local(at, area));
            Some(Target {
                key,
                coords,
                value,
                ranges,
                part,
                cursor,
                pointer: local_clamped(at, area),
            })
        })
    }

    /// The first area where a press at `at` would draw a new box, with the press in the area's
    /// pixels and its position, which the area shows.
    fn new_area(&self, app: &State, at: [f32; 2]) -> Option<(A, [f32; 2], [f64; 2])> {
        (self.binding.pressable_areas(app).into_iter()).find_map(|(key, area)| {
            if !area.contains(at) {
                return None;
            }
            let coords = self.coords(app, &key, area).ok()??;
            let pointer = local(at, area);
            let position = coords.projection.position(pointer)?;
            let shown = (coords.dimensions.iter().zip(position))
                .all(|(dimension, position)| dimension.shows(position));
            shown.then_some((key, pointer, position))
        })
    }

    /// The gesture that edits `target`.
    fn edit(&self, app: &State, target: Target<A>) -> Option<Gesture<A>> {
        let Target {
            key,
            coords,
            value,
            ranges,
            part,
            cursor,
            pointer,
        } = target;
        let position = coords.projection.position(pointer)?;
        let change = self.binding.change(app, key);
        let (op, anchor) = if self.ratio(&coords).is_some() && part[0] != 0 && part[1] != 0 {
            // The corner moves from the opposite corner, as a new box grows from its press.
            let opposite = std::array::from_fn(|index| ranges[index][usize::from(part[index] < 0)]);
            (Op::Corner, opposite)
        } else if part == [0, 0] {
            (Op::Move { ranges, value }, position)
        } else {
            let op = Op::Resize {
                part,
                ranges,
                value,
            };
            (op, position)
        };
        Some(Gesture {
            op,
            cursor,
            anchor,
            pointer,
            last: position,
            swept: [0.0; 2],
            change,
        })
    }

    /// The box of `gesture` with the pointer at its last position.
    fn next(&self, coords: &Coords, gesture: &Gesture<A>) -> Ranges {
        let [x, y] = &coords.dimensions;
        let current = [gesture.current(x, 0), gesture.current(y, 1)];
        match &gesture.op {
            Op::New | Op::Corner => {
                let anchor = [x.clamp(gesture.anchor[0]), y.clamp(gesture.anchor[1])];
                if let Some(ratio) = self.ratio(coords) {
                    return at_ratio(anchor, current, [x.extent, y.extent], ratio);
                }
                let mut ranges = [x.whole(), y.whole()];
                for (index, dimension) in coords.dimensions.iter().enumerate() {
                    if !self.covers[index] {
                        continue;
                    }
                    let (from, to) = (anchor[index], current[index]);
                    ranges[index] = match dimension.period {
                        Some(period) => {
                            let swept = gesture.swept[index];
                            let low = from + swept.min(0.0);
                            [low, low + swept.abs().min(period)]
                        }
                        None => [from.min(to), from.max(to)],
                    };
                }
                ranges
            }
            Op::Move { ranges, .. } => {
                let mut next = *ranges;
                for (index, dimension) in coords.dimensions.iter().enumerate() {
                    if !self.covers[index] {
                        continue;
                    }
                    let [low, high] = ranges[index];
                    let delta = match dimension.period {
                        Some(_) => gesture.swept[index],
                        None => {
                            // Keep the box's edges from moving farther outside the extent than
                            // they were.
                            let [start, end] = dimension.extent;
                            (current[index] - gesture.anchor[index])
                                .clamp((start - low).min(0.0), (end - high).max(0.0))
                        }
                    };
                    next[index] = [low + delta, high + delta];
                }
                next
            }
            Op::Resize { part, ranges, .. } => {
                let mut next = *ranges;
                for (index, dimension) in coords.dimensions.iter().enumerate() {
                    if part[index] == 0 {
                        continue;
                    }
                    let [mut low, mut high] = ranges[index];
                    let moved = match dimension.period {
                        Some(_) => [low, high][usize::from(part[index] > 0)] + gesture.swept[index],
                        None => current[index],
                    };
                    if part[index] < 0 {
                        low = moved;
                    } else {
                        high = moved;
                    }
                    if low > high {
                        (low, high) = (high, low);
                    }
                    if let Some(period) = dimension.period {
                        high = high.min(low + period);
                    }
                    next[index] = [low, high];
                }
                next
            }
        }
    }

    /// Follow the pointer to `at` during `gesture`, and report a changed value through
    /// `on_change` if `report`. Return `None` when the value stays the same.
    fn moved(
        &self,
        app: &mut State,
        gesture: &mut Gesture<A>,
        at: [f32; 2],
        report: bool,
    ) -> Option<Rebuild> {
        let key = &gesture.change.area;
        let area = self.binding.rect(app, key)?;
        let coords = self.coords(app, key, area).ok()??;
        gesture.follow(&coords, local_clamped(at, area));
        let ranges = self.next(&coords, gesture);
        let base = match &gesture.op {
            Op::Move { ranges, value } | Op::Resize { ranges, value, .. } => Some((ranges, value)),
            Op::New | Op::Corner => None,
        };
        let next = self.interval(&coords, &ranges, base).ok()?;
        (gesture.change).update(app, &self.binding, Some(next), report)
    }

    /// What the chrome of the box at `ranges` shows in `area`: its fill, its outline along the
    /// edges that it shows, and with `handles`, a handle at each corner between two shown edges
    /// and, without a fixed aspect, at the middle of the longest piece of each shown edge.
    fn view(&self, coords: &Coords, ranges: &Ranges, area: Rect, handles: bool) -> View {
        let shown = shown(coords, ranges);
        let fill = coords.projection.polygon(&ring(coords, &shown));
        let edges = self.edges(coords, ranges);
        let mut points = Vec::new();
        if handles {
            if self.covers == [true, true] {
                for (x, y) in [(-1, -1), (1, -1), (1, 1), (-1, 1)] {
                    let corner = [shown[0][usize::from(x > 0)], shown[1][usize::from(y > 0)]];
                    let both = self.shows_edge(coords, ranges, 0, x)
                        && self.shows_edge(coords, ranges, 1, y);
                    points.extend(both.then(|| coords.projection.pixel(corner)).flatten());
                }
            }
            if self.ratio(coords).is_none() {
                for edge in &edges {
                    let longest = edge.pieces.iter().max_by_key(|piece| piece.len());
                    points.extend(longest.and_then(|piece| middle(piece)));
                }
            }
        }
        let outline = joined(edges.into_iter().flat_map(|edge| edge.pieces));
        View {
            area,
            fill,
            outline,
            handles: points,
        }
    }

    /// The marks of `view`.
    fn marks(&self, view: &View, style: &BrushStyle) -> Vec<SceneMark> {
        if view.fill.is_empty() && view.outline.is_empty() {
            return Vec::new();
        }
        let fill = outline(view.fill.iter().map(|ring| (ring.as_slice(), true)));
        let edges = outline(
            view.outline
                .iter()
                .map(|(line, closes)| (line.as_slice(), *closes)),
        );
        let mut marks = vec![
            path_mark("brush_box", fill, style.fill, None),
            stroked("brush_outline", &edges, style),
        ];
        marks.extend(handle_marks(&view.handles, style));
        vec![clipped(&self.id, view.area, marks)]
    }
}

/// A box in range positions: the range of each dimension from low to high. The range of a
/// dimension that wraps can reach past the end of its extent, and spans at most one period.
type Ranges = [[f64; 2]; 2];

/// The ranges of the box at `ranges` that its area shows: each range of a dimension that does
/// not wrap moved inside the extent.
fn shown(coords: &Coords, ranges: &Ranges) -> Ranges {
    std::array::from_fn(|index| ranges[index].map(|end| coords.dimensions[index].clamp(end)))
}

/// The ring of the box at `shown`, from its low corner around as its edges run, with a vertex at
/// least every quarter period along a dimension that wraps, so that each side runs the short way
/// around.
fn ring(coords: &Coords, shown: &Ranges) -> Vec<[f64; 2]> {
    let [[x0, x1], [y0, y1]] = *shown;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let mut ring = Vec::new();
    for (index, from) in corners.iter().enumerate() {
        let to = corners[(index + 1) % corners.len()];
        let steps = (coords.dimensions.iter().enumerate())
            .filter_map(|(axis, dimension)| {
                let quarter = dimension.period? / 4.0;
                Some(((to[axis] - from[axis]).abs() / quarter).ceil())
            })
            .fold(1.0, f64::max) as usize;
        ring.extend((0..steps).map(|step| {
            let along = step as f64 / steps as f64;
            [0, 1].map(|axis| from[axis] + along * (to[axis] - from[axis]))
        }));
    }
    ring
}

/// The box from `anchor` toward `at`, both inside `extent`, at the width-to-height `ratio`: large
/// enough to reach `at`, and no larger than the room in the extent on that side of `anchor`.
fn at_ratio(anchor: [f64; 2], at: [f64; 2], extent: [[f64; 2]; 2], ratio: f64) -> Ranges {
    let [dx, dy] = [at[0] - anchor[0], at[1] - anchor[1]];
    let (mut width, mut height) = (dx.abs(), dy.abs());
    if width > height * ratio {
        height = width / ratio;
    } else {
        width = height * ratio;
    }
    let room = |axis: usize, delta: f64| {
        let [start, end] = extent[axis];
        if delta < 0.0 {
            anchor[axis] - start
        } else {
            end - anchor[axis]
        }
    };
    // A box without size has room to spare, and `min` skips the NaN of 0 / 0.
    let fit = (room(0, dx) / width).min(room(1, dy) / height).min(1.0);
    let (width, height) = (width * fit, height * fit);
    let side = |start: f64, length: f64, delta: f64| {
        if delta < 0.0 {
            [start - length, start]
        } else {
            [start, start + length]
        }
    };
    [side(anchor[0], width, dx), side(anchor[1], height, dy)]
}

/// One edge of a box: the dimension whose range it ends, its side, -1 for the low end and 1 for
/// the high end, and its pieces in the area's pixels.
struct Edge {
    dimension: usize,
    side: i8,
    pieces: Vec<Vec<[f32; 2]>>,
}

/// An interval brush's gesture, the area whose box is under the pointer, and each area's
/// drawing.
struct IntervalState<A> {
    gesture: Option<Gesture<A>>,
    /// The area whose box a press under the pointer would edit.
    hot: Option<A>,
    drawn: Drawn<A, View>,
}

impl<A> Default for IntervalState<A> {
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
    cursor: CursorIcon,
    /// The position under the press, or for a corner at an aspect, the opposite corner.
    anchor: [f64; 2],
    /// The pointer's last position, in the area's pixels from its top-left corner, moved inside
    /// the area.
    pointer: [f32; 2],
    /// The position under the pointer's last place where the area shows one.
    last: [f64; 2],
    /// How far the pointer has swept along each dimension that wraps since the press.
    swept: [f64; 2],
    change: Change<A, Option<Interval>>,
}

impl<A> Gesture<A> {
    /// Follow the pointer's path to the area's pixel `to` in short steps, however far apart the
    /// moves come, adding up the change of position along each dimension that wraps.
    fn follow(&mut self, coords: &Coords, to: [f32; 2]) {
        let from = self.pointer;
        let steps = ((to[0] - from[0]).hypot(to[1] - from[1]) / SWEEP_STEP).ceil();
        let steps = steps.max(1.0) as usize;
        for step in 1..=steps {
            let along = step as f32 / steps as f32;
            let at = [0, 1].map(|axis| from[axis] + along * (to[axis] - from[axis]));
            let Some(position) = coords.projection.position(at) else {
                continue;
            };
            for (index, dimension) in coords.dimensions.iter().enumerate() {
                if dimension.period.is_some() {
                    self.swept[index] += dimension.short(position[index] - self.last[index]);
                }
            }
            self.last = position;
        }
        self.pointer = to;
    }

    /// The pointer's position along `dimension`, at `index`: the anchor moved by the sweep along
    /// a dimension that wraps, and otherwise the last position, moved inside the extent.
    fn current(&self, dimension: &Dimension, index: usize) -> f64 {
        match dimension.period {
            Some(_) => self.anchor[index] + self.swept[index],
            None => dimension.clamp(self.last[index]),
        }
    }
}

enum Op {
    /// Draw a new box from the press.
    New,
    /// Move the corner under the press, at the brush's aspect, from the opposite corner, which is
    /// the gesture's anchor.
    Corner,
    /// Move the box that was at `ranges` at the press, with the value `value`.
    Move { ranges: Ranges, value: Interval },
    /// Move the edges that `part` names, -1 for a dimension's low edge and 1 for its high edge, of
    /// the box that was at `ranges` at the press, with the value `value`.
    Resize {
        part: [i8; 2],
        ranges: Ranges,
        value: Interval,
    },
}

/// A box that a press would edit, the part of it under the press, and the press in the area's
/// pixels.
struct Target<A> {
    key: A,
    coords: Coords,
    value: Interval,
    ranges: Ranges,
    part: [i8; 2],
    cursor: CursorIcon,
    pointer: [f32; 2],
}

/// What an area's chrome shows, in the area's pixels from its top-left corner. The brush draws
/// the area again when it changes.
#[derive(Clone, PartialEq)]
struct View {
    area: Rect,
    /// The rings of the box's fill, or none without a box.
    fill: Vec<Vec<[f32; 2]>>,
    /// The lines of the box's outline, each with whether it closes.
    outline: Vec<(Vec<[f32; 2]>, bool)>,
    handles: Vec<[f32; 2]>,
}

impl View {
    fn empty(area: Rect) -> Self {
        Self {
            area,
            fill: Vec::new(),
            outline: Vec::new(),
            handles: Vec::new(),
        }
    }
}

impl<State, A> ErasedBrush<State> for KeyedIntervalBrush<State, A>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    fn new_state(&self) -> Own {
        Box::new(IntervalState::<A>::default())
    }

    fn claim(&self, app: &State, at: [f32; 2]) -> Option<Claim> {
        if let Some(target) = self.target(app, at) {
            return Some(Claim::Edit(target.cursor));
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
        state_mut::<IntervalState<A>>(own).gesture = match claim {
            Claim::Edit(_) => (self.target(app, at)).and_then(|target| self.edit(app, target)),
            Claim::New => self
                .new_area(app, at)
                .map(|(key, pointer, position)| Gesture {
                    op: Op::New,
                    cursor: CursorIcon::Crosshair,
                    anchor: position,
                    pointer,
                    last: position,
                    swept: [0.0; 2],
                    change: self.binding.change(app, key),
                }),
        };
    }

    fn drag(&self, app: &mut State, own: &mut Own, at: [f32; 2], report: bool) -> Option<Rebuild> {
        let gesture = state_mut::<IntervalState<A>>(own).gesture.as_mut()?;
        self.moved(app, gesture, at, report)
    }

    fn flush(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<IntervalState<A>>(own).gesture.as_mut()?;
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
        let mut gesture = state_mut::<IntervalState<A>>(own).gesture.take()?;
        if click {
            return Some(gesture.change.clear(app, &self.binding));
        }
        self.moved(app, &mut gesture, at, false);
        Some(gesture.change.settle(app, &self.binding))
    }

    fn cancel(&self, app: &mut State, own: &mut Own) -> Option<Rebuild> {
        let gesture = state_mut::<IntervalState<A>>(own).gesture.take()?;
        Some(gesture.change.cancel(app, &self.binding))
    }

    fn cursor(&self, own: &Own) -> Option<CursorIcon> {
        let gesture = state::<IntervalState<A>>(own).gesture.as_ref()?;
        Some(gesture.cursor)
    }

    fn hover(&self, app: &State, own: &mut Own, at: Option<[f32; 2]>) -> bool {
        let hot = at
            .and_then(|at| self.target(app, at))
            .map(|target| target.key);
        let own = state_mut::<IntervalState<A>>(own);
        let changed = own.hot != hot;
        own.hot = hot;
        changed
    }

    fn draw(&self, app: &State, own: &mut Own, style: &BrushStyle) -> Drawing {
        let own = state_mut::<IntervalState<A>>(own);
        let active = (self.binding.active)(app);
        let mut drawing = Drawing::default();
        let mut views = Vec::new();
        for (key, area) in self.binding.usable_areas(app) {
            let change = (own.gesture.as_ref())
                .map(|gesture| &gesture.change)
                .filter(|change| change.area == key);
            // A value that a change interval keeps from the app shows while it waits.
            let kept = change.and_then(Change::kept).copied();
            let value = kept.unwrap_or_else(|| (self.binding.value)(app, &key));
            let shown = change.is_some() || own.hot.as_ref() == Some(&key);
            let handles = self.handles && active && shown;
            let mapped = value.map(|value| {
                let coords = self.coords(app, &key, area)?;
                let Some(coords) = coords else {
                    return Ok(None);
                };
                let ranges = self.ranges(&coords, &value).map_err(|error| {
                    format!(
                        "interval brush {:?} couldn't map its value: {error}",
                        self.id
                    )
                })?;
                let finite = ranges.as_flattened().iter().all(|end| end.is_finite());
                Ok(finite.then(|| self.view(&coords, &ranges, area, handles)))
            });
            let view = match mapped {
                Some(Ok(Some(view))) => view,
                Some(Err(error)) => {
                    drawing.error.get_or_insert(error);
                    View::empty(area)
                }
                _ => View::empty(area),
            };
            views.push((key, view));
        }
        (own.drawn).update(views, &mut drawing, |view| self.marks(view, style));
        drawing
    }
}

impl<State, A> From<KeyedIntervalBrush<State, A>> for Brush<State>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    fn from(brush: KeyedIntervalBrush<State, A>) -> Self {
        Brush(Arc::new(brush))
    }
}
