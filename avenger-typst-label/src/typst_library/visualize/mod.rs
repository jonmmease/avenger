//! Ported from crates/typst-library/src/visualize/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Drawing and visualization.
//!
//! avenger: the paints, strokes and shapes frames carry. Gradients, tilings, images and the
//! drawing elements are out of scope.

mod color;
mod curve;
mod paint;
mod shape;
mod stroke;

pub use self::color::*;
pub use self::curve::*;
pub use self::paint::*;
pub use self::shape::*;
pub use self::stroke::*;
