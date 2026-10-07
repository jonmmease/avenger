//! Ported from crates/typst-library/src/routines.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: no `Routines` table. Upstream links its crates through it; the label pipeline is
//! one crate and calls the routines directly.

use crate::typst_library::foundations::{Content, StyleChain, Styles};

/// Defines what kind of realization we are performing.
// avenger: only paragraph and math realization; a label is one paragraph of inline content.
pub enum RealizationKind {
    /// A nested realization in a paragraph (i.e. a `par`).
    Par,
    /// A realization within math.
    Math,
}

/// Temporary storage arenas for lifetime extension during realization.
///
/// Must be kept live while the content returned from realization is processed.
// avenger: a typed arena for style chains in place of upstream's `bumpalo::Bump`, which
// realization and math resolution use only to lifetime-extend style chains.
#[derive(Default)]
pub struct Arenas<'a> {
    /// A typed arena for owned content.
    pub content: typed_arena::Arena<Content>,
    /// A typed arena for owned styles.
    pub styles: typed_arena::Arena<Styles>,
    /// A typed arena for style chains.
    pub chains: typed_arena::Arena<StyleChain<'a>>,
}

/// A pair of content and a style chain that applies to it.
pub type Pair<'a> = (&'a Content, StyleChain<'a>);
