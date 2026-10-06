//! Ported from crates/typst-library/src/foundations/bool.rs @ v0.15.1, modified for Avenger.

use ecow::EcoString;

use crate::typst_library::foundations::{Repr, ty};

// avenger: upstream's `#[ty(cast, title = "Boolean")]` on its `bool` docs item.
ty!(bool, name = "bool", title = "Boolean", long = "boolean");

impl Repr for bool {
    fn repr(&self) -> EcoString {
        match self {
            true => "true".into(),
            false => "false".into(),
        }
    }
}
