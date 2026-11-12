//! The avenger window events of the browser's events: the fields of DOM events, in the canvas's
//! CSS pixels, to `WindowEvent`s. The conversion has no browser types, so native tests cover it.

use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};

/// When wheel events over the canvas reach the chart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wheel {
    /// Only while the canvas has the focus, so a page scrolls past a chart that it does not use.
    #[default]
    Focus,
    /// Always, for a page that does not scroll, such as one chart that fills the window.
    Always,
}

impl Wheel {
    /// The option of `name`, `"focus"` or `"always"`.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "focus" => Ok(Self::Focus),
            "always" => Ok(Self::Always),
            other => Err(format!(
                "the wheel option is \"focus\" or \"always\", not \"{other}\""
            )),
        }
    }

    /// Whether a wheel event over the canvas reaches the chart.
    pub fn reaches(self, focused: bool) -> bool {
        self == Self::Always || focused
    }
}

/// The pixels of one wheel notch, which the navigation tool zooms by one step.
const NOTCH: f64 = 100.0;

/// The lines of a page, for a wheel that scrolls by pages.
const PAGE_LINES: f32 = 10.0;

/// The wheel event of a DOM wheel event's deltas in its `deltaMode`: 0 for pixels, 1 for lines,
/// and 2 for pages. The DOM's deltas grow as the page scrolls down, and the window event's grow
/// as the content moves down, as winit's do.
pub fn wheel(delta_x: f64, delta_y: f64, delta_mode: u32) -> WindowEvent {
    let delta = match delta_mode {
        1 => MouseScrollDelta::LineDelta(-delta_x as f32, -delta_y as f32),
        2 => {
            MouseScrollDelta::LineDelta(-delta_x as f32 * PAGE_LINES, -delta_y as f32 * PAGE_LINES)
        }
        _ => MouseScrollDelta::PixelDelta(-delta_x, -delta_y),
    };
    WindowEvent::MouseWheel(WindowMouseWheel { delta })
}

/// The button of a DOM mouse or pointer event's `button`.
pub fn button(button: i16) -> MouseButton {
    match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        other => MouseButton::Other(other.max(0) as u16),
    }
}

pub fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

pub fn press(button: MouseButton, state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput { state, button })
}

/// The key event of a DOM keyboard event's `key`, or `None` for a key that the window events do
/// not name, such as `Unidentified`.
pub fn key(key: &str, state: ElementState) -> Option<WindowEvent> {
    let named = |named| Some(Key::Named(named));
    let key = match key {
        "ArrowDown" => named(NamedKey::ArrowDown),
        "ArrowLeft" => named(NamedKey::ArrowLeft),
        "ArrowRight" => named(NamedKey::ArrowRight),
        "ArrowUp" => named(NamedKey::ArrowUp),
        "End" => named(NamedKey::End),
        "Home" => named(NamedKey::Home),
        "PageDown" => named(NamedKey::PageDown),
        "PageUp" => named(NamedKey::PageUp),
        "Backspace" => named(NamedKey::Backspace),
        "Delete" => named(NamedKey::Delete),
        "Enter" => named(NamedKey::Enter),
        "Escape" => named(NamedKey::Escape),
        "Tab" => named(NamedKey::Tab),
        " " => named(NamedKey::Space),
        "Alt" => named(NamedKey::Alt),
        "CapsLock" => named(NamedKey::CapsLock),
        "Control" => named(NamedKey::Control),
        "Shift" => named(NamedKey::Shift),
        "Meta" => named(NamedKey::Super),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(char), None) => Some(Key::Character(char)),
                _ => None,
            }
        }
    }?;
    // A press of a key that types a character types it.
    let text = match (key, state) {
        (Key::Character(char), ElementState::Pressed) => Some(char.to_string()),
        (Key::Named(NamedKey::Space), ElementState::Pressed) => Some(" ".to_string()),
        _ => None,
    };
    Some(WindowEvent::KeyboardInput(WindowKeyboardInput {
        key,
        state,
        text,
    }))
}

/// The touch pointers on the canvas and the gesture that they make. One finger scrolls the page,
/// and a tap moves the cursor to its point and clicks there. A tap and a drag of two fingers start
/// with the cursor's entering, which the chart's tools wait for, as a mouse's entering is. Two fingers pan the chart as a drag
/// of their midpoint, and zoom it by the change of their distance as wheel steps at the midpoint,
/// so the chart's navigation takes touch without touch events of its own.
#[derive(Debug, Default)]
pub struct Touches {
    /// The pointers that are down, by their `pointerId`, in the order that they went down.
    down: Vec<(i32, [f32; 2])>,
    /// The pointer of a tap while it can still be one: down alone, and not far from its start.
    tap: Option<(i32, [f32; 2])>,
    /// The midpoint and the distance of two fingers while they drag.
    pinch: Option<Pinch>,
}

#[derive(Clone, Copy, Debug)]
struct Pinch {
    midpoint: [f32; 2],
    distance: f32,
}

/// How far a tap's finger moves before it no longer taps, in CSS pixels.
const TAP_DISTANCE: f32 = 8.0;

impl Touches {
    /// Whether two fingers drag the chart, while the page must not scroll or zoom.
    pub fn pinching(&self) -> bool {
        self.pinch.is_some()
    }

    /// A finger goes down at `position`.
    pub fn down(&mut self, id: i32, position: [f32; 2]) -> Vec<WindowEvent> {
        self.down.retain(|(down, _)| *down != id);
        self.down.push((id, position));
        match self.down.as_slice() {
            [_] => {
                self.tap = Some((id, position));
                Vec::new()
            }
            [(_, a), (_, b), ..] if self.pinch.is_none() => {
                self.tap = None;
                let pinch = Pinch::of(*a, *b);
                self.pinch = Some(pinch);
                vec![
                    WindowEvent::CursorEntered,
                    moved(pinch.midpoint),
                    press(MouseButton::Left, ElementState::Pressed),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// A finger moves to `position`.
    pub fn moved(&mut self, id: i32, position: [f32; 2]) -> Vec<WindowEvent> {
        let Some(index) = self.down.iter().position(|(down, _)| *down == id) else {
            return Vec::new();
        };
        self.down[index].1 = position;
        if let Some((tap, start)) = self.tap {
            if tap == id && distance(start, position) > TAP_DISTANCE {
                self.tap = None;
            }
        }
        let (Some(pinch), [(_, a), (_, b), ..]) = (self.pinch, self.down.as_slice()) else {
            return Vec::new();
        };
        if index > 1 {
            return Vec::new();
        }
        let next = Pinch::of(*a, *b);
        self.pinch = Some(next);
        let mut events = vec![moved(next.midpoint)];
        if pinch.distance > 0.0 && next.distance > 0.0 && next.distance != pinch.distance {
            // A notch zooms by 1.2, so the change of the distance is the notches of its ratio.
            let notches =
                (f64::from(next.distance) / f64::from(pinch.distance)).ln() / 1.2_f64.ln();
            events.push(WindowEvent::MouseWheel(WindowMouseWheel {
                delta: MouseScrollDelta::PixelDelta(0.0, notches * NOTCH),
            }));
        }
        events
    }

    /// A finger goes up at `position`.
    pub fn up(&mut self, id: i32, position: [f32; 2]) -> Vec<WindowEvent> {
        let tapped = self.tap.take().is_some_and(|(tap, _)| tap == id);
        let ended = self.end(id);
        if tapped {
            return vec![
                WindowEvent::CursorEntered,
                moved(position),
                press(MouseButton::Left, ElementState::Pressed),
                press(MouseButton::Left, ElementState::Released),
            ];
        }
        ended
    }

    /// The browser takes a finger, as when one finger scrolls the page.
    pub fn cancel(&mut self, id: i32) -> Vec<WindowEvent> {
        if self.tap.is_some_and(|(tap, _)| tap == id) {
            self.tap = None;
        }
        self.end(id)
    }

    /// Removes the finger `id`, and ends the drag of two fingers when it was one of them.
    fn end(&mut self, id: i32) -> Vec<WindowEvent> {
        let Some(index) = self.down.iter().position(|(down, _)| *down == id) else {
            return Vec::new();
        };
        self.down.remove(index);
        if index < 2 && self.pinch.take().is_some() {
            return vec![press(MouseButton::Left, ElementState::Released)];
        }
        Vec::new()
    }
}

impl Pinch {
    fn of(a: [f32; 2], b: [f32; 2]) -> Self {
        Self {
            midpoint: [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
            distance: distance(a, b),
        }
    }
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
