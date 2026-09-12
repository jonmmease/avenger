use std::path::PathBuf;

use smol_str::SmolStr;

use crate::runtime::{InputSession, RuntimeWakeEvent};

#[cfg(feature = "winit_support")]
mod winit;

/// Native window events, in logical coordinates
#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    WindowResize(WindowResizeEvent),
    WindowResizeSettled(WindowResizeEvent),
    CanvasResize(CanvasResizeEvent),
    CanvasResizeSettled(CanvasResizeEvent),
    WindowMoved(WindowMovedEvent),
    WindowFocused(bool),
    WindowCloseRequested,
    MouseInput(WindowMouseInput),
    CursorMoved(WindowCursorMoved),
    CursorEntered,
    CursorLeft,
    MouseWheel(WindowMouseWheel),
    KeyboardInput(WindowKeyboardInput),
    ModifiersChanged(crate::scene::ModifiersState),
    Ime(ImeEvent),
    TextInput(SessionInputEvent),
    PointerCaptureLost,
    FocusEntered { reverse: bool },
    Clipboard(ClipboardEvent),
    RuntimeWake(RuntimeWakeEvent),
    Touch(WindowTouch),
    InteractionSettled { generation: u64 },
    FileChanged(WindowFileChangedEvent),
}

impl WindowEvent {
    pub fn position(&self) -> Option<[f32; 2]> {
        match self {
            Self::CursorMoved(event) => Some(event.position),
            Self::Touch(event) => Some(event.position),
            _ => None,
        }
    }

    pub fn skip_if_render_pending(&self) -> bool {
        !matches!(
            self,
            Self::MouseInput(_)
                | Self::KeyboardInput(_)
                | Self::ModifiersChanged(_)
                | Self::Ime(_)
                | Self::TextInput(_)
                | Self::PointerCaptureLost
                | Self::FocusEntered { .. }
                | Self::Clipboard(_)
                | Self::WindowFocused(_)
                | Self::WindowCloseRequested
                | Self::CursorLeft
                | Self::RuntimeWake(_)
                | Self::FileChanged(_)
                | Self::InteractionSettled { .. }
                | Self::WindowResizeSettled(_)
                | Self::CanvasResizeSettled(_)
        )
    }
}

/// Input stamped with the focus session that owned it at the host boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInputEvent {
    pub session: InputSession,
    pub event: TextInputEvent,
}

/// Actions delivered to a focused text input.
#[derive(Debug, Clone, PartialEq)]
pub enum TextInputEvent {
    Keyboard(WindowKeyboardInput),
    Ime(ImeEvent),
    Clipboard(ClipboardEvent),
}

impl WindowEvent {
    /// Preserve legacy input when no session owner is installed.
    pub fn with_input_session(self, session: Option<InputSession>) -> Self {
        let Some(session) = session else {
            return self;
        };
        let event = match self {
            Self::KeyboardInput(event) => TextInputEvent::Keyboard(event),
            Self::Ime(event) => TextInputEvent::Ime(event),
            Self::Clipboard(event) => TextInputEvent::Clipboard(event),
            other => return other,
        };
        Self::TextInput(SessionInputEvent { session, event })
    }

    /// Return keyboard data from ordinary or session-owned input.
    pub fn keyboard_input(&self) -> Option<&WindowKeyboardInput> {
        match self {
            Self::KeyboardInput(event) => Some(event),
            Self::TextInput(SessionInputEvent {
                event: TextInputEvent::Keyboard(event),
                ..
            }) => Some(event),
            _ => None,
        }
    }
}

/// Host-neutral input-method-editor event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    Enabled,
    Preedit {
        text: SmolStr,
        cursor: Option<(usize, usize)>,
    },
    Commit(SmolStr),
    Disabled,
}

/// Host-neutral clipboard action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardEvent {
    Cut,
    Copy,
    Paste(SmolStr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowResizeEvent {
    pub size: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasResizeEvent {
    pub size: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowMovedEvent {
    pub position: [i32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowMouseInput {
    pub state: ElementState,
    pub button: MouseButton,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowCursorMoved {
    pub position: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowMouseWheel {
    pub delta: MouseScrollDelta,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowKeyboardInput {
    /// Whether this press repeats a held key.
    pub repeat: bool,
    pub key: Key,
    /// Text produced by this key event, including multi-code-point input.
    ///
    /// `key` remains the shortcut/navigation identity; text insertion must
    /// use this field rather than deriving text from [`Key::Character`].
    pub text: Option<SmolStr>,
    pub state: ElementState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowTouch {
    pub phase: TouchPhase,
    pub position: [f32; 2],
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum Key {
    Named(NamedKey),
    Character(char),
}

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
    Super,

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseScrollDelta {
    LineDelta(f32, f32),
    PixelDelta(f64, f64),
}

/// Event for file system changes
#[derive(Debug, Clone, PartialEq)]
pub struct WindowFileChangedEvent {
    /// Path to the file that changed
    pub file_path: PathBuf,
    /// Error message if the file couldn't be read
    pub error: Option<String>,
}
