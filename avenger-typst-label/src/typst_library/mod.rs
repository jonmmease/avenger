//! Retained Typst library concepts.
//!
//! This mirrors the small subset of upstream `typst-library` needed by labels:
//! font resources, text style, math style, colors, strokes, and compact
//! text/math content models.

pub mod font;
pub mod math;
pub mod text;
pub mod visualize;

pub use font::{FontStyle, FontWeight, MathFontBytesId, MathFontSpec};
pub use math::{MathDisplayStyle, MathStyle};
pub use text::PlainTextStyle;
pub use visualize::Color;
