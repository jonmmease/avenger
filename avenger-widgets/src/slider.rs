//! A slider that sets an `f64` value within a range.

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
use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// A slider with an optional label above its track and an optional readout beside it.
///
/// A left-button press on the track sets the value at the pointer. Dragging updates the value.
/// Arrow keys move the value by one step, Page Up and Page Down by ten steps, and Home and End
/// select the endpoints. Canceling a drag, including with Escape, restores its starting value.
pub struct Slider<State> {
    id: String,
    label: Option<Text>,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    value: Option<Get<State, f64>>,
    on_change: Option<Set<State, f64>>,
    on_settle: Option<Set<State, f64>>,
    readout: Option<Readout>,
    enabled: Option<Get<State, bool>>,
}

/// Number formatting and optional Typst markup for a slider's readout.
struct Readout {
    pattern: String,
    label: Option<String>,
}

impl<State> Slider<State> {
    /// Create a slider named `id` over `range`, without a label.
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

    /// Snap new values to the range's start, each `step` after it, or its end. Without a positive
    /// step, the slider is continuous and arrow keys move by one hundredth of the range.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Read the value from app state. Defaults to the range's start.
    pub fn value(mut self, value: impl Fn(&State) -> f64 + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Update app state for each changed value and return the scenes to rebuild. This includes
    /// pointer and keyboard changes, and restoring a canceled drag's starting value.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, f64) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Run a callback once a gesture that changed the value settles, passing the final value.
    /// A drag settles on release or a `Cancel` notification from the tool. Escape restores its
    /// starting value but waits for one of those events to settle. Keyboard changes settle on
    /// key release, widget blur or cancellation, without restoring the previous value.
    ///
    /// Use this callback for deferred work, such as rebuilding a chart's hit-testing tree.
    /// Return the scenes to rebuild.
    pub fn on_settle(
        mut self,
        on_settle: impl Fn(&mut State, f64) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_settle = Some(Arc::new(on_settle));
        self
    }

    /// Show the value using the label engine's number provider and `pattern`, such as `".1f"`.
    /// An empty pattern uses the provider's default, as with an axis's `tick_format`. A pattern
    /// without a precision takes the precision of the slider's step, or of one hundredth of the
    /// range without a step, as an axis's ticks take the precision of their step. The reserved
    /// width is the larger readout width at the range's endpoints. Wider values can overflow
    /// it, as when a pattern trims trailing zeros.
    pub fn readout(mut self, pattern: &str) -> Self {
        let label = self.readout.take().and_then(|readout| readout.label);
        self.readout = Some(Readout {
            pattern: pattern.to_string(),
            label,
        });
        self
    }

    /// Set Typst markup around the formatted `#value`, such as `#value m/s`, as with an axis's
    /// `tick_label`. Use [`readout`](Slider::readout) to override the provider's default number
    /// format.
    pub fn readout_label(mut self, markup: &str) -> Self {
        let pattern = self
            .readout
            .take()
            .map_or(String::new(), |readout| readout.pattern);
        self.readout = Some(Readout {
            pattern,
            label: Some(markup.to_string()),
        });
        self
    }

    /// Read the slider's enabled state from app state. Defaults to enabled.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn current(&self, app: &State) -> f64 {
        self.value
            .as_ref()
            .map_or(*self.range.start(), |value| value(app))
    }

    /// Clamp `value` to the range. With a positive step, snap to the nearest allowed value,
    /// resolving ties upward.
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

    /// Move `steps` from `value` through the allowed values. Without a positive step size,
    /// each step is one hundredth of the range.
    fn step_from(&self, value: f64, steps: f64) -> f64 {
        let (start, end) = (*self.range.start(), *self.range.end());
        let Some(step) = self.step.filter(|step| *step > 0.0) else {
            return (value + steps * (end - start) / 100.0).clamp(start, end);
        };
        // The admitted values are the start, `whole` steps after it, and the end, unless the
        // last step reaches it.
        let whole = ((end - start) / step).floor();
        let last = match start + whole * step < end - step * 1e-9 {
            true => whole + 1.0,
            false => whole,
        };
        let value = self.normalize(value);
        let index = match value >= end {
            true => last,
            false => ((value - start) / step).round(),
        };
        let index = (index + steps).clamp(0.0, last);
        match index > whole {
            true => end,
            false => start + index * step,
        }
    }

    /// Assign the allowed value nearest `value`. Return whether it changed and the scenes to
    /// rebuild.
    fn set(&self, value: f64, app: &mut State) -> (bool, Rebuild) {
        self.assign(self.normalize(value), app)
    }

    /// Assign `value` without snapping or clamping, for exact restoration after cancellation.
    /// Call `on_change` only if the value differs. Return whether it changed and the requested
    /// rebuild.
    fn assign(&self, value: f64, app: &mut State) -> (bool, Rebuild) {
        match &self.on_change {
            Some(on_change) if value != self.current(app) => (true, on_change(app, value)),
            _ => (false, Rebuild::none()),
        }
    }

    fn settle(&self, app: &mut State) -> Rebuild {
        match &self.on_settle {
            Some(on_settle) => {
                let value = self.current(app);
                on_settle(app, value)
            }
            None => Rebuild::none(),
        }
    }

    /// The readout of `value`, if the slider shows one. As an axis's ticks share the precision
    /// of the step between them, the readout takes the precision of the slider's step, or of one
    /// hundredth of its range without a step, so it keeps its decimals during a drag. A value
    /// that no step admits, which only app state sets, keeps the precision that it needs.
    fn readout_text(&self, value: f64, cx: &WidgetContext<'_>) -> Result<Option<Text>, LabelError> {
        let Some(readout) = &self.readout else {
            return Ok(None);
        };
        let start = *self.range.start();
        let step =
            (self.step.filter(|step| *step > 0.0)).unwrap_or((self.range.end() - start) / 100.0);
        let pattern = &readout.pattern;
        let formatted = match TickStep::infer(&[start, start + step]) {
            Some(grid) if self.normalize(value) == value => {
                let shown = round_to(value, grid.resolution);
                let values = FormatValues::Numbers(&[shown, shown + step]);
                cx.engine
                    .format(values, pattern, PrecisionSharing::Shared)?
            }
            _ => {
                let values = FormatValues::Numbers(&[value]);
                cx.engine
                    .format(values, pattern, PrecisionSharing::PerValue)?
            }
        };
        let formatted = formatted.into_iter().next().unwrap_or(LabelValue::None);
        let bound = |template: &str, value| {
            bind(template, &LabelValues::from([("value".to_string(), value)]))
        };
        Ok(Some(match (&readout.label, formatted) {
            (Some(label), value) => Text::markup(bound(label, value)?),
            (None, LabelValue::Str(text)) => Text::plain(text),
            (None, LabelValue::Markup(markup)) => Text::markup(markup),
            (None, value) => Text::markup(bound("#value", value)?),
        }))
    }

    /// The maximum readout width and height at the range's endpoints. Formatting or typesetting
    /// failures measure as empty.
    fn readout_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        let size = |value: f64| match self.readout_text(value, cx) {
            Ok(Some(text)) => text.size(cx),
            _ => [0.0; 2],
        };
        let [start, end] = [size(*self.range.start()), size(*self.range.end())];
        [start[0].max(end[0]), start[1].max(end[1])]
    }

    /// The label's width, and its height with the gap below it, or zero without a label.
    fn label_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        match &self.label {
            Some(label) => {
                let [width, height] = label.size(cx);
                [width, height + cx.style.label_gap]
            }
            None => [0.0; 2],
        }
    }

    /// The slider's parts in `rect`.
    fn parts(&self, rect: Rect, cx: &WidgetContext<'_>) -> Parts {
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
        Parts {
            row: [row_top, row_height],
            track: [
                rect.x + half,
                (rect.right() - reserved - half).max(rect.x + half),
            ],
            readout_x: rect.right() - readout[0],
        }
    }

    /// The value at `x` along the track.
    fn value_at(&self, x: f32, track: [f32; 2]) -> f64 {
        let [start_x, end_x] = track;
        let along = if end_x > start_x {
            f64::from(((x - start_x) / (end_x - start_x)).clamp(0.0, 1.0))
        } else {
            0.0
        };
        let (start, end) = (*self.range.start(), *self.range.end());
        start + along * (end - start)
    }
}

/// Where a slider's parts lie in its rectangle.
struct Parts {
    /// The top and height of the row that holds the track, the thumb and the readout.
    row: [f32; 2],
    /// Horizontal bounds for the thumb's center, from the range's start to its end.
    track: [f32; 2],
    readout_x: f32,
}

/// A slider's drag state and pending keyboard changes.
#[derive(Debug, Default)]
pub struct SliderState {
    drag: Option<Drag>,
    key_changed: bool,
}

#[derive(Debug)]
struct Drag {
    /// The value at the press, which Escape and a cancellation restore.
    start: f64,
    /// Whether the drag changed the value, including by restoring it.
    changed: bool,
    /// The drag was canceled and ignores pointer movement until release.
    canceled: bool,
}

impl Drag {
    fn active(&self) -> bool {
        !self.canceled
    }
}

/// What a slider shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderView {
    value: f64,
    dragging: bool,
}

impl<State: Send + Sync + 'static> Widget<State> for Slider<State> {
    type State = SliderState;
    type View = SliderView;

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

    /// The larger of the label width and the minimum track width, including thumb and readout.
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

    fn view(&self, app: &State, own: &SliderState) -> SliderView {
        SliderView {
            value: self.current(app),
            dragging: own.drag.as_ref().is_some_and(Drag::active),
        }
    }

    fn draw(
        &self,
        view: &SliderView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let look = placed.look;
        let r = placed.rect;
        let parts = self.parts(r, cx);
        let [row_top, row_height] = parts.row;
        let middle = row_top + row_height / 2.0;
        let [start_x, end_x] = parts.track;
        let (start, end) = (*self.range.start(), *self.range.end());
        let along = if end > start {
            ((view.value - start) / (end - start)).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let thumb_x = start_x + along * (end_x - start_x);
        let base = Look {
            hovered: false,
            pressed: false,
            ..look
        };
        let track = style.neutral.paint(base);
        let filled = style.accent.paint(base);
        let thumb = style.neutral.paint(Look {
            pressed: view.dragging,
            ..look
        });
        let height = style.track_height;
        let top = middle - height / 2.0;
        let size = style.thumb_size;
        let thumb_bounds = [thumb_x - size / 2.0, middle - size / 2.0, size, size];
        let color = label_color(style, look);
        let mut marks: Vec<SceneMark> = Vec::new();
        if let Some(label) = &self.label {
            label.bounds(cx)?;
            marks.push(label.mark("slider_label", [r.x, r.y], color, style).into());
        }
        marks.extend::<[SceneMark; 3]>([
            rect(
                "track",
                [start_x, top, end_x - start_x, height],
                track.border,
                [0.0; 4],
                0.0,
                height / 2.0,
            )
            .into(),
            rect(
                "track_fill",
                [start_x, top, thumb_x - start_x, height],
                filled.fill,
                [0.0; 4],
                0.0,
                height / 2.0,
            )
            .into(),
            rect(
                "thumb",
                thumb_bounds,
                thumb.fill,
                thumb.border,
                style.border_width,
                size / 2.0,
            )
            .into(),
        ]);
        if let Some(readout) = self.readout_text(view.value, cx)? {
            let bounds = readout.bounds(cx)?;
            let at = [parts.readout_x, middle - bounds.height / 2.0];
            marks.push(readout.mark("readout", at, color, style).into());
        }
        if look.focus_visible {
            marks.push(focus_ring(thumb_bounds, size / 2.0, style));
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
        own: &SliderState,
    ) -> Disposition {
        let dragging = own.drag.as_ref().is_some_and(Drag::active);
        match event {
            SceneGraphEvent::KeyPress(press) if moves(press.key) => Disposition::Consume,
            SceneGraphEvent::KeyPress(press)
                if press.key == Key::Named(NamedKey::Escape) && dragging =>
            {
                Disposition::Consume
            }
            SceneGraphEvent::KeyRelease(release) if moves(release.key) => Disposition::Consume,
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
        own: &mut SliderState,
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => event,
            // A press or a held key ended without its release: restore a drag's value at its
            // press, and settle a gesture that changed the value.
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
        match event {
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                let parts = self.parts(placed.rect, cx);
                let [row_top, row_height] = parts.row;
                if !(row_top..=row_top + row_height).contains(&down.position[1]) {
                    return Rebuild::none();
                }
                own.drag = Some(Drag {
                    start: self.current(app),
                    changed: false,
                    canceled: false,
                });
                self.drag_to(down.position[0], parts.track, app, own)
            }
            SceneGraphEvent::CursorMoved(moved) if own.drag.as_ref().is_some_and(Drag::active) => {
                let track = self.parts(placed.rect, cx).track;
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
                let value = match press.key {
                    Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => {
                        self.step_from(current, 1.0)
                    }
                    Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => {
                        self.step_from(current, -1.0)
                    }
                    Key::Named(NamedKey::PageUp) => self.step_from(current, 10.0),
                    Key::Named(NamedKey::PageDown) => self.step_from(current, -10.0),
                    Key::Named(NamedKey::Home) => *self.range.start(),
                    Key::Named(NamedKey::End) => *self.range.end(),
                    _ => return Rebuild::none(),
                };
                let (changed, rebuild) = self.set(value, app);
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

impl<State> Slider<State> {
    /// Set the value at `x` during a drag.
    fn drag_to(&self, x: f32, track: [f32; 2], app: &mut State, own: &mut SliderState) -> Rebuild {
        let (changed, rebuild) = self.set(self.value_at(x, track), app);
        if let Some(drag) = &mut own.drag {
            drag.changed |= changed;
        }
        rebuild
    }

    /// Restore the drag's exact starting value and ignore pointer movement until release.
    fn cancel_drag(&self, app: &mut State, own: &mut SliderState) -> Rebuild {
        let Some(drag) = own.drag.as_mut().filter(|drag| drag.active()) else {
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

/// `value` rounded to a multiple of 10^`exponent`.
fn round_to(value: f64, exponent: i32) -> f64 {
    let unit = 10_f64.powi(exponent.abs());
    if exponent < 0 {
        (value * unit).round() / unit
    } else {
        (value / unit).round() * unit
    }
}

/// The gap between the end of the track and the readout, which holds the thumb's focus ring.
fn readout_gap(style: &WidgetStyle) -> f32 {
    style.label_gap + style.focus_offset + style.focus_width
}

/// Whether `key` moves a slider's value.
fn moves(key: Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowLeft
                | NamedKey::ArrowDown
                | NamedKey::PageUp
                | NamedKey::PageDown
                | NamedKey::Home
                | NamedKey::End
        )
    )
}
