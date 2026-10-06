//! Ported from crates/typst-library/src/math/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Mathematical formulas.
//!
//! avenger: a partial port so far; the math library adds the elements.

mod equation;
mod style;

pub use self::equation::*;
pub use self::style::*;

use crate::typst_library::layout::Em;

// Spacings.
pub const THIN: Em = Em::new(1.0 / 6.0);
pub const MEDIUM: Em = Em::new(2.0 / 9.0);
pub const THICK: Em = Em::new(5.0 / 18.0);
pub const QUAD: Em = Em::new(1.0);
pub const WIDE: Em = Em::new(2.0);
