//! The pointer's cursor over the canvas.

/// A cursor that the host shows over the canvas, as the app or a tool requests it: one of the
/// CSS `cursor` keywords. Winit and browsers show each natively, with a fallback where a
/// platform lacks one, as Windows does for [`CursorIcon::VerticalText`].
pub use cursor_icon::CursorIcon;
