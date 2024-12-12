/// Events that can be emitted by a window
#[derive(Debug, Clone, PartialEq)]
pub enum AvengerEvent {
    /// Window has been resized
    WindowResize(WindowResizeEvent),

    /// Window has been moved
    WindowMoved(WindowMovedEvent),

    /// Window has gained or lost focus
    WindowFocused(bool),

    /// Window has been requested to close
    WindowCloseRequested,

    /// Mouse button was pressed or released
    MouseInput(MouseInputEvent),

    /// Mouse cursor position changed
    CursorMoved(CursorMovedEvent),

    /// Mouse cursor entered window
    CursorEntered,

    /// Mouse cursor left window
    CursorLeft,

    /// Mouse wheel movement
    MouseWheel(MouseWheelEvent),

    /// Keyboard key was pressed or released
    KeyboardInput(KeyboardInputEvent),

    /// Touch event occurred
    Touch(TouchEvent),

    /// Double click event occurred
    DoubleClick(DoubleClickEvent),
}

/// Window resize event data
#[derive(Debug, Clone, PartialEq)]
pub struct WindowResizeEvent {
    pub size: [f32; 2],
}

/// Window moved event data
#[derive(Debug, Clone, PartialEq)]
pub struct WindowMovedEvent {
    pub position: [i32; 2],
}

/// Mouse input event data
#[derive(Debug, Clone, PartialEq)]
pub struct MouseInputEvent {
    pub state: ElementState,
    pub button: MouseButton,
}

/// Cursor moved event data
#[derive(Debug, Clone, PartialEq)]
pub struct CursorMovedEvent {
    pub position: [f32; 2],
}

/// Mouse wheel event data
#[derive(Debug, Clone, PartialEq)]
pub struct MouseWheelEvent {
    pub delta: MouseScrollDelta,
}

/// Keyboard input event data
#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardInputEvent {
    pub key: Key,
    pub state: ElementState,
}

/// Touch event data
#[derive(Debug, Clone, PartialEq)]
pub struct TouchEvent {
    pub phase: TouchPhase,
    pub position: [f32; 2],
}

/// Double click event data
#[derive(Debug, Clone, PartialEq)]
pub struct DoubleClickEvent {
    pub position: [f32; 2],
}

/// Keyboard key identifier
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum Key {
    /// Named key (e.g., Enter, Space)
    Named(NamedKey),
    /// Character key
    Character(char),
}

/// Named keyboard keys
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum NamedKey {
    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Navigation
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    End,
    Home,
    PageDown,
    PageUp,

    // UI control
    Backspace,
    Delete,
    Enter,
    Escape,
    Tab,
    Space,

    // Modifiers
    Alt,
    CapsLock,
    Control,
    Shift,
    Meta,

    // Media
    MediaPlayPause,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,

    // Volume
    AudioVolumeDown,
    AudioVolumeMute,
    AudioVolumeUp,
}

/// Touch event phase
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Input element state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementState {
    Pressed,
    Released,
}

/// Mouse button identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

/// Mouse scroll input
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseScrollDelta {
    LineDelta(f32, f32),
    PixelDelta(f64, f64),
}

#[cfg(feature = "winit_support")]
impl AvengerEvent {
    /// Convert a winit WindowEvent into an AvengerEvent
    ///
    /// Note: This conversion will not generate DoubleClick events. These must be
    /// synthesized by tracking the timing between consecutive click events.
    pub fn from_winit_event(event: winit::event::WindowEvent, scale: f32) -> Option<Self> {
        use winit::event::WindowEvent;
        match event {
            WindowEvent::Resized(size) => Some(AvengerEvent::WindowResize(WindowResizeEvent {
                size: [size.width as f32 / scale, size.height as f32 / scale],
            })),

            WindowEvent::Moved(position) => Some(AvengerEvent::WindowMoved(WindowMovedEvent {
                position: [
                    (position.x as f32 / scale) as i32,
                    (position.y as f32 / scale) as i32,
                ],
            })),

            WindowEvent::CloseRequested => Some(AvengerEvent::WindowCloseRequested),

            WindowEvent::Focused(focused) => Some(AvengerEvent::WindowFocused(focused)),

            WindowEvent::CursorMoved { position, .. } => {
                Some(AvengerEvent::CursorMoved(CursorMovedEvent {
                    position: [position.x as f32 / scale, position.y as f32 / scale],
                }))
            }

            WindowEvent::CursorEntered { .. } => Some(AvengerEvent::CursorEntered),

            WindowEvent::CursorLeft { .. } => Some(AvengerEvent::CursorLeft),

            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x, y),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        (pos.x as f32 / scale, pos.y as f32 / scale)
                    }
                };
                Some(AvengerEvent::MouseWheel(MouseWheelEvent {
                    delta: MouseScrollDelta::LineDelta(dx, dy),
                }))
            }

            WindowEvent::MouseInput { state, button, .. } => {
                Some(AvengerEvent::MouseInput(MouseInputEvent {
                    state: match state {
                        winit::event::ElementState::Pressed => ElementState::Pressed,
                        winit::event::ElementState::Released => ElementState::Released,
                    },
                    button: match button {
                        winit::event::MouseButton::Left => MouseButton::Left,
                        winit::event::MouseButton::Right => MouseButton::Right,
                        winit::event::MouseButton::Middle => MouseButton::Middle,
                        winit::event::MouseButton::Back => MouseButton::Back,
                        winit::event::MouseButton::Forward => MouseButton::Forward,
                        winit::event::MouseButton::Other(val) => MouseButton::Other(val),
                    },
                }))
            }

            WindowEvent::KeyboardInput { event, .. } => {
                Some(AvengerEvent::KeyboardInput(KeyboardInputEvent {
                    state: match event.state {
                        winit::event::ElementState::Pressed => ElementState::Pressed,
                        winit::event::ElementState::Released => ElementState::Released,
                    },
                    key: match event.logical_key {
                        winit::keyboard::Key::Named(named) => match NamedKey::try_from(named) {
                            Ok(key) => Key::Named(key),
                            Err(_) => return None,
                        },
                        winit::keyboard::Key::Character(c) => {
                            Key::Character(c.as_str().chars().next().unwrap_or('\0'))
                        }
                        _ => return None,
                    },
                }))
            }

            WindowEvent::Touch(touch) => Some(AvengerEvent::Touch(TouchEvent {
                phase: match touch.phase {
                    winit::event::TouchPhase::Started => TouchPhase::Started,
                    winit::event::TouchPhase::Moved => TouchPhase::Moved,
                    winit::event::TouchPhase::Ended => TouchPhase::Ended,
                    winit::event::TouchPhase::Cancelled => TouchPhase::Cancelled,
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

#[cfg(feature = "winit_support")]
impl TryFrom<winit::keyboard::NamedKey> for NamedKey {
    type Error = ();

    fn try_from(key: winit::keyboard::NamedKey) -> Result<Self, Self::Error> {
        use winit::keyboard::NamedKey as WinitKey;
        match key {
            // Function keys
            WinitKey::F1 => Ok(NamedKey::F1),
            WinitKey::F2 => Ok(NamedKey::F2),
            WinitKey::F3 => Ok(NamedKey::F3),
            WinitKey::F4 => Ok(NamedKey::F4),
            WinitKey::F5 => Ok(NamedKey::F5),
            WinitKey::F6 => Ok(NamedKey::F6),
            WinitKey::F7 => Ok(NamedKey::F7),
            WinitKey::F8 => Ok(NamedKey::F8),
            WinitKey::F9 => Ok(NamedKey::F9),
            WinitKey::F10 => Ok(NamedKey::F10),
            WinitKey::F11 => Ok(NamedKey::F11),
            WinitKey::F12 => Ok(NamedKey::F12),

            // Navigation
            WinitKey::ArrowDown => Ok(NamedKey::ArrowDown),
            WinitKey::ArrowLeft => Ok(NamedKey::ArrowLeft),
            WinitKey::ArrowRight => Ok(NamedKey::ArrowRight),
            WinitKey::ArrowUp => Ok(NamedKey::ArrowUp),
            WinitKey::End => Ok(NamedKey::End),
            WinitKey::Home => Ok(NamedKey::Home),
            WinitKey::PageDown => Ok(NamedKey::PageDown),
            WinitKey::PageUp => Ok(NamedKey::PageUp),

            // UI control
            WinitKey::Backspace => Ok(NamedKey::Backspace),
            WinitKey::Delete => Ok(NamedKey::Delete),
            WinitKey::Enter => Ok(NamedKey::Enter),
            WinitKey::Escape => Ok(NamedKey::Escape),
            WinitKey::Tab => Ok(NamedKey::Tab),
            WinitKey::Space => Ok(NamedKey::Space),

            // Modifiers
            WinitKey::Alt => Ok(NamedKey::Alt),
            WinitKey::CapsLock => Ok(NamedKey::CapsLock),
            WinitKey::Control => Ok(NamedKey::Control),
            WinitKey::Shift => Ok(NamedKey::Shift),
            WinitKey::Meta => Ok(NamedKey::Meta),

            // Media
            WinitKey::MediaPlayPause => Ok(NamedKey::MediaPlayPause),
            WinitKey::MediaStop => Ok(NamedKey::MediaStop),
            WinitKey::MediaTrackNext => Ok(NamedKey::MediaTrackNext),
            WinitKey::MediaTrackPrevious => Ok(NamedKey::MediaTrackPrevious),

            // Volume
            WinitKey::AudioVolumeDown => Ok(NamedKey::AudioVolumeDown),
            WinitKey::AudioVolumeMute => Ok(NamedKey::AudioVolumeMute),
            WinitKey::AudioVolumeUp => Ok(NamedKey::AudioVolumeUp),

            // Return Err for unhandled keys
            _ => Err(()),
        }
    }
}
