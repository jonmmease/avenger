//! Ported from crates/typst-library/src/lib.rs @ v0.15.1, modified for Avenger.
//!
//! Typst's standard library: the subset a single label line uses.

pub mod diag;
pub mod engine;
pub mod foundations;
pub mod layout;
pub mod math;
pub mod symbols;
pub mod text;
pub mod visualize;

use crate::typst_library::text::{Font, FontBook};

/// The environment in which typesetting occurs.
///
/// All loading functions (`main`, `source`, `file`, `font`) should perform
/// internal caching so that they are relatively cheap on repeated invocations
/// with the same argument. [`Source`], [`Bytes`], and [`Font`] are
/// all reference-counted and thus cheap to clone.
///
/// The compiler doesn't do the caching itself because the world has much more
/// information on when something can change. For example, fonts typically don't
/// change and can thus even be cached across multiple compilations (for
/// long-running applications like `typst watch`). Source files on the other
/// hand can change and should thus be cleared after each compilation. Advanced
/// clients like language servers can also retain the source files and
/// [edit](Source::edit) them in-place to benefit from better incremental
/// performance.
// avenger: only fonts, since a label has no files, library, packages or dates to load.
pub trait World: Send + Sync {
    /// Metadata about all known fonts.
    fn book(&self) -> &FontBook;

    /// Try to access the font with the given index in the font book.
    ///
    /// Note that the index is not guaranteed to be in bounds of the font book
    /// returned by this world's `book()` function. This is the case because
    /// this function may be invoked with indices from an outdated or different
    /// font book during incremental compilation validation.
    fn font(&self, index: usize) -> Option<Font>;
}

#[cfg(test)]
mod tests;
