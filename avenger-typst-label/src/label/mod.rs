//! The label typesetter: Avenger's engine around the ported Typst pipeline.

mod format;
#[cfg(test)]
pub(crate) mod oracle;
mod world;

pub(crate) use self::format::{FormattingCache, define};

#[allow(unused_imports, reason = "the label engine typesets in this world")]
pub use self::world::LabelWorld;

#[cfg(test)]
pub(crate) use self::world::fixtures;
