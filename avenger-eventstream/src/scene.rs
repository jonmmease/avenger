use avenger_scenegraph::marks::mark::MarkInstance;

use crate::window::{Key, MouseButton, MouseScrollDelta, WindowMovedEvent, WindowResizeEvent};

/// Events that can be handled by event streams
#[derive(Debug, Clone, PartialEq)]
pub enum SceneGraphEvent {
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
}

impl SceneGraphEvent {
    pub fn position(&self) -> Option<[f32; 2]> {
        match self {
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

    pub fn event_type(&self) -> SceneGraphEventType {
        match self {
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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneGraphEventType {
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneClickEvent {
    pub position: [f32; 2],
    pub button: MouseButton,
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneDoubleClickEvent {
    pub position: [f32; 2],
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseWheelEvent {
    pub position: [f32; 2],
    pub delta: MouseScrollDelta,
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneKeyPressEvent {
    pub position: [f32; 2],
    pub key: Key,
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneKeyReleaseEvent {
    pub position: [f32; 2],
    pub key: Key,
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneCursorMovedEvent {
    pub position: [f32; 2],
    pub mark_instance: Option<MarkInstance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseEnterEvent {
    pub position: [f32; 2],
    pub mark_instance: MarkInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneMouseLeaveEvent {
    pub position: [f32; 2],
    pub mark_instance: MarkInstance,
}
