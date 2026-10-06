//! Ported from crates/typst-library/src/math/ir/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Intermediate representation for math.
//!
//! avenger: no multiline items, since a label's equation is one inline line.

mod item;
mod process;
mod resolve;
#[cfg(test)]
mod tests;

pub use self::item::*;

use self::resolve::MathResolver;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{Packed, StyleChain};
use crate::typst_library::math::EquationElem;
use crate::typst_library::routines::Arenas;

/// Resolves an equation's body into a [`MathItem`].
///
/// The returned `MathItem` has the same lifetime as the provided arenas.
// avenger: no locator, since labels have no introspection.
pub fn resolve_equation<'a>(
    elem: &'a Packed<EquationElem>,
    engine: &mut Engine,
    arenas: &'a Arenas<'a>,
    styles: StyleChain<'a>,
) -> SourceResult<MathItem<'a>> {
    let mut context = MathResolver::new(engine, arenas);
    context.resolve_into_item(&elem.body, styles)
}
