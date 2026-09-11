//! Retained Typst library concepts.
//!
//! This mirrors the small subset of upstream `typst-library` needed by labels:
//! font resources, text style, math style, colors, strokes, and compact
//! text/math content models.

pub mod font;
pub(crate) mod foundations;
pub mod math;
pub mod symbols;
pub mod text;
pub mod visualize;

pub use font::{FontStyle, FontWeight, MathFontBytesId, MathFontSpec};
pub use math::MathStyle;
pub use text::TextStyle;
pub use visualize::Color;
