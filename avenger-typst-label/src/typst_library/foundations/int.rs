//! Ported from crates/typst-library/src/foundations/int.rs @ v0.15.1, modified for Avenger.
//!
//! Only the type and its representation; the scripting methods are out of scope.

use ecow::{EcoString, eco_format};

use crate::typst_library::foundations::{Repr, ty};

ty!(i64, name = "int", title = "Integer", long = "integer");

impl Repr for i64 {
    fn repr(&self) -> EcoString {
        eco_format!("{self:?}")
    }
}
