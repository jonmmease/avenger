//! Ported from crates/typst-library/src/text/space.rs @ v0.15.1, modified for Avenger.

use ecow::EcoString;
use typst_utils::singleton;

use crate::typst_library::foundations::{Content, NativeElement, Repr, elem};

elem! {
/// A text space.
#[elem(name = "space", Unlabellable, PlainText, Repr)]
pub struct SpaceElem {}
}

impl SpaceElem {
    /// Get the globally shared space element.
    pub fn shared() -> &'static Content {
        singleton!(Content, SpaceElem::new().pack())
    }
}

impl Repr for SpaceElem {
    fn repr(&self) -> EcoString {
        "[ ]".into()
    }
}
