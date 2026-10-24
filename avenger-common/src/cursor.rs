//! The pointer's cursor over the canvas.

/// The cursor that the host shows over the canvas, as the app and its tools call for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorStyle {
    /// The platform's default cursor, often an arrow.
    #[default]
    Default,
    /// An open hand, over something that a drag moves, such as a map.
    Grab,
    /// A closed hand, while a drag moves something.
    Grabbing,
    /// An I-beam, over text that a drag selects.
    Text,
}
