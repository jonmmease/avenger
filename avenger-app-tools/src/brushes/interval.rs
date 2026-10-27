//! Interval brushes, which select ranges of continuous scales.

use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;

use super::axis::{spans, Axes, Axis, ScaleOf};
use super::binding::{Binding, Change};
use super::brush::{
    part_cursor, state, state_mut, Brush, Claim, Drawing, ErasedBrush, Own, HANDLE_TOLERANCE,
};
use super::draw::{box_marks, clipped, local, Drawn, Pixels};
use super::style::BrushStyle;

/// The ranges that an [`IntervalBrush`] selects, in the domains of its scales. Each pair runs
/// from low to high. An axis that the brush does not cover is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Interval {
    pub x: Option<[f64; 2]>,
    pub y: Option<[f64; 2]>,
}

impl Interval {
    /// Whether the interval contains the point `[x, y]`, including its bounds. An axis that is
    /// `None` contains every value.
    pub fn contains(&self, [x, y]: [f64; 2]) -> bool {
        let within = |range: Option<[f64; 2]>, value: f64| {
            range.is_none_or(|[low, high]| low <= value && value <= high)
        };
        within(self.x, x) && within(self.y, y)
    }

    fn axis(&self, axis: usize) -> Option<[f64; 2]> {
        [self.x, self.y][axis]
    }

    /// Set `axis` to the range between the values of `ends`.
    fn set_axis(&mut self, axis: usize, [a, b]: [f64; 2]) {
        let range = Some([a.min(b), a.max(b)]);
        if axis == 0 {
            self.x = range;
        } else {
            self.y = range;
        }
    }
}

/// The shape on the screen that an `xy` [`IntervalBrush`] keeps its boxes at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aspect {
    /// The width-to-height ratio of each box's area, so that a zoom to a box scales both axes by
    /// one factor.
    Area,
    /// A width-to-height ratio in pixels, such as 1 for squares.
    Ratio(f32),
}

/// Selects ranges of continuous scales with a box in one area.
///
/// An `x` box spans the area's height and selects an x range, a `y` box spans its width and
/// selects a y range, and an `xy` box selects both. The app stores the value: `value` reads the
/// interval, and `on_change` and `on_settle` receive the next interval, or `None` when a click
/// clears the box.
///
/// Each scale's range must span the area, in any units, as for
/// [`PanZoom`](crate::navigation::PanZoom), and the scales must invert, as linear, log, pow,
/// and symlog scales do. For a brush over several areas, such as the panels of a facet grid, use
/// a [`KeyedIntervalBrush`].
pub struct IntervalBrush<State>(KeyedIntervalBrush<State, ()>);

impl<State: Send + Sync + 'static> IntervalBrush<State> {
    /// Create a brush named `id` that selects x ranges. Set its x scale with
    /// [`x_scale`](Self::x_scale).
    pub fn x(id: &str) -> Self {
        Self(KeyedIntervalBrush::x(id))
    }

    /// Create a brush named `id` that selects y ranges. Set its y scale with
    /// [`y_scale`](Self::y_scale).
    pub fn y(id: &str) -> Self {
        Self(KeyedIntervalBrush::y(id))
    }

    /// Create a brush named `id` that selects x and y ranges. Set its scales with
    /// [`x_scale`](Self::x_scale) and [`y_scale`](Self::y_scale).
    pub fn xy(id: &str) -> Self {
        Self(KeyedIntervalBrush::xy(id))
    }

    /// Set the area where the brush draws its box, in scene coordinates.
    pub fn area(self, area: impl Fn(&State) -> Rect + Send + Sync + 'static) -> Self {
        Self(self.0.areas(move |state| vec![((), area(state))]))
    }

    /// Set the x scale.
    pub fn x_scale(
        self,
        scale: impl Fn(&State) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.x_scale(move |state, _| scale(state)))
    }

    /// Set the y scale.
    pub fn y_scale(
        self,
        scale: impl Fn(&State) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        Self(self.0.y_scale(move |state, _| scale(state)))
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

    /// Keep the box of an `xy` brush at `aspect` on the screen, as
    /// [`KeyedIntervalBrush::aspect`] describes.
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

/// Selects ranges of continuous scales with a box in each of a set of areas, as an
/// [`IntervalBrush`] does in one.
///
/// The app names each area with a key of type `A`, such as a panel ID, and stores the values:
/// `value` reads an area's interval, and `on_change` and `on_settle` receive the area's key and
/// its next interval, or `None` when a click clears its box. The bindings choose how the areas
/// combine, such as one selection for the figure, one for each area, or one range that every
/// area shows.
pub struct KeyedIntervalBrush<State, A> {
    id: String,
    /// Whether the brush covers the x axis and the y axis.
    covers: [bool; 2],
    binding: Binding<State, A, Option<Interval>>,
    scales: [Option<ScaleOf<State, A>>; 2],
    /// Whether the brush draws handles on its boxes.
    handles: bool,
    aspect: Option<Aspect>,
}

impl<State, A> KeyedIntervalBrush<State, A>
where
    State: Send + Sync + 'static,
    A: Clone + PartialEq + Send + Sync + 'static,
{
    /// Create a brush named `id` that selects x ranges. Set its x scale with
    /// [`x_scale`](Self::x_scale).
    pub fn x(id: &str) -> Self {
        Self::new(id, [true, false])
    }

    /// Create a brush named `id` that selects y ranges. Set its y scale with
    /// [`y_scale`](Self::y_scale).
    pub fn y(id: &str) -> Self {
        Self::new(id, [false, true])
    }

    /// Create a brush named `id` that selects x and y ranges. Set its scales with
    /// [`x_scale`](Self::x_scale) and [`y_scale`](Self::y_scale).
    pub fn xy(id: &str) -> Self {
        Self::new(id, [true, true])
    }

    fn new(id: &str, covers: [bool; 2]) -> Self {
        Self {
            id: id.to_string(),
            covers,
            binding: Binding::default(),
            scales: [None, None],
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

    /// Set the x scale of each area.
    pub fn x_scale(
        mut self,
        scale: impl Fn(&State, &A) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        self.scales[0] = Some(Box::new(scale));
        self
    }

    /// Set the y scale of each area.
    pub fn y_scale(
        mut self,
        scale: impl Fn(&State, &A) -> ConfiguredScale + Send + Sync + 'static,
    ) -> Self {
        self.scales[1] = Some(Box::new(scale));
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

    /// Keep the boxes of an `xy` brush at `aspect` on the screen. A new box grows from the press
    /// at that shape to reach the pointer, and shrinks to stay inside its area. A press on a
    /// corner resizes the box at that shape from the opposite corner, and a press on an edge
    /// moves the box. Defaults to any shape. `x` and `y` brushes ignore it.
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

    /// Map each axis of `area` that the brush covers through its scale.
    fn axes(&self, app: &State, key: &A, area: Rect) -> Result<Axes, String> {
        let spans = spans(area);
        let mut axes = [None, None];
        for (index, covers) in self.covers.into_iter().enumerate() {
            if !covers {
                continue;
            }
            let Some(scale) = &self.scales[index] else {
                let name = ["x", "y"][index];
                return Err(format!("interval brush {:?} has no {name} scale", self.id));
            };
            let axis = Axis::new(scale(app, key), spans[index]).map_err(|e| self.failed(e))?;
            axes[index] = Some(axis);
        }
        Ok(axes)
    }

    fn failed(&self, error: AvengerScaleError) -> String {
        format!(
            "interval brush {:?} couldn't map its scales: {error}",
            self.id
        )
    }

    /// The width-to-height ratio that the brush keeps its boxes at in `area`, if any.
    fn ratio(&self, area: Rect) -> Option<f32> {
        let aspect = self.aspect.filter(|_| self.covers == [true, true])?;
        Some(match aspect {
            Aspect::Area => area.width / area.height,
            Aspect::Ratio(ratio) => ratio,
        })
    }

    /// The box that a press at `at` would edit, in the last area that has one there.
    fn target(&self, app: &State, at: [f32; 2]) -> Option<Target<A>> {
        (self.binding.pressable_areas(app).into_iter().rev()).find_map(|(key, area)| {
            let value = (self.binding.value)(app, &key)?;
            let axes = self.axes(app, &key, area).ok()?;
            let pixels = pixels(&axes, area, &value).ok()?;
            let part = self.part(area, pixels, at)?;
            Some(Target {
                key,
                area,
                axes,
                value,
                pixels,
                part,
            })
        })
    }

    /// The first area where a press at `at` would draw a new box.
    fn new_area(&self, app: &State, at: [f32; 2]) -> Option<A> {
        (self.binding.pressable_areas(app).into_iter())
            .find(|(key, area)| area.contains(at) && self.axes(app, key, *area).is_ok())
            .map(|(key, _)| key)
    }

    /// The part of the box at `pixels` in `area` under `at`: for each axis, -1 near its low
    /// edge, 1 near its high edge, and 0 between them.
    fn part(&self, area: Rect, pixels: Pixels, at: [f32; 2]) -> Option<[i8; 2]> {
        let spans = spans(area);
        let x = locate(at[0], pixels[0], spans[0], self.covers[0])?;
        let y = locate(at[1], pixels[1], spans[1], self.covers[1])?;
        // A box at a fixed aspect resizes from its corners, so a press on an edge moves it.
        if self.ratio(area).is_some() && (x == 0) != (y == 0) {
            return Some([0, 0]);
        }
        Some([x, y])
    }

    /// The gesture that edits `target` from a press at `at`.
    fn edit(&self, app: &State, target: Target<A>, at: [f32; 2]) -> Option<Gesture<A>> {
        let Target {
            key,
            area,
            axes,
            value,
            pixels,
            part,
        } = target;
        let change = self.binding.change(app, key);
        if self.ratio(area).is_some() && part[0] != 0 && part[1] != 0 {
            // The corner moves from the opposite corner, as a new box grows from its press.
            let opposite = |axis: usize| pixels[axis][usize::from(part[axis] < 0)];
            return Some(Gesture {
                op: Op::Corner { part },
                press: [opposite(0), opposite(1)],
                change,
            });
        }
        let base = complete(&axes, pixels, value).ok()?;
        let op = if part == [0, 0] {
            Op::Move { pixels, base }
        } else {
            let fixed = std::array::from_fn(|index| {
                let (axis, [low, high]) = (axes[index].as_ref()?, base.axis(index)?);
                if part[index] == 0 {
                    return None;
                }
                let increasing = axis.pixel(low).ok()? <= axis.pixel(high).ok()?;
                // The edge opposite the moving edge stays.
                Some(if (part[index] < 0) == increasing {
                    high
                } else {
                    low
                })
            });
            Op::Resize { part, base, fixed }
        };
        Some(Gesture {
            op,
            press: at,
            change,
        })
    }

    /// The value of `gesture` with the pointer at `at`, in `area`.
    fn next(
        &self,
        axes: &Axes,
        area: Rect,
        gesture: &Gesture<A>,
        at: [f32; 2],
    ) -> Result<Interval, AvengerScaleError> {
        let spans = spans(area);
        let clamp = |point: [f32; 2], index: usize| {
            let [start, end] = spans[index];
            point[index].clamp(start, end)
        };
        match &gesture.op {
            Op::New | Op::Corner { .. } => {
                let [from, to] = [gesture.press, at].map(|point| [0, 1].map(|i| clamp(point, i)));
                let mut pixels = spans;
                match self.ratio(area) {
                    Some(ratio) => pixels = at_ratio(from, to, area, ratio),
                    None => {
                        for (index, covers) in self.covers.into_iter().enumerate() {
                            if covers {
                                let [a, b] = [from[index], to[index]];
                                pixels[index] = [a.min(b), a.max(b)];
                            }
                        }
                    }
                }
                interval(axes, pixels)
            }
            Op::Move { pixels, base } => {
                let mut next = *base;
                for (index, axis) in axes.iter().enumerate() {
                    let Some(axis) = axis else {
                        continue;
                    };
                    let ([low, high], [start, end]) = (pixels[index], spans[index]);
                    // Keep the box's edges from moving farther outside the area than they were.
                    let delta = (at[index] - gesture.press[index])
                        .clamp((start - low).min(0.0), (end - high).max(0.0));
                    if delta != 0.0 {
                        let ends = [axis.value(low + delta)?, axis.value(high + delta)?];
                        next.set_axis(index, ends);
                    }
                }
                Ok(next)
            }
            Op::Resize { base, fixed, .. } => {
                let mut next = *base;
                for (index, axis) in axes.iter().enumerate() {
                    if let (Some(axis), Some(fixed)) = (axis, fixed[index]) {
                        next.set_axis(index, [fixed, axis.value(clamp(at, index))?]);
                    }
                }
                Ok(next)
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
        let axes = self.axes(app, key, area).ok()?;
        let next = self.next(&axes, area, gesture, at).ok()?;
        (gesture.change).update(app, &self.binding, Some(next), report)
    }

    /// The handles of the box at `pixels` in `area`: on its edges for an `x` or `y` brush, on its
    /// corners for an `xy` brush at a fixed aspect, and on its corners and edges for another `xy`
    /// brush. Edge handles sit at the middle of the box's part inside the area.
    fn handle_points(&self, area: Rect, [[left, right], [top, bottom]]: Pixels) -> Vec<[f32; 2]> {
        let [x_span, y_span] = spans(area);
        let middle =
            |[low, high]: [f32; 2], [start, end]: [f32; 2]| (low.max(start) + high.min(end)) / 2.0;
        let (x, y) = (middle([left, right], x_span), middle([top, bottom], y_span));
        match self.covers {
            [true, false] => vec![[left, y], [right, y]],
            [false, true] => vec![[x, top], [x, bottom]],
            _ if self.ratio(area).is_some() => {
                vec![[left, top], [right, top], [left, bottom], [right, bottom]]
            }
            _ => vec![
                [left, top],
                [x, top],
                [right, top],
                [left, y],
                [right, y],
                [left, bottom],
                [x, bottom],
                [right, bottom],
            ],
        }
    }

    /// The marks of `view`.
    fn marks(&self, view: &View, style: &BrushStyle) -> Vec<SceneMark> {
        let Some(pixels) = view.pixels else {
            return Vec::new();
        };
        let area = view.area;
        let handles = if view.handles {
            self.handle_points(area, pixels)
        } else {
            Vec::new()
        };
        // The chrome draws from the area's top-left corner.
        let handles: Vec<[f32; 2]> = handles.iter().map(|at| local(*at, area)).collect();
        let [[left, right], [top, bottom]] = pixels;
        let pixels = [
            [left - area.x, right - area.x],
            [top - area.y, bottom - area.y],
        ];
        vec![clipped(&self.id, area, box_marks(pixels, &handles, style))]
    }
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
    press: [f32; 2],
    change: Change<A, Option<Interval>>,
}

enum Op {
    /// Draw a new box from the press.
    New,
    /// Move the corner that `part` names, at the brush's aspect, from the opposite corner, which
    /// is the gesture's press.
    Corner { part: [i8; 2] },
    /// Move the box that was at `pixels` at the press. `base` is its value with every covered
    /// axis set, and an axis that does not move keeps its value.
    Move { pixels: Pixels, base: Interval },
    /// Move the edges that `part` names, -1 for an axis's low edge and 1 for its high edge.
    /// `fixed` holds the value of the opposite edge of each moving axis, and the other axes keep
    /// their values in `base`.
    Resize {
        part: [i8; 2],
        base: Interval,
        fixed: [Option<f64>; 2],
    },
}

/// A box that a press would edit, and the part of it under the press.
struct Target<A> {
    key: A,
    area: Rect,
    axes: Axes,
    value: Interval,
    pixels: Pixels,
    part: [i8; 2],
}

/// What an area's chrome shows. The brush draws the area again when it changes.
#[derive(Clone, PartialEq)]
struct View {
    area: Rect,
    /// The box's pixels, or `None` without a box.
    pixels: Option<Pixels>,
    handles: bool,
}

/// The pixels of `value`'s box in `area`. An axis that the brush does not cover, or that the
/// value leaves out, spans the area.
fn pixels(axes: &Axes, area: Rect, value: &Interval) -> Result<Pixels, AvengerScaleError> {
    let mut pixels = spans(area);
    for (index, axis) in axes.iter().enumerate() {
        if let (Some(axis), Some([low, high])) = (axis, value.axis(index)) {
            let [a, b] = [axis.pixel(low)?, axis.pixel(high)?];
            pixels[index] = [a.min(b), a.max(b)];
        }
    }
    Ok(pixels)
}

/// The value of the box at `pixels`, on the covered axes.
fn interval(axes: &Axes, pixels: Pixels) -> Result<Interval, AvengerScaleError> {
    let mut interval = Interval::default();
    for (index, axis) in axes.iter().enumerate() {
        if let Some(axis) = axis {
            let [low, high] = pixels[index];
            interval.set_axis(index, [axis.value(low)?, axis.value(high)?]);
        }
    }
    Ok(interval)
}

/// `value`, whose box is at `pixels`, with each covered axis that it leaves out set to the
/// pixels' range.
fn complete(axes: &Axes, pixels: Pixels, value: Interval) -> Result<Interval, AvengerScaleError> {
    let mut complete = value;
    for (index, axis) in axes.iter().enumerate() {
        if let (Some(axis), None) = (axis, value.axis(index)) {
            let [low, high] = pixels[index];
            complete.set_axis(index, [axis.value(low)?, axis.value(high)?]);
        }
    }
    Ok(complete)
}

/// The box from `anchor` toward `at`, both inside `area`, at the width-to-height `ratio`: large
/// enough to reach `at`, and no larger than the room in `area` on that side of `anchor`.
fn at_ratio(anchor: [f32; 2], at: [f32; 2], area: Rect, ratio: f32) -> Pixels {
    let [dx, dy] = [at[0] - anchor[0], at[1] - anchor[1]];
    let (mut width, mut height) = (dx.abs(), dy.abs());
    if width > height * ratio {
        height = width / ratio;
    } else {
        width = height * ratio;
    }
    let room_x = if dx < 0.0 {
        anchor[0] - area.x
    } else {
        area.right() - anchor[0]
    };
    let room_y = if dy < 0.0 {
        anchor[1] - area.y
    } else {
        area.bottom() - anchor[1]
    };
    // A box without size has room to spare, and `min` skips the NaN of 0 / 0.
    let fit = (room_x / width).min(room_y / height).min(1.0);
    let (width, height) = (width * fit, height * fit);
    let side = |start: f32, length: f32, delta: f32| {
        if delta < 0.0 {
            [start - length, start]
        } else {
            [start, start + length]
        }
    };
    [side(anchor[0], width, dx), side(anchor[1], height, dy)]
}

/// Where `at` lies along one axis of a box from `low` to `high`, in an area from `start` to
/// `end`: -1 near the low edge, 1 near the high edge, 0 between the edges, or `None` outside
/// the box. An axis without `edges` has none, and an edge outside the area is hidden.
fn locate(at: f32, [low, high]: [f32; 2], [start, end]: [f32; 2], edges: bool) -> Option<i8> {
    // Allow for rounding of an edge on the area's border.
    let shown = |edge: f32| edge >= start - 0.5 && edge <= end + 0.5;
    let near = |edge: f32| edges && shown(edge) && (at - edge).abs() <= HANDLE_TOLERANCE;
    match (near(low), near(high)) {
        (true, true) => Some(if at - low < high - at { -1 } else { 1 }),
        (true, false) => Some(-1),
        (false, true) => Some(1),
        (false, false) => (at >= low.max(start) && at <= high.min(end)).then_some(0),
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
            return Some(Claim::Edit(part_cursor(target.part)));
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
            Claim::Edit(_) => (self.target(app, at)).and_then(|target| self.edit(app, target, at)),
            Claim::New => self.new_area(app, at).map(|area| Gesture {
                op: Op::New,
                press: at,
                change: self.binding.change(app, area),
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
        Some(match gesture.op {
            Op::New => CursorIcon::Crosshair,
            Op::Move { .. } => CursorIcon::Move,
            Op::Resize { part, .. } | Op::Corner { part } => part_cursor(part),
        })
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
            let pixels = value.and_then(|value| {
                let mapped = self.axes(app, &key, area).and_then(|axes| {
                    pixels(&axes, area, &value).map_err(|error| self.failed(error))
                });
                match mapped {
                    Ok(pixels) => pixels
                        .as_flattened()
                        .iter()
                        .all(|p| p.is_finite())
                        .then_some(pixels),
                    Err(error) => {
                        drawing.error.get_or_insert(error);
                        None
                    }
                }
            });
            let shown = change.is_some() || own.hot.as_ref() == Some(&key);
            let handles = self.handles && active && shown;
            let view = View {
                area,
                pixels,
                handles,
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
