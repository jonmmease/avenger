mod editor;
pub(crate) mod shaped_line;

pub use editor::{
    normalize_single_line, Action, CommittedText, Cursor, Granularity, Motion, SelectionState,
    SingleLineEditor,
};
pub use shaped_line::{
    byte_offset_for_x, cursor_rect_for_offset, next_grapheme, next_word_boundary, prev_grapheme,
    prev_word_boundary, selection_rects, word_range_at, Affinity, ShapedGlyph, ShapedLine,
    ShapedRun, TextRect,
};
