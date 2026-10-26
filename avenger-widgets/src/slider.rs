//! A slider, which sets an `f64` in a range.

use std::ops::RangeInclusive;
use std::sync::Arc;

use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_format::{FormatValues, PrecisionSharing};
use avenger_layout::{Direction, Rect};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{bind, LabelError, LabelValue, LabelValues};

use crate::binding::{self, Get, Set};
use crate::draw::{focus_ring, label_color, rect, Text};
use crate::style::WidgetStyle;
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// A slider with a label above its track, and an optional readout of its value beside it.
///
/// A left-button press on the track sets the value at the pointer, and the drag follows it.
/// Arrow keys move the value by one step, Page Up and Page Down by ten steps, and Home and End
/// select the ends. Escape during a drag, or a canceled drag, restores the value at the press.
pub struct Slider<State> {
    id: String,
    label: Text,
    range: RangeInclusive<f64>,
    step: Option<f64>,
    value: Option<Get<State, f64>>,
    on_change: Option<Set<State, f64>>,
    on_commit: Option<Set<State, f64>>,
    readout: Option<Readout>,
    enabled: Option<Get<State, bool>>,
}

/// How a slider's readout shows its value, as an axis's tick labels show its ticks.
struct Readout {
    pattern: String,
    label: Option<String>,
}

impl<State> Slider<State> {
    /// Create a slider named `id` with the plain-text `label`, over `range`.
    pub fn new(id: &str, label: &str, range: RangeInclusive<f64>) -> Self {
        Self {
            id: id.to_string(),
            label: Text::plain(label),
            range,
            step: None,
            value: None,
            on_change: None,
            on_commit: None,
            readout: None,
            enabled: None,
        }
    }

    /// Admit only the range's start, each `step` after it, and its end. Without it, the slider
    /// admits any value in its range, and its keys step by one hundredth of the range.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Read the value from app state. Without it, the value is the range's start.
    pub fn value(mut self, value: impl Fn(&State) -> f64 + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Change app state for each new value during a drag or from a key, and return the scenes
    /// to rebuild.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, f64) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Change app state once a drag that changed the value ends, or a key that changed it is
    /// released, as for expensive work that should wait for the value to settle.
    pub fn on_commit(
        mut self,
        on_commit: impl Fn(&mut State, f64) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_commit = Some(Arc::new(on_commit));
        self
    }

    /// Show a readout of the value, formatted by the label engine's number provider with
    /// `pattern`, such as `".1f"`, as an axis's `tick_format` is. An empty pattern takes the
    /// provider's default. The readout reserves the wider of its widths at the range's start and
    /// end, so a value whose readout is wider than both, as a pattern without a fixed precision
    /// can produce, draws past it.
    pub fn readout(mut self, pattern: &str) -> Self {
        let label = self.readout.take().and_then(|readout| readout.label);
        self.readout = Some(Readout {
            pattern: pattern.to_string(),
            label,
        });
        self
    }

    /// Set Typst markup for the readout, where `#value` is the formatted value, such as
    /// `#value m/s`, as an axis's `tick_label` is. It applies once [`readout`](Slider::readout)
    /// shows the readout.
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

    /// Read from app state whether the slider takes input. Without it, it always does.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn current(&self, app: &State) -> f64 {
        self.value
            .as_ref()
            .map_or(*self.range.start(), |value| value(app))
    }

    /// The admitted value nearest `value`: in the range, and with a step, the start, a step
    /// after it or the end, with ties upward.
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

    /// The value `steps` steps from `value`, as keys move it: by the step through the admitted
    /// values, or by one hundredth of the range without a step.
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

    /// Set the admitted value nearest `value`, if it differs from the current value. Returns
    /// whether the value changed, and the scenes to rebuild.
    fn set(&self, value: f64, app: &mut State) -> (bool, Rebuild) {
        let value = self.normalize(value);
        match &self.on_change {
            Some(on_change) if value != self.current(app) => (true, on_change(app, value)),
            _ => (false, Rebuild::none()),
        }
    }

    fn commit(&self, app: &mut State) -> Rebuild {
        match &self.on_commit {
            Some(on_commit) => {
                let value = self.current(app);
                on_commit(app, value)
            }
            None => Rebuild::none(),
        }
    }

    /// The readout of `value`, if the slider shows one.
    fn readout_text(&self, value: f64, cx: &WidgetContext<'_>) -> Result<Option<Text>, LabelError> {
        let Some(readout) = &self.readout else {
            return Ok(None);
        };
        let values = FormatValues::Numbers(&[value]);
        let formatted = cx
            .engine
            .format(values, &readout.pattern, PrecisionSharing::PerValue)?;
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

    /// The readout's reserved size: the wider of its sizes at the range's start and end. A
    /// readout that fails to format or typeset measures as empty.
    fn readout_size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        let size = |value: f64| match self.readout_text(value, cx) {
            Ok(Some(text)) => text.size(cx),
            _ => [0.0; 2],
        };
        let [start, end] = [size(*self.range.start()), size(*self.range.end())];
        [start[0].max(end[0]), start[1].max(end[1])]
    }

    /// The slider's parts in `rect`.
    fn parts(&self, rect: Rect, cx: &WidgetContext<'_>) -> Parts {
        let style = cx.style;
        let [_, label_height] = self.label.size(cx);
        let readout = self.readout_size(cx);
        let row_top = rect.y + label_height + style.label_gap;
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
    /// Where the thumb's center runs, from the start of the range to its end.
    track: [f32; 2],
    readout_x: f32,
}

/// A slider's drag, and whether a key changed its value since the key went down.
#[derive(Debug, Default)]
pub struct SliderState {
    drag: Option<Drag>,
    key_changed: bool,
}

#[derive(Debug)]
struct Drag {
    /// The value at the press, which Escape and a cancellation restore.
    start: f64,
    changed: bool,
    /// Escape canceled the drag, which ignores the pointer until the release.
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
        let [label_width, label_height] = self.label.size(cx);
        let readout = self.readout_size(cx);
        match direction {
            Direction::Horizontal => {
                let reserved = match self.readout {
                    Some(_) => readout_gap(style) + readout[0],
                    None => 0.0,
                };
                label_width.max(style.track_length[0] + style.thumb_size + reserved)
            }
            Direction::Vertical => {
                label_height + style.label_gap + style.thumb_size.max(readout[1])
            }
        }
    }

    /// The width of the label, or of the track at its least length with the thumb and readout.
    fn width(&self, _height: f32, cx: &WidgetContext<'_>) -> avenger_layout::Measurement {
        let style = cx.style;
        let [label_width, _] = self.label.size(cx);
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
        self.label.bounds(cx)?;
        let mut marks: Vec<SceneMark> = vec![
            self.label
                .mark("slider_label", [r.x, r.y], color, style)
                .into(),
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
        ];
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
            WidgetEvent::Cancel => return self.restore(app, own),
            WidgetEvent::Blur if std::mem::take(&mut own.key_changed) => return self.commit(app),
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
                    Some(drag) if drag.changed && drag.active() => self.commit(app),
                    _ => Rebuild::none(),
                }
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                let rebuild = self.restore(app, own);
                own.drag = Some(Drag {
                    start: 0.0,
                    changed: false,
                    canceled: true,
                });
                rebuild
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
                    true => self.commit(app),
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

    /// End an active drag, restoring the value at its press.
    fn restore(&self, app: &mut State, own: &mut SliderState) -> Rebuild {
        match own.drag.take() {
            Some(drag) if drag.active() => self.set(drag.start, app).1,
            _ => Rebuild::none(),
        }
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
