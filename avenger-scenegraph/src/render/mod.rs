//! How renderers draw a scene: the order of its items, the filled and stroked paths that draw its
//! marks and labels, and the paint, strokes, fills, and patterns of those paths. The scene graph's
//! README lists the rules that every renderer follows.

pub mod fill;
pub mod items;
pub mod order;
pub mod paint;
pub mod pattern;
pub mod stroke;
pub mod text;
