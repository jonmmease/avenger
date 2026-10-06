//! Ported from crates/typst-library/src/foundations/float.rs @ v0.15.1, modified for Avenger.
//!
//! Only the type and its representation; the scripting methods are out of scope.

use ecow::EcoString;

use crate::typst_library::foundations::{Repr, repr, ty};

ty!(f64, name = "float", title = "Float", long = "float");

impl Repr for f64 {
    fn repr(&self) -> EcoString {
        repr::format_float(*self, None, true, "")
    }
}
