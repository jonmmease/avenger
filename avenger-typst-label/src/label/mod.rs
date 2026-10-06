//! The label typesetter: Avenger's engine around the ported Typst pipeline.

mod world;

#[allow(unused_imports, reason = "the label engine typesets in this world")]
pub use self::world::LabelWorld;

#[cfg(test)]
pub(crate) use self::world::fixtures;
