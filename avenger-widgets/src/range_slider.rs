//! A slider with two thumbs that sets a range of `f64` values within a range.

use std::ops::RangeInclusive;
use std::sync::Arc;

use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_format::{FormatValues, PrecisionSharing, TickStep};
use avenger_layout::{Direction, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{bind, LabelError, LabelValue, LabelValues};

use crate::binding::{self, Get, Set};
use crate::draw::{focus_ring, label_color, rect, Text};
use crate::slider::{moves, readout_gap, round_to};
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// A slider with two thumbs over a range, which set the low and the high end of a range of values,
/// with an optional label above its track and an optional readout of both ends beside it. The
/// thumbs cannot pass each other.
///
/// A left-button press on the track moves the nearer thumb to the pointer, and dragging moves it.
/// Arrow keys move the thumb that moved last by one step, Page Up and Page Down by ten steps, and
/// Home and End move it to an endpoint, as far as the other thumb allows. Canceling a drag,
/// including with Escape, restores its starting range.
pub struct RangeSlider<State> {
    id: String,
    label: Option<Text>,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    value: Option<Get<State, [f64; 2]>>,
    on_change: Option<Set<State, [f64; 2]>>,
    on_settle: Option<Set<State, [f64; 2]>>,
    readout: Option<String>,
    enabled: Option<Get<State, bool>>,
}

impl<State> RangeSlider<State> {
    /// Create a range slider named `id` over `range`, without a label.
    pub fn new(id: &str, range: RangeInclusive<f64>) -> Self {
        Self {
            id: id.to_string(),
            label: None,
            range,
            step: None,
            value: None,
            on_change: None,
            on_settle: None,
            readout: None,
            enabled: None,
        }
    }

    /// Show the plain-text `label` above the track.
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(Text::plain(label));
        self
    }

    /// Snap new values to the range's start, each `step` after it, or its end.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Read the low and the high end from app state. Defaults to the whole range.
    pub fn value(mut self, value: impl Fn(&State) -> [f64; 2] + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Update app state for each changed range and return the scenes to rebuild.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, [f64; 2]) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Run a callback once a gesture that changed the range settles, passing the final range.
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, [f64; 2]) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_settle = Some(Arc::new(on_settle));
        self
    }

    /// Show both ends using the label engine's number provider and `pattern`, as a slider's
    /// readout does.
    pub fn readout(mut self, pattern: &str) -> Self {
        self.readout = Some(pattern.to_string());
        self
    }

    /// Read the slider's enabled state from app state. Defaults to enabled.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn current(&self, app: &State) -> [f64; 2] {
        let whole = [*self.range.start(), *self.range.end()];
        let [low, high] = self.value.as_ref().map_or(whole, |value| value(app));
        [low.min(high), low.max(high)]
    }

    /// Clamp `value` to the range, snapped to its step.
    fn normalize(&self, value: f64) -> f64 {
        let (start, end) = (*self.range.start(), *self.range.end());
        let value = value.clamp(start, end);
        let Some(step) = self.step.filter(|step| *step > 0.0) else {
            return value;
        };
        let lower = start + ((value - start) / step).floor() * step;
        let upper = (lower + step).min(end);
        if value - lower < upper - value {
            lower
        } else {
            upper
        }
    }

    /// The size of one keyboard step.
    fn unit(&self) -> f64 {
        let (start, end) = (*self.range.start(), *self.range.end());
        (self.step.filter(|step| *step > 0.0)).unwrap_or((end - start) / 100.0)
    }

    /// The range with the thumb `thumb`, 0 for the low end and 1 for the high end, at `value`, as
    /// far as the other thumb allows.
    fn moved(&self, current: [f64; 2], thumb: usize, value: f64) -> [f64; 2] {
        let value = self.normalize(value);
        match thumb {
            0 => [value.min(current[1]), current[1]],
            _ => [current[0], value.max(current[0])],
        }
    }

    /// Assign `range`. Call `on_change` only if it differs. Return whether it changed and the
    /// scenes to rebuild.
    fn assign(&self, range: [f64; 2], app: &mut State) -> (bool, Rebuild) {
        match &self.on_change {
            Some(on_change) if range != self.current(app) => (true, on_change(app, range)),
            _ => (false, Rebuild::none()),
        }
    }

    fn settle(&self, app: &mut State) -> Rebuild {
        match &self.on_settle {
            Some(on_settle) => {
                let range = self.current(app);
                on_settle(app, range)
            }
            None => Rebuild::none(),
        }
    }

    /// The readout of `range`, if the slider shows one: both ends, with the precision of the
    /// step.
    fn readout_text(
        &self,
        range: [f64; 2],
        cx: &WidgetContext<'_>,
    ) -> Result<Option<Text>, LabelError> {
        let Some(pattern) = &self.readout else {
            return Ok(None);
        };
        let start = *self.range.start();
        let step = self.unit();
        let grid = TickStep::infer(&[start, start + step]);
        let shown = range.map(|value| match grid {
            Some(grid) => round_to(value, grid.resolution),
            None => value,
        });
        let formatted = cx.engine.format(
            FormatValues::Numbers(&[shown[0], shown[1], shown[0] + step]),
            pattern,
            PrecisionSharing::Shared,
        )?;
        let mut values = formatted.into_iter();
        let low = values.next().unwrap_or(LabelValue::None);
        let high = values.next().unwrap_or(LabelValue::None);
        let markup = bind(
            "#low – #high",
            &LabelValues::from([("low".to_string(), low), ("high".to_string(), high)]),
        )?;
        Ok(Some(Text::markup(markup)))
    }

    /// The maximum readout width and height at the range's endpoints.
    fn readout_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        let whole = [*self.range.start(), *self.range.end()];
        match self.readout_text(whole, cx) {
            Ok(Some(text)) => text.size(cx),
            _ => [0.0; 2],
        }
    }

    fn label_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        match &self.label {
            Some(label) => {
                let [width, height] = label.size(cx);
                [width, height + cx.style.label_gap]
            }
            None => [0.0; 2],
        }
    }

    /// The top and the height of the row of the track, and the bounds of the thumbs' centers.
    fn parts(&self, rect: Rect, cx: &WidgetContext<'_>) -> ([f32; 2], [f32; 2], f32) {
        let style = cx.style;
        let [_, label_height] = self.label_size(cx);
        let readout = self.readout_size(cx);
        let row_top = rect.y + label_height;
        let row_height = style.thumb_size.max(readout[1]);
        let reserved = match self.readout {
            Some(_) => readout_gap(style) + readout[0],
            None => 0.0,
        };
        let half = style.thumb_size / 2.0;
        let track = [
            rect.x + half,
            (rect.right() - reserved - half).max(rect.x + half),
        ];
        ([row_top, row_height], track, rect.right() - readout[0])
    }

    fn value_at(&self, x: f32, [start_x, end_x]: [f32; 2]) -> f64 {
        let along = match end_x > start_x {
            true => f64::from(((x - start_x) / (end_x - start_x)).clamp(0.0, 1.0)),
            false => 0.0,
        };
        let (start, end) = (*self.range.start(), *self.range.end());
        start + along * (end - start)
    }

    fn x_of(&self, value: f64, [start_x, end_x]: [f32; 2]) -> f32 {
        let (start, end) = (*self.range.start(), *self.range.end());
        let along = match end > start {
            true => ((value - start) / (end - start)).clamp(0.0, 1.0) as f32,
            false => 0.0,
        };
        start_x + along * (end_x - start_x)
    }
}

/// A range slider's drag and the thumb that moved last.
#[derive(Debug, Default)]
pub struct RangeSliderState {
    drag: Option<RangeDrag>,
    /// The thumb that arrow keys move: 0 for the low end, 1 for the high end.
    thumb: usize,
    key_changed: bool,
}

#[derive(Debug)]
struct RangeDrag {
    start: [f64; 2],
    thumb: usize,
    changed: bool,
    canceled: bool,
}

/// What a range slider shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeSliderView {
    range: [f64; 2],
    dragging: Option<usize>,
    thumb: usize,
}

impl<State: Send + Sync + 'static> Widget<State> for RangeSlider<State> {
    type State = RangeSliderState;
    type View = RangeSliderView;

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let style = cx.style;
        let [label_width, label_height] = self.label_size(cx);
        let readout = self.readout_size(cx);
        match direction {
            Direction::Horizontal => {
                let reserved = match self.readout {
                    Some(_) => readout_gap(style) + readout[0],
                    None => 0.0,
                };
                label_width.max(style.track_length[0] + style.thumb_size + reserved)
            }
            Direction::Vertical => label_height + style.thumb_size.max(readout[1]),
        }
    }

    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> avenger_layout::Measurement {
        let style = cx.style;
        let [label_width, _] = self.label_size(cx);
        let reserved = match self.readout {
            Some(_) => readout_gap(style) + self.readout_size(cx)[0],
            None => 0.0,
        };
        let track = style.track_length[1] + style.thumb_size + reserved;
        avenger_layout::Measurement::always(label_width.max(track))
    }

    fn view(&self, app: &State, own: &RangeSliderState) -> RangeSliderView {
        RangeSliderView {
            range: self.current(app),
            dragging: (own.drag.as_ref())
                .filter(|drag| !drag.canceled)
                .map(|drag| drag.thumb),
            thumb: own.thumb,
        }
    }

    fn draw(
        &self,
        view: &RangeSliderView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let look = placed.look;
        let r = placed.rect;
        let ([row_top, row_height], track, readout_x) = self.parts(r, cx);
        let middle = row_top + row_height / 2.0;
        let [low_x, high_x] = view.range.map(|value| self.x_of(value, track));
        let base = Look {
            hovered: false,
            pressed: false,
            ..look
        };
        let height = style.track_height;
        let top = middle - height / 2.0;
        let size = style.thumb_size;
        let color = label_color(style, look);
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(label) = &self.label {
            label.bounds(cx)?;
            marks.push(
                label
                    .mark("range_slider_label", [r.x, r.y], color, style)
                    .into(),
            );
        }
        marks.push(
            rect(
                "track",
                [track[0], top, track[1] - track[0], height],
                style.neutral.paint(base).border,
                [0.0; 4],
                0.0,
                height / 2.0,
            )
            .into(),
        );
        marks.push(
            rect(
                "track_fill",
                [low_x, top, high_x - low_x, height],
                style.accent.paint(base).fill,
                [0.0; 4],
                0.0,
                height / 2.0,
            )
            .into(),
        );
        for (thumb, x) in [low_x, high_x].into_iter().enumerate() {
            let paint = style.neutral.paint(Look {
                pressed: view.dragging == Some(thumb),
                ..look
            });
            let bounds = [x - size / 2.0, middle - size / 2.0, size, size];
            marks.push(
                rect(
                    "thumb",
                    bounds,
                    paint.fill,
                    paint.border,
                    style.border_width,
                    size / 2.0,
                )
                .into(),
            );
            if look.focus_visible && thumb == view.thumb {
                marks.push(focus_ring(bounds, size / 2.0, style));
            }
        }
        if let Some(readout) = self.readout_text(view.range, cx)? {
            let bounds = readout.bounds(cx)?;
            let at = [readout_x, middle - bounds.height / 2.0];
            marks.push(readout.mark("readout", at, color, style).into());
        }
        Ok(marks)
    }

    fn enabled(&self, app: &State) -> bool {
        binding::enabled(&self.enabled, app)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        _placed: &Placed,
        _app: &State,
        _own: &RangeSliderState,
    ) -> Disposition {
        match event {
            SceneGraphEvent::KeyPress(press) if moves(press.key) => Disposition::Consume,
            SceneGraphEvent::KeyRelease(release) if moves(release.key) => Disposition::Consume,
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                Disposition::Consume
            }
            SceneGraphEvent::KeyPress(_) | SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut RangeSliderState,
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => event,
            WidgetEvent::Cancel => {
                let mut rebuild = self.cancel_drag(app, own);
                let dragged = own.drag.take().is_some_and(|drag| drag.changed);
                if dragged | std::mem::take(&mut own.key_changed) {
                    rebuild.merge(self.settle(app));
                }
                return rebuild;
            }
            WidgetEvent::Blur if std::mem::take(&mut own.key_changed) => return self.settle(app),
            WidgetEvent::Focus | WidgetEvent::Blur => return Rebuild::none(),
        };
        let (row, track, _) = self.parts(placed.rect, cx);
        match event {
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                if !(row[0]..=row[0] + row[1]).contains(&down.position[1]) {
                    return Rebuild::none();
                }
                let current = self.current(app);
                let [low_x, high_x] = current.map(|value| self.x_of(value, track));
                let x = down.position[0];
                // The nearer thumb moves, and the high thumb of two that stand together moves
                // toward a press past them.
                let thumb = match (x - low_x).abs() < (x - high_x).abs() {
                    true => 0,
                    false if low_x == high_x && x < low_x => 0,
                    false => 1,
                };
                own.thumb = thumb;
                own.drag = Some(RangeDrag {
                    start: current,
                    thumb,
                    changed: false,
                    canceled: false,
                });
                self.drag_to(x, track, app, own)
            }
            SceneGraphEvent::CursorMoved(moved)
                if own.drag.as_ref().is_some_and(|drag| !drag.canceled) =>
            {
                self.drag_to(moved.position[0], track, app, own)
            }
            SceneGraphEvent::MouseUp(up) if up.button == MouseButton::Left => {
                match own.drag.take() {
                    Some(drag) if drag.changed => self.settle(app),
                    _ => Rebuild::none(),
                }
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                self.cancel_drag(app, own)
            }
            SceneGraphEvent::KeyPress(press) => {
                let current = self.current(app);
                let at = current[own.thumb];
                let unit = self.unit();
                let value = match press.key {
                    Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => at + unit,
                    Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => at - unit,
                    Key::Named(NamedKey::PageUp) => at + 10.0 * unit,
                    Key::Named(NamedKey::PageDown) => at - 10.0 * unit,
                    Key::Named(NamedKey::Home) => *self.range.start(),
                    Key::Named(NamedKey::End) => *self.range.end(),
                    _ => return Rebuild::none(),
                };
                let (changed, rebuild) = self.assign(self.moved(current, own.thumb, value), app);
                own.key_changed |= changed;
                rebuild
            }
            SceneGraphEvent::KeyRelease(release) if moves(release.key) => {
                match std::mem::take(&mut own.key_changed) {
                    true => self.settle(app),
                    false => Rebuild::none(),
                }
            }
            _ => Rebuild::none(),
        }
    }
}

impl<State> RangeSlider<State> {
    /// Moves the dragged thumb to `x`.
    fn drag_to(
        &self,
        x: f32,
        track: [f32; 2],
        app: &mut State,
        own: &mut RangeSliderState,
    ) -> Rebuild {
        let Some(thumb) = own.drag.as_ref().map(|drag| drag.thumb) else {
            return Rebuild::none();
        };
        let range = self.moved(self.current(app), thumb, self.value_at(x, track));
        let (changed, rebuild) = self.assign(range, app);
        if let Some(drag) = &mut own.drag {
            drag.changed |= changed;
        }
        rebuild
    }

    /// Restores the drag's starting range and ignores pointer movement until release.
    fn cancel_drag(&self, app: &mut State, own: &mut RangeSliderState) -> Rebuild {
        let Some(drag) = own.drag.as_mut().filter(|drag| !drag.canceled) else {
            return Rebuild::none();
        };
        drag.canceled = true;
        let start = drag.start;
        let (changed, rebuild) = self.assign(start, app);
        if let Some(drag) = &mut own.drag {
            drag.changed |= changed;
        }
        rebuild
    }
}
