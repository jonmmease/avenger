mod editor;
mod shaped_line;

pub use editor::{Action, Cursor, Motion, SelectionState, SingleLineEditor, normalize_single_line};
pub use shaped_line::{Affinity, ShapedLine, cursor_rect_for_offset, selection_rects, shape_line};
