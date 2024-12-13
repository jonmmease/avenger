use super::*;
use ::winit::{
    event::{
        ElementState as WinitElementState, MouseButton as WinitMouseButton,
        MouseScrollDelta as WinitMouseScrollDelta, TouchPhase as WinitTouchPhase,
        WindowEvent as WinitEvent,
    },
    keyboard::{Key as WinitKey, NamedKey as WinitNamedKey},
};

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
                Some(Self::KeyboardInput(WindowKeyboardInput {
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
                }))
            }

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
