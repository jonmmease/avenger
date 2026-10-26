//! A button, which runs an action.

use std::sync::Arc;

use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{Disposition, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_layout::Direction;
use avenger_scenegraph::marks::mark::SceneMark;

use crate::binding::{self, Get};
use crate::draw::{focus_ring, label_color, rect, Text};
use crate::widget::{Look, Placed, Widget, WidgetContext, WidgetError, WidgetEvent};

type Press<State> = Arc<dyn Fn(&mut State) -> Rebuild + Send + Sync>;

/// A button with a label. It activates on a left-button release inside it, on an Enter press,
/// and on a Space release. Escape cancels a pending press, and a held key activates it once.
pub struct Button<State> {
    id: String,
    label: Text,
    on_press: Option<Press<State>>,
    enabled: Option<Get<State, bool>>,
}

impl<State> Button<State> {
    /// Create a button named `id` with the plain-text `label`.
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: Text::plain(label),
            on_press: None,
            enabled: None,
        }
    }

    /// Run `press` on activation, which changes app state and returns the scenes to rebuild.
    pub fn on_press(
        mut self,
        press: impl Fn(&mut State) -> Rebuild + Send + Sync + 'static,
    ) -> Self {
        self.on_press = Some(Arc::new(press));
        self
    }

    /// Read from app state whether the button takes input. Without it, it always does.
    pub fn enabled(mut self, enabled: impl Fn(&State) -> bool + Send + Sync + 'static) -> Self {
        self.enabled = Some(Arc::new(enabled));
        self
    }

    fn press(&self, app: &mut State) -> Rebuild {
        match &self.on_press {
            Some(press) => press(app),
            None => Rebuild::none(),
        }
    }
}

/// A button's pending key and pointer presses.
#[derive(Debug, Default)]
pub struct ButtonState {
    /// Space is down, and its release activates the button.
    space: bool,
    /// Enter is down, so that its repeats don't activate the button again.
    enter: bool,
    /// Escape canceled the current pointer press.
    canceled: bool,
}

impl<State: Send + Sync + 'static> Widget<State> for Button<State> {
    type State = ButtonState;
    /// Whether the button looks pressed by a key, and whether Escape canceled its pointer press.
    type View = (bool, bool);

    fn id(&self) -> &str {
        &self.id
    }

    fn max_content(&self, direction: Direction, cx: &WidgetContext<'_>) -> f32 {
        let [width, height] = self.label.size(cx);
        let [across, down] = cx.style.button_padding;
        match direction {
            Direction::Horizontal => width + 2.0 * across,
            Direction::Vertical => height + 2.0 * down,
        }
    }

    fn view(&self, _app: &State, own: &ButtonState) -> (bool, bool) {
        (own.space, own.canceled)
    }

    fn draw(
        &self,
        &(space, canceled): &(bool, bool),
        placed: &Placed,
        cx: &WidgetContext<'_>,
    ) -> Result<Vec<SceneMark>, WidgetError> {
        let style = cx.style;
        let bounds = self.label.bounds(cx)?;
        let look = placed.look;
        let pressed = (look.pressed && look.hovered && !canceled) || space;
        let paint = style.neutral.paint(Look { pressed, ..look });
        let r = placed.rect;
        let frame = [r.x, r.y, r.width, r.height];
        let at = [
            r.x + (r.width - bounds.width) / 2.0,
            r.y + (r.height - bounds.height) / 2.0,
        ];
        let mut marks: Vec<SceneMark> = vec![
            rect(
                "button",
                frame,
                paint.fill,
                paint.border,
                style.border_width,
                style.corner_radius,
            )
            .into(),
            self.label
                .mark("button_label", at, label_color(style, look), style)
                .into(),
        ];
        if look.focus_visible {
            marks.push(focus_ring(frame, style.corner_radius, style));
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
        own: &ButtonState,
    ) -> Disposition {
        let pending = own.space || (placed.look.pressed && !own.canceled);
        match event {
            SceneGraphEvent::KeyPress(press) => match press.key {
                Key::Named(NamedKey::Enter | NamedKey::Space) => Disposition::Consume,
                Key::Named(NamedKey::Escape) if pending => Disposition::Consume,
                _ => Disposition::Ignore,
            },
            SceneGraphEvent::KeyRelease(release) => match release.key {
                Key::Named(NamedKey::Enter | NamedKey::Space) => Disposition::Consume,
                _ => Disposition::Ignore,
            },
            _ => Disposition::Consume,
        }
    }

    fn event(
        &self,
        event: WidgetEvent<'_>,
        placed: &Placed,
        _cx: &WidgetContext<'_>,
        app: &mut State,
        own: &mut ButtonState,
    ) -> Rebuild {
        let event = match event {
            WidgetEvent::Input(event) => event,
            WidgetEvent::Blur => {
                own.space = false;
                own.enter = false;
                return Rebuild::none();
            }
            WidgetEvent::Focus | WidgetEvent::Cancel => return Rebuild::none(),
        };
        match event {
            SceneGraphEvent::MouseDown(_) => own.canceled = false,
            SceneGraphEvent::MouseUp(up) if up.button == MouseButton::Left => {
                let inside = placed.rect.contains(up.position);
                if placed.look.pressed && inside && !own.canceled {
                    return self.press(app);
                }
            }
            SceneGraphEvent::KeyPress(press) => match press.key {
                Key::Named(NamedKey::Enter) if !own.enter => {
                    own.enter = true;
                    return self.press(app);
                }
                Key::Named(NamedKey::Space) => own.space = true,
                Key::Named(NamedKey::Escape) => {
                    own.space = false;
                    own.canceled = true;
                }
                _ => {}
            },
            SceneGraphEvent::KeyRelease(release) => match release.key {
                Key::Named(NamedKey::Enter) => own.enter = false,
                Key::Named(NamedKey::Space) if own.space => {
                    own.space = false;
                    return self.press(app);
                }
                _ => {}
            },
            _ => {}
        }
        Rebuild::none()
    }
}
