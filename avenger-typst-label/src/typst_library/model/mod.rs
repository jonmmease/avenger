//! Ported from crates/typst-library/src/model/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Structuring elements that define the document model.
//!
//! avenger: only the emphasis elements and the paragraph properties inline layout reads;
//! labels have no document structure.

mod emph;
mod par;
mod strong;

pub use self::emph::*;
pub use self::par::*;
pub use self::strong::*;

use crate::typst_library::foundations::Scope;

/// Hook up all `model` definitions.
// avenger: strong and emphasized text; labels have no document model.
pub(super) fn define(global: &mut Scope) {
    global.define_elem::<StrongElem>();
    global.define_elem::<EmphElem>();
}
