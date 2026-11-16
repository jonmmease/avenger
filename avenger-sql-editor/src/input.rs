//! The translation of egui's input into avenger's window events, in the logical pixels of a chart
//! whose top-left corner is at an origin in egui's points.

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, MouseScrollDelta, NamedKey, WindowCursorMoved, WindowEvent,
    WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};
use eframe::egui;

/// The avenger key of an egui key that avenger names, such as Escape or an arrow. Characters
/// arrive as text instead.
pub fn named_key(key: egui::Key) -> Option<NamedKey> {
    use egui::Key as K;
    Some(match key {
        K::Escape => NamedKey::Escape,
        K::Enter => NamedKey::Enter,
        K::Tab => NamedKey::Tab,
        K::Backspace => NamedKey::Backspace,
        K::Delete => NamedKey::Delete,
        K::ArrowDown => NamedKey::ArrowDown,
        K::ArrowLeft => NamedKey::ArrowLeft,
        K::ArrowRight => NamedKey::ArrowRight,
        K::ArrowUp => NamedKey::ArrowUp,
        K::Home => NamedKey::Home,
        K::End => NamedKey::End,
        K::PageUp => NamedKey::PageUp,
        K::PageDown => NamedKey::PageDown,
        K::F1 => NamedKey::F1,
        K::F2 => NamedKey::F2,
        K::F3 => NamedKey::F3,
        K::F4 => NamedKey::F4,
        K::F5 => NamedKey::F5,
        K::F6 => NamedKey::F6,
        K::F7 => NamedKey::F7,
        K::F8 => NamedKey::F8,
        K::F9 => NamedKey::F9,
        K::F10 => NamedKey::F10,
        K::F11 => NamedKey::F11,
        K::F12 => NamedKey::F12,
        _ => return None,
    })
}

pub fn key_event(key: Key, pressed: bool, text: Option<String>) -> WindowEvent {
    WindowEvent::KeyboardInput(WindowKeyboardInput {
        key,
        state: if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        },
        text,
    })
}

pub fn button(button: egui::PointerButton) -> MouseButton {
    match button {
        egui::PointerButton::Primary => MouseButton::Left,
        egui::PointerButton::Secondary => MouseButton::Right,
        egui::PointerButton::Middle => MouseButton::Middle,
        egui::PointerButton::Extra1 => MouseButton::Back,
        egui::PointerButton::Extra2 => MouseButton::Forward,
    }
}

pub fn mouse_input(button: MouseButton, pressed: bool) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state: if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        },
        button,
    })
}

pub fn cursor_moved(position: egui::Vec2) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved {
        position: [position.x, position.y],
    })
}

/// A wheel event. egui reports deltas as winit does, in points or in lines, so points pass as
/// logical pixels.
pub fn wheel(unit: egui::MouseWheelUnit, delta: egui::Vec2) -> WindowEvent {
    let delta = match unit {
        egui::MouseWheelUnit::Point => {
            MouseScrollDelta::PixelDelta(f64::from(delta.x), f64::from(delta.y))
        }
        egui::MouseWheelUnit::Line => MouseScrollDelta::LineDelta(delta.x, delta.y),
        // A page is about ten lines.
        egui::MouseWheelUnit::Page => MouseScrollDelta::LineDelta(10.0 * delta.x, 10.0 * delta.y),
    };
    WindowEvent::MouseWheel(WindowMouseWheel { delta })
}

/// The key events that change `before` into `after`: a press or a release of Shift, Control,
/// Alt, and Super (Command on macOS), as winit sends them.
pub fn modifier_events(before: egui::Modifiers, after: egui::Modifiers) -> Vec<WindowEvent> {
    let pairs = [
        (before.shift, after.shift, NamedKey::Shift),
        (before.ctrl, after.ctrl, NamedKey::Control),
        (before.alt, after.alt, NamedKey::Alt),
        (before.mac_cmd, after.mac_cmd, NamedKey::Super),
    ];
    pairs
        .into_iter()
        .filter(|(was, is, _)| was != is)
        .map(|(_, is, key)| key_event(Key::Named(key), is, None))
        .collect()
}

/// The egui cursor that shows an avenger cursor.
pub fn cursor(icon: CursorIcon) -> egui::CursorIcon {
    use egui::CursorIcon as E;
    match icon {
        CursorIcon::Default => E::Default,
        CursorIcon::ContextMenu => E::ContextMenu,
        CursorIcon::Help => E::Help,
        CursorIcon::Pointer => E::PointingHand,
        CursorIcon::Progress => E::Progress,
        CursorIcon::Wait => E::Wait,
        CursorIcon::Cell => E::Cell,
        CursorIcon::Crosshair => E::Crosshair,
        CursorIcon::Text => E::Text,
        CursorIcon::VerticalText => E::VerticalText,
        CursorIcon::Alias => E::Alias,
        CursorIcon::Copy => E::Copy,
        CursorIcon::Move => E::Move,
        CursorIcon::NoDrop => E::NoDrop,
        CursorIcon::NotAllowed => E::NotAllowed,
        CursorIcon::Grab => E::Grab,
        CursorIcon::Grabbing => E::Grabbing,
        CursorIcon::AllScroll => E::AllScroll,
        CursorIcon::ZoomIn => E::ZoomIn,
        CursorIcon::ZoomOut => E::ZoomOut,
        CursorIcon::EResize => E::ResizeEast,
        CursorIcon::NResize => E::ResizeNorth,
        CursorIcon::NeResize => E::ResizeNorthEast,
        CursorIcon::NwResize => E::ResizeNorthWest,
        CursorIcon::SResize => E::ResizeSouth,
        CursorIcon::SeResize => E::ResizeSouthEast,
        CursorIcon::SwResize => E::ResizeSouthWest,
        CursorIcon::WResize => E::ResizeWest,
        CursorIcon::EwResize => E::ResizeHorizontal,
        CursorIcon::NsResize => E::ResizeVertical,
        CursorIcon::NeswResize => E::ResizeNeSw,
        CursorIcon::NwseResize => E::ResizeNwSe,
        CursorIcon::ColResize => E::ResizeColumn,
        CursorIcon::RowResize => E::ResizeRow,
        _ => E::Default,
    }
}
