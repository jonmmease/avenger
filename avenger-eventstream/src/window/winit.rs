use ::winit::{
    event::{
        ElementState as WinitElementState, Ime as WinitIme, MouseButton as WinitMouseButton,
        MouseScrollDelta as WinitMouseScrollDelta, TouchPhase as WinitTouchPhase,
        WindowEvent as WinitEvent,
    },
    keyboard::{Key as WinitKey, NamedKey as WinitNamedKey},
};

use super::*;

fn is_ime_consumed_key(key: &WinitKey) -> bool {
    // Windows reports VK_PROCESSKEY while the IME owns the physical key. It
    // must not also enter the editor's ordinary keybinding/text path.
    matches!(key, WinitKey::Named(WinitNamedKey::Process))
}

impl WindowEvent {
    /// Convert a winit WindowEvent into an Avenger WindowEvent
    ///
    /// Avenger's window events use logical coordinates, so scale is required for
    /// the conversion.
    pub fn from_winit_event(event: WinitEvent, scale: f32) -> Option<Self> {
        match event {
            WinitEvent::Resized(size) => Some(Self::WindowResize(WindowResizeEvent {
                size: [size.width as f32 / scale, size.height as f32 / scale],
            })),

            WinitEvent::Moved(position) => Some(Self::WindowMoved(WindowMovedEvent {
                position: [
                    (position.x as f32 / scale) as i32,
                    (position.y as f32 / scale) as i32,
                ],
            })),

            WinitEvent::CloseRequested => Some(Self::WindowCloseRequested),

            WinitEvent::Focused(focused) => Some(Self::WindowFocused(focused)),

            WinitEvent::ModifiersChanged(modifiers) => {
                let modifiers = modifiers.state();
                Some(Self::ModifiersChanged(crate::scene::ModifiersState {
                    shift: modifiers.shift_key(),
                    control: modifiers.control_key(),
                    alt: modifiers.alt_key(),
                    meta: modifiers.super_key(),
                }))
            }

            WinitEvent::CursorMoved { position, .. } => {
                Some(Self::CursorMoved(WindowCursorMoved {
                    position: [position.x as f32 / scale, position.y as f32 / scale],
                }))
            }

            WinitEvent::CursorEntered { .. } => Some(Self::CursorEntered),

            WinitEvent::CursorLeft { .. } => Some(Self::CursorLeft),

            WinitEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    WinitMouseScrollDelta::LineDelta(x, y) => (x, y),
                    WinitMouseScrollDelta::PixelDelta(pos) => {
                        (pos.x as f32 / scale, pos.y as f32 / scale)
                    }
                };
                Some(Self::MouseWheel(WindowMouseWheel {
                    delta: MouseScrollDelta::LineDelta(dx, dy),
                }))
            }

            WinitEvent::MouseInput { state, button, .. } => {
                Some(Self::MouseInput(WindowMouseInput {
                    state: match state {
                        WinitElementState::Pressed => ElementState::Pressed,
                        WinitElementState::Released => ElementState::Released,
                    },
                    button: match button {
                        WinitMouseButton::Left => MouseButton::Left,
                        WinitMouseButton::Right => MouseButton::Right,
                        WinitMouseButton::Middle => MouseButton::Middle,
                        WinitMouseButton::Back => MouseButton::Back,
                        WinitMouseButton::Forward => MouseButton::Forward,
                        WinitMouseButton::Other(val) => MouseButton::Other(val),
                    },
                }))
            }

            WinitEvent::KeyboardInput { event, .. } => {
                if is_ime_consumed_key(&event.logical_key) {
                    return None;
                }
                Some(Self::KeyboardInput(WindowKeyboardInput {
                    repeat: event.repeat,
                    state: match event.state {
                        WinitElementState::Pressed => ElementState::Pressed,
                        WinitElementState::Released => ElementState::Released,
                    },
                    key: match event.logical_key {
                        WinitKey::Named(named) => match NamedKey::try_from(named) {
                            Ok(key) => Key::Named(key),
                            Err(_) => return None,
                        },
                        WinitKey::Character(c) => {
                            Key::Character(c.as_str().chars().next().unwrap_or('\0'))
                        }
                        _ => return None,
                    },
                    text: event.text,
                }))
            }

            WinitEvent::Ime(event) => Some(Self::Ime(match event {
                WinitIme::Enabled => ImeEvent::Enabled,
                WinitIme::Preedit(text, cursor) => ImeEvent::Preedit {
                    text: text.into(),
                    cursor,
                },
                WinitIme::Commit(text) => ImeEvent::Commit(text.into()),
                WinitIme::Disabled => ImeEvent::Disabled,
            })),

            WinitEvent::Touch(touch) => Some(Self::Touch(WindowTouch {
                phase: match touch.phase {
                    WinitTouchPhase::Started => TouchPhase::Started,
                    WinitTouchPhase::Moved => TouchPhase::Moved,
                    WinitTouchPhase::Ended => TouchPhase::Ended,
                    WinitTouchPhase::Cancelled => TouchPhase::Cancelled,
                },
                position: [
                    touch.location.x as f32 / scale,
                    touch.location.y as f32 / scale,
                ],
            })),

            _ => None,
        }
    }
}

impl TryFrom<WinitNamedKey> for NamedKey {
    type Error = ();

    fn try_from(key: WinitNamedKey) -> Result<Self, Self::Error> {
        match key {
            // Function keys
            WinitNamedKey::F1 => Ok(NamedKey::F1),
            WinitNamedKey::F2 => Ok(NamedKey::F2),
            WinitNamedKey::F3 => Ok(NamedKey::F3),
            WinitNamedKey::F4 => Ok(NamedKey::F4),
            WinitNamedKey::F5 => Ok(NamedKey::F5),
            WinitNamedKey::F6 => Ok(NamedKey::F6),
            WinitNamedKey::F7 => Ok(NamedKey::F7),
            WinitNamedKey::F8 => Ok(NamedKey::F8),
            WinitNamedKey::F9 => Ok(NamedKey::F9),
            WinitNamedKey::F10 => Ok(NamedKey::F10),
            WinitNamedKey::F11 => Ok(NamedKey::F11),
            WinitNamedKey::F12 => Ok(NamedKey::F12),

            // Navigation
            WinitNamedKey::ArrowDown => Ok(NamedKey::ArrowDown),
            WinitNamedKey::ArrowLeft => Ok(NamedKey::ArrowLeft),
            WinitNamedKey::ArrowRight => Ok(NamedKey::ArrowRight),
            WinitNamedKey::ArrowUp => Ok(NamedKey::ArrowUp),
            WinitNamedKey::End => Ok(NamedKey::End),
            WinitNamedKey::Home => Ok(NamedKey::Home),
            WinitNamedKey::PageDown => Ok(NamedKey::PageDown),
            WinitNamedKey::PageUp => Ok(NamedKey::PageUp),

            // UI control
            WinitNamedKey::Backspace => Ok(NamedKey::Backspace),
            WinitNamedKey::Delete => Ok(NamedKey::Delete),
            WinitNamedKey::Enter => Ok(NamedKey::Enter),
            WinitNamedKey::Escape => Ok(NamedKey::Escape),
            WinitNamedKey::Tab => Ok(NamedKey::Tab),
            WinitNamedKey::Space => Ok(NamedKey::Space),

            // Modifiers
            WinitNamedKey::Alt => Ok(NamedKey::Alt),
            WinitNamedKey::CapsLock => Ok(NamedKey::CapsLock),
            WinitNamedKey::Control => Ok(NamedKey::Control),
            WinitNamedKey::Shift => Ok(NamedKey::Shift),
            WinitNamedKey::Meta => Ok(NamedKey::Super),
            WinitNamedKey::Super => Ok(NamedKey::Super),

            // Media
            WinitNamedKey::MediaPlayPause => Ok(NamedKey::MediaPlayPause),
            WinitNamedKey::MediaStop => Ok(NamedKey::MediaStop),
            WinitNamedKey::MediaTrackNext => Ok(NamedKey::MediaTrackNext),
            WinitNamedKey::MediaTrackPrevious => Ok(NamedKey::MediaTrackPrevious),

            // Volume
            WinitNamedKey::AudioVolumeDown => Ok(NamedKey::AudioVolumeDown),
            WinitNamedKey::AudioVolumeMute => Ok(NamedKey::AudioVolumeMute),
            WinitNamedKey::AudioVolumeUp => Ok(NamedKey::AudioVolumeUp),

            // Return Err for unhandled keys
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winit_ime_events_preserve_preedit_cursor_and_commit_text() {
        assert_eq!(
            WindowEvent::from_winit_event(
                WinitEvent::Ime(WinitIme::Preedit("かな".to_string(), Some((3, 6)))),
                2.0,
            ),
            Some(WindowEvent::Ime(ImeEvent::Preedit {
                text: "かな".into(),
                cursor: Some((3, 6)),
            }))
        );
        assert_eq!(
            WindowEvent::from_winit_event(
                WinitEvent::Ime(WinitIme::Commit("仮名".to_string())),
                2.0,
            ),
            Some(WindowEvent::Ime(ImeEvent::Commit("仮名".into())))
        );
    }

    #[test]
    fn process_key_consumed_by_windows_ime_is_filtered() {
        assert!(is_ime_consumed_key(&WinitKey::Named(
            WinitNamedKey::Process
        )));
        assert!(!is_ime_consumed_key(&WinitKey::Named(
            WinitNamedKey::ArrowLeft
        )));
    }
}
