//! Ported from crates/typst-library/src/foundations/float.rs @ v0.15.1, modified for Avenger.
//!
//! The type, its representation and its constants; the constructor and the scripting methods are
//! out of scope.

use ecow::EcoString;

use crate::typst_library::foundations::{Repr, Scope, repr, ty};

ty!(f64, name = "float", title = "Float", long = "float", scope = scope);

/// The constants of upstream's `#[scope(ext)] impl f64`.
fn scope() -> Scope {
    let mut scope = Scope::new();
    // Positive infinity.
    scope.define("inf", f64::INFINITY);
    // A NaN value, as defined by the IEEE 754 standard.
    scope.define("nan", f64::NAN);
    scope
}

impl Repr for f64 {
    fn repr(&self) -> EcoString {
        repr::format_float(*self, None, true, "")
    }
}
