//! Cursor styles for the canvas.

/// Cursor style requested by the app or a tool and displayed by the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorStyle {
    /// The platform's default cursor, often an arrow.
    #[default]
    Default,
    /// An open hand over draggable content, such as a map.
    Grab,
    /// A closed hand during a drag.
    Grabbing,
    /// An I-beam over selectable text.
    Text,
}
