use std::path::PathBuf;

use avenger_scenegraph::marks::mark::MarkInstance;

use crate::window::{Key, MouseButton, MouseScrollDelta, WindowMovedEvent, WindowResizeEvent};

/// Events that can be handled by event streams
#[derive(Debug, Clone, PartialEq)]
pub enum SceneGraphEvent {
    MouseDown(SceneMouseDownEvent),
    MouseUp(SceneMouseUpEvent),
    /// A press that ends without its release, as when the window loses focus before it.
    MouseCancel(SceneMouseCancelEvent),
    Click(SceneClickEvent),
    DoubleClick(SceneDoubleClickEvent),
    MouseWheel(SceneMouseWheelEvent),
    KeyPress(SceneKeyPressEvent),
    KeyRelease(SceneKeyReleaseEvent),
    CursorMoved(SceneCursorMovedEvent),
    MouseEnter(SceneMouseEnterEvent),
    MouseLeave(SceneMouseLeaveEvent),
    WindowResize(WindowResizeEvent),
    WindowMoved(WindowMovedEvent),
    WindowFocused(bool),
    WindowCloseRequested,
    FileChanged(SceneFileChangedEvent),
    /// The pointer left the window.
    CursorLeft,
}

impl SceneGraphEvent {
    pub fn position(&self) -> Option<[f32; 2]> {
        match self {
            Self::MouseDown(event) => Some(event.position),
            Self::MouseUp(event) => Some(event.position),
            Self::MouseCancel(event) => Some(event.position),
            Self::Click(event) => Some(event.position),
            Self::DoubleClick(event) => Some(event.position),
            Self::MouseWheel(event) => Some(event.position),
            Self::KeyPress(event) => Some(event.position),
            Self::KeyRelease(event) => Some(event.position),
            Self::CursorMoved(event) => Some(event.position),
            Self::MouseEnter(event) => Some(event.position),
            Self::MouseLeave(event) => Some(event.position),
            _ => None,
        }
    }

    pub fn mark_instance(&self) -> Option<&MarkInstance> {
        match self {
            Self::MouseDown(event) => event.mark_instance.as_ref(),
            Self::MouseUp(event) => event.mark_instance.as_ref(),
            Self::MouseCancel(event) => event.mark_instance.as_ref(),
            Self::Click(event) => event.mark_instance.as_ref(),
            Self::DoubleClick(event) => event.mark_instance.as_ref(),
            Self::MouseWheel(event) => event.mark_instance.as_ref(),
            Self::KeyPress(event) => event.mark_instance.as_ref(),
            Self::KeyRelease(event) => event.mark_instance.as_ref(),
            Self::CursorMoved(event) => event.mark_instance.as_ref(),
            Self::MouseEnter(event) => Some(&event.mark_instance),
            Self::MouseLeave(event) => Some(&event.mark_instance),
            _ => None,
        }
    }

    /// The event, with `mark_instance` as the mark under its position, for an event that carries
    /// one.
    pub(crate) fn with_mark(&self, mark_instance: Option<MarkInstance>) -> Self {
        let mut event = self.clone();
        match &mut event {
            Self::MouseDown(event) => event.mark_instance = mark_instance,
            Self::MouseUp(event) => event.mark_instance = mark_instance,
            Self::MouseCancel(event) => event.mark_instance = mark_instance,
            Self::Click(event) => event.mark_instance = mark_instance,
            Self::DoubleClick(event) => event.mark_instance = mark_instance,
            Self::MouseWheel(event) => event.mark_instance = mark_instance,
            Self::KeyPress(event) => event.mark_instance = mark_instance,
            Self::KeyRelease(event) => event.mark_instance = mark_instance,
            Self::CursorMoved(event) => event.mark_instance = mark_instance,
            _ => {}
        }
        event
    }

    /// Whether this event routes as a pointer event: an interactive hit stops it, and during a
    /// press it goes to the press's owner and the scenes above it.
    pub(crate) fn is_pointer(&self) -> bool {
        matches!(
            self,
            Self::MouseDown(_)
                | Self::MouseUp(_)
                | Self::MouseCancel(_)
                | Self::Click(_)
                | Self::DoubleClick(_)
                | Self::MouseWheel(_)
                | Self::CursorMoved(_)
        )
    }

    pub fn event_type(&self) -> SceneGraphEventType {
        match self {
            Self::MouseDown(..) => SceneGraphEventType::MouseDown,
            Self::MouseUp(..) => SceneGraphEventType::MouseUp,
            Self::MouseCancel(..) => SceneGraphEventType::MouseCancel,
            Self::Click(..) => SceneGraphEventType::Click,
            Self::DoubleClick(..) => SceneGraphEventType::DoubleClick,
            Self::MouseWheel(..) => SceneGraphEventType::MouseWheel,
            Self::KeyPress(..) => SceneGraphEventType::KeyPress,
            Self::KeyRelease(..) => SceneGraphEventType::KeyRelease,
            Self::CursorMoved(..) => SceneGraphEventType::CursorMoved,
            Self::MouseEnter(..) => SceneGraphEventType::MarkMouseEnter,
            Self::MouseLeave(..) => SceneGraphEventType::MarkMouseLeave,
            Self::WindowResize(..) => SceneGraphEventType::WindowResize,
            Self::WindowMoved(..) => SceneGraphEventType::WindowMoved,
            Self::WindowFocused(..) => SceneGraphEventType::WindowFocused,
            Self::WindowCloseRequested => SceneGraphEventType::WindowCloseRequested,
            Self::FileChanged(SceneFileChangedEvent { file_path, .. }) => {
                SceneGraphEventType::FileChanged(file_path.clone())
            }
            Self::CursorLeft => SceneGraphEventType::CursorLeft,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneGraphEventType {
    MouseDown,
    MouseUp,
    MouseCancel,
    Click,
    DoubleClick,
    MouseWheel,
    KeyPress,
    KeyRelease,
    CursorMoved,
    MarkMouseEnter,
    MarkMouseLeave,
    WindowResize,
    WindowMoved,
    WindowFocused,
    WindowCloseRequested,
    FileChanged(PathBuf),
    CursorLeft,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseDownEvent {
    pub position: [f32; 2],
    pub button: MouseButton,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseUpEvent {
    pub position: [f32; 2],
    pub button: MouseButton,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseCancelEvent {
    pub position: [f32; 2],
    pub button: MouseButton,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneClickEvent {
    pub position: [f32; 2],
    pub button: MouseButton,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneDoubleClickEvent {
    pub position: [f32; 2],
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseWheelEvent {
    pub position: [f32; 2],
    pub delta: MouseScrollDelta,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneKeyPressEvent {
    pub position: [f32; 2],
    pub key: Key,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneKeyReleaseEvent {
    pub position: [f32; 2],
    pub key: Key,
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneCursorMovedEvent {
    pub position: [f32; 2],
    pub mark_instance: Option<MarkInstance>,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseEnterEvent {
    pub position: [f32; 2],
    pub mark_instance: MarkInstance,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseLeaveEvent {
    pub position: [f32; 2],
    pub mark_instance: MarkInstance,
    pub modifiers: ModifiersState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModifiersState {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Event triggered when a monitored file changes
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFileChangedEvent {
    /// The path of the file that changed
    pub file_path: PathBuf,
    /// Error message if file couldn't be read
    pub error: Option<String>,
}
