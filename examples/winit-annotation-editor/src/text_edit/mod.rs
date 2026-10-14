mod editor;
mod shaped_line;

pub use editor::{Action, Cursor, Motion, SelectionState, SingleLineEditor};
pub use shaped_line::{
    cursor_rect_for_offset, selection_rects, shape_line, Affinity, ShapedLine, TextRect,
};
