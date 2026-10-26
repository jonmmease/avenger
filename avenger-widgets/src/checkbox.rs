//! A check box, which turns a `bool` on and off.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_layout::Direction;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use lyon_path::geom::point;
use lyon_path::Path;

use crate::binding::{self, Get, Set};
use crate::draw::{cover, focus_ring, label_color, rect, Text};
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

/// A check box, with an optional label beside it. It toggles on a left-button release inside it and on a Space
/// release. Escape cancels a pending press.
pub struct Checkbox<State> {
    id: String,
    label: Option<Text>,
    value: Option<Get<State, bool>>,
    on_change: Option<Set<State, bool>>,
    enabled: Option<Get<State, bool>>,
}

impl<State> Checkbox<State> {
    /// Create a check box named `id`, without a label.
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            label: None,
            value: None,
            on_change: None,
            enabled: None,
        }
    }

    /// Show the plain-text `label` beside the box.
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(Text::plain(label));
        self
    }

    /// Read whether the box is checked from app state. Without it, the box is unchecked.
    pub fn value(mut self, value: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.value = Some(Arc::new(value));
        self
    }

    /// Change app state for a toggle, and return the scenes to rebuild.
    pub fn on_change(
        mut self,
        on_change: impl Fn(&mut State, bool) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_change = Some(Arc::new(on_change));
        self
    }

    /// Read from app state whether the box takes input. Without it, it always does.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn checked(&self, app: &State) -> bool {
        self.value.as_ref().is_some_and(|value| value(app))
    }

    fn toggle(&self, app: &mut State) -> Rebuild {
        let checked = !self.checked(app);
        match &self.on_change {
            Some(on_change) => on_change(app, checked),
            None => Rebuild::none(),
        }
    }

    /// The width of the box and the label after it, and the height of the row that holds them.
    fn row(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        let style = cx.style;
        match &self.label {
            Some(label) => {
                let [width, height] = label.size(cx);
                [
                    style.control_size + style.label_gap + width,
                    height.max(style.control_size),
                ]
            }
            None => [style.control_size; 2],
        }
    }
}

/// A check box's pending key and pointer presses.
#[derive(Debug, Default)]
pub struct CheckboxState {
    /// Space is down, and its release toggles the box.
    space: bool,
    /// Escape canceled the current pointer press.
    canceled: bool,
}

/// What a check box shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckboxView {
    checked: bool,
    space: bool,
    canceled: bool,
}

impl<State: Send + Sync + 'static> Widget<State> for Checkbox<State> {
    type State = CheckboxState;
    type View = CheckboxView;

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let [width, height] = self.row(cx);
        match direction {
            Direction::Horizontal => width,
            Direction::Vertical => height,
        }
    }

    fn view(&self, app: &State, own: &CheckboxState) -> CheckboxView {
        CheckboxView {
            checked: self.checked(app),
            space: own.space,
            canceled: own.canceled,
        }
    }

    fn draw(
        &self,
        view: &CheckboxView,
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let bounds = self
            .label
            .as_ref()
            .map(|label| label.bounds(cx))
            .transpose()?;
        let row = bounds
            .map_or(0.0, |bounds| bounds.height)
            .max(style.control_size);
        let look = placed.look;
        let pressed = (look.pressed && look.hovered && !view.canceled) || view.space;
        let palette = if view.checked {
            &style.accent
        } else {
            &style.neutral
        };
        let paint = palette.paint(Look { pressed, ..look });
        let r = placed.rect;
        let size = style.control_size;
        let frame = [r.x, r.y + (row - size) / 2.0, size, size];
        let radius = style.corner_radius.min(size / 4.0);
        let mut marks: Vec<SceneMark> = vec![rect(
            "checkbox",
            frame,
            paint.fill,
            paint.border,
            style.border_width,
            radius,
        )
        .into()];
        if view.checked {
            marks.push(check(frame, paint.foreground).into());
        }
        if let (Some(label), Some(bounds)) = (&self.label, bounds) {
            let at = [
                r.x + size + style.label_gap,
                r.y + (row - bounds.height) / 2.0,
            ];
            let color = label_color(style, look);
            marks.push(label.mark("checkbox_label", at, color, style).into());
        }
        marks.push(cover([r.x, r.y, r.width, r.height]));
        if look.focus_visible {
            marks.push(focus_ring(frame, radius, style));
        }
        Ok(marks)
    }

    fn enabled(&self, app: &State) -> bool {
        binding::enabled(&self.enabled, app)
    }

    fn disposition(
        &self,
        event: &SceneGraphEvent,
        placed: &Placed,
        _app: &State,
        own: &CheckboxState,
    ) -> Disposition {
        let pending = own.space || (placed.look.pressed && !own.canceled);
        match event {
            SceneGraphEvent::KeyPress(press) => match press.key {
                Key::Named(NamedKey::Space) => Disposition::Consume,
                Key::Named(NamedKey::Escape) if pending => Disposition::Consume,
                _ => Disposition::Ignore,
            },
            SceneGraphEvent::KeyRelease(release) if release.key == Key::Named(NamedKey::Space) => {
                Disposition::Consume
            }
            SceneGraphEvent::KeyRelease(_) => Disposition::Ignore,
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        _cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut CheckboxState,
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => event,
            WidgetEvent::Blur => {
                own.space = false;
                return Rebuild::none();
            }
            WidgetEvent::Focus | WidgetEvent::Cancel => return Rebuild::none(),
        };
        match event {
            SceneGraphEvent::MouseDown(_) => own.canceled = false,
            SceneGraphEvent::MouseUp(up) if up.button == MouseButton::Left => {
                let inside = placed.rect.contains(up.position);
                if placed.look.pressed && inside && !own.canceled {
                    return self.toggle(app);
                }
            }
            SceneGraphEvent::KeyPress(press) => match press.key {
                Key::Named(NamedKey::Space) => own.space = true,
                Key::Named(NamedKey::Escape) => {
                    own.space = false;
                    own.canceled = true;
                }
                _ => {}
            },
            SceneGraphEvent::KeyRelease(release)
                if release.key == Key::Named(NamedKey::Space) && own.space =>
            {
                own.space = false;
                return self.toggle(app);
            }
            _ => {}
        }
        Rebuild::none()
    }
}

/// A check mark in the box `frame`.
fn check([x, y, size, _]: [f32; 4], color: [f32; 4]) -> ScenePathMark {
    let at = |u: f32, v: f32| point(x + u * size, y + v * size);
    let mut path = Path::builder();
    path.begin(at(0.24, 0.52));
    path.line_to(at(0.43, 0.7));
    path.line_to(at(0.76, 0.32));
    path.end(false);
    ScenePathMark {
        name: "check".to_string(),
        path: path.build().into(),
        fill: ColorOrGradient::Color([0.0; 4]).into(),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: Some((size / 7.0).max(1.5)),
        ..Default::default()
    }
}
