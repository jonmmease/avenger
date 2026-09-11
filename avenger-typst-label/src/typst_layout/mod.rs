//! Single-line frame and layout logic.
//!
//! This mirrors the parts of upstream `typst-layout` needed for label-sized
//! inline text and math: frame items, font fallback/shaping, glyph paths, line
//! metrics, and compact math layout. It does not include page, paragraph,
//! block, table, or document layout.

pub(crate) mod frame;
pub(crate) mod glyph_path;
pub(crate) mod inline;
pub(crate) mod line;
pub(crate) mod math;
