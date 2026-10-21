//! The pointer's cursor over the canvas.

/// The cursor that the host shows over the canvas, as the app's state calls for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorStyle {
    /// The platform's default cursor, often an arrow.
    #[default]
    Default,
    /// A hand, over something that a click acts on, such as a legend entry.
    Pointer,
    /// Over a place to pick precisely, such as a brush's corner.
    Crosshair,
    /// Over something that a drag moves, such as a map.
    Grab,
    /// While a drag moves something.
    Grabbing,
    /// Over something that a drag moves in any direction, such as a brush.
    Move,
    /// Over text that a click edits.
    Text,
    /// Over an edge that a drag moves left or right.
    ResizeHorizontal,
    /// Over an edge that a drag moves up or down.
    ResizeVertical,
    /// Over a top left or bottom right corner that a drag moves.
    ResizeNwSe,
    /// Over a top right or bottom left corner that a drag moves.
    ResizeNeSw,
}
